"""Independent C++/Python extraction and SQL fault injection on real native2.

    SPUR_GRAPH_EXTENSION=/approved/spur_graph.duckdb_extension \
    PYTHONDONTWRITEBYTECODE=1 /private/tmp/spur-graph-duckdb-v2-20261114/bin/python \
      -m unittest discover -s crates/spur-graph-duckdb/tests \
      -p test_project_quality.py -v

No Rust build, native oracle, parser substitute, production writes or project
ingestion. Missing/wrong engine or extension is a failure, never a skipped test.
Temporary databases retain raw facts and all five typed collections. SQL faults
are injected after authentic extraction, independently of the reader queries.
"""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import duckdb
import _duckdb


MODULE_PATH = Path(__file__).resolve().parents[1] / "examples/project_quality.py"
SPEC = importlib.util.spec_from_file_location("project_quality", MODULE_PATH)
quality = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(quality)
EXPECTED_EXTENSION_SHA256 = "e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028"
CPP = """int leaf(int x) { return x + 1; }
struct Connection {
    int Query(int x) { return leaf(x); }
    int Query(double x) { return leaf(2); }
};
struct Parser {
    int ParseQuery() { return leaf(3); }
};
int entry() { return external_cpp(); }
"""
PYTHON = """# Independent UTF-8 byte-length control: café, π.
def leaf():
    return 7

def entry():
    return leaf() + external_python()
"""


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class ProjectQualityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if duckdb.__version__ != "2.0.0":
            raise AssertionError("tests require actual DuckDB 2.0.0 native2")
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        if not extension:
            raise AssertionError("set SPUR_GRAPH_EXTENSION to the approved real extension")
        extension = Path(extension).resolve(strict=True)
        expected_sha = EXPECTED_EXTENSION_SHA256
        if any(os.environ.get(key) for key in (
                "SPUR_RELEASE_MANIFEST", "SPUR_RELEASE_MANIFEST_SHA256", "SPUR_RELEASE_SOURCE_IDENTITY")):
            scripts = str(MODULE_PATH.parents[1] / "scripts")
            if scripts not in sys.path:
                sys.path.insert(0, scripts)
            from check_release import approved_extension_sha, release_trust
            expected_sha = approved_extension_sha(extension, **release_trust())
        if digest(extension) != expected_sha:
            raise AssertionError("unapproved extension SHA256")
        cls.workspace = tempfile.TemporaryDirectory(prefix="project-quality-engine-")
        cls.addClassCleanup(cls.workspace.cleanup)
        cls.baseline = Path(cls.workspace.name) / "baseline.duckdb"
        with duckdb.connect(str(cls.baseline), config={"allow_unsigned_extensions": "true"}) as c:
            c.execute("LOAD '" + str(extension).replace("'", "''") + "'")
            c.execute("CREATE TABLE inventory(path VARCHAR,git_blob_oid VARCHAR,bytes BIGINT,suffix VARCHAR,area VARCHAR,status VARCHAR,error VARCHAR)")
            c.execute("CREATE TABLE sources(logical_path VARCHAR,source VARCHAR,source_bytes BLOB,file_oid VARCHAR,sha256 VARCHAR)")
            for path, source, area in (("src/control.cpp", CPP, "core"),
                                       ("tools/control.py", PYTHON, "tools"),
                                       ("src/empty.cpp", "", "core")):
                data = source.encode("utf-8")
                oid = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
                c.execute("INSERT INTO inventory VALUES (?,?,?,?,?,'extracted',NULL)",
                          [path, oid, len(data), Path(path).suffix, area])
                c.execute("INSERT INTO sources VALUES (?,?,?,?,?)",
                          [path, source, data, oid, hashlib.sha256(data).hexdigest()])
            c.execute("INSERT INTO inventory VALUES ('notes/readme.unsupported','oid',19,'.unsupported','other','unsupported','unsupported exact path')")
            # No TRY/fallback: extension failures abort the test setup.
            c.execute("CREATE TABLE facts AS SELECT logical_path,spur_graph_file_facts(logical_path,source) AS value FROM sources")
            for table in quality.COLLECTIONS:
                c.execute(f"CREATE TABLE {table} AS SELECT logical_path,item.* FROM facts,UNNEST(value.{table}) AS u(item)")
            cls.baseline_counts = dict(c.execute(" UNION ALL ".join(
                f"SELECT '{table}', count(*) FROM {table}" for table in quality.COLLECTIONS)).fetchall())
            if cls.baseline_counts["nodes"] < 8 or cls.baseline_counts["edges"] < 5:
                raise AssertionError("real fixtures did not produce the expected nontrivial graph")
        print(json.dumps({"engine_receipt": {
            "duckdb": duckdb.__version__, "native_engine": _duckdb.__file__,
            "native_engine_sha256": digest(_duckdb.__file__),
            "extension": str(extension), "extension_sha256": digest(extension),
            "reader_sha256": digest(MODULE_PATH), "test_sha256": digest(__file__),
            "typed_counts": cls.baseline_counts,
        }}), flush=True)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="project-quality-test-")
        self.addCleanup(self.temp.cleanup)
        self.database = Path(self.temp.name) / "audit.duckdb"
        shutil.copyfile(self.baseline, self.database)

    def mutate(self, sql, params=()):
        with duckdb.connect(str(self.database)) as c:
            c.execute(sql, list(params))

    def audit(self, limit=100):
        return quality.audit_database(self.database, limit)

    def rows(self, report, name):
        return report["queries"][name]["rows"]

    def invariants(self, report):
        return {r["invariant"]: r["affected_rows"] for r in self.rows(report, "structural_invariants")}

    def test_real_cpp_python_all_five_collections_and_denominators(self):
        report = self.audit()
        self.assertEqual(self.rows(report, "inventory_totals")[0]["inventory_rows"], 4)
        counts = {r["collection"]: r for r in self.rows(report, "typed_counts")}
        self.assertEqual(set(counts), set(quality.COLLECTIONS))
        for table, expected in self.baseline_counts.items():
            self.assertEqual(counts[table]["typed_rows"], expected)
            self.assertEqual(counts[table]["raw_rows"], expected)
        self.assertTrue(all(v == 0 for v in self.invariants(report).values()), self.invariants(report))
        statuses = {r["status"]: r["inventory_rows"] for r in self.rows(report, "inventory_by_status")}
        self.assertEqual(statuses, {"extracted": 3, "unsupported": 1})
        examples = self.rows(report, "file_only_empty_examples")
        unsupported = next(r for r in examples if r["path"].endswith(".unsupported"))
        self.assertEqual(unsupported["extraction"], "no_fact_row")
        self.assertIn("src/empty.cpp", {r["path"] for r in examples})
        self.assertFalse(report["extension_provenance_verified"])

    def test_real_calls_overloads_and_contextual_exploration(self):
        report = self.audit()
        rates = self.rows(report, "call_rates_by_area")
        self.assertEqual({r["area"] for r in rates}, {"core", "tools"})
        for row in rates:
            self.assertGreater(row["resolved_only_calls"], 0)
            self.assertGreater(row["unresolved_calls"], 0)
            self.assertEqual(row["calls"], sum(row[k] for k in (
                "resolved_only_calls", "unresolved_calls", "dangling_target_calls", "ambiguous_target_calls")))
            self.assertAlmostEqual(row["resolved_only_rate"], row["resolved_only_calls"] / row["calls"])
        seeds = self.rows(report, "exploration_seeds")
        self.assertIn(("Connection", "Query"), {(r["owner_label"], r["method_label"]) for r in seeds})
        self.assertIn(("Parser", "ParseQuery"), {(r["owner_label"], r["method_label"]) for r in seeds})
        self.assertGreater(report["queries"]["exploration_outgoing_examples"]["total_rows"], 0)
        overloads = self.rows(report, "overloaded_names")
        self.assertTrue(any(r["label"] == "Query" and r["definitions"] == 2 for r in overloads))

    def test_orphans_are_path_local_even_when_id_exists_in_another_file(self):
        with duckdb.connect(str(self.database)) as c:
            c.execute("CREATE TEMP TABLE foreign_only AS SELECT node_id,source_span_id FROM nodes WHERE logical_path='tools/control.py' LIMIT 1")
            node_id, span_id = c.execute("SELECT * FROM foreign_only").fetchone()
            c.execute("UPDATE nodes SET node_id=1000000+node_id, source_span_id=? WHERE logical_path='src/control.cpp'", [span_id])
            c.execute("UPDATE spans SET span_id=1000000+span_id WHERE logical_path='src/control.cpp'")
            c.execute("UPDATE edges SET source_node_id=?,target_node_id=? WHERE logical_path='src/control.cpp'", [node_id, node_id])
            cpp_nodes = c.execute("SELECT count(*) FROM nodes WHERE logical_path='src/control.cpp'").fetchone()[0]
            cpp_edges = c.execute("SELECT count(*) FROM edges WHERE logical_path='src/control.cpp'").fetchone()[0]
        inv = self.invariants(self.audit())
        self.assertEqual(inv["nodes_without_span"], cpp_nodes)
        self.assertEqual(inv["edges_without_source_node"], cpp_edges)
        self.assertEqual(inv["edges_with_nonnull_missing_target"], cpp_edges)

    def test_out_of_bounds_reversed_null_and_utf8_byte_lengths(self):
        with duckdb.connect(str(self.database)) as c:
            length = c.execute("SELECT octet_length(source_bytes) FROM sources WHERE logical_path='tools/control.py'").fetchone()[0]
            span_ids = [r[0] for r in c.execute("SELECT span_id FROM spans WHERE logical_path='tools/control.py' ORDER BY span_id").fetchall()]
            self.assertGreaterEqual(len(span_ids), 3)
            # End equal to byte length is legal, including a multibyte comment.
            c.execute("UPDATE spans SET end_byte=? WHERE logical_path='tools/control.py'", [length])
        self.assertEqual(self.invariants(self.audit())["invalid_byte_spans"], 0)
        for sql, params in (
            ("UPDATE spans SET end_byte=? WHERE logical_path='tools/control.py' AND span_id=?", [length + 1, span_ids[0]]),
            ("UPDATE spans SET start_byte=5,end_byte=4 WHERE logical_path='tools/control.py' AND span_id=?", [span_ids[1]]),
            ("UPDATE spans SET start_byte=NULL WHERE logical_path='tools/control.py' AND span_id=?", [span_ids[2]]),
        ):
            self.mutate(sql, params)
        report = self.audit()
        self.assertEqual(self.invariants(report)["invalid_byte_spans"], 3)
        self.assertEqual(report["queries"]["invalid_span_examples"]["total_rows"], 3)

    def test_missing_and_duplicate_local_ids_do_not_multiply_call_rates(self):
        self.mutate("""INSERT INTO nodes SELECT * FROM nodes
        WHERE logical_path='src/control.cpp' AND node_id=(
          SELECT target_node_id FROM edges WHERE logical_path='src/control.cpp'
          AND relation='calls' AND target_node_id IS NOT NULL LIMIT 1)""")
        for table in ("edges", "spans"):
            self.mutate(f"INSERT INTO {table} SELECT * FROM {table} WHERE logical_path='src/control.cpp' LIMIT 1")
        self.mutate("UPDATE spans SET span_id=NULL WHERE logical_path='src/empty.cpp'")
        report = self.audit()
        for table in ("nodes", "edges", "spans"):
            self.assertEqual(self.rows(report, table + "_id_health")[0]["duplicate_id_groups"], 1)
        self.assertEqual(self.rows(report, "spans_id_health")[0]["missing_ids"], 1)
        with duckdb.connect(str(self.database), read_only=True) as c:
            call_count = c.execute("SELECT count(*) FROM edges WHERE relation='calls'").fetchone()[0]
        self.assertEqual(sum(r["calls"] for r in self.rows(report, "call_rates_by_area")), call_count)
        self.assertGreater(sum(r["ambiguous_target_calls"] for r in self.rows(report, "call_rates_by_area")), 0)
        self.assertGreater(self.invariants(report)["nodes_raw_typed_count_mismatch_paths"], 0)

    def test_global_id_reuse_is_legal_and_stable_key_duplicates_are_diagnostic(self):
        report = self.audit()
        self.assertEqual(self.rows(report, "nodes_id_health")[0]["duplicate_id_groups"], 0)
        with duckdb.connect(str(self.database)) as c:
            self.assertGreater(c.execute("SELECT count(*) FROM nodes WHERE node_id=1").fetchone()[0], 1)
            key = c.execute("SELECT stable_key FROM nodes WHERE logical_path='src/control.cpp' LIMIT 1").fetchone()[0]
            c.execute("UPDATE nodes SET stable_key=? WHERE logical_path='tools/control.py'", [key])
        report = self.audit()
        self.assertGreater(report["queries"]["stable_key_duplicate_evidence"]["total_rows"], 1)
        self.assertTrue(all(v == 0 for v in self.invariants(report).values()))
        self.assertNotIn("quality_pass", report)

    def test_null_facts_missing_source_and_unknown_paths_are_visible(self):
        self.mutate("UPDATE facts SET value=NULL WHERE logical_path='src/empty.cpp'")
        self.mutate("DELETE FROM sources WHERE logical_path='tools/control.py'")
        # Typed rows survive even when their originating facts are NULL.
        self.mutate("UPDATE nodes SET logical_path='not-in-inventory.cpp' WHERE logical_path='src/empty.cpp'")
        report = self.audit()
        inv = self.invariants(report)
        self.assertEqual(inv["nodes_raw_typed_count_mismatch_paths"], 1)
        self.assertEqual(inv["nodes_without_inventory"], 1)
        self.assertGreater(inv["spans_without_unique_source_bytes"], 0)
        self.assertEqual(self.rows(report, "extraction_totals")[0]["null_fact_rows"], 1)

    def test_balanced_global_count_still_detects_two_per_path_mismatches(self):
        self.mutate("""UPDATE nodes SET logical_path='src/control.cpp'
        WHERE logical_path='tools/control.py' AND node_id=(
          SELECT min(node_id) FROM nodes WHERE logical_path='tools/control.py' AND kind='function')""")
        report = self.audit()
        node_count = next(r for r in self.rows(report, "typed_counts") if r["collection"] == "nodes")
        self.assertEqual(node_count["typed_rows"], node_count["raw_rows"])
        self.assertEqual(self.invariants(report)["nodes_raw_typed_count_mismatch_paths"], 2)

    def test_macro_identity_checks_use_site_and_scope_struct_without_global_binding(self):
        # Fault injection into genuinely typed empty extension collections;
        # no synthetic facts replace the C++/Python extraction baseline.
        self.mutate("""INSERT INTO macro_invocations (logical_path,site_id) VALUES
          ('src/control.cpp','site'), ('tools/control.py','site'),
          ('src/control.cpp','site'), ('src/control.cpp',NULL)""")
        self.mutate("""INSERT INTO macro_scopes (logical_path,scope) VALUES
          ('src/control.cpp',struct_pack(stable_symbol_id := 's',kind := 'function')),
          ('tools/control.py',struct_pack(stable_symbol_id := 's',kind := 'function')),
          ('src/control.cpp',struct_pack(stable_symbol_id := 's',kind := 'function')),
          ('src/control.cpp',struct_pack(stable_symbol_id := 's',kind := 'method')),
          ('src/control.cpp',struct_pack(stable_symbol_id := 'missing_kind',kind := NULL))""")
        report = self.audit()
        for table in ('macro_invocations', 'macro_scopes'):
            health = self.rows(report, table + "_id_health")[0]
            self.assertEqual(health["missing_ids"], 1)
            self.assertEqual(health["duplicate_id_groups"], 1)
            self.assertEqual(report["queries"][table + "_duplicate_ids"]["total_rows"], 1)

    def test_all_receipts_replay_exactly_and_limits_keep_full_denominators(self):
        report = self.audit(limit=1)
        self.assertEqual(self.rows(report, "inventory_totals")[0]["inventory_rows"], 4)
        self.assertEqual(report["queries"]["inventory_by_status"]["total_rows"], 2)
        self.assertTrue(report["queries"]["inventory_by_status"]["truncated"])
        with duckdb.connect(str(self.database), read_only=True) as c:
            for name, receipt in report["queries"].items():
                with self.subTest(query=name):
                    total = c.execute(receipt["count_sql"], receipt["count_params"]).fetchone()[0]
                    cur = c.execute(receipt["sql"], receipt["params"])
                    replay = [dict(zip(receipt["columns"], r)) for r in cur.fetchall()]
                    self.assertEqual(receipt["total_rows"], total)
                    self.assertEqual(receipt["rows"], replay)
                    self.assertLessEqual(len(replay), receipt["limit"])
        json.dumps(report, allow_nan=False)

    def test_read_only_cli_does_not_change_database_and_refuses_overwrite(self):
        before = digest(self.database)
        output = Path(self.temp.name) / "report.json"
        command = [sys.executable, str(MODULE_PATH), "--database", str(self.database),
                   "--out", str(output), "--limit", "2"]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        saved = output.read_bytes()
        report = json.loads(saved)
        self.assertTrue(report["runtime"]["read_only"])
        self.assertFalse(report["runtime"]["extension_loaded_by_reader"])
        self.assertEqual(before, digest(self.database))
        self.assertFalse(Path(str(self.database) + ".wal").exists())
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("output already exists", result.stderr)
        self.assertEqual(saved, output.read_bytes())

    def test_missing_schema_and_invalid_limit_fail_explicitly(self):
        self.mutate("ALTER TABLE edges DROP COLUMN confidence")
        with self.assertRaisesRegex(ValueError, "edges.confidence"):
            self.audit()
        for limit in (0, 101, True):
            with self.subTest(limit=limit), self.assertRaisesRegex(ValueError, "limit"):
                quality.audit_database(self.database, limit)


if __name__ == "__main__":
    unittest.main()
