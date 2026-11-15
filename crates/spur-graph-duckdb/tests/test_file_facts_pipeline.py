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


if __name__ == "__main__":
    unittest.main()
