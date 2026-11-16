"""Execute examples/file_facts.sql with the real pinned DuckDB 2.0.0 extension.

SPUR_GRAPH_EXTENSION=/path/to/spur_graph.duckdb_extension python -m unittest \
    discover -s crates/spur-graph-duckdb/tests -p test_file_facts_pipeline.py -v
"""

from contextlib import chdir
import os
from pathlib import Path
import tempfile
import unittest

import duckdb


SQL_PATH = Path(__file__).resolve().parents[1] / "examples" / "file_facts.sql"
KEY = "blob_oid, logical_path, extractor_sha"
CHILDREN = {
    "nodes": "node",
    "edges": "edge",
    "spans": "span",
    "macro_invocations": "invocation",
    "macro_scopes": "macro_scope",
}
RUST = '''macro_rules! local { () => { helper() }; }
fn helper() {}
#[cfg(test)]
mod tests {
    #[test]
    fn entry() { local!(); helper(); remote_call(); }
}
'''


class FileFactsPipelineTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        self.assertTrue(extension, "set SPUR_GRAPH_EXTENSION to the packaged native extension")
        self.output = tempfile.TemporaryDirectory()
        self.addCleanup(self.output.cleanup)
        self.database = str(Path(self.output.name) / "facts.duckdb")
        self.con = duckdb.connect(self.database, config={"allow_unsigned_extensions": True})
        self.addCleanup(self.con.close)
        quoted = str(Path(extension).resolve()).replace("'", "''")
        self.con.execute(f"LOAD '{quoted}'")
        self.con.execute("SET threads = 2")
        self.con.execute("""
            CREATE TABLE staged_sources (
                blob_oid VARCHAR, logical_path VARCHAR, source VARCHAR, extractor_sha VARCHAR
            );
            CREATE TABLE source_lineage (
                occurrence_id VARCHAR, repository VARCHAR, commit_oid VARCHAR,
                parent_oid VARCHAR, blob_oid VARCHAR, logical_path VARCHAR, extractor_sha VARCHAR
            );
        """)

    def stage(self, rows):
        self.con.executemany("INSERT INTO staged_sources VALUES (?, ?, ?, ?)", rows)

    def run_pipeline(self):
        # COPY destinations in the example are relative to the caller's output directory.
        with chdir(self.output.name):
            self.con.execute(SQL_PATH.read_text())

    def rows(self, sql):
        return self.con.execute(sql).fetchall()

    def scalar(self, sql):
        return self.rows(sql)[0][0]

    def cache_keys(self):
        return self.rows(f"SELECT {KEY} FROM file_facts_cache ORDER BY ALL")

    def child_snapshot(self, where="true"):
        return {collection: self.rows(
            f"SELECT * FROM file_facts_{collection} WHERE {where} ORDER BY ALL"
        ) for collection in CHILDREN}

    def child_table_oids(self):
        names = ", ".join(f"'file_facts_{collection}'" for collection in CHILDREN)
        result = self.rows(f"""
            SELECT table_name, table_oid FROM duckdb_tables()
            WHERE database_name = current_database() AND schema_name = current_schema()
              AND NOT temporary AND table_name IN ({names}) ORDER BY table_name
        """)
        self.assertEqual(len(result), len(CHILDREN))
        return result

    def assert_full_children(self):
        # This oracle expands the complete cache, with no incremental state/delta logic.
        # EXCEPT ALL in both directions catches missing, extra and duplicate native rows.
        for collection, child in CHILDREN.items():
            reference = f"""
                SELECT {KEY}, fact_ordinal, {child}.* FROM file_facts_cache,
                UNNEST(facts.{collection}) WITH ORDINALITY AS u({child}, fact_ordinal)
            """
            actual = f"SELECT * FROM file_facts_{collection}"
            with self.subTest(collection=collection, check="types"):
                self.assertEqual(
                    [(c[0], c[1]) for c in self.rows(f"DESCRIBE {actual}")],
                    [(c[0], c[1]) for c in self.rows(f"DESCRIBE {reference}")],
                )
            for left, right in ((reference, actual), (actual, reference)):
                with self.subTest(collection=collection, check="multiset", left=left):
                    self.assertEqual(self.rows(f"({left}) EXCEPT ALL ({right})"), [])
            with self.subTest(collection=collection, check="key_and_ordinal"):
                self.assertEqual(self.rows(f"""
                    SELECT {KEY}, fact_ordinal FROM file_facts_{collection}
                    GROUP BY {KEY}, fact_ordinal HAVING count(*) > 1
                """), [])

    def assert_key_relation(self, table, expected, temporary):
        # Assert absence as a contract failure before binding a missing relation.
        # TEMP relations live in temp.main regardless of the active persistent schema.
        location = ("database_name = 'temp' AND schema_name = 'main'" if temporary else
                    "database_name = current_database() AND schema_name = current_schema()")
        metadata = self.rows(f"""
            SELECT temporary, column_count FROM duckdb_tables()
            WHERE table_name = '{table}' AND {location}
        """)
        self.assertEqual(metadata, [(temporary, 3)],
                         f"{table} must exist as a key-only {'TEMP' if temporary else 'durable'} table")
        self.assertEqual([(c[0], c[1]) for c in self.rows(f"DESCRIBE {table}")],
                         [(column, "VARCHAR") for column in KEY.split(", ")])
        self.assertCountEqual(self.rows(f"SELECT * FROM {table}"), expected)

    def assert_child_state(self):
        self.assert_key_relation("file_facts_child_state", self.cache_keys(), temporary=False)

    def assert_child_deltas(self, refresh=(), removed=()):
        for suffix, expected in (("refresh", refresh), ("removed", removed)):
            with self.subTest(delta=suffix):
                self.assert_key_relation(f"file_facts_child_{suffix}", expected, temporary=True)

    def seed_native_children(self):
        # Every full-key component matters; local IDs overlap across these parents.
        self.stage([
            ("blob", "src/one.rs", RUST, "v1"),
            ("blob", "src/two.rs", RUST, "v1"),
            ("blob", "src/one.rs", RUST, "v2"),
        ])
        self.run_pipeline()
        self.assert_full_children()
        for collection in CHILDREN:
            self.assertGreater(self.scalar(f"SELECT count(*) FROM file_facts_{collection}"), 0)

    def reopen(self):
        self.con.close()
        self.con = duckdb.connect(self.database, config={"allow_unsigned_extensions": True})
        self.addCleanup(self.con.close)
        extension = str(Path(os.environ["SPUR_GRAPH_EXTENSION"]).resolve()).replace("'", "''")
        self.con.execute(f"LOAD '{extension}'")
        self.con.execute("SET threads = 2")

    def test_path_dependent_keys_and_duplicate_lineage(self):
        self.stage([
            ("same-blob", "src/one.rs", RUST, "v1"),
            ("same-blob", "src/two.rs", RUST, "v1"),
            ("same-blob", "src/one.rs", RUST, "v1"),
        ])
        lineage = [
            ("root", "repo", "c0", None, "same-blob", "src/one.rs", "v1"),
            ("merge-a", "repo", "c2", "c0", "same-blob", "src/one.rs", "v1"),
            ("merge-b", "repo", "c2", "c1", "same-blob", "src/one.rs", "v1"),
            ("merge-b", "repo", "c2", "c1", "same-blob", "src/one.rs", "v1"),
            ("other-path", "repo", "c2", "c1", "same-blob", "src/two.rs", "v1"),
        ]
        self.con.executemany("INSERT INTO source_lineage VALUES (?, ?, ?, ?, ?, ?, ?)", lineage)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 2)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 2)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_cache"), 2)
        self.assertCountEqual(self.rows("SELECT * EXCLUDE(extraction_status) FROM file_facts_occurrences"), lineage)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_occurrences WHERE extraction_status = 'ok'"), 5)
        facts = self.rows("SELECT logical_path, facts FROM file_facts_cache ORDER BY logical_path")
        self.assertNotEqual(facts[0][1], facts[1][1])
        for path, result in facts:
            self.assertEqual(result, self.con.execute(
                "SELECT spur_graph_file_facts(?, ?)", [path, RUST]).fetchone()[0])
        # IDs restart per file. Keyed joins resolve each edge to exactly one source node.
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_nodes WHERE node_id = 1"), 2)
        self.assertEqual(self.scalar(f"""
            SELECT count(*) FROM file_facts_edges e
            JOIN file_facts_nodes n USING ({KEY})
            WHERE e.source_node_id = n.node_id
        """), self.scalar("SELECT count(*) FROM file_facts_edges"))
        self.assertGreater(self.scalar("""
            SELECT count(*) FROM file_facts_edges e
            JOIN file_facts_nodes n ON e.source_node_id = n.node_id
        """), self.scalar("SELECT count(*) FROM file_facts_edges"))
        # Joining one file's facts back to occurrences preserves both merge parents and repeats.
        self.assertEqual(self.scalar(f"""
            SELECT count(*) FROM file_facts_occurrences l
            JOIN file_facts_nodes n USING ({KEY}) WHERE n.node_id = 1
        """), len(lineage))

    def test_restart_skips_cached_keys_and_version_change_adds_keys(self):
        self.stage([("blob", "src/lib.rs", RUST, "v1")])
        self.run_pipeline()
        original = self.rows("SELECT * FROM file_facts_cache")
        self.con.close()
        # Reopen the durable cache; TEMP missing/extracted tables cannot survive restart.
        self.con = duckdb.connect(self.database, config={"allow_unsigned_extensions": True})
        self.addCleanup(self.con.close)
        extension = str(Path(os.environ["SPUR_GRAPH_EXTENSION"]).resolve()).replace("'", "''")
        self.con.execute(f"LOAD '{extension}'")
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.rows("SELECT * FROM file_facts_cache"), original)
        self.con.execute("UPDATE staged_sources SET extractor_sha = 'v2'")
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 1)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 1)
        self.assertEqual(self.rows("SELECT extractor_sha FROM file_facts_cache ORDER BY 1"), [("v1",), ("v2",)])
        self.assertEqual(self.rows("SELECT * FROM file_facts_cache WHERE extractor_sha = 'v1'"), original)

    def test_parent_statuses_survive_null_error_and_empty_lists(self):
        self.stage([
            ("empty", "empty.rs", "", "v1"),
            ("null", "null.rs", None, "v1"),
            ("unsupported", "bad.unknown", RUST, "v1"),
            ("invalid", "../escape.rs", RUST, "v1"),
            ("good", "good.rs", RUST, "v1"),
        ])
        self.run_pipeline()
        self.assertEqual(dict(self.rows("SELECT blob_oid, status FROM file_facts_cache")), {
            "empty": "empty_source", "null": "null_input", "unsupported": "extraction_error",
            "invalid": "extraction_error", "good": "ok",
        })
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_cache WHERE facts IS NULL"), 3)
        self.assertEqual(self.scalar("SELECT facts IS NOT NULL FROM file_facts_cache WHERE blob_oid = 'empty'"), True)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_edges WHERE blob_oid = 'empty'"), 0)
        for child in CHILDREN:
            self.assertEqual(self.scalar(f"SELECT count(*) FROM file_facts_{child} WHERE blob_oid IN ('null', 'unsupported', 'invalid')"), 0)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_cache"), 5)

    def test_diagnostics_persist_exact_native_details_and_replay_only_missing(self):
        cases = [("bad.unknown", RUST), ("../bad.rs", RUST),
                 ("bad.ipynb", "not JSON"), ("good.rs", RUST),
                 ("empty.rs", ""), ("null.rs", None)]
        expected = {path: self.con.execute(
            "SELECT spur_graph_file_facts_result(?::VARCHAR, ?::VARCHAR)",
            [path, source]).fetchone()[0] for path, source in cases}
        self.stage([(path, path, source, "v1") for path, source in cases])
        self.run_pipeline()
        columns = [c[0] for c in self.rows("DESCRIBE file_facts_cache")]
        self.assertIn("error_code", columns, "cache must persist native error diagnostics")
        self.assertIn("error_message", columns)
        for path, result in expected.items():
            with self.subTest(path=path):
                self.assertEqual(self.rows("SELECT status, error_code, error_message, facts "
                    "FROM file_facts_cache WHERE logical_path = '" + path + "'"),
                    [(result["status"], result["error_code"], result["error_message"], result["facts"])])
        self.assertEqual(self.scalar("SELECT error_code FROM file_facts_cache "
                                     "WHERE logical_path='bad.unknown'"), "unsupported_language")
        original = self.rows("SELECT * FROM file_facts_cache ORDER BY ALL")
        # Conflicting bytes on a single staged cache HIT must not silently reparse it.
        self.con.execute("UPDATE staged_sources SET source = 'now different'")
        self.reopen()
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.rows("SELECT * FROM file_facts_cache ORDER BY ALL"), original)
        # Retry only through explicit full-key deletion; error parents checkpoint too.
        self.con.execute("DELETE FROM file_facts_cache WHERE logical_path='bad.unknown'")
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 1)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assert_child_state()
        self.assert_full_children()

    def test_prior_cache_additive_migration_keeps_unknown_details_null(self):
        self.use_facts_schema()
        self.seed_native_children()
        children, oids = self.child_snapshot(), self.child_table_oids()
        # Reconstruct the actual prior five-column layout, including an old error.
        self.con.execute("DROP INDEX file_facts_cache_key")
        self.con.execute("CREATE OR REPLACE TABLE file_facts_cache AS SELECT "
                         "blob_oid, logical_path, extractor_sha, facts, status FROM file_facts_cache")
        self.con.execute("INSERT INTO file_facts_cache SELECT 'old', 'bad.unknown', 'v1', "
                         "NULL, 'extraction_error'")
        old = self.rows("SELECT * FROM file_facts_cache ORDER BY ALL")
        self.stage([("old", "bad.unknown", RUST, "v1")])
        self.reopen()
        self.con.execute("SET schema='facts'")
        self.run_pipeline()
        columns = [c[0] for c in self.rows("DESCRIBE file_facts_cache")]
        self.assertIn("error_code", columns, "prior cache requires explicit additive migration")
        self.assertEqual(self.rows("SELECT blob_oid, logical_path, extractor_sha, facts, status "
                                  "FROM file_facts_cache ORDER BY ALL"), old)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_cache "
                                     "WHERE error_code IS NOT NULL OR error_message IS NOT NULL"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.child_snapshot(), children)
        self.assertEqual(self.child_table_oids(), oids)
        self.assert_child_state()
        self.assert_full_children()
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_child_refresh"), 0)

    def test_separate_unnests_and_complete_typed_parquet_roundtrip(self):
        self.stage([("blob", "src/lib.rs", RUST, "v1"), ("null", "null.py", None, "v1")])
        self.con.execute("""
            INSERT INTO source_lineage VALUES
            ('unavailable', 'repo', 'c1', NULL, 'unstaged', 'missing.rs', 'v1')
        """)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT extraction_status FROM file_facts_occurrences"), "not_cached")
        for collection, child in CHILDREN.items():
            with self.subTest(collection=collection):
                self.assertGreater(self.scalar(f"SELECT sum(len(facts.{collection})) FROM file_facts_cache"), 0)
                self.assertEqual(self.scalar(f"SELECT count(*) FROM file_facts_{collection}"),
                                 self.scalar(f"SELECT sum(len(facts.{collection})) FROM file_facts_cache"))
                self.assertEqual(self.scalar(f"""
                    SELECT count(*) FROM (
                        (SELECT {KEY}, fact_ordinal, {child}.* FROM file_facts_cache,
                         UNNEST(facts.{collection}) WITH ORDINALITY AS u({child}, fact_ordinal))
                        EXCEPT ALL (SELECT * FROM file_facts_{collection})
                    )
                """), 0)
        # A fresh connection can read every primitive/list/struct field without the extension.
        with duckdb.connect() as reader:
            for suffix in ("cache", "occurrences", *CHILDREN):
                with self.subTest(parquet=suffix):
                    table = f"file_facts_{suffix}"
                    path = str(Path(self.output.name) / f"file-facts-{suffix}.parquet").replace("'", "''")
                    query = f"SELECT * FROM read_parquet('{path}')"
                    expected = self.rows(f"SELECT * FROM {table} ORDER BY ALL")
                    self.assertEqual(reader.execute(query + " ORDER BY ALL").fetchall(), expected)
                    self.assertEqual([(c[0], c[1]) for c in self.con.execute(f"DESCRIBE {table}").fetchall()],
                                     [(c[0], c[1]) for c in reader.execute(f"DESCRIBE {query}").fetchall()])

    def test_empty_staging_is_typed_and_restartable(self):
        self.run_pipeline()
        self.run_pipeline()
        for suffix in ("cache", "occurrences", *CHILDREN):
            self.assertEqual(self.scalar(f"SELECT count(*) FROM file_facts_{suffix}"), 0)

    def test_rejects_ambiguous_sources_and_null_keys(self):
        for rows in ([('blob', 'lib.rs', RUST, 'v1'), ('blob', 'lib.rs', '', 'v1')],
                     [('blob', 'lib.rs', None, 'v1'), ('blob', 'lib.rs', '', 'v1')],
                     [('blob', None, RUST, 'v1')], [(None, 'lib.rs', RUST, 'v1')],
                     [('blob', 'lib.rs', RUST, None)]):
            with self.subTest(rows=rows):
                self.stage(rows)
                with self.assertRaisesRegex(duckdb.InvalidInputException, "staged_sources"):
                    self.run_pipeline()
                self.con.execute("ROLLBACK")
                self.con.execute("DELETE FROM staged_sources")

    def test_complete_cache_oracle_covers_every_native_collection(self):
        self.seed_native_children()
        self.stage([("null", "null.rs", None, "v1"), ("empty", "empty.rs", "", "v1")])
        self.run_pipeline()
        self.assert_full_children()

    def test_noop_reuses_child_tables_and_has_zero_deltas(self):
        self.seed_native_children()
        before, oids = self.child_snapshot(), self.child_table_oids()
        self.run_pipeline()
        self.assert_full_children()
        self.assertEqual(self.child_snapshot(), before)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        with self.subTest(check="physical_table_reuse"):
            self.assertEqual(self.child_table_oids(), oids)
        self.assert_child_deltas()
        self.assert_child_state()

    def use_facts_schema(self):
        self.con.execute("""
            CREATE SCHEMA facts;
            SET schema = 'facts';
            CREATE TABLE staged_sources AS SELECT * FROM main.staged_sources WHERE false;
            CREATE TABLE source_lineage AS SELECT * FROM main.source_lineage WHERE false;
        """)

    def test_custom_schema_noop_reuses_child_tables_and_has_zero_deltas(self):
        self.use_facts_schema()
        self.seed_native_children()
        before, oids = self.child_snapshot(), self.child_table_oids()
        self.run_pipeline()
        self.assert_full_children()
        self.assertEqual(self.child_snapshot(), before)
        self.assertEqual(self.child_table_oids(), oids)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_child_refresh_facts"), 0)
        self.assert_child_deltas()
        self.assert_child_state()

    def test_custom_schema_missing_child_repairs_with_intact_main_layout(self):
        self.seed_native_children()
        main_oids = self.child_table_oids()
        main_tables = ["staged_sources", "source_lineage", *(
            f"file_facts_{suffix}" for suffix in ("cache", "child_state", "occurrences", *CHILDREN)
        )]
        main_rows = {table: self.rows(f"SELECT * FROM main.{table} ORDER BY ALL")
                     for table in main_tables}
        self.use_facts_schema()
        self.stage([("facts-blob", "facts.rs", RUST, "v1")])
        self.run_pipeline()
        self.assert_full_children()
        before, keys = self.child_snapshot(), self.cache_keys()
        oids = self.child_table_oids()
        # Both layouts exist on this connection; OIDs must identify facts, not main.
        self.assertTrue(set(oids).isdisjoint(main_oids))
        self.con.execute("DELETE FROM staged_sources")
        for collection in CHILDREN:
            with self.subTest(missing_collection=collection):
                self.con.execute(f"DROP TABLE file_facts_{collection}")
                with self.assertRaises(AssertionError):
                    self.child_table_oids()
                self.run_pipeline()
                self.assert_full_children()
                self.assertEqual(self.child_snapshot(), before)
                self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
                self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
                self.assert_child_deltas(refresh=keys)
                self.assert_child_state()
                self.assertEqual(
                    {table: self.rows(f"SELECT * FROM main.{table} ORDER BY ALL")
                     for table in main_tables}, main_rows)
                self.assertEqual(self.rows("""
                    SELECT table_name, table_oid FROM duckdb_tables()
                    WHERE database_name = current_database() AND schema_name = 'main'
                      AND NOT temporary AND starts_with(table_name, 'file_facts_')
                      AND table_name NOT IN ('file_facts_cache', 'file_facts_child_state',
                                             'file_facts_occurrences') ORDER BY table_name
                """), main_oids)
                repaired_oids = self.child_table_oids()
                self.run_pipeline()
                self.assertEqual(self.child_table_oids(), repaired_oids)
                self.assert_full_children()
                self.assert_child_deltas()
                self.assert_child_state()

    def test_reopen_preserves_checkpoint_and_child_content(self):
        self.seed_native_children()
        before = self.child_snapshot()
        self.reopen()
        # The checkpoint itself must survive, before the next run can recreate anything.
        with self.subTest(check="durable_checkpoint"):
            self.assert_child_state()
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.child_snapshot(), before)
        self.assert_full_children()
        self.assert_child_deltas()
        self.assert_child_state()
        # Internal OIDs are deliberately never compared across connections.

    def test_lineage_refreshes_independently_of_child_deltas(self):
        self.seed_native_children()
        before = self.child_snapshot()
        self.con.execute("""
            INSERT INTO source_lineage VALUES
            ('old', 'repo', 'c0', NULL, 'blob', 'src/one.rs', 'v1')
        """)
        self.run_pipeline()
        self.con.execute("DELETE FROM source_lineage")
        lineage = [
            ("root", "repo", "c0", None, "blob", "src/one.rs", "v1"),
            ("merge-a", "repo", "c2", "c0", "blob", "src/one.rs", "v1"),
            ("merge-b", "repo", "c2", "c1", "blob", "src/one.rs", "v1"),
            ("merge-b", "repo", "c2", "c1", "blob", "src/one.rs", "v1"),
            ("absent", "repo", "c2", "c1", "absent", "missing.rs", "v1"),
        ]
        self.con.executemany("INSERT INTO source_lineage VALUES (?, ?, ?, ?, ?, ?, ?)", lineage)
        self.run_pipeline()
        self.assertCountEqual(self.rows("SELECT * FROM file_facts_occurrences"),
                              [row + ("not_cached" if row[0] == "absent" else "ok",) for row in lineage])
        self.assertEqual(self.child_snapshot(), before)
        self.assert_full_children()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assert_child_deltas()
        self.assert_child_state()

    def test_append_path_and_extractor_version_retain_existing_children(self):
        self.stage([("blob", "src/one.rs", RUST, "v1")])
        self.run_pipeline()
        original_key = "blob_oid = 'blob' AND logical_path = 'src/one.rs' AND extractor_sha = 'v1'"
        before, oids = self.child_snapshot(original_key), self.child_table_oids()
        additions = [
            ("blob", "src/two.rs", RUST, "v1"),
            ("blob", "src/one.rs", RUST, "v2"),
            ("new-blob", "src/one.rs", RUST, "v1"),
        ]
        self.stage(additions)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), len(additions))
        self.assertEqual(self.child_snapshot(original_key), before)
        self.assert_full_children()
        with self.subTest(check="physical_table_reuse"):
            self.assertEqual(self.child_table_oids(), oids)
        self.assert_child_deltas(refresh=[(blob, path, version) for blob, path, _, version in additions])
        self.assert_child_state()

    def test_cache_deletion_without_restaging_removes_children_state_and_updates_lineage(self):
        self.seed_native_children()
        self.con.execute("""
            INSERT INTO source_lineage VALUES
            ('deleted', 'repo', 'c0', NULL, 'blob', 'src/one.rs', 'v1'),
            ('retained', 'repo', 'c0', NULL, 'blob', 'src/one.rs', 'v2')
        """)
        self.con.execute("DELETE FROM staged_sources")
        self.con.execute("""
            DELETE FROM file_facts_cache
            WHERE blob_oid = 'blob' AND logical_path = 'src/one.rs' AND extractor_sha = 'v1'
        """)
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_cache"), 2)
        self.assertEqual(dict(self.rows("SELECT occurrence_id, extraction_status FROM file_facts_occurrences")),
                         {"deleted": "not_cached", "retained": "ok"})
        self.assert_full_children()
        self.assert_child_deltas(removed=[("blob", "src/one.rs", "v1")])
        self.assert_child_state()

    def test_deleted_and_retried_key_replaces_prior_children_without_duplicates(self):
        self.seed_native_children()
        before = self.child_snapshot()
        # S and final C still contain this key: the fresh extraction must force replacement.
        self.con.execute("""
            DELETE FROM file_facts_cache
            WHERE blob_oid = 'blob' AND logical_path = 'src/one.rs' AND extractor_sha = 'v1'
        """)
        self.run_pipeline()
        self.assertEqual(self.rows(f"SELECT {KEY} FROM file_facts_extracted"),
                         [("blob", "src/one.rs", "v1")])
        self.assertEqual(self.child_snapshot(), before)
        self.assert_full_children()
        self.assert_child_deltas(refresh=[("blob", "src/one.rs", "v1")])
        self.assert_child_state()
        self.run_pipeline()
        self.assertEqual(self.child_snapshot(), before)
        self.assert_child_deltas()

    def test_checkpoint_covers_null_error_and_empty_parents(self):
        self.stage([
            ("good", "good.rs", RUST, "v1"),
            ("null", "null.rs", None, "v1"),
            ("unsupported", "bad.unknown", RUST, "v1"),
            ("invalid", "../escape.rs", RUST, "v1"),
            ("empty", "empty.rs", "", "v1"),
        ])
        self.run_pipeline()
        # Seed a typed, all-empty cached payload too; native empty-source facts may
        # still contain a file node. This is a new cache key, not a payload update.
        self.con.execute("""
            INSERT INTO file_facts_cache
            SELECT 'empty-facts', 'empty-facts.rs', 'v1',
                   struct_update(facts, nodes := [], edges := [], spans := [],
                                 macro_invocations := [], macro_scopes := []), 'empty_facts'
            FROM file_facts_cache WHERE blob_oid = 'good'
        """)
        self.run_pipeline()
        self.assertEqual(dict(self.rows("SELECT blob_oid, status FROM file_facts_cache")), {
            "good": "ok", "null": "null_input", "unsupported": "extraction_error",
            "invalid": "extraction_error", "empty": "empty_source", "empty-facts": "empty_facts",
        })
        self.assert_full_children()
        with self.subTest(check="all_parent_checkpoint_coverage"):
            self.assert_child_state()
        self.assert_child_deltas(refresh=[("empty-facts", "empty-facts.rs", "v1")])
        self.run_pipeline()
        self.assert_child_deltas()
        self.assert_child_state()

    def test_legacy_cache_without_checkpoint_bootstraps_once_without_extraction(self):
        self.seed_native_children()
        self.stage([("null", "null.rs", None, "v1")])
        self.run_pipeline()
        before, keys = self.child_snapshot(), self.cache_keys()
        # Cache plus full children but no checkpoint is the legacy on-disk layout.
        self.con.execute("DROP TABLE IF EXISTS file_facts_child_state")
        self.con.execute("DELETE FROM staged_sources")
        self.reopen()
        self.run_pipeline()
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
        self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
        self.assertEqual(self.child_snapshot(), before)
        self.assert_full_children()
        self.assert_child_deltas(refresh=keys)
        self.assert_child_state()
        oids = self.child_table_oids()
        self.run_pipeline()
        self.assertEqual(self.child_table_oids(), oids)
        self.assert_child_deltas()

    def test_missing_child_table_repairs_all_collections_from_cache(self):
        self.seed_native_children()
        before, keys = self.child_snapshot(), self.cache_keys()
        self.con.execute("DELETE FROM staged_sources")
        for collection in CHILDREN:
            with self.subTest(missing_collection=collection):
                self.con.execute(f"DROP TABLE file_facts_{collection}")
                self.run_pipeline()
                self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_missing"), 0)
                self.assertEqual(self.scalar("SELECT count(*) FROM file_facts_extracted"), 0)
                self.assertEqual(self.child_snapshot(), before)
                self.assert_full_children()
                self.assert_child_deltas(refresh=keys)
                self.assert_child_state()

    def test_empty_stage_has_stable_tables_state_and_zero_deltas_after_reopen(self):
        self.run_pipeline()
        for reopen in (False, True):
            with self.subTest(reopen=reopen):
                if reopen:
                    self.reopen()
                oids = self.child_table_oids()
                self.run_pipeline()
                self.assertEqual(self.cache_keys(), [])
                self.assert_full_children()
                with self.subTest(check="physical_table_reuse"):
                    self.assertEqual(self.child_table_oids(), oids)
                self.assert_child_deltas()
                self.assert_child_state()


if __name__ == "__main__":
    unittest.main()
