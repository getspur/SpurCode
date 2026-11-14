"""Run in DuckDB 2.0.0 with SPUR_GRAPH_EXTENSION pointing to the frozen artifact."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import duckdb


RUNNER = Path(__file__).resolve().parents[1] / "examples/parse_history_shards.py"


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class HistoryShardTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        self.extension = Path(os.environ["SPUR_GRAPH_EXTENSION"]).resolve()
        self.temporary = tempfile.TemporaryDirectory(prefix="spur-history-shards-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q", "--initial-branch=main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.source = b"pub fn zeta() -> i32 { 2 }\npub fn alpha() -> i32 { 1 }\n"
        payloads = {
            "source.rs": self.source,
            "empty.rs": b"",
            "binary.rs": b"\xff" + b" " * (8192 - 2) + b"\x00",
            "invalid.rs": b"\xffinvalid",
            "after-prefix.rs": b" " * 8192 + b"\x00\xff",
        }
        for name, payload in payloads.items():
            (self.repo / name).write_bytes(payload)
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.head = self.git("rev-parse", "HEAD").strip()
        self.tasks = {
            name: {"blob_oid": self.git("rev-parse", "HEAD:" + name).strip(),
                   "language": "Rust", "path": "history/" + name,
                   "byte_length": len(payload)}
            for name, payload in payloads.items()
        }
        empty_cpp = dict(self.tasks["empty.rs"], language="Cpp", path="history/empty.cpp")
        unsupported = dict(self.tasks["source.rs"], language="Unsupported", path="history/source.unknown")
        batches = [
            {"batch_id": 0, "tasks": [self.tasks["source.rs"], self.tasks["empty.rs"], empty_cpp]},
            {"batch_id": 1, "tasks": [self.tasks[name] for name in
                                     ("binary.rs", "invalid.rs", "after-prefix.rs")]},
            {"batch_id": 2, "tasks": [unsupported]},
        ]
        self.plan_value = {
            "head": self.head,
            "row_limit": max(len(batch["tasks"]) for batch in batches),
            "byte_budget": max(sum(task["byte_length"] for task in batch["tasks"]) for batch in batches),
            "batches": batches,
        }
        self.plan = self.root / "plan.json"
        self.output = self.root / "output"
        self.write_plan()
        self.source_before = self.repo_files()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True)

    def write_plan(self):
        self.plan.write_text(json.dumps(self.plan_value))

    def generation(self):
        return self.output / "generations" / sha256(self.extension) / sha256(self.plan)

    def repo_files(self):
        return {str(path.relative_to(self.repo)): sha256(path)
                for path in self.repo.rglob("*") if path.is_file()}

    def run_parser(self):
        self.assertTrue(RUNNER.is_file(), "bounded history shard runner is missing")
        return subprocess.run(
            [sys.executable, str(RUNNER), "--repo", str(self.repo), "--plan", str(self.plan),
             "--extension", str(self.extension), "--output", str(self.output)],
            text=True, capture_output=True, timeout=60,
        )

    def result(self):
        return json.loads((self.generation() / "history-parse-result.json").read_text())

    def parquet_paths(self):
        return sorted((self.generation() / "batches").glob("*.parquet"))

    def test_ordered_native_facts_and_all_terminal_parent_rows(self):
        result = self.run_parser()
        self.assertEqual(result.returncode, 1, result.stderr)
        summary = self.result()
        self.assertIs(summary["complete"], True)
        self.assertEqual(summary["state"], "completed_with_errors")
        self.assertEqual(summary["planned_tasks"], 7)
        self.assertEqual(summary["completed_tasks"], 7)
        self.assertEqual(summary["error_tasks"], 3)
        self.assertEqual(summary["zero_symbol_tasks"], 2)
        self.assertEqual(summary["binary_skipped_tasks"], 1)
        self.assertEqual(summary["missing_tasks"], 0)
        self.assertEqual(summary["new_parse_tasks_this_run"], 7)
        self.assertEqual(summary["head"], self.head)
        self.assertEqual(summary["engine"], "2.0.0")
        self.assertEqual(summary["extractor_sha256"], sha256(self.extension))
        self.assertEqual(summary["plan_sha256"], sha256(self.plan))
        with duckdb.connect(config={"allow_unsigned_extensions": True}) as connection:
            connection.execute("LOAD '" + str(self.extension).replace("'", "''") + "'")
            expected = connection.execute("SELECT spur_graph_symbols('history/source.rs', ?)",
                                          [self.source.decode()]).fetchone()[0]
            paths = [str(path) for path in self.parquet_paths()]
            schema = connection.execute("DESCRIBE SELECT * FROM read_parquet(?)", [paths]).fetchall()
            self.assertEqual(schema[-1][1], "TIMESTAMP WITH TIME ZONE")
            rows = connection.execute(
                "SELECT * REPLACE(CAST(parsed_at AS VARCHAR) AS parsed_at) FROM read_parquet(?) ORDER BY blob_oid,language",
                [paths],
            ).fetchall()
            columns = [entry[0] for entry in connection.description]
            self.assertEqual(columns, ["blob_oid", "language", "extractor_sha", "representative_path",
                                       "symbols", "status", "error", "input_bytes", "binary_prefix", "parsed_at"])
            keyed = {(row[0], row[1]): dict(zip(columns, row)) for row in rows}
            self.assertEqual(len(rows), 7)
            self.assertEqual(len(keyed), 7)
            native = keyed[(self.tasks["source.rs"]["blob_oid"], "Rust")]
            self.assertEqual(native["symbols"], expected)
            self.assertEqual(native["status"], "completed")
            for language in ("Rust", "Cpp"):
                empty = keyed[(self.tasks["empty.rs"]["blob_oid"], language)]
                self.assertEqual(empty["symbols"], [])
                self.assertEqual(empty["status"], "completed")
            binary = keyed[(self.tasks["binary.rs"]["blob_oid"], "Rust")]
            self.assertEqual(binary["status"], "binary_skipped")
            self.assertIs(binary["binary_prefix"], True)
            self.assertIsNone(binary["symbols"])
            for name in ("invalid.rs", "after-prefix.rs"):
                invalid = keyed[(self.tasks[name]["blob_oid"], "Rust")]
                self.assertEqual(invalid["status"], "encoding_error")
                self.assertIs(invalid["binary_prefix"], False)
                self.assertIsNone(invalid["symbols"])
            failed = keyed[(self.tasks["source.rs"]["blob_oid"], "Unsupported")]
            self.assertEqual(failed["status"], "extension_error")
            self.assertIsNone(failed["symbols"])
        self.assertEqual(len(summary["checksums"]), 3)
        for checksum in summary["checksums"]:
            self.assertEqual(checksum["sha256"], sha256(self.generation() / checksum["parquet"]))
        checkpoints = [json.loads(line) for line in result.stdout.splitlines()
                       if json.loads(line).get("state") == "checkpoint"]
        self.assertEqual(len(checkpoints), 3)
        self.assertEqual(checkpoints[0]["source_blobs_read"], 2)
        self.assertTrue(all(row["batch_parse_seconds"] >= 0 and row["max_rss_bytes"] > 0
                            for row in checkpoints))
        self.assertFalse(list(self.output.rglob("*.duckdb*")))
        self.assertFalse(list(self.output.rglob("*.blob")))
        self.assertEqual(self.repo_files(), self.source_before)

    def test_restart_verifies_empty_binary_and_error_rows_without_reparsing(self):
        first = self.run_parser()
        self.assertEqual(first.returncode, 1, first.stderr)
        original = {path: sha256(path) for path in self.parquet_paths()}
        partial = self.generation() / "batches" / "leftover.parquet.tmp"
        partial.write_bytes(b"retained partial")
        second = self.run_parser()
        self.assertEqual(second.returncode, 1, second.stderr)
        summary = self.result()
        self.assertIs(summary["complete"], True)
        self.assertEqual(summary["new_parse_tasks_this_run"], 0)
        self.assertEqual(summary["source_blobs_read"], 0)
        self.assertEqual(summary["completed_tasks"], 7)
        self.assertEqual({path: sha256(path) for path in self.parquet_paths()}, original)
        self.assertEqual(partial.read_bytes(), b"retained partial")
        self.assertEqual(self.repo_files(), self.source_before)

    def test_missing_or_corrupt_receipt_and_corrupt_parquet_refuse_completion(self):
        first = self.run_parser()
        self.assertEqual(first.returncode, 1, first.stderr)
        parquet = self.parquet_paths()[0]
        receipt = parquet.with_suffix(".receipt.json")
        original_receipt = receipt.read_bytes()
        original_parquet = parquet.read_bytes()
        for corruption in ("missing_receipt", "receipt_counts", "parquet_checksum"):
            with self.subTest(corruption=corruption):
                if corruption == "missing_receipt":
                    receipt.rename(receipt.with_suffix(".retained"))
                elif corruption == "receipt_counts":
                    value = json.loads(original_receipt)
                    value["key_count"] = 0
                    receipt.write_text(json.dumps(value))
                else:
                    parquet.write_bytes(original_parquet + b"corrupt")
                result = self.run_parser()
                self.assertNotEqual(result.returncode, 0)
                self.assertIs(self.result()["complete"], False)
                self.assertEqual(self.result()["state"], "infrastructure_error")
                self.assertEqual(self.result()["new_parse_tasks_this_run"], 0)
                self.assertTrue(parquet.exists())
                receipt.write_bytes(original_receipt)
                parquet.write_bytes(original_parquet)

    def test_byte_budget_and_duplicate_keys_are_rejected_before_parse(self):
        for failure in ("bytes", "duplicate"):
            with self.subTest(failure=failure):
                if failure == "bytes":
                    self.plan_value["byte_budget"] -= 1
                else:
                    self.plan_value["byte_budget"] += 1
                    self.plan_value["batches"][2]["tasks"] = [self.tasks["source.rs"]]
                self.write_plan()
                result = self.run_parser()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("byte budget" if failure == "bytes" else "duplicate parse key", result.stderr)
                self.assertFalse(self.parquet_paths())
        self.assertEqual(self.repo_files(), self.source_before)

    def test_valid_checksum_with_wrong_keys_still_refuses_completion(self):
        first = self.run_parser()
        self.assertEqual(first.returncode, 1, first.stderr)
        parquet = self.parquet_paths()[0]
        replacement = self.root / "different-keys.parquet"
        with duckdb.connect() as connection:
            connection.execute("COPY (SELECT * REPLACE('Other' AS language) FROM read_parquet(?)) TO '" +
                               str(replacement).replace("'", "''") + "' (FORMAT PARQUET)", [str(parquet)])
        replacement.replace(parquet)
        receipt = parquet.with_suffix(".receipt.json")
        value = json.loads(receipt.read_text())
        value.update(sha256=sha256(parquet), file_bytes=parquet.stat().st_size)
        receipt.write_text(json.dumps(value))
        result = self.run_parser()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly match planned tasks", result.stderr)
        self.assertIs(self.result()["complete"], False)
        self.assertEqual(self.result()["new_parse_tasks_this_run"], 0)

    def test_plan_generations_are_separate_and_success_exits_zero(self):
        self.assertEqual(self.run_parser().returncode, 1)
        older_generation = self.generation()
        original = {path: sha256(path) for path in older_generation.rglob("*") if path.is_file()}
        self.plan_value["batches"] = self.plan_value["batches"][:1]
        self.write_plan()
        result = self.run_parser()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(self.generation(), older_generation)
        self.assertIs(self.result()["complete"], True)
        self.assertEqual(self.result()["state"], "completed")
        self.assertEqual({path: sha256(path) for path in older_generation.rglob("*") if path.is_file()}, original)


if __name__ == "__main__":
    unittest.main()
