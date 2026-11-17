"""Bind all attempt-2 GREEN results to one freshly built native artifact."""
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.with_name('2026-11-17-notebook-u3-bd-lnhs')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    phases = {}
    for path in sorted(HERE.glob('*.json')):
        if path.name == 'verification.json':
            continue  # The report may be the currently redirected stdout file.
        data = json.loads(path.read_text())
        if 'argv' not in data:
            continue
        for suffix in ('stdout', 'stderr'):
            assert sha(path.with_suffix('.' + suffix)) == data[suffix + '_sha256'], path
        phases[path.stem] = data
    for name, expected in {'rust-red': 101, 'native-diagnostic-red': 1,
                           'rust-green': 0, 'host-build': 0, 'package': 0,
                           'notebook-green': 0, 'provenance-green': 0,
                           'regressions-green': 0, 'native-diagnostic-green': 0,
                           'native-interface': 0}.items():
        assert phases[name]['exit'] == expected, name
    for pair in [('rust-red', 'rust-green'), ('native-diagnostic-red', 'native-diagnostic-green')]:
        assert phases[pair[0]]['argv'] == phases[pair[1]]['argv'], pair
    rust_red = (HERE / 'rust-red.stdout').read_text()
    for name in ['malformed_notebook_json_keeps_core_diagnostic',
                 'invalid_notebook_inventory_keeps_core_diagnostic']:
        assert name + ' ... FAILED' in rust_red
    assert '2 passed; 2 failed; 0 ignored' in rust_red
    assert '4 passed; 0 failed; 0 ignored' in (HERE / 'rust-green.stdout').read_text()
    assert 'FAILED (errors=4)' in (HERE / 'native-diagnostic-red.stderr').read_text()
    for name, count in [('notebook-green', 7), ('provenance-green', 8),
                        ('regressions-green', 76), ('native-diagnostic-green', 1)]:
        log = (HERE / (name + '.stderr')).read_text()
        assert f'Ran {count} test' in log and log.rstrip().endswith('OK'), name
        assert 'skipped' not in log.lower() and 'expected failure' not in log.lower(), name

    interface = json.loads((HERE / 'native-interface.stdout').read_text())
    package_hash = interface['package_sha256']
    package = Path(interface['package'])
    assert sha(package) == package_hash
    assert package_hash != '0ea46655f5968dec09e3116e6941eb6fae6f2d2607a7e50b00049f9c55c66955'
    assert interface['version'] == 'v2.0.0' and interface['platform'] == 'osx_arm64'
    assert [len(s['functions']) for s in interface['snapshots']] == [17, 11]
    assert [s['symbols'] for s in interface['snapshots']] == [46, 24]
    prior_interface = json.loads((PRIOR / 'native-interface.stdout').read_text())
    assert interface['describe'] == prior_interface['describe'], 'all exact SQL schemas remain unchanged'
    assert interface['info']['extractor_id'] != prior_interface['info']['extractor_id']
    for name in ['notebook-green', 'provenance-green', 'regressions-green',
                 'native-diagnostic-green', 'native-interface']:
        receipt = phases[name]
        assert receipt['package']['sha256'] == package_hash, name
        assert sha(receipt['package']['path'] + '.json') == receipt['package']['sidecar_sha256']
        for key, digest in receipt['source_sha256'].items():
            assert sha(ROOT / key) == digest, (name, key)
        for key, digest in receipt['native_sha256'].items():
            assert sha(receipt['native_environment'][key]) == digest, (name, key)
    source = phases['host-build']['source_sha256']
    assert phases['rust-green']['source_sha256'] == source
    assert phases['package']['source_sha256'] == source
    assert phases['notebook-green']['source_sha256'] == source
    raw = Path(phases['package']['native_environment']['SPUR_GRAPH_LIBRARY'])
    assert sha(raw) == phases['package']['native_sha256']['SPUR_GRAPH_LIBRARY']
    assert package.read_bytes().startswith(raw.read_bytes()), 'package wraps the rebuilt library'
    manifest = json.loads(Path(str(package) + '.json').read_text())
    provenance = json.loads((HERE / 'build-source-identity.json').read_text())
    assert manifest['source_identity'] == interface['info']['extractor_id'] == provenance['extractor_id']
    digest = hashlib.sha256(b'spur-graph-extractor-source-v1\0')
    paths = [entry['path'] for entry in provenance['inputs']]
    assert paths == sorted(set(paths))
    for entry in provenance['inputs']:
        label = entry['path'].encode()
        data = entry['content'].encode() if 'content' in entry else (ROOT / entry['path']).read_bytes()
        assert hashlib.sha256(data).hexdigest() == entry['sha256'], entry['path']
        digest.update(len(label).to_bytes(8, 'little'))
        digest.update(label)
        digest.update(len(data).to_bytes(8, 'little'))
        digest.update(data)
    assert provenance['extractor_id'] == 'source-sha256-v1:' + digest.hexdigest()
    for path in ['crates/spur-graph-duckdb/src/file_facts.rs',
                 'crates/spur-graph-duckdb/src/notebook.rs',
                 'crates/spur-graph/src/extract/notebook_model.rs',
                 'crates/spur-graph/src/extract/notebook_symbols.rs',
                 'crates/spur-graph/src/extract/notebook.rs']:
        assert path in paths, path
    build_rows = [json.loads(line) for line in (HERE / 'host-build.stdout').read_text().splitlines()
                  if line.startswith('{')]
    native = [r for r in build_rows if r.get('target', {}).get('name') == 'spur_graph_duckdb'
              and r['reason'] == 'compiler-artifact']
    assert len(native) == 1 and native[0]['fresh'] is False, 'native library was actually rebuilt'
    script_rows = [r for r in build_rows if 'spur-graph-duckdb#' in r.get('package_id', '')
                   and r.get('target', {}).get('kind') == ['custom-build']]
    assert len(script_rows) == 1
    build_script = Path(phases['provenance-green']['native_environment']['SPUR_GRAPH_BUILD_SCRIPT'])
    assert str(build_script) in script_rows[0]['filenames']
    for name in ['rust-red', 'rust-green', 'host-build']:
        assert phases[name]['mode'] == 'host' and phases[name]['environment']['SPUR_REMOTE'] == '0'

    prior_receipts = 0
    prior_packages = set()
    for path in PRIOR.glob('*.json'):
        receipt = json.loads(path.read_text())
        if 'argv' not in receipt:
            continue
        prior_receipts += 1
        for suffix in ('stdout', 'stderr'):
            assert sha(path.with_suffix('.' + suffix)) == receipt[suffix + '_sha256']
        if 'package' in receipt:
            p = receipt['package']
            assert sha(p['path']) == p['sha256']
            assert sha(p['path'] + '.json') == p['sidecar_sha256']
            prior_packages.add(p['sha256'])
    assert json.loads((PRIOR / 'remote-baseline.json').read_text())['exit'] == 200
    prior_raw = Path('/private/tmp/spur-notebook-u3-bd-lnhs-target/debug/libspur_graph_duckdb.dylib')
    prior_package = Path('/private/tmp/spur-notebook-u3-bd-lnhs-final/spur_graph.duckdb_extension')
    assert prior_package.read_bytes().startswith(prior_raw.read_bytes()), 'prior raw library preserved'
    historical = Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/'
                      '2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension')
    historical_hash = sha(historical)
    assert historical_hash == '28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122'
    subprocess.run(['git', 'diff', '--exit-code', '9dd6014ab', '--', 'crates/spur-graph',
                    'crates/spur-graph-duckdb/build.rs', 'crates/spur-graph-duckdb/src/metadata.rs',
                    'crates/spur-graph-duckdb/src/lib.rs', 'crates/spur-graph-duckdb/src/notebook.rs',
                    'crates/spur-graph-duckdb/tests', str(PRIOR.relative_to(ROOT))], cwd=ROOT, check=True)
    for commit in ['9dd6014ab', '14d1cd28c', 'cceffb0cb', '61d65f537']:
        subprocess.run(['git', 'merge-base', '--is-ancestor', commit, 'HEAD'], cwd=ROOT, check=True)
    print(json.dumps(dict(phase_count=len(phases), package=str(package), package_sha256=package_hash,
                          sidecar_sha256=sha(str(package) + '.json'), raw_library_sha256=sha(raw),
                          build_script=str(build_script), build_script_sha256=sha(build_script),
                          extractor_id=provenance['extractor_id'], all_green_bound_to_source=True,
                          suites=dict(notebook=7, provenance=8, rust=4, legacy_consumer=76),
                          prior_receipts_preserved=prior_receipts,
                          prior_package_hashes_preserved=sorted(prior_packages),
                          prior_raw_library_sha256=sha(prior_raw),
                          historical_package_sha256=historical_hash, mode='HOST'), indent=2))


if __name__ == '__main__':
    main()
