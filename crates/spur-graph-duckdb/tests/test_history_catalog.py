"""Verify the all-parent bulk Git manifest against an independent Git fixture."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.environ.get("SPUR_HISTORY_CATALOG"), "build the history_catalog example")
class HistoryCatalogTests(unittest.TestCase):
    def test_merge_empty_commit_and_rename_preserve_every_comparison(self):
        executable = Path(os.environ["SPUR_HISTORY_CATALOG"])
        self.assertTrue(executable.is_file(), "history_catalog helper must be implemented and built")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / "repo"
            repo.mkdir()

            def git(*args):
                return subprocess.check_output(
                    ["git", "-C", str(repo), "-c", "user.name=Fixture",
                     "-c", "user.email=fixture@example.invalid", *args], text=True,
                ).strip()

            git("init", "-q")
            (repo / "old.cpp").write_text("int alpha() { return 1; }\n")
            git("add", ".")
            git("commit", "-qm", "root")
            branch = git("symbolic-ref", "--short", "HEAD")
            git("commit", "--allow-empty", "-qm", "empty")
            git("checkout", "-qb", "side")
            (repo / "old.cpp").write_text("int alpha() { return 2; }\n")
            git("commit", "-qam", "side")
            git("checkout", "-q", branch)
            (repo / "note.md").write_text("# Note\n")
            git("add", ".")
            git("commit", "-qm", "main")
            git("merge", "-q", "--no-ff", "side", "-m", "merge")
            git("mv", "old.cpp", "name\nfile.cpp")
            git("commit", "-qm", "rename")
            head = git("rev-parse", "HEAD")
            output = root / "catalog"
            result = subprocess.run([str(executable), str(repo), head, str(output)], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            commits = [json.loads(line) for line in (output / "commits.jsonl").read_text().splitlines()]
            comparisons = [json.loads(line) for line in (output / "comparisons.jsonl").read_text().splitlines()]
            changes = [json.loads(line) for line in (output / "changes.jsonl").read_text().splitlines()]
            expected = set()
            for line in git("rev-list", "--parents", head).splitlines():
                commit, *parents = line.split()
                expected.update((commit, parent) for parent in parents or [None])
            self.assertEqual({(r["commit_oid"], r["parent_oid"]) for r in comparisons}, expected)
            self.assertEqual(len(comparisons), len(expected))
            self.assertEqual({r["commit_oid"] for r in commits}, {r[0] for r in expected})
            renames = [r for r in changes if r["status"].startswith("R")]
            self.assertEqual(len(renames), 1)
            self.assertEqual(renames[0]["old_path"], "old.cpp")
            self.assertEqual(renames[0]["path"], "name\nfile.cpp")
            self.assertEqual(renames[0]["language"], "Cpp")
            self.assertEqual(len({r["change_id"] for r in changes}), len(changes))
            with self.subTest("completion marker identifies the frozen revision"):
                summary = json.loads((output / "inventory-summary.json").read_text())
                self.assertEqual(summary["head"], head)
            with self.subTest("completed inventory cannot be overwritten"):
                original = (output / "changes.jsonl").read_bytes()
                repeated = subprocess.run([str(executable), str(repo), head, str(output)], capture_output=True, text=True, timeout=30)
                self.assertNotEqual(repeated.returncode, 0)
                self.assertEqual((output / "changes.jsonl").read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
