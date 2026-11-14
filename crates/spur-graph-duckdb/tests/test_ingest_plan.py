"""End-to-end completion and resume checks for the example repository caller."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import duckdb


class IngestPlanTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repository"
        self.repo.mkdir()
        self.git("init", "-q")
        sources = {"input.cpp": b"int alpha() { return 1; }\n", "empty.cpp": b""}
        for path, source in sources.items():
            (self.repo / path).write_bytes(source)
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")
        self.head = self.git("rev-parse", "HEAD").strip()
        self.catalog_rows = [
            {"path": path, "mode": "100644", "kind": "blob",
             "blob_oid": self.git("rev-parse", f"HEAD:{path}").strip(),
             "byte_length": len(source), "language": "Cpp"}
            for path, source in sources.items()
        ]
        self.catalog = self.root / "catalog.jsonl"
        self.write_catalog()
        self.plan = self.root / "plan.json"
        self.plan_value = {
            "head": self.head, "row_limit": duckdb.__standard_vector_size__,
            "byte_budget": sum(len(source) for source in sources.values()),
            "batches": [{"batch_id": 0, "tasks": self.catalog_rows}],
        }
        self.write_plan()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True)

    def write_catalog(self):
        self.catalog.write_text("".join(json.dumps(row) + "\n" for row in self.catalog_rows))

    def write_plan(self):
        self.plan.write_text(json.dumps(self.plan_value))

    def run_ingest(self):
        extension = os.environ["SPUR_GRAPH_EXTENSION"]
        return subprocess.run(
            [sys.executable, str(Path(__file__).resolve().parents[1] / "examples/ingest_plan.py"),
             "--repo", str(self.repo), "--catalog", str(self.catalog), "--plan", str(self.plan),
             "--extension", extension, "--output", str(self.root / "output")],
            text=True, capture_output=True, timeout=30,
        )

    def test_incomplete_plan_cannot_claim_catalog_completion(self):
        self.plan_value["batches"][0]["tasks"] = self.catalog_rows[:1]
        self.write_plan()
        result = self.run_ingest()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("supported catalog keys missing from plan", result.stderr)

    def test_resume_retains_empty_parse_and_skips_all_completed_keys(self):
        first = self.run_ingest()
        self.assertEqual(first.returncode, 0, first.stderr)
        result_path = self.root / "output/head-ingestion-result.json"
        result = json.loads(result_path.read_text())
        self.assertEqual(result["completed_parse_tasks"], 2)
        self.assertEqual(result["zero_symbol_tasks"], 1)
        self.assertEqual(result["new_parse_tasks_this_run"], 2)
        second = self.run_ingest()
        self.assertEqual(second.returncode, 0, second.stderr)
        result = json.loads(result_path.read_text())
        self.assertEqual(result["new_parse_tasks_this_run"], 0)
        self.assertEqual(result["zero_symbol_tasks"], 1)
        self.assertEqual(result["error_tasks"], 0)

    def test_task_path_must_match_its_catalog_metadata(self):
        self.plan_value["batches"][0]["tasks"][0]["path"] = "different.py"
        self.write_plan()
        result = self.run_ingest()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("task metadata does not match", result.stderr)

    def test_existing_catalog_conflict_cannot_be_hidden_by_cache(self):
        first = self.run_ingest()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.catalog_rows[0]["byte_length"] += 1
        self.write_catalog()
        self.write_plan()
        result = self.run_ingest()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("catalog conflicts", result.stderr)


if __name__ == "__main__":
    unittest.main()
