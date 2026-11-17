"""Independently bind U4 RED/GREEN, preserved native inputs, and readback hashes."""
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = '1d23aee62d80b1964d322fb5af742feddbccbacb'
PRIOR = HERE.with_name('2026-11-17-notebook-u3-bd-lnhs')
U3 = HERE.with_name('2026-11-17-notebook-u3-bd-lnhs-attempt2')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    phases = {}
    for name, expected in [('native-red', 1), ('native-green-attempt1', 1), ('native-green-attempt2', 1),
                           ('native-green-attempt3', 0), ('consumer-green', 0),
                           ('native-readback', 1), ('native-readback-final', 0)]:
        receipt = json.loads((HERE / (name + '.json')).read_bytes())
        assert receipt['exit'] == expected, name
        assert receipt['mode'] == 'host' and receipt['environment']['SPUR_REMOTE'] == '0'
        for stream in ('stdout', 'stderr'):
            assert sha(HERE / (name + '.' + stream)) == receipt[stream + '_sha256'], name
        phases[name] = receipt
    assert phases['native-red']['argv'] == phases['native-green-attempt3']['argv']
    assert 'FAILED (failures=5)' in (HERE / 'native-red.stderr').read_text()
    for name, count in [('native-green-attempt3', 5), ('consumer-green', 19)]:
        log = (HERE / (name + '.stderr')).read_text()
        assert f'Ran {count} tests' in log and log.rstrip().endswith('OK')
        assert 'skipped' not in log.lower()
        for path, value in phases[name]['source_sha256'].items():
            assert sha(ROOT / path) == value, (name, path)

    native = json.loads((HERE / 'native-readback-final.stdout').read_bytes())
    assert native['symbol_refs_checked'] == 70
    assert sorted(s['functions'] for s in native['snapshots']) == [11, 17]
    assert sorted(s['symbols'] for s in native['snapshots']) == [24, 46]
    assert native['ordinary_schemas_equal']
    assert native['legacy_manifest']['contract_version'] == 1
    assert native['ordinary_v2_manifest']['contract_version'] == 2
    assert native['changes']['real-change']['symbols'] == 50
    assert native['changes']['real-change']['cells'] == 10
    package = Path(native['package'])
    assert sha(package) == native['package_sha256'] == 'ed42b39bd3fda2d79ffce5c482c3c869643340f689eded84e9894e709714e35b'
    assert sha(str(package) + '.json') == native['sidecar_sha256']
    for receipt in phases.values():
        assert receipt['package']['sha256'] == native['package_sha256']
    provenance = json.loads((U3 / 'build-source-identity.json').read_bytes())
    digest = hashlib.sha256(b'spur-graph-extractor-source-v1\0')
    for entry in provenance['inputs']:
        label = entry['path'].encode()
        data = entry['content'].encode() if 'content' in entry else (ROOT / entry['path']).read_bytes()
        assert hashlib.sha256(data).hexdigest() == entry['sha256'], entry['path']
        digest.update(len(label).to_bytes(8, 'little'))
        digest.update(label)
        digest.update(len(data).to_bytes(8, 'little'))
        digest.update(data)
    assert 'source-sha256-v1:' + digest.hexdigest() == provenance['extractor_id'] == native['extractor_id']
    assert sha(ROOT / 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json') == native['real_input_sha256']

    # Re-read all retained immutable files rather than trusting saved hashes.
    work = Path(native['work'])
    checked = 0
    for manifest in (native['real_manifest'], native['legacy_manifest'], native['ordinary_v2_manifest']):
        run = work / 'results/runs' / manifest['run_id']
        assert json.loads((run / 'manifest.json').read_bytes()) == manifest
        for item in [*manifest['tables'].values(), manifest['snapshot_manifest']]:
            path = work / 'results' / item['path']
            assert sha(path) == item['sha256']
            if 'bytes' in item:
                assert path.stat().st_size == item['bytes']
            checked += 1
    legacy = subprocess.check_output(['git', 'show', BASE + ':crates/spur-graph-duckdb/examples/app_consumer.py'], cwd=ROOT)
    assert hashlib.sha256(legacy).hexdigest() == native['legacy_source_sha256']
    remote = json.loads((PRIOR / 'remote-baseline.json').read_bytes())
    assert remote['exit'] == 200 and remote['mode'] == 'remote'
    for stream in ('stdout', 'stderr'):
        assert sha(PRIOR / ('remote-baseline.' + stream)) == remote[stream + '_sha256']
    for phase in ('solver-pre', 'solver-post'):
        checks = json.loads((HERE / (phase + '.json')).read_bytes())
        for check, expected in zip(checks, ['fail', 'pass', 'fail', 'pass', 'pass', 'fail']):
            assert check['status'] == 'fulfilled'
            item = check['value']
            assert item['request']['persist'] is False
            result = json.loads(item['result']['content'][0]['text'])
            assert result['outcome'] == expected
    unchanged = ['crates/spur-graph', 'crates/spur-graph-duckdb/src', 'crates/spur-graph-duckdb/build.rs',
                 'crates/spur-graph-duckdb/examples/file_facts.sql', 'crates/spur-graph-duckdb/examples/publish_snapshot.py',
                 'crates/spur-graph-duckdb/tests/fixtures/app_consumer.json',
                 str(PRIOR.relative_to(ROOT)), str(U3.relative_to(ROOT)),
                 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp']
    subprocess.run(['git', 'diff', '--exit-code', BASE, '--', *unchanged], cwd=ROOT, check=True)
    historical = Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/'
                      '2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension')
    assert sha(historical) == '28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122'
    print(json.dumps(dict(phases={name: r['exit'] for name, r in phases.items()}, mode='HOST',
        native_inputs_verified=len(provenance['inputs']), extractor_id=native['extractor_id'],
        package_sha256=sha(package), sidecar_sha256=native['sidecar_sha256'], immutable_files_verified=checked,
        symbols=[46, 24], functions=[17, 11], independent_anchors_tested=28, source_refs_resolved=70,
        real_symbol_changes=50, real_cell_changes=10, consumer_tests_passed=19,
        historical_v1_readback=True, ordinary_schemas_equal=True,
        preserved_remote_receipt_sha256=sha(PRIOR / 'remote-baseline.json'),
        preserved_remote_exit=200, historical_package_sha256=sha(historical),
        production_paths=['crates/spur-graph-duckdb/examples/app_consumer.py']), indent=2))


if __name__ == '__main__':
    main()
