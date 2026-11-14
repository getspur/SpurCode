"""Reference-runner checks; build both Rust examples before running this file.

Set SPUR_TEMPORAL_EXAMPLES_DIR to the directory containing the two executables,
or set CARGO_TARGET_DIR to the target used for the release example build.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


CRATE = Path(__file__).resolve().parents[1]
WORKSPACE = CRATE.parents[1]
EXAMPLES = Path(os.environ.get(
    "SPUR_TEMPORAL_EXAMPLES_DIR",
    str(Path(os.environ.get("CARGO_TARGET_DIR", str(CRATE / "target"))) / "release/examples"),
))


class RegistrationTests(unittest.TestCase):
    def test_reference_examples_are_registered(self):
        result = subprocess.run(
            [str(WORKSPACE / "scripts/spur-cargo"), "--workdir", str(CRATE),
             "metadata", "--no-deps", "--format-version", "1", "--offline"],
            text=True, capture_output=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        metadata = json.loads(result.stdout)
        package = next(p for p in metadata["packages"] if p["name"] == "spur-graph-duckdb")
        examples = {t["name"] for t in package["targets"] if "example" in t["kind"]}
        self.assertTrue({"temporal_reference", "temporal_symbol_oracle"} <= examples, examples)


class TemporalReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        for name in ("temporal_reference", "temporal_symbol_oracle"):
            if not (EXAMPLES / name).is_file():
                raise AssertionError(f"build release example {name}, then set SPUR_TEMPORAL_EXAMPLES_DIR")

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="spur-temporal-reference-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.git(self.source, "init", "-q", "--initial-branch=main")
        self.git(self.source, "config", "user.name", "Fixture")
        self.git(self.source, "config", "user.email", "fixture@example.invalid")
        (self.source / "input.py").write_text("def alpha():\n    return 1\n")
        (self.source / "empty.py").write_text("")
        self.root_commit = self.commit("root")
        self.empty_commit = self.commit("empty", "--allow-empty")
        self.git(self.source, "checkout", "-qb", "side", self.root_commit)
        (self.source / "side.py").write_text("def side():\n    return 3\n")
        self.side_commit = self.commit("side")
        self.git(self.source, "checkout", "-q", "main")
        (self.source / "input.py").write_text("def alpha():\n    return 2\n")
        self.commit("modify")
        self.git(self.source, "mv", "input.py", "renamed.py")
        self.rename_commit = self.commit("rename")
        self.git(self.source, "merge", "--no-ff", "-qm", "merge", "side")
        self.head = self.git(self.source, "rev-parse", "HEAD").strip()
        self.git(self.source, "checkout", "-qb", "unreachable", self.root_commit)
        (self.source / "excluded.py").write_text("def excluded():\n    return 0\n")
        self.unreachable_commit = self.commit("unreachable")
        self.git(self.source, "checkout", "-q", "main")
        self.repo = self.root / "frozen.git"
        self.git(self.root, "clone", "--bare", "-q", str(self.source), str(self.repo))
        self.expected = set(self.git(self.repo, "rev-list", "HEAD").split())
        self.source_before = self.source_files()

    def git(self, repo, *args):
        return subprocess.check_output(["git", "-C", str(repo), *args], text=True)

    def commit(self, message, *args):
        self.git(self.source, "add", ".")
        self.git(self.source, "commit", "-qm", message, *args)
        return self.git(self.source, "rev-parse", "HEAD").strip()

    def source_files(self):
        return {
            str(path.relative_to(self.repo)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in self.repo.rglob("*") if path.is_file()
        }

    def run_example(self, name, *args):
        return subprocess.run(
            [str(EXAMPLES / name), *map(str, args)],
            text=True, capture_output=True, timeout=60,
        )

    def test_reachable_artifacts_and_native_symbol_oracle(self):
        baseline = None
        for backend, jobs in (("cli", 1), ("gix", 2)):
            with self.subTest(backend=backend, jobs=jobs):
                output = self.root / backend
                result = self.run_example("temporal_reference", self.repo, output, jobs, backend)
                self.assertEqual(result.returncode, 0, result.stderr)
                summary = json.loads(result.stdout)
                self.assertEqual(summary, json.loads((output / "temporal-reference-result.json").read_text()))
                self.assertIs(summary["complete"], True)
                self.assertEqual(summary["head"], self.head)
                self.assertEqual(summary["backend"], backend)
                self.assertEqual(summary["jobs"], jobs)
                self.assertEqual(summary["commits"], len(self.expected))
                index = json.loads((output / "commit-index.json").read_text())
                self.assertEqual(index["walk_strategy"], "reachable")
                self.assertEqual(index["refs"], {"HEAD": self.head})
                self.assertTrue(index["indexed_at"])
                commits = {row["sha"]: row for row in index["commits"]}
                self.assertEqual(set(commits), self.expected)
                self.assertIn(self.side_commit, commits)
                self.assertIn(self.empty_commit, commits)
                self.assertNotIn(self.unreachable_commit, commits)
                self.assertEqual(commits[self.root_commit]["parents"], [])
                self.assertEqual(len(commits[self.head]["parents"]), 2)
                manifest = json.loads((output / "manifest.json").read_text())
                self.assertIs(manifest["complete"], True)
                self.assertEqual(manifest["indexed_commit_oid"], self.head)
                for key in ("commits", "symbol_snapshots", "temporal_edges", "diagnostics"):
                    self.assertEqual(summary[key], manifest["row_counts"][key])
                self.assertGreater(summary["symbol_snapshots"], 0)
                self.assertGreater(summary["temporal_edges"], summary["symbol_snapshots"])
                for parquet in output.rglob("*.parquet"):
                    with parquet.open("rb") as stream:
                        self.assertEqual(stream.read(4), b"PAR1", parquet)
                for seconds in summary["phase_seconds"].values():
                    self.assertGreaterEqual(seconds, 0)
                stable = (commits, manifest["row_counts"], manifest["temporal_shards"])
                if baseline is None:
                    baseline = stable
                else:
                    self.assertEqual(stable, baseline)

        requests = self.root / "commits.jsonl"
        requests.write_text("".join(json.dumps({"commit_oid": oid, "ignored": True}) + "\n"
                                  for oid in self.git(self.repo, "rev-list", "--reverse", "HEAD").split()))
        output = self.root / "symbols.jsonl"
        result = self.run_example("temporal_symbol_oracle", self.repo, requests, output)
        self.assertEqual(result.returncode, 0, result.stderr)
        summary = json.loads(result.stdout)
        self.assertEqual(summary, json.loads(Path(str(output) + ".summary.json").read_text()))
        self.assertIs(summary["complete"], True)
        self.assertEqual(summary["requested_commits"], len(self.expected))
        self.assertEqual(summary["completed_commits"], len(self.expected))
        rows = [json.loads(line) for line in output.read_text().splitlines()]
        self.assertEqual(summary["symbol_changes"], len(rows))
        self.assertEqual(len(rows), baseline[1]["symbol_snapshots"])
        self.assertEqual(summary["zero_symbol_commits"], 1)
        for row in rows:
            self.assertEqual(set(row), {"commit_oid", "parent_sha", "snapshot", "change_kind"})
            self.assertIn(row["commit_oid"], self.expected)
            self.assertIn(row["parent_sha"], commits[row["commit_oid"]]["parents"] or [None])
            self.assertEqual(row["snapshot"]["key"]["commit"], row["commit_oid"])
        self.assertFalse(any(row["commit_oid"] == self.empty_commit for row in rows))
        self.assertTrue(any(row["commit_oid"] == self.root_commit and row["parent_sha"] is None for row in rows))
        self.assertTrue(any(row["commit_oid"] == self.rename_commit and "renamed_from" in row["change_kind"]
                            for row in rows if isinstance(row["change_kind"], dict)))
        self.assertEqual({row["parent_sha"] for row in rows if row["commit_oid"] == self.head},
                         set(commits[self.head]["parents"]))
        self.assertEqual(Path(str(output) + ".diagnostics.jsonl").read_text(), "")
        self.assertEqual(self.source_files(), self.source_before)

    def test_invalid_backend_and_jobs_do_not_create_output(self):
        for jobs, backend in (("0", "cli"), ("1", "unknown")):
            output = self.root / (jobs + backend)
            result = self.run_example("temporal_reference", self.repo, output, jobs, backend)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_existing_output_and_invalid_oracle_rows_cannot_claim_completion(self):
        existing = self.root / "existing"
        existing.mkdir()
        marker = existing / "keep"
        marker.write_text("sentinel")
        result = self.run_example("temporal_reference", self.repo, existing, 1, "cli")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(marker.read_text(), "sentinel")
        for body in ('{"commit_oid":"invalid"}\n', '{"wrong_field":"HEAD"}\n',
                     json.dumps({"commit_oid": "0" * 40}) + "\n"):
            requests = self.root / "invalid.jsonl"
            requests.write_text(body)
            output = self.root / ("invalid-" + str(len(body)) + ".jsonl")
            result = self.run_example("temporal_symbol_oracle", self.repo, requests, output)
            self.assertNotEqual(result.returncode, 0)
            sidecar = Path(str(output) + ".summary.json")
            self.assertFalse(sidecar.exists() and json.loads(sidecar.read_text())["complete"])
        self.assertEqual(self.source_files(), self.source_before)

    def test_oracle_diagnostics_are_separate_from_symbol_rows(self):
        (self.source / "binary.py").write_bytes(b"\x00binary\n")
        commit = self.commit("binary")
        requests = self.root / "binary-request.jsonl"
        requests.write_text(json.dumps({"commit_oid": commit}) + "\n")
        output = self.root / "binary-symbols.jsonl"
        result = self.run_example("temporal_symbol_oracle", self.source, requests, output)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_text(), "")
        summary = json.loads(result.stdout)
        self.assertIs(summary["complete"], True)
        self.assertEqual(summary["diagnostics"], 1)
        self.assertEqual(summary["zero_symbol_commits"], 1)
        diagnostics = [json.loads(line) for line in
                       Path(str(output) + ".diagnostics.jsonl").read_text().splitlines()]
        self.assertEqual(len(diagnostics), 1)
        self.assertEqual(diagnostics[0]["commit_oid"], commit)
        self.assertIn("binary_blob", diagnostics[0]["diagnostic"])
        self.assertEqual(self.source_files(), self.source_before)


if __name__ == "__main__":
    unittest.main()
