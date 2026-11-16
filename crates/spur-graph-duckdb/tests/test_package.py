"""Real native packaging acceptance. Requires SPUR_GRAPH_LIBRARY from a build receipt.

Run with the existing pinned DuckDB 2.0.0 Python environment. No installation,
source rebuild, or substitute library is performed by this suite.
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
from unittest.mock import patch

import duckdb


CRATE = Path(__file__).resolve().parents[1]
PACKAGER = CRATE / 'scripts/package_extension.py'
VERIFIER = CRATE / 'scripts/verify_package.py'


class PackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if duckdb.__version__ != '2.0.0':
            raise AssertionError('native package acceptance requires DuckDB exactly 2.0.0')
        cls.library = Path(os.environ['SPUR_GRAPH_LIBRARY']).resolve(strict=True)

    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='spur-package-r4-')
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.out = self.root / 'release/spur_graph.duckdb_extension'

    def package(self, library=None, out=None, script=PACKAGER):
        return subprocess.run([sys.executable, str(script), '--library',
                               str(library or self.library), '--out', str(out or self.out)],
                              capture_output=True, text=True)

    def completed_package(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        sidecar = Path(str(self.out) + '.json')
        self.assertTrue(sidecar.is_file(), 'a loaded package needs its versioned sidecar')
        return json.loads(sidecar.read_text())

    def verifier(self):
        self.assertTrue(VERIFIER.is_file(), 'independent native package verifier is required')
        spec = importlib.util.spec_from_file_location('package_verifier', VERIFIER)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def copied_package(self):
        self.completed_package()
        copy = self.root / 'copy/spur_graph.duckdb_extension'
        copy.parent.mkdir()
        shutil.copyfile(self.out, copy)
        shutil.copyfile(Path(str(self.out) + '.json'), Path(str(copy) + '.json'))
        return copy

    def rewrite_manifest(self, binary, edit):
        path = Path(str(binary) + '.json')
        manifest = json.loads(path.read_text())
        edit(manifest)
        path.write_text(json.dumps(manifest))

    def test_real_package_has_bound_identity_and_no_persisted_readiness_claim(self):
        manifest = self.completed_package()
        verified = self.verifier().verify_package(self.out)
        self.assertEqual(verified, manifest)
        self.assertEqual(manifest['format_version'], 1)
        self.assertEqual(manifest['duckdb_api_version'], '2.0.0')
        self.assertEqual(manifest['abi'], 'C_STRUCT_UNSTABLE')
        self.assertEqual(manifest['trust'], {'mode': 'unsigned_local', 'signature': 'absent'})
        self.assertEqual(manifest['source_identity'], manifest['extractor_id'])
        self.assertRegex(manifest['source_identity'], r'^source-sha256-v1:[0-9a-f]{64}$')
        self.assertNotIn('ready', manifest)
        self.assertNotIn('live_readiness', manifest)
        self.assertEqual(manifest['binary']['sha256'], hashlib.sha256(self.out.read_bytes()).hexdigest())
        self.assertEqual(manifest['binary']['size_bytes'], self.out.stat().st_size)
        with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
            con.execute("LOAD '" + str(self.out.resolve()).replace("'", "''") + "'")
            info = con.execute('SELECT spur_graph_info()').fetchone()[0]
            for key, value in info.items():
                self.assertEqual(manifest[key], value)
            self.assertEqual(manifest['platform'], con.execute('PRAGMA platform').fetchone()[0])
        result = subprocess.run([sys.executable, str(VERIFIER), str(self.out)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['artifact'], manifest)
        self.assertEqual(report['live_readiness']['status'], 'verified_now')

    def test_packaging_does_not_use_checkout_version_or_source_files(self):
        isolated = self.root / 'unrelated/scripts'
        isolated.mkdir(parents=True)
        shutil.copyfile(PACKAGER, isolated / PACKAGER.name)
        if VERIFIER.exists():
            shutil.copyfile(VERIFIER, isolated / VERIFIER.name)
        (isolated.parent / 'Cargo.toml').write_text('[package]\nversion = "9.9.9"\n')
        result = self.package(script=isolated / PACKAGER.name)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        sidecar = Path(str(self.out) + '.json')
        self.assertTrue(sidecar.is_file(), 'artifact metadata must be probed without source-tree inputs')
        self.assertEqual(json.loads(sidecar.read_text())['extension_version'], '0.1.0')

    def test_bad_library_is_not_published(self):
        bad = self.root / 'bad.dylib'
        bad.write_bytes(b'this is not a native extension')
        result = self.package(library=bad)
        self.assertNotEqual(result.returncode, 0, 'packaging must really load its staged library')
        self.assertIn('load', result.stderr.lower())
        self.assertFalse(self.out.exists())
        self.assertFalse(Path(str(self.out) + '.json').exists())

    def test_failed_repackage_preserves_last_complete_generation(self):
        manifest = self.completed_package()
        old = self.out.resolve()
        old_bytes = old.read_bytes()
        bad = self.root / 'bad.dylib'
        bad.write_bytes(b'broken native library')
        result = self.package(library=bad)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.out.resolve(), old)
        self.assertEqual(old.read_bytes(), old_bytes)
        self.assertEqual(self.verifier().verify_package(self.out), manifest)

    def test_successful_repackage_retains_old_complete_generation(self):
        manifest = self.completed_package()
        old = self.out.resolve()
        old_sidecar = Path(str(old) + '.json').read_bytes()
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(self.out.resolve(), old)
        self.assertEqual(self.verifier().verify_package(self.out), manifest)
        self.assertEqual(self.verifier().verify_package(old), manifest)
        self.assertEqual(Path(str(old) + '.json').read_bytes(), old_sidecar)

    def test_unmanaged_existing_output_is_not_destroyed(self):
        self.out.parent.mkdir()
        self.out.write_bytes(b'previous unowned release')
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.out.read_bytes(), b'previous unowned release')

    def test_checksum_mismatch_rejected_before_native_load(self):
        copy = self.copied_package()
        data = bytearray(copy.read_bytes())
        data[0] ^= 1
        copy.write_bytes(data)
        # Even without a compatible runtime, tampering must be diagnosed first.
        module = self.verifier()
        with patch.object(module.duckdb, '__version__', '1.0.0'):
            with self.assertRaisesRegex(ValueError, 'SHA256|checksum'):
                module.verify_package(copy)

    def test_missing_binary_sidecar_and_malformed_manifest_rejected(self):
        copy = self.copied_package()
        module = self.verifier()
        sidecar = Path(str(copy) + '.json')
        good = sidecar.read_bytes()
        sidecar.unlink()
        with self.assertRaisesRegex(ValueError, 'sidecar|manifest'):
            module.verify_package(copy)
        sidecar.write_text('{broken json')
        with self.assertRaisesRegex(ValueError, 'sidecar|manifest'):
            module.verify_package(copy)
        sidecar.write_bytes(good)
        copy.unlink()
        with self.assertRaisesRegex(ValueError, 'binary|artifact'):
            module.verify_package(copy)

    def test_inconsistent_manifest_and_footer_rejected(self):
        copy = self.copied_package()
        module = self.verifier()
        sidecar = Path(str(copy) + '.json')
        original = sidecar.read_bytes()
        cases = [
            ('format', lambda m: m.update(format_version=999)),
            ('size', lambda m: m['binary'].update(size_bytes=0)),
            ('binary', lambda m: m['binary'].update(name='../other.duckdb_extension')),
            ('trust|unsigned', lambda m: m.update(trust={'mode': 'signed', 'signature': 'fake'})),
            ('ABI|abi', lambda m: m.update(abi='C_STRUCT')),
            ('version|runtime', lambda m: m.update(duckdb_api_version='1.0.0')),
            ('version|footer', lambda m: m.update(extension_version='9.9.9')),
            ('platform', lambda m: m.update(platform='unavailable_platform')),
            ('source|identity', lambda m: m.update(source_identity='source-sha256-v1:' + '0' * 64)),
            ('info|identity|metadata', lambda m: m.update(extractor_id='source-sha256-v1:' + '0' * 64,
                                                         source_identity='source-sha256-v1:' + '0' * 64)),
            ('info|schema|metadata', lambda m: m.update(facts_schema_version=99)),
        ]
        for message, edit in cases:
            with self.subTest(message=message):
                sidecar.write_bytes(original)
                self.rewrite_manifest(copy, edit)
                with self.assertRaisesRegex(ValueError, message):
                    module.verify_package(copy)

    def test_corrupt_footer_is_rejected_even_with_updated_checksum(self):
        copy = self.copied_package()
        data = bytearray(copy.read_bytes())
        data[-1] = 1  # Claims unsigned, but carries a nonzero signature.
        copy.write_bytes(data)
        self.rewrite_manifest(copy, lambda m: m['binary'].update(sha256=hashlib.sha256(data).hexdigest()))
        with self.assertRaisesRegex(ValueError, 'signature|unsigned'):
            self.verifier().verify_package(copy)

    def test_incompatible_runtime_and_platform_rejected_explicitly(self):
        self.completed_package()
        module = self.verifier()
        # Mechanics only: these runtimes/platforms are unavailable on this host.
        with patch.object(module.duckdb, '__version__', '1.0.0'):
            with self.assertRaisesRegex(ValueError, 'runtime.*2.0.0'):
                module.verify_package(self.out)
        with patch.object(module, 'runtime_contract', return_value=('v2.0.0', 'unavailable_platform')):
            with self.assertRaisesRegex(ValueError, 'platform'):
                module.verify_package(self.out)

    def test_publication_filesystem_failure_leaves_previous_release_readable(self):
        manifest = self.completed_package()
        old = self.out.resolve()
        # Real filesystem fault: the stable sidecar alias is occupied by a directory.
        alias = Path(str(self.out) + '.json')
        target = os.readlink(alias)
        alias.unlink()
        alias.mkdir()
        try:
            result = self.package()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.out.resolve(), old)
            self.assertEqual(self.verifier().verify_package(old), manifest)
        finally:
            alias.rmdir()
            alias.symlink_to(target)
        self.assertEqual(self.verifier().verify_package(self.out), manifest)


if __name__ == '__main__':
    unittest.main()
