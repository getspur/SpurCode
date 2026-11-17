"""Verify attempt-2 receipts, preserved ancestry/native inputs and archived reads."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

import duckdb

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.with_name('2026-11-17-notebook-u4-bd-qwpo')
U3 = HERE.with_name('2026-11-17-notebook-u3-bd-lnhs-attempt2')
BASE = '1d23aee62d80b1964d322fb5af742feddbccbacb'
PREVIOUS = 'a135b0bfb'
CONSUMER = 'crates/spur-graph-duckdb/examples/app_consumer.py'
TESTS = 'crates/spur-graph-duckdb/tests/test_app_consumer.py'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def git_bytes(commit, path):
    return subprocess.check_output(['git', 'show', commit + ':' + path], cwd=ROOT)


def main():
    assert duckdb.__version__ == '2.0.0'
    phases = {}
    for name, expected in [('review-red', 1), ('review-red-final', 1),
            ('review-green-attempt1', 1), ('review-green-attempt2', 0),
            ('review-green-final', 0), ('consumer-green', 0), ('native-readback', 0)]:
        r = json.loads((HERE / (name + '.json')).read_bytes())
        assert r['mode'] == 'host' and r['environment']['SPUR_REMOTE'] == '0'
        assert r['exit'] == expected, name
        for stream in ('stdout', 'stderr'):
            assert sha(HERE / (name + '.' + stream)) == r[stream + '_sha256'], name
        phases[name] = r
    assert phases['review-red-final']['argv'] == phases['review-green-final']['argv']
    assert 'FAILED (failures=6)' in (HERE / 'review-red-final.stderr').read_text()
    for name, count in [('review-green-final', 4), ('consumer-green', 22)]:
        log = (HERE / (name + '.stderr')).read_text()
        assert f'Ran {count} tests' in log and log.rstrip().endswith('OK')
        assert 'skipped' not in log.lower()
        for path, value in phases[name]['source_sha256'].items():
            assert sha(ROOT / path) == value, (name, path)
    assert phases['review-red-final']['source_sha256'][CONSUMER] == hashlib.sha256(
        git_bytes(PREVIOUS, CONSUMER)).hexdigest()
    assert phases['review-red-final']['source_sha256'][TESTS] == sha(ROOT / TESTS)
    for oid in ('b359a4123', '0342d3f1b', '8427e33e9', PREVIOUS, '31e5eb148', '8f89b31a1', '6df9ef629'):
        subprocess.run(['git', 'merge-base', '--is-ancestor', oid, 'HEAD'], cwd=ROOT, check=True)

    native = json.loads((HERE / 'native-readback.stdout').read_bytes())
    prior = json.loads((PRIOR / 'native-readback-final.stdout').read_bytes())
    assert native['symbol_refs_checked'] == 70
    assert sorted(s['symbols'] for s in native['snapshots']) == [24, 46]
    assert sorted(s['functions'] for s in native['snapshots']) == [11, 17]
    assert native['changes']['real-change']['symbols'] == 50
    assert native['changes']['real-change']['cells'] == 10
    package = Path(native['package'])
    assert sha(package) == native['package_sha256'] == prior['package_sha256'] == (
        'ed42b39bd3fda2d79ffce5c482c3c869643340f689eded84e9894e709714e35b')
    assert sha(str(package) + '.json') == native['sidecar_sha256'] == prior['sidecar_sha256']
    for r in phases.values():
        assert r['package']['sha256'] == native['package_sha256']
        assert r['package']['sidecar_sha256'] == native['sidecar_sha256']
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
    assert len(provenance['inputs']) == 325
    assert 'source-sha256-v1:' + digest.hexdigest() == provenance['extractor_id'] == native['extractor_id']
    real_inputs = ROOT / 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json'
    assert sha(real_inputs) == native['real_input_sha256'] == prior['real_input_sha256']

    spec = importlib.util.spec_from_file_location('app', ROOT / CONSUMER)
    app = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(app)
    archived = []
    for evidence in (prior, native):
        output = Path(evidence['work']) / 'results'
        checked = refs = 0
        for name in ('real_manifest', 'legacy_manifest', 'ordinary_v2_manifest'):
            m = evidence[name]
            sid = next(iter(m['snapshots']))
            assert app.read_result(output, sid, run_id=m['run_id'])['manifest'] == m
            for item in [*m['tables'].values(), m['snapshot_manifest']]:
                assert sha(output / item['path']) == item['sha256']
                checked += 1
            assert m['source_identity_scheme'] == 'sha256:utf8-v1'
            if name == 'real_manifest':
                with duckdb.connect() as reader:
                    rows = reader.execute('SELECT snapshot_id,blob_oid,document FROM read_parquet(?)',
                        [str(output / m['tables']['notebook_explore']['path'])]).fetchall()
                for snapshot, blob, document in rows:
                    for cell in document['cells']:
                        for symbol in cell['symbols']:
                            assert app.read_notebook_source(output, snapshot, blob, symbol['source_ref'], run_id=m['run_id'])
                            refs += 1
        assert refs == 70 and checked == 37
        for table, item in evidence['legacy_manifest']['tables'].items():
            assert item['schema'] == evidence['ordinary_v2_manifest']['tables'][table]['schema']
            assert item['schema'] == native['ordinary_v2_manifest']['tables'][table]['schema']
        archived.append(dict(work=str(output), immutable_files=checked, symbol_refs=refs,
                             historical_v1=True, historical_v2=True, ordinary_schemas_equal=True))

    for name, outcomes in [('solver-pre', ['fail', 'pass']),
                           ('solver-source-pre', ['fail', 'pass', 'fail', 'fail']),
                           ('solver-post', ['fail', 'pass', 'fail', 'pass', 'fail', 'fail'])]:
        values = json.loads((HERE / (name + '.json')).read_bytes())
        assert len(values) == len(outcomes)
        for check, outcome in zip(values, outcomes):
            assert check['status'] == 'fulfilled'
            assert check['value']['request']['persist'] is False
            result = json.loads(check['value']['result']['content'][0]['text'])
            assert result['outcome'] == outcome
            assert result['status'] == ('sat' if outcome == 'pass' else 'unsat')
    remote_dir = HERE.with_name('2026-11-17-notebook-u3-bd-lnhs')
    remote = json.loads((remote_dir / 'remote-baseline.json').read_bytes())
    assert remote['exit'] == 200 and remote['mode'] == 'remote'
    for stream in ('stdout', 'stderr'):
        assert sha(remote_dir / ('remote-baseline.' + stream)) == remote[stream + '_sha256']
    historical = Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/'
        '2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension')
    assert sha(historical) == '28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122'
    unchanged = ['crates/spur-graph', 'crates/spur-graph-duckdb/src', 'crates/spur-graph-duckdb/build.rs',
        'crates/spur-graph-duckdb/examples/file_facts.sql', 'crates/spur-graph-duckdb/examples/publish_snapshot.py',
        'crates/spur-graph-duckdb/tests/fixtures', str(PRIOR.relative_to(ROOT)),
        str(U3.relative_to(ROOT)), str(remote_dir.relative_to(ROOT)),
        'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp']
    subprocess.run(['git', 'diff', '--exit-code', BASE, '--', *[p for p in unchanged if str(PRIOR.relative_to(ROOT)) != p]],
        cwd=ROOT, check=True)
    subprocess.run(['git', 'diff', '--exit-code', PREVIOUS, '--', str(PRIOR.relative_to(ROOT))], cwd=ROOT, check=True)
    print(json.dumps(dict(phases={k: r['exit'] for k, r in phases.items()}, mode='HOST',
        package_sha256=sha(package), sidecar_sha256=sha(str(package) + '.json'), extractor_id=native['extractor_id'],
        native_inputs_verified=325, preserved_immutable_files=37, fresh_immutable_files=37, archived=archived,
        symbols=[46, 24], functions=[17, 11], independent_anchors=28, resolved_symbol_refs=70,
        source_identity_scheme='sha256:utf8-v1', source_sha256={CONSUMER: sha(ROOT / CONSUMER), TESTS: sha(ROOT / TESTS)},
        real_input_sha256=sha(real_inputs), real_source_hashes=[s['source_sha256'] for s in native['snapshots']],
        ordinary_schemas_equal=True, preserved_remote_exit=200,
        preserved_remote_receipt_sha256=sha(remote_dir / 'remote-baseline.json'),
        historical_package_sha256=sha(historical), public_signatures={
            name: str(__import__('inspect').signature(getattr(app, name)))
            for name in ('run_consumer', 'read_result', 'read_notebook_source')},
        limitations=['HOST validation, not remote compiler success', 'no Rust/native changes or rebuild',
                    'finite solver snapshots do not prove arbitrary runtime behavior', 'U5 remains gated']), indent=2))


if __name__ == '__main__':
    main()
