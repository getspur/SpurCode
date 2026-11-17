"""Native U4 readback, real v1 compatibility, and notebook failure evidence."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile

import duckdb

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
CRATE = ROOT / 'crates/spur-graph-duckdb'
BASE = '1d23aee62d80b1964d322fb5af742feddbccbacb'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    app = module(CRATE / 'examples/app_consumer.py', 'app')
    package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
    artifact = json.loads(Path(str(package) + '.json').read_bytes())
    work = Path(tempfile.mkdtemp(prefix='spur-u4-native-readback-'))
    output = work / 'results'
    inputs_path = ROOT / 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json'
    inputs = json.loads(inputs_path.read_bytes())
    fixture = dict(format_version=1, source_identity_scheme='sha256:utf8-v1', snapshots=[], lineage=[], pairs=[])
    for item in inputs['snapshots']:
        sid, path, source = item['snapshot_id'], item['notebook_path'], item['source']
        assert hashlib.sha256(source.encode()).hexdigest() == item['source_sha256']
        blob = 'sha256:' + item['source_sha256']
        fixture['snapshots'].append(dict(snapshot_id=sid, repository='frozen-real-input',
            commit_oid='caller-' + sid, parent_oid=None, sources=[dict(logical_path=path, source=source, blob_oid=blob)]))
        fixture['lineage'].append(dict(snapshot_id=sid, occurrence_id=sid, logical_path=path, blob_oid=blob))
        fixture['pairs'].append(dict(pair_id=sid + '-noop', base_snapshot_id=sid, head_snapshot_id=sid,
                                   logical_path=path, old_path=path, kind='M'))
    fixture['pairs'].append(dict(pair_id='real-change', base_snapshot_id=inputs['snapshots'][0]['snapshot_id'],
        head_snapshot_id=inputs['snapshots'][1]['snapshot_id'], logical_path=inputs['snapshots'][1]['notebook_path'],
        old_path=inputs['snapshots'][0]['notebook_path'], kind='M'))
    path = work / 'real-fixture.json'
    path.write_text(json.dumps(fixture, ensure_ascii=False))

    def run(fixture_path=path, **kwargs):
        return app.run_consumer(fixture_path, output, package,
            expected_extractor_id=artifact['extractor_id'], timeout=60, **kwargs)

    manifest = run()
    app.read_result(output, inputs['snapshots'][0]['snapshot_id'])
    snapshots = []
    with duckdb.connect() as reader:
        documents = reader.execute('SELECT snapshot_id,blob_oid,document FROM read_parquet(?) ORDER BY snapshot_id',
            [str(output / manifest['tables']['notebook_explore']['path'])]).fetchall()
        refs = 0
        for sid, blob, doc in documents:
            symbols = [s for c in doc['cells'] for s in c['symbols']]
            for symbol in symbols:
                assert app.read_notebook_source(output, sid, blob, symbol['source_ref'])
                refs += 1
            snapshots.append(dict(snapshot_id=sid, source_sha256=doc['source_sha256'], cells=len(doc['cells']),
                symbols=len(symbols), functions=sum(s['symbol']['symbol_kind'] == 'function' for s in symbols),
                function_refs=[dict(name=s['symbol']['entity_name'], source_ref=s['source_ref']) for s in symbols
                               if s['symbol']['symbol_kind'] == 'function']))
        reviews = reader.execute('SELECT pair_id,result FROM read_parquet(?)',
            [str(output / manifest['tables']['notebook_review']['path'])]).fetchall()
        changes = {}
        for pair, result in reviews:
            changes[pair] = dict(status=result['status'], symbols=len(result['symbol_changes']),
                                cells=len(result['cell_changes']), documents=len(result['document_changes']))
            if pair.endswith('-noop'):
                assert not result['symbol_changes'] and not result['cell_changes'] and not result['document_changes']
            else:
                assert result['symbol_changes'] and result['cell_changes']
    assert sorted(s['functions'] for s in snapshots) == [11, 17]
    assert refs == 70
    before = (output / 'current.json').read_bytes()
    failures = []
    for fault in ('exit_after_candidate', 'interrupt_after_candidate'):
        try:
            run(fault=fault)
            raise AssertionError('fault must fail')
        except app.ChildFailure as error:
            assert 'candidate written' in error.stdout
            failures.append(dict(fault=fault, returncode=error.returncode, pid=error.pid))
            try:
                os.kill(error.pid, 0)
                raise AssertionError('child was not reaped')
            except ProcessLookupError:
                pass
        assert (output / 'current.json').read_bytes() == before
        assert not list((output / 'runs').glob('.staging-*'))
    forged = copy.deepcopy(fixture)
    forged['snapshots'][0]['sources'][0]['source'] += ' '
    bad = work / 'forged.json'
    bad.write_text(json.dumps(forged))
    try:
        run(bad)
        raise AssertionError('forged bytes must fail')
    except app.ChildFailure as error:
        assert 'source byte identity mismatch' in error.stderr
        failures.append(dict(fault='forged_source', returncode=error.returncode, pid=error.pid))
    assert (output / 'current.json').read_bytes() == before

    # Execute the exact old consumer bytes in a private crate-shaped harness.
    # Only scripts and unchanged file_facts.sql are symlinked; both parent and
    # child execute the retained v1 consumer, not the new code with relabeling.
    legacy_crate = work / 'legacy'
    (legacy_crate / 'examples').mkdir(parents=True)
    legacy_source = subprocess.check_output(['git', 'show', BASE + ':crates/spur-graph-duckdb/examples/app_consumer.py'], cwd=ROOT)
    legacy_path = legacy_crate / 'examples/app_consumer.py'
    legacy_path.write_bytes(legacy_source)
    (legacy_crate / 'scripts').symlink_to(CRATE / 'scripts', target_is_directory=True)
    (legacy_crate / 'examples/file_facts.sql').symlink_to(CRATE / 'examples/file_facts.sql')
    (legacy_crate / 'examples/publish_snapshot.py').symlink_to(CRATE / 'examples/publish_snapshot.py')
    old_app = module(legacy_path, 'legacy_app')
    ordinary_fixture = CRATE / 'tests/fixtures/app_consumer.json'
    old = old_app.run_consumer(ordinary_fixture, output, package,
        expected_extractor_id=artifact['extractor_id'], timeout=60)
    assert old['contract_version'] == 1
    assert app.read_result(output, 'base')['manifest'] == old
    assert app.read_result(output, inputs['snapshots'][0]['snapshot_id'], run_id=manifest['run_id'])['manifest'] == manifest
    new = run(ordinary_fixture)
    assert new['contract_version'] == 2
    assert app.read_result(output, 'base', run_id=old['run_id'])['manifest'] == old
    for table, item in old['tables'].items():
        assert item['schema'] == new['tables'][table]['schema'], table
    print(json.dumps(dict(work=str(work), package=str(package), package_sha256=sha(package),
        sidecar_sha256=sha(str(package) + '.json'), extractor_id=artifact['extractor_id'],
        real_input_sha256=sha(inputs_path), fixture_sha256=sha(path), symbol_refs_checked=refs,
        snapshots=snapshots, changes=changes, failure_guards=failures,
        legacy_source_commit=BASE, legacy_source_sha256=sha(legacy_path), ordinary_schemas_equal=True,
        legacy_manifest=old, ordinary_v2_manifest=new, real_manifest=manifest), indent=2))


if __name__ == '__main__':
    main()
