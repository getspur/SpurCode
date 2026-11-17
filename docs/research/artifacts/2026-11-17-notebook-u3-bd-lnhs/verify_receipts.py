"""Verify retained receipt bytes and the independently loaded candidate identity."""
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def main():
    phases = {}
    for path in sorted(HERE.glob('*.json')):
        data = json.loads(path.read_text())
        if 'argv' not in data:
            continue
        phase = path.stem
        for suffix in ('stdout', 'stderr'):
            assert sha(HERE / f'{phase}.{suffix}') == data[suffix + '_sha256'], phase
        phases[phase] = data
    for phase, code in dict(remote_baseline=200).items():
        assert phases[phase.replace('_', '-')]['exit'] == code
    for phase in ['native-red', 'native-red-final', 'provenance-red', 'baseline-diagnostic-red']:
        assert phases[phase]['exit'] == 1
    for phase in ['host-baseline-build', 'host-final-build', 'final-package', 'native-green-final',
                  'provenance-green', 'host-rust-tests', 'native-interface']:
        assert phases[phase]['exit'] == 0, phase
    key = 'crates/spur-graph-duckdb/tests/test_notebook.py'
    assert phases['native-red-final']['source_sha256'][key] == phases['native-green-final']['source_sha256'][key] == sha(ROOT / key)
    assert phases['native-red-final']['argv'] == phases['native-green-final']['argv']
    info = json.loads((HERE / 'native-interface.stdout').read_text())
    assert info['package_sha256'] != info['baseline_sha256']
    assert sha(info['package']) == info['package_sha256']
    for phase in ['native-red-final', 'native-green-final']:
        package = phases[phase]['package']
        assert sha(package['path']) == package['sha256']
        assert sha(package['path'] + '.json') == package['sidecar_sha256']
    assert phases['native-green-final']['package']['sha256'] == info['package_sha256']
    assert info['version'] == 'v2.0.0' and info['platform'] == 'osx_arm64'
    assert [len(s['functions']) for s in info['snapshots']] == [17, 11]
    assert [s['symbols'] for s in info['snapshots']] == [46, 24]
    expected = ['unsat', 'sat', 'unsat', 'sat', 'sat', 'unsat']
    for path in ['solver-pre.json', 'solver-post.json']:
        checks = json.loads((HERE / path).read_text())['checks']
        assert [json.loads(c['result']['content'][0]['text'])['status'] for c in checks] == expected
        assert all(c['request']['persist'] is False for c in checks)
    historical = ROOT / 'docs/research/artifacts/2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension'
    if not historical.exists():
        historical = Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension')
    if historical.exists():
        assert sha(historical) == '28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122'
    print(json.dumps(dict(verified_phase_count=len(phases), package_sha256=info['package_sha256'],
                          extractor_id=info['info']['extractor_id'],
                          test_predicate_sha256=sha(ROOT / key),
                          historical_package=str(historical), historical_package_preserved=historical.exists()), indent=2))

if __name__ == '__main__':
    main()
