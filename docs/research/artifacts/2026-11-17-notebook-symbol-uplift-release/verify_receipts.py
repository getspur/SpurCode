"""Verify retained U5 receipts, immutable outputs and original evidence bytes."""
import hashlib
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def main():
    phases = {}
    for path in HERE.glob('*.json'):
        record = json.loads(path.read_bytes())
        if isinstance(record, dict) and 'stdout_sha256' in record:
            assert sha(path.with_suffix('.stdout')) == record['stdout_sha256'], path
            assert sha(path.with_suffix('.stderr')) == record['stderr_sha256'], path
            phases[path.stem] = record['exit']
    assert phases['driver-red'] == phases['fixture-sync-red'] == 1
    assert phases['driver-green'] == phases['fixture-sync-green'] == 0
    assert phases['release-remote'] == 200
    assert phases['release-network'] == 1
    assert phases['release-stable'] == 101
    assert phases['release-final'] == phases['native-acceptance'] == 0
    assert phases['native-names-acceptance'] == 0
    for path in HERE.glob('*/archive.json'):
        archive = json.loads(path.read_bytes())
        for relative, digest in archive['files'].items():
            assert sha(path.parent / relative) == digest
    for phase in ['fixture-sync-green', 'release-final', 'native-names-acceptance']:
        record = json.loads((HERE / (phase + '.json')).read_bytes())
        for relative, digest in record['source_sha256'].items():
            # Acceptance authoring is independent of driver/compiler identity.
            # The final invocation below binds its exact bytes separately.
            if relative.endswith('check_native_notebook.py') and phase != 'native-names-acceptance':
                continue
            assert sha(ROOT / relative) == digest, (phase, relative)
    release = json.loads((HERE / 'driver/receipt.json').read_bytes())
    assert release['status'] == 'passed' and release['rust_tests_mode'] == 'remote'
    assert release['unittests']['release_pass']
    assert release['unittests']['skipped'] == release['unittests']['expected_failures'] == 0
    assert all(p['returncode'] == 0 for p in release['commands'])
    assert '61 passed; 0 failed; 0 ignored' in (HERE / 'driver/core-notebook-tests.stdout').read_text()
    assert '4 passed; 0 failed; 0 ignored' in (HERE / 'driver/rust-tests.stdout').read_text()
    driver = module(ROOT / 'crates/spur-graph-duckdb/scripts/check_release.py', 'verified_driver')
    source = json.loads((HERE / 'driver/source-probe-final/spur_graph_source_identity.json').read_bytes())
    assert driver.verify_source(source) == release['release_trust']['source_identity']
    for item in release['native_artifacts'].values():
        assert sha(item['path']) == item['sha256']
    manifest = Path(release['release_trust']['manifest'])
    assert sha(manifest) == release['release_trust']['manifest_sha256']
    assert sha(manifest.with_suffix('')) == release['package']['binary']['sha256']
    original = json.loads((HERE / 'preservation-before.json').read_bytes())
    for relative, digest in original['files'].items():
        assert sha(ROOT / relative) == digest, relative
    for path, digest in original['binaries'].items():
        assert sha(path) == digest, path
    for path, digest in json.loads((HERE / 'prior-native-before.json').read_bytes()).items():
        assert sha(path) == digest, path
    native = json.loads((HERE / 'native-names/evidence.json').read_bytes())
    assert native['status'] == 'passed' and native['function_anchors_checked'] == 28
    assert native['symbol_refs_checked'] == 70 and native['pointer_preserved_on_failures']
    assert native['documented_schema_sha256'] == sha(ROOT / 'crates/spur-graph-duckdb/docs/release-contract.md')
    assert native['acceptance_sha256'] == sha(HERE / 'check_native_notebook.py')
    assert native['counts']['symbol_changes'] > 0 and native['counts']['cell_changes'] > 0
    assert [x['functions'] for x in native['snapshots']] == [17, 11]
    assert native['package_sha256'] == release['package']['binary']['sha256']
    for child in native['children']:
        prefix = HERE / 'native-names' / child['stream_prefix']
        assert sha(str(prefix) + '.stdout') == child['stdout_sha256']
        assert sha(str(prefix) + '.stderr') == child['stderr_sha256']
    app = module(ROOT / 'crates/spur-graph-duckdb/examples/app_consumer.py', 'verified_consumer')
    output = Path(native['children'][0]['cwd']).parent.parent
    for item in native['snapshots']:
        assert app.read_result(output, item['snapshot_id'], run_id=native['manifest']['run_id'])['manifest'] == native['manifest']
        assert app.read_result(output, item['snapshot_id'])['manifest'] == native['final_manifest']
    print(json.dumps(dict(status='passed', phases=phases, rust='REMOTE 61 core + 4 adapter',
        native_suite=release['unittests'], extractor_id=driver.verify_source(source),
        package_sha256=native['package_sha256'], sidecar_sha256=native['sidecar_sha256'],
        preserved_prior_files=len(original['files']), preserved_historical_package=original['binaries'],
        source_inputs=len(source['inputs']), real_counts=native['counts'],
        anchors=28, refs=70, old_and_current_runs_reopened=True), indent=2))


if __name__ == '__main__':
    main()
