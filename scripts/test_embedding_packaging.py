"""Exercise packaging commands with local fixtures; never compile or publish."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class WrapperPackaging(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="spur packaging ü ")
        self.root = Path(self.tmp.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copy(ROOT / "scripts/spur-cargo", scripts)
        self.bin = self.root / "fake tools"
        self.bin.mkdir()
        self.log = self.root / "commands.jsonl"
        cargo = self.bin / "cargo"
        cargo.write_text("#!/usr/bin/env python3\nimport json, os, sys\nwith open(os.environ['PACKAGING_LOG'], 'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n")
        cargo.chmod(0o755)
        self.env = dict(os.environ, SPUR_REMOTE="0", SPUR_SCCACHE_S3="0",
                        PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        PACKAGING_LOG=str(self.log))

    def tearDown(self):
        self.tmp.cleanup()

    def run_wrapper(self, *args):
        result = subprocess.run([str(self.root / "scripts/spur-cargo"), *args],
                                cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_default_host_build_includes_normal_helper(self):
        commands = self.run_wrapper("build", "-p", "spur-cli", "--bin", "spur", "--locked")
        args = commands[-1]
        self.assertIn("spur-embedding-worker", args)
        self.assertNotIn("--no-default-features", args)
        self.assertNotIn("test-support", args)

    def test_default_run_builds_sibling_before_original_command(self):
        commands = self.run_wrapper("run", "--release", "-p", "spur-cli", "--", "--help")
        self.assertEqual(commands[-1], ["run", "--release", "-p", "spur-cli", "--", "--help"])
        self.assertTrue(any(c[0] == "build" and "spur-embedding-worker" in c
                            and "--release" in c for c in commands[:-1]), commands)

    def test_unrelated_build_keeps_exact_arguments(self):
        self.assertEqual(self.run_wrapper("build", "-p", "spur-cost"),
                         [["build", "-p", "spur-cost"]])

    def test_source_install_places_both_binaries_under_requested_root(self):
        commands = self.run_wrapper("install", "--path", "crates/spur-cli", "--root", "private install ü", "--bin", "spur", "--locked")
        self.assertEqual(commands[-1], ["install", "--path", "crates/spur-cli", "--root", "private install ü", "--bin", "spur", "--locked"])
        self.assertTrue(any("crates/spur-embedding-worker" in c and "private install ü" in c
                            and "spur-embedding-worker" in c for c in commands[:-1]), commands)


class ReleasePackaging(unittest.TestCase):
    def test_active_release_checks_sibling_and_checksum_before_publication(self):
        workflow = (ROOT / ".github/workflows/release-dist.yml").read_text()
        smoke = workflow.split("      - name: Smoke-test", 1)[1].split("      - name: Upload", 1)[0]
        self.assertIn("spur-embedding-worker", smoke)
        self.assertIn("--compatibility", smoke)
        self.assertIn("SHA256SUMS", smoke)

    def test_bundle_preserves_generic_build_transport_without_publishing(self):
        with tempfile.TemporaryDirectory(prefix="spur bundle ü ") as tmp:
            root = Path(tmp)
            scripts = root / "scripts"
            cloud = scripts / "cloud-build"
            cloud.mkdir(parents=True)
            shutil.copy(ROOT / "scripts/cloud-build-publish-bundle.sh", scripts)
            (cloud / "build.sh").write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            (cloud / "build.sh").chmod(0o755)
            (cloud / "config.env").write_text('SCCACHE_BUCKET=fixture\nAWS_REGION=fixture\n')
            (cloud / "secret.local.env").write_text("must not ship")
            fake = root / "bin"
            fake.mkdir()
            aws = fake / "aws"
            aws.write_text('#!/bin/sh\n[ "$1 $2" = "s3 cp" ] || exit 2\ncp "$3" "$PACKAGING_BUNDLE"\n')
            aws.chmod(0o755)
            bundle = root / "bundle.tgz"
            result = subprocess.run([str(scripts / "cloud-build-publish-bundle.sh")],
                                    env=dict(os.environ, PATH=str(fake)+os.pathsep+os.environ['PATH'],
                                             PACKAGING_BUNDLE=str(bundle)), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            import tarfile
            with tarfile.open(bundle) as tar:
                self.assertIn("cloud-build/build.sh", tar.getnames())
                self.assertNotIn("cloud-build/secret.local.env", tar.getnames())


if __name__ == "__main__":
    unittest.main()
