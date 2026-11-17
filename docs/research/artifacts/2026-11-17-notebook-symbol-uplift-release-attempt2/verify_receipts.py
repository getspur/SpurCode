"""Check new documentation binding, unchanged production/artifacts and old receipts.

Historical documentation hashes bind to their original commit, not today's prose.
No native build or Rust test is run by this verifier.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.with_name('2026-11-17-notebook-symbol-uplift-release')
CONTRACT = 'crates/spur-graph-duckdb/docs/release-contract.md'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_bytes())


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def main():
    before = read(HERE / 'preservation-before.json')
    prior_commit = before['prior_commit']
    old_contract = subprocess.check_output(['git', 'show', f'{prior_commit}:{CONTRACT}'], cwd=ROOT)
    old_claim = (b'snapshot/repository labels. The reader verifies one immutable manifest, native\n'
        b'and consumer contracts, table schemas/counts/checksums and exact source context.\n')
    corrected = (b'snapshot/repository labels. `read_result` checks the declared consumer/native\n'
        b'contract metadata and source identity scheme, exact manifest/run/snapshot\n'
        b'context, and exported file paths, byte sizes and checksums. Native publication\n'
        b'records table schemas and row counts; `read_result` does not revalidate them.\n')
    # Exact prose-only delta also preserves every public schema/signature/example.
    assert old_contract.count(old_claim) == 1
    assert (ROOT / CONTRACT).read_bytes() == old_contract.replace(old_claim, corrected)
    changed = subprocess.check_output(['git', 'diff', prior_commit, '--name-only', '--', 'crates'], cwd=ROOT, text=True).splitlines()
    assert changed == [CONTRACT], changed
    for commit in ('fe06191df', '43a96744d', '779b5a1e6', '5545f6d1e', '040dc7ba6'):
        subprocess.run(['git', 'merge-base', '--is-ancestor', commit, 'HEAD'], cwd=ROOT, check=True)
    for path, digest in before['files'].items():
        assert sha(ROOT / path) == digest, path
        assert hashlib.sha256(subprocess.check_output(['git', 'show', f'{prior_commit}:{path}'], cwd=ROOT)).hexdigest() == digest
    for path, digest in before['artifact_files'].items():
        assert sha(path) == digest, path
    historical = read(PRIOR / 'preservation-before.json')
    for path, digest in historical['files'].items():
        assert sha(ROOT / path) == digest, path
    for collection in (historical['binaries'], read(PRIOR / 'prior-native-before.json')):
        for path, digest in collection.items():
            assert sha(path) == digest, path
    phases = {}
    for directory in (PRIOR, HERE):
        for path in sorted(directory.glob('*.json')):
            record = read(path)
            if isinstance(record, dict) and 'stdout_sha256' in record:
                assert sha(path.with_suffix('.stdout')) == record['stdout_sha256'], path
                assert sha(path.with_suffix('.stderr')) == record['stderr_sha256'], path
                if directory == HERE:
                    phases[path.stem] = record['exit']
                    assert record['exit'] == 0, path
                    assert record['package_sha256'] == 'c5f36a0f99213b570c1fe862561f3bb8882f40936c0afe6565f46ceca1878d48'
                    for relative, digest in record['source_sha256'].items():
                        assert sha(ROOT / relative) == digest, (path, relative)
        for path in sorted(directory.glob('*/archive.json')):
            archive = read(path)
            for relative, digest in archive['files'].items():
                assert sha(path.parent / relative) == digest, path
    assert {'examples-extraction', 'notebook-schema', 'ordinary-signatures', 'documented-python',
            'documented-sql', 'native-acceptance', 'archive-native'} <= phases.keys()
    release = read(PRIOR / 'driver/receipt.json')
    assert release['status'] == 'passed' and release['rust_tests_mode'] == 'remote'
    assert release['unittests']['tests_run'] == 223 and release['unittests']['release_pass']
    assert release['unittests']['skipped'] == release['unittests']['expected_failures'] == 0
    assert all(command['returncode'] == 0 for command in release['commands'])
    assert '61 passed; 0 failed; 0 ignored' in (PRIOR / 'driver/core-notebook-tests.stdout').read_text()
    assert '4 passed; 0 failed; 0 ignored' in (PRIOR / 'driver/rust-tests.stdout').read_text()
    driver = module(ROOT / 'crates/spur-graph-duckdb/scripts/check_release.py', 'verified_driver')
    source = read(PRIOR / 'driver/source-probe-final/spur_graph_source_identity.json')
    source_id = driver.verify_source(source)
    assert len(source['inputs']) == 325 and source_id == release['release_trust']['source_identity']
    for value in release['native_artifacts'].values():
        assert sha(value['path']) == value['sha256']
    package = Path(release['release_trust']['manifest']).with_suffix('')
    assert driver.approved_extension_sha(package, **release['release_trust']) == release['package']['binary']['sha256']
    old = read(PRIOR / 'native-names/evidence.json')
    new = read(HERE / 'native/evidence.json')
    assert old['documented_schema_sha256'] == hashlib.sha256(old_contract).hexdigest()
    assert new['documented_schema_sha256'] == sha(ROOT / CONTRACT)
    for key in ('acceptance_sha256', 'consumer_sha256', 'source_input_count', 'extractor_id',
                'package_sha256', 'sidecar_sha256', 'release_receipt_sha256', 'schemas', 'registry',
                'counts', 'function_anchors_checked', 'symbol_refs_checked'):
        assert new[key] == old[key], key
    assert new['acceptance_sha256'] == sha(PRIOR / 'check_native_notebook.py')
    assert new['consumer_sha256'] == sha(ROOT / 'crates/spur-graph-duckdb/examples/app_consumer.py')
    assert new['status'] == 'passed' and new['pointer_preserved_on_failures'] and new['historical_readback']
    assert [s['functions'] for s in new['snapshots']] == [17, 11]
    assert [c['returncode'] for c in new['children']] == [0, 23, -15, 1, 0]
    assert len(new['manifest']['tables']) == len(new['final_manifest']['tables']) == 12
    app = module(ROOT / 'crates/spur-graph-duckdb/examples/app_consumer.py', 'verified_consumer')
    for native in (old, new):
        output = Path(native['children'][0]['cwd']).parent.parent
        for snapshot in native['snapshots']:
            assert app.read_result(output, snapshot['snapshot_id'], run_id=native['manifest']['run_id'])['manifest'] == native['manifest']
            assert app.read_result(output, snapshot['snapshot_id'])['manifest'] == native['final_manifest']
    print(json.dumps(dict(status='passed', contract_sha256=new['documented_schema_sha256'],
        prior_contract_sha256=old['documented_schema_sha256'], package_sha256=new['package_sha256'],
        sidecar_sha256=new['sidecar_sha256'], extractor_id=source_id, source_inputs=325,
        preserved_prior_attempt_files=len(before['files']), preserved_historical_files=len(historical['files']),
        historical_rust='REMOTE 61 core + 4 adapter; not rerun for prose',
        historical_native_suite='HOST 223 passed, zero skips; not rerun for prose',
        phases=phases, counts=new['counts'], function_anchors_checked=28, symbol_refs_checked=70,
        schemas_and_registry_unchanged=True, original_and_new_runs_reopened=True), indent=2))


if __name__ == '__main__':
    main()
