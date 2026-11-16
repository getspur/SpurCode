"""Release orchestration contracts; child failures use real subprocesses."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


CRATE = Path(__file__).resolve().parents[1]
DRIVER = CRATE / "scripts/check_release.py"


class DriverFixture:
    def setUp(self):
        self.assertTrue(DRIVER.is_file(), "checked release entry point must exist")
        spec = importlib.util.spec_from_file_location("check_release", DRIVER)
        self.driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.driver)
        self.temp = tempfile.TemporaryDirectory(prefix="release-driver-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)


class DriverTests(DriverFixture, unittest.TestCase):
    def test_remote_target_path_does_not_embed_host_filesystem(self):
        self.assertTrue(callable(getattr(self.driver, "rust_test_arguments", None)),
                        "remote Rust target paths must be portable")
        remote = self.driver.rust_test_arguments(self.root, "remote")
        target = Path(remote[remote.index("--target-dir") + 1])
        self.assertFalse(target.is_absolute(), target)
        self.assertIn(self.root.name, str(target))
        local = self.driver.rust_test_arguments(self.root, "host")
        self.assertTrue(Path(local[local.index("--target-dir") + 1]).is_relative_to(self.root))

    def test_child_failure_preserves_raw_exit_and_stops_later_phases(self):
        marker = self.root / "must-not-run"
        phases = [("failed-child", [sys.executable, "-c", "print('child receipt'); raise SystemExit(23)"]),
                  ("later", [sys.executable, "-c", f"open({str(marker)!r},'w').close()"])]
        receipt = {"commands": []}
        code = self.driver.run_phases(phases, self.root, dict(os.environ), receipt)
        self.assertEqual(code, 23)
        self.assertFalse(marker.exists())
        self.assertEqual(receipt["commands"][0]["returncode"], 23)
        self.assertIn("child receipt", (self.root / "failed-child.stdout").read_text())

    def test_missing_runtime_fails_cli_preflight(self):
        process = subprocess.run([sys.executable, "-S", str(DRIVER), "--out", str(self.root / "missing")],
                                 capture_output=True, text=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("DuckDB exactly 2.0.0", process.stderr)
        receipt = json.loads((self.root / "missing/receipt.json").read_text())
        self.assertEqual(receipt["status"], "failed")

    def test_wrong_runtime_fails_cli_preflight(self):
        (self.root / "duckdb.py").write_text("__version__ = '1.4.0'\n")
        process = subprocess.run([sys.executable, str(DRIVER), "--out", str(self.root / "wrong")],
                                 env=dict(os.environ, PYTHONPATH=str(self.root)), capture_output=True, text=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn("DuckDB exactly 2.0.0", process.stderr)

    def test_existing_output_is_refused_without_overwriting(self):
        receipt = self.root / "receipt.json"
        receipt.write_text("historical evidence")
        process = subprocess.run([sys.executable, str(DRIVER), "--out", str(self.root)],
                                 capture_output=True, text=True)
        self.assertNotEqual(process.returncode, 0)
        self.assertEqual(receipt.read_text(), "historical evidence")

    def test_missing_native_oracle_fails_preflight(self):
        with self.assertRaisesRegex(ValueError, "native.*oracle|oracle.*native"):
            self.driver.native_environment({}, self.root)

    def test_unittest_skip_is_a_release_failure(self):
        tests = self.root / "tests"
        tests.mkdir()
        (tests / "test_skipped.py").write_text(
            "import unittest\nclass T(unittest.TestCase):\n"
            " @unittest.skip('missing oracle')\n def test_oracle(self): pass\n")
        code = self.driver.run_unittests(tests, self.root / "summary.json")
        self.assertNotEqual(code, 0)
        self.assertEqual(json.loads((self.root / "summary.json").read_text())["skipped"], 1)

    def test_unittest_failure_is_a_release_failure(self):
        tests = self.root / "tests"
        tests.mkdir()
        (tests / "test_failed.py").write_text(
            "import unittest\nclass T(unittest.TestCase):\n"
            " def test_child(self): self.fail('controlled failure')\n")
        self.assertNotEqual(self.driver.run_unittests(tests, self.root / "summary.json"), 0)

    def test_empty_unittest_discovery_is_a_release_failure(self):
        tests = self.root / "tests"
        tests.mkdir()
        self.assertNotEqual(self.driver.run_unittests(tests, self.root / "summary.json"), 0)


class ReleaseManifestTests(DriverFixture, unittest.TestCase):
    # Keep native tests separate so driver failure/preflight checks run without DuckDB.
    def release(self):
        extension = Path(os.environ["SPUR_GRAPH_EXTENSION"])
        return extension, dict(manifest=os.environ["SPUR_RELEASE_MANIFEST"],
                               manifest_sha256=os.environ["SPUR_RELEASE_MANIFEST_SHA256"],
                               source_identity=os.environ["SPUR_RELEASE_SOURCE_IDENTITY"])

    def test_explicit_current_manifest_is_verified(self):
        extension, trust = self.release()
        self.assertEqual(self.driver.approved_extension_sha(extension, **trust),
                         hashlib.sha256(extension.read_bytes()).hexdigest())

    def test_current_manifest_checksum_and_source_mismatches_are_rejected(self):
        extension, trust = self.release()
        for key, value in (("manifest_sha256", "0" * 64),
                           ("source_identity", "source-sha256-v1:" + "0" * 64)):
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "manifest|source"):
                self.driver.approved_extension_sha(extension, **dict(trust, **{key: value}))

    def test_current_binary_cannot_self_approve_without_manifest(self):
        extension, _ = self.release()
        with self.assertRaisesRegex(ValueError, "unapproved"):
            self.driver.approved_extension_sha(extension)

    def test_tampered_binary_is_rejected_with_original_manifest(self):
        extension, trust = self.release()
        tampered = self.root / extension.name
        tampered.write_bytes(extension.read_bytes() + b"tampered")
        with self.assertRaisesRegex(ValueError, "manifest|artifact|checksum"):
            self.driver.approved_extension_sha(tampered, **trust)


if __name__ == "__main__":
    unittest.main()
