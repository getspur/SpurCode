"""Fixture-only derivation checks; never run these against a full history."""

from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import duckdb


CRATE = Path(__file__).resolve().parents[1]
RUNNER = CRATE / "examples/derive_temporal_pairs.py"
EXTENSION = Path(os.environ.get("SPUR_GRAPH_EXTENSION", "/private/tmp/spur-full-temporal-native/spur_graph.duckdb_extension"))
EXAMPLES = Path(os.environ.get("SPUR_TEMPORAL_EXAMPLES_DIR", "/private/tmp/spur-graph-duckdb-v2-target/release/examples"))


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


class TemporalPairsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="spur-pair-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Pair fixture")
        self.git("config", "user.email", "pair@example.invalid")
        self.producer = hashlib.sha256(EXTENSION.read_bytes()).hexdigest()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True).strip()

    def commit(self, message, *args):
        self.git("add", "-A")
        self.git("commit", "-qm", message, *args)
        return self.git("rev-parse", "HEAD")

    def write(self, path, source):
        (self.repo / path).write_bytes(source if isinstance(source, bytes) else source.encode())

    def fixture(self, history=False):
        self.write("input.py", "def alpha(x):\n    doubled = x * 2\n    return doubled + 1\n")
        self.write("empty.py", "")
        self.write("shared.py", "def shared():\n    return 1\n")
        self.write("notes.unknown", "unsupported file still receives a touch\n")
        self.root_commit = self.commit("root")
        if history:
            self.git("branch", "side")
            self.empty_commit = self.commit("empty", "--allow-empty")
            self.write("input.py", "def alpha(x):\n    doubled = x * 2\n    return doubled + 2\n")
            self.write("shared.py", "def shared():\n    return 2\n")
            self.commit("modify")
            self.git("mv", "input.py", "renamed.py")
            self.commit("rename file")
            self.write("renamed.py", "def beta(x):\n    doubled = x * 2\n    return doubled + 2\n")
            self.commit("rename symbol")
            self.git("checkout", "-q", "side")
            self.write("side.py", "def side():\n    return 3\n")
            self.write("shared.py", "def shared():\n    return 3\n")
            self.commit("side")
            self.git("checkout", "-q", "main")
            merged = subprocess.run(["git", "-C", str(self.repo), "merge", "--no-ff", "-qm", "merge", "side"],
                                    capture_output=True, text=True)
            self.assertEqual(merged.returncode, 1, merged.stdout + merged.stderr)
            self.write("shared.py", "def shared():\n    return 4\n")
            self.commit("resolve merge")
        self.head = self.git("rev-parse", "HEAD")
        inventory = self.root / "inventory"
        subprocess.run([str(EXAMPLES / "history_catalog"), str(self.repo), self.head, str(inventory)],
                       check=True, capture_output=True, text=True, timeout=60)
        self.metadata = self.root / "metadata.duckdb"
        c = duckdb.connect(str(self.metadata))
        for table in ("commits", "comparisons", "changes"):
            c.execute(f"CREATE TABLE {table} AS SELECT * FROM read_json_auto({quote(inventory / (table + '.jsonl'))})")
        c.execute("ALTER TABLE changes ALTER COLUMN old_blob_oid TYPE VARCHAR")
        c.execute("ALTER TABLE changes ALTER COLUMN parent_oid TYPE VARCHAR")
        c.execute("ALTER TABLE changes ALTER COLUMN old_path TYPE VARCHAR")
        c.execute("CREATE TABLE parse_tasks(language VARCHAR, blob_oid VARCHAR, path VARCHAR, byte_length BIGINT)")
        inputs = c.execute("SELECT DISTINCT language, oid, path FROM (SELECT language, old_blob_oid oid, "
                           "coalesce(old_path,path) path FROM changes UNION ALL SELECT language, new_blob_oid, path FROM changes) "
                           "WHERE language IS NOT NULL AND oid IS NOT NULL ORDER BY language,oid,path").fetchall()
        self.payloads = {}
        for language, oid, path in inputs:
            key = (language, oid)
            if key not in self.payloads:
                payload = subprocess.check_output(["git", "-C", str(self.repo), "cat-file", "blob", oid])
                self.payloads[key] = (path, payload)
                c.execute("INSERT INTO parse_tasks VALUES(?,?,?,?)", [language, oid, path, len(payload)])
        c.execute("CREATE TABLE symbol_pairs AS SELECT row_number() OVER(ORDER BY language,path,old_path,kind,old_blob_oid,new_blob_oid)-1 pair_id,* FROM "
                  "(SELECT DISTINCT language,path,CASE WHEN left(status,1) IN ('R','C') THEN old_path ELSE path END old_path," 
                  "CASE WHEN left(status,1) IN ('R','C') THEN 'R' WHEN left(status,1)='T' THEN 'M' ELSE left(status,1) END kind,"
                  "old_blob_oid,new_blob_oid FROM changes WHERE language IS NOT NULL AND NOT gitlink)")
        c.execute("CREATE TABLE change_pairs AS SELECT c.change_id,p.pair_id FROM changes c JOIN symbol_pairs p ON "
                  "c.language=p.language AND c.path=p.path AND coalesce(c.old_blob_oid,'')=coalesce(p.old_blob_oid,'') "
                  "AND coalesce(c.new_blob_oid,'')=coalesce(p.new_blob_oid,'') AND "
                  "CASE WHEN left(c.status,1) IN ('R','C') THEN c.old_path ELSE c.path END=p.old_path AND "
                  "CASE WHEN left(c.status,1) IN ('R','C') THEN 'R' WHEN left(c.status,1)='T' THEN 'M' ELSE left(c.status,1) END=p.kind")
        c.close()
        self.cache = self.root / "cache.parquet"
        self.flags = self.root / "flags.parquet"
        self.cache_db = self.root / "cache.duckdb"
        self.make_cache()
        self.config = self.root / "inputs.json"
        self.config.write_text(json.dumps({"sources": [{"cache_glob": str(self.cache), "flags_glob": str(self.flags),
                                                        "expected_extractor_sha": self.producer}]}))

    def make_cache(self, inline=False):
        c = duckdb.connect(str(self.cache_db), config={"allow_unsigned_extensions": True})
        c.execute(f"LOAD {quote(EXTENSION)}")
        symbols = c.execute("SELECT return_type FROM duckdb_functions() WHERE function_name='spur_graph_symbols'").fetchone()[0]
        c.execute(f"CREATE OR REPLACE TABLE cache(blob_oid VARCHAR,language VARCHAR,extractor_sha VARCHAR,representative_path VARCHAR,"
                  f"symbols {symbols},status VARCHAR,error VARCHAR,input_bytes BIGINT,parsed_at TIMESTAMPTZ,binary_prefix BOOLEAN)")
        for (language, oid), (path, payload) in self.payloads.items():
            try:
                source = payload.decode("utf-8")
            except UnicodeDecodeError:
                source = None
            c.execute("INSERT INTO cache SELECT ?,?,?,?,CASE WHEN ? IS NULL THEN NULL ELSE spur_graph_symbols(?,?) END,"
                      "CASE WHEN ? IS NULL THEN 'encoding_error' ELSE 'completed' END,"
                      "CASE WHEN ? IS NULL THEN 'invalid UTF-8' ELSE NULL END,?,current_timestamp,?",
                      [oid, language, self.producer, path, source, path, source, source, source, len(payload), b'\0' in payload[:8192]])
        columns = "*" if inline else "* EXCLUDE(binary_prefix)"
        c.execute(f"COPY (SELECT {columns} FROM cache) TO {quote(self.cache)} (FORMAT PARQUET)")
        c.execute(f"COPY (SELECT DISTINCT blob_oid,binary_prefix FROM cache) TO {quote(self.flags)} (FORMAT PARQUET)")
        c.close()

    def export_cache(self, update, inline=False):
        c = duckdb.connect(str(self.cache_db))
        c.execute(update)
        c.execute(f"COPY (SELECT {'*' if inline else '* EXCLUDE(binary_prefix)'} FROM cache) TO {quote(self.cache)} (FORMAT PARQUET)")
        c.execute(f"COPY (SELECT DISTINCT blob_oid,binary_prefix FROM cache) TO {quote(self.flags)} (FORMAT PARQUET)")
        c.close()

    def run_runner(self, output="result", budget=100000, row_limit=2, success=True):
        result = subprocess.run([sys.executable, str(RUNNER), "--metadata", str(self.metadata), "--parse-inputs", str(self.config),
                                 "--extension", str(EXTENSION), "--output", str(self.root / output), "--byte-budget", str(budget),
                                 "--row-limit", str(row_limit)], text=True, capture_output=True, timeout=90)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout.splitlines()[-1])
        self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def test_runner_is_available(self):
        self.assertTrue(RUNNER.is_file(), "bounded temporal pair derivation runner is missing")

    def test_missing_cache_is_not_valid_empty(self):
        self.fixture()
        self.export_cache("DELETE FROM cache WHERE input_bytes=0")
        self.assertIn("missing cache", self.run_runner(success=False).stderr)
        self.make_cache()
        summary = self.run_runner(output="valid")
        with duckdb.connect(summary["database"], read_only=True) as c:
            self.assertGreater(c.execute("SELECT count(*) FROM pair_occurrences WHERE status='completed' AND change_count=0").fetchone()[0], 0)

    def test_provenance_duplicate_byte_and_flag_validation(self):
        self.fixture()
        original = json.loads(self.config.read_text())
        wrong = json.loads(self.config.read_text())
        wrong["sources"][0]["expected_extractor_sha"] = "wrong"
        self.config.write_text(json.dumps(wrong))
        self.assertIn("extractor", self.run_runner(success=False).stderr)
        self.config.write_text(json.dumps({"sources": original["sources"] * 2}))
        self.assertIn("duplicate", self.run_runner(success=False).stderr)
        self.config.write_text(json.dumps(original))
        self.export_cache("UPDATE cache SET input_bytes=input_bytes+1")
        self.assertIn("input_bytes", self.run_runner(success=False).stderr)
        self.make_cache()
        c = duckdb.connect()
        c.execute(f"COPY (SELECT * FROM read_parquet({quote(self.flags)}) WHERE false) TO {quote(self.root / 'missing-flags.parquet')} (FORMAT PARQUET)")
        c.close()
        original["sources"][0]["flags_glob"] = str(self.root / "missing-flags.parquet")
        self.config.write_text(json.dumps(original))
        self.assertIn("binary", self.run_runner(success=False).stderr)

    def test_binary_precedes_parse_failure_and_inline_flags_work(self):
        self.write("binary.py", b"\0\xff")
        self.write("invalid.py", b"\xff")
        self.commit("bad encodings")
        self.fixture()
        self.make_cache(inline=True)
        config = json.loads(self.config.read_text())
        del config["sources"][0]["flags_glob"]
        self.config.write_text(json.dumps(config))
        summary = self.run_runner()
        with duckdb.connect(summary["database"], read_only=True) as c:
            statuses = dict(c.execute("SELECT path,status FROM pair_results JOIN symbol_pairs USING(pair_id)").fetchall())
            self.assertEqual(statuses["binary.py"], "binary_skipped")
            self.assertEqual(statuses["invalid.py"], "parse_failed")
            messages = c.execute("SELECT diagnostic FROM diagnostics").fetchall()
            self.assertTrue(any("left" in d or "right" in d for (d,) in messages))

    def test_disjoint_producers_with_old_and_inline_flag_schemas(self):
        self.fixture()
        c = duckdb.connect(str(self.cache_db))
        first = self.root / "first.parquet"
        second = self.root / "second.parquet"
        c.execute(f"COPY (SELECT * EXCLUDE(binary_prefix) FROM cache WHERE input_bytes=0) TO {quote(first)} (FORMAT PARQUET)")
        c.execute(f"COPY (SELECT * REPLACE ({quote('b' * 64)} AS extractor_sha) FROM cache WHERE input_bytes<>0) TO {quote(second)} (FORMAT PARQUET)")
        c.close()
        self.config.write_text(json.dumps({"sources": [
            {"cache_glob": str(first), "flags_glob": str(self.flags), "expected_extractor_sha": self.producer},
            {"cache_glob": str(second), "expected_extractor_sha": "b" * 64}]}))
        summary = self.run_runner()
        with duckdb.connect(summary["database"], read_only=True) as c:
            self.assertEqual(c.execute("SELECT count(DISTINCT extractor_sha) FROM cache_index").fetchone()[0], 2)

    def test_actual_producer_binary_skipped_schema(self):
        self.write("binary.py", b"\0\xff")
        self.commit("binary source")
        self.fixture()
        self.make_cache(inline=True)
        self.export_cache("UPDATE cache SET status='binary_skipped',symbols=NULL,error=NULL WHERE binary_prefix", inline=True)
        config = json.loads(self.config.read_text())
        del config["sources"][0]["flags_glob"]
        self.config.write_text(json.dumps(config))
        summary = self.run_runner()
        with duckdb.connect(summary["database"], read_only=True) as c:
            rows = c.execute("SELECT status,new_status,native_invoked,array_length(changes) FROM pair_results "
                             "JOIN symbol_pairs USING(pair_id) WHERE path='binary.py'").fetchall()
            self.assertEqual(rows, [("binary_skipped", "binary_skipped", False, 0)])

    def test_binary_skipped_requires_true_flag_null_symbols_and_null_error(self):
        self.write("binary.py", b"\0\xff")
        self.commit("binary source")
        self.fixture()
        config = json.loads(self.config.read_text())
        del config["sources"][0]["flags_glob"]
        self.config.write_text(json.dumps(config))
        for index, assignment in enumerate(("binary_prefix=false", "symbols=[]", "error='unexpected error'")):
            self.make_cache(inline=True)
            self.export_cache("UPDATE cache SET status='binary_skipped',symbols=NULL,error=NULL WHERE binary_prefix; "
                              "UPDATE cache SET " + assignment + " WHERE status='binary_skipped'", inline=True)
            result = self.run_runner(output=f"invalid-binary-{index}", success=False)
            self.assertIn("binary_skipped", result.stderr)

    def test_checkpoint_reuse_corruption_and_oversized_pair_receipt(self):
        self.fixture(history=True)
        first = self.run_runner(budget=1)
        receipts = sorted(Path(first["generation_directory"]).glob("receipts/*.json"))
        self.assertTrue(receipts)
        self.assertTrue(any(json.loads(p.read_text())["oversized"] for p in receipts))
        before = {p: p.read_bytes() for p in receipts}
        resumed = self.run_runner(budget=1)
        self.assertEqual(resumed["native_invocations_this_run"], 0)
        self.assertEqual(resumed["reused_batches"], len(receipts))
        self.assertEqual(before, {p: p.read_bytes() for p in receipts})
        data = json.loads(receipts[0].read_text())
        shard = Path(first["generation_directory"]) / data["file"]
        with shard.open("ab") as stream:
            stream.write(b"corruption")
        self.assertIn("checksum", self.run_runner(budget=1, success=False).stderr)
        current = Path(resumed.get("current_status_file", Path(first["generation_directory"]) / "result.json"))
        self.assertFalse(json.loads(current.read_text())["complete"], "failed checkpoint verification must revoke current completion")
        self.assertTrue(shard.read_bytes().endswith(b"corruption"))

    def test_thin_fold_first_payload_and_parent_scoping(self):
        self.fixture()
        later_oid = "f" * 40
        self.export_cache("INSERT INTO cache SELECT " + quote(later_oid) + ",language,extractor_sha,representative_path,"
                          "list_transform(symbols,lambda s: struct_update(s,anchor_hash:='later_payload')),status,error,"
                          "input_bytes,parsed_at,binary_prefix FROM cache WHERE representative_path='input.py'")
        c = duckdb.connect(str(self.metadata))
        pair = c.execute("SELECT pair_id FROM symbol_pairs WHERE new_blob_oid IS NOT NULL AND path='input.py'").fetchone()[0]
        change = c.execute("SELECT * FROM changes WHERE path='input.py'").fetchone()
        columns = [r[0] for r in c.description]
        mapping = dict(zip(columns, change))
        c.execute("INSERT INTO parse_tasks SELECT language,?,path,byte_length FROM parse_tasks WHERE path='input.py' LIMIT 1", [later_oid])
        c.execute("INSERT INTO symbol_pairs SELECT 1000,language,path,old_path,'D',?,NULL FROM symbol_pairs WHERE pair_id=?", [later_oid, pair])
        mapping.update(change_id=1000, status="D", old_blob_oid=later_oid, new_blob_oid=None)
        c.execute("INSERT INTO changes VALUES(" + ",".join("?" for _ in columns) + ")", [mapping[k] for k in columns])
        c.execute("INSERT INTO change_pairs VALUES(1000,1000)")
        c.close()
        summary = self.run_runner()
        with duckdb.connect(summary["database"], read_only=True) as c:
            rows = c.execute("SELECT change_kind,previous_stable_symbol_id,symbol.anchor_hash,change_id FROM symbol_changes WHERE symbol.entity_name='alpha'").fetchall()
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0][0:2], ("modified", None))
            self.assertNotEqual(rows[0][2], "later_payload", "the first snapshot payload must win")
            self.assertEqual(rows[0][3], change[columns.index("change_id")])
            fold_columns = c.execute("DESCRIBE folded_pointers").fetchall()
            self.assertFalse(any("tokens" in r[1] or "STRUCT(entity_name" in r[1] for r in fold_columns))

    def test_matching_error_is_a_status_not_an_empty_success(self):
        self.fixture()
        self.export_cache("UPDATE cache SET symbols=list_transform(symbols,lambda s: struct_update(s,entity_name:=NULL)) "
                          "WHERE representative_path='input.py'")
        summary = self.run_runner()
        self.assertEqual(summary["state"], "completed_with_errors")
        with duckdb.connect(summary["database"], read_only=True) as c:
            self.assertEqual(c.execute("SELECT status FROM pair_results JOIN symbol_pairs USING(pair_id) WHERE path='input.py'").fetchone()[0], "matching_error")
            self.assertGreater(c.execute("SELECT count(*) FROM diagnostics WHERE diagnostic LIKE 'matching_error:%'").fetchone()[0], 0)

    def test_no_supported_pairs_still_has_file_touches_and_gitlink_diagnostic(self):
        self.fixture()
        c = duckdb.connect(str(self.metadata))
        c.execute("DELETE FROM changes WHERE language IS NOT NULL")
        c.execute("DELETE FROM symbol_pairs")
        c.execute("DELETE FROM change_pairs")
        c.execute("UPDATE changes SET status='T'")
        c.execute("INSERT INTO changes SELECT 1000,comparison_id,commit_oid,parent_oid,parent_index,'A',old_mode,'160000',"
                  "NULL,'0123456789012345678901234567890123456789',NULL,'vendor/submodule.rs',NULL,true FROM changes LIMIT 1")
        c.close()
        summary = self.run_runner()
        self.assertEqual(summary["native_invocations_this_run"], 0)
        with duckdb.connect(summary["database"], read_only=True) as c:
            self.assertEqual(c.execute("SELECT change_kind FROM temporal_edges").fetchall(), [("modified",), ("modified",)])
            self.assertEqual(c.execute("SELECT count(*) FROM symbol_snapshots").fetchone()[0], 0)
            self.assertEqual(c.execute("SELECT count(*) FROM diagnostics WHERE diagnostic LIKE 'gitlink:%'").fetchone()[0], 1)

    def test_native_snapshot_edge_multisets_and_oracle_parent_provenance(self):
        self.fixture(history=True)
        summary = self.run_runner()
        reference = self.root / "reference"
        subprocess.run([str(EXAMPLES / "temporal_reference"), str(self.repo), str(reference), "1", "cli"],
                       check=True, capture_output=True, text=True, timeout=60)
        commits = self.root / "requests.jsonl"
        commits.write_text("".join(json.dumps({"commit_oid": oid}) + "\n" for oid in self.git("rev-list", "--reverse", "HEAD").split()))
        oracle = self.root / "oracle.jsonl"
        subprocess.run([str(EXAMPLES / "temporal_symbol_oracle"), str(self.repo), str(commits), str(oracle)],
                       check=True, capture_output=True, text=True, timeout=60)
        with duckdb.connect(summary["database"], read_only=True) as c:
            for table in ("symbol_snapshots", "temporal_edges"):
                expected = c.execute(f"SELECT * FROM read_parquet({quote(reference / table / '*.parquet')})").fetchall()
                actual = c.execute(f"SELECT * FROM {table}").fetchall()
                freeze = lambda rows: Counter(json.dumps(row, sort_keys=True) for row in rows)
                self.assertEqual(freeze(actual), freeze(expected), table)
            self.assertEqual(c.execute("SELECT count(*) FROM commits WHERE commit_oid=?", [self.empty_commit]).fetchone()[0], 1)
            actual = Counter(c.execute("SELECT commit_oid,parent_oid,stable_symbol_id,change_kind,previous_stable_symbol_id FROM symbol_changes").fetchall())
            expected = Counter()
            for line in oracle.read_text().splitlines():
                row = json.loads(line)
                kind = row["change_kind"]
                previous = None
                if isinstance(kind, dict):
                    kind, previous = "renamed_from_symbol", kind["renamed_from"]["symbol"]["stable_symbol_id"]
                expected[(row["commit_oid"], row["parent_sha"], row["snapshot"]["key"]["stable_symbol_id"], kind, previous)] += 1
            self.assertEqual(actual, expected)
            parents = c.execute("SELECT DISTINCT parent_oid FROM symbol_changes WHERE commit_oid=?", [self.head]).fetchall()
            self.assertEqual(len(parents), 2)
            duplicates = c.execute("SELECT count(*) FROM symbol_snapshots WHERE key_commit=? AND entity_name='shared'", [self.head]).fetchone()[0]
            self.assertEqual(duplicates, 2, "identical snapshots from two parents must remain two rows")
            self.assertEqual(c.execute("SELECT count(*) FROM diagnostics").fetchone()[0], 0)


if __name__ == "__main__":
    unittest.main()
