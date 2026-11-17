"""Standalone native acceptance. Requires a completed *fresh* release receipt.

Run with the DuckDB 2 Python: check_native_notebook.py --release DIR --out NEW_DIR.
No historical binary is loaded. Every input, source probe and package is checked.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from unittest.mock import patch

import duckdb

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
CRATE = ROOT / 'crates/spur-graph-duckdb'
INPUTS = ROOT / 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json'
ANCHORS = ROOT / 'docs/research/artifacts/2026-11-17-notebook-symbol-diagnosis-bd-nb4c/frozen-cell-model-input.json'
INPUT_SHA = 'bc0e5f23ba34c20149d5c8168f4a3a1fa8b3af3c8759adaef418f2343686b608'
ANCHOR_SHA = '93623d013f4767ac621853e7855aa84a700b73d1814f84c6b3c02a555928fd83'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def save(path, value):
    with path.open('x') as stream:
        stream.write(json.dumps(value, indent=2, sort_keys=True) + '\n')


def resolve(document, symbol):
    """Independent byte/hash/line checks against original decoded cell bytes."""
    ref, fact = symbol['source_ref'], symbol['symbol']
    cell = document['cells'][ref['cell_ordinal']]
    source = cell['source']['original'].encode()
    assert sha(source) == cell['source']['original_sha256'] == ref['cell_source_sha256']
    assert ref['notebook_path'] == document['path']
    assert ref['notebook_source_sha256'] == document['source_sha256']
    assert ref['cell_id'] == cell['id'] and ref['cell_language'] == cell['language']
    assert ref['cell_identity_quality'] == cell['identity_quality']
    assert ref['coordinate_space'] == 'decoded_code_cell_utf8'
    start, end = ref['byte_start'], ref['byte_end']
    assert 0 <= start < end <= len(source)
    assert ref['line_start'] == source[:start].count(b'\n') + 1
    assert ref['line_end'] == source[:end].count(b'\n') + 1
    assert all(ref[k] == fact[k] for k in ('byte_start', 'byte_end', 'line_start', 'line_end'))
    text = source[start:end].decode()
    assert fact['entity_name'] and fact['entity_name'] in text
    return text


def notebook(source, cell_id='rust', language='rust'):
    return json.dumps(dict(metadata={}, cells=[dict(id=cell_id, cell_type='code',
        metadata={'spur': {'code_type': language}}, source=source)]), ensure_ascii=False)


def check_documented_schemas(schemas):
    """Expand the published ordered type aliases and compare actual DESCRIBE."""
    doc = (CRATE / 'docs/release-contract.md').read_text()
    aliases = {}
    name, lines, depth = None, [], 0
    for line in doc.splitlines():
        match = re.match(r'^(\w+) = (STRUCT\(.*)', line)
        if match:
            name, line = match.groups()
            lines, depth = [], 0
        if name:
            lines.append(line.strip())
            depth += line.count('(') - line.count(')')
            if depth == 0:
                aliases[name] = ' '.join(lines)
                name = None
    def expand(name):
        return re.sub(r'\b(' + '|'.join(aliases) + r')\b',
                      lambda m: expand(m[0]), aliases[name])
    for function, alias in [('spur_graph_info', 'Info'), ('spur_graph_notebook_symbols', 'Document'),
                            ('spur_graph_notebook_changes', 'Comparison')]:
        actual = next(v for k, v in schemas.items() if k.startswith(function + '('))
        assert re.sub(r'\s+', '', expand(alias)) == re.sub(r'\s+', '', actual), function
    return sha(doc.encode())


def accept(release, work, report):
    driver = module(CRATE / 'scripts/check_release.py', 'checked_release')
    app = module(CRATE / 'examples/app_consumer.py', 'checked_consumer')
    receipt = json.loads((release / 'receipt.json').read_bytes())
    assert receipt['status'] == 'passed' and receipt['exit_code'] == 0
    assert receipt['unittests']['release_pass'] and receipt['unittests']['skipped'] == 0
    source_receipt = json.loads((release / 'source-probe-final/spur_graph_source_identity.json').read_bytes())
    source_id = driver.verify_source(source_receipt)
    assert source_id == receipt['release_trust']['source_identity']
    package = release / 'spur_graph.duckdb_extension'
    package_sha = driver.approved_extension_sha(package, **receipt['release_trust'])
    assert package_sha != '28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122'
    assert sha(INPUTS.read_bytes()) == INPUT_SHA and sha(ANCHORS.read_bytes()) == ANCHOR_SHA
    assert duckdb.__version__ == '2.0.0'
    inputs = json.loads(INPUTS.read_bytes())
    anchors = json.loads(ANCHORS.read_bytes())['anchors']
    report.update(release=str(release), runtime=driver.runtime_preflight(), extractor_id=source_id,
        package_sha256=package_sha, sidecar_sha256=sha(Path(str(package) + '.json').read_bytes()),
        release_receipt_sha256=sha((release / 'receipt.json').read_bytes()),
        inputs_sha256=INPUT_SHA, anchors_sha256=ANCHOR_SHA,
        consumer_sha256=sha((CRATE / 'examples/app_consumer.py').read_bytes()),
        acceptance_sha256=sha(Path(__file__).read_bytes()), source_input_count=len(source_receipt['inputs']))
    fixture = dict(format_version=1, source_identity_scheme='sha256:utf8-v1', snapshots=[], lineage=[], pairs=[])
    for item in inputs['snapshots']:
        sid, path, source = item['snapshot_id'], item['notebook_path'], item['source']
        assert sha(source.encode()) == item['source_sha256']
        blob = 'sha256:' + item['source_sha256']
        fixture['snapshots'].append(dict(snapshot_id=sid, repository='frozen-real-input',
            commit_oid=sid.removeprefix('git:'), parent_oid=None,
            sources=[dict(logical_path=path, source=source, blob_oid=blob)]))
        fixture['lineage'].append(dict(snapshot_id=sid, occurrence_id=sid, logical_path=path, blob_oid=blob))
        fixture['pairs'].append(dict(pair_id=sid + '-noop', base_snapshot_id=sid, head_snapshot_id=sid,
            logical_path=path, old_path=path, kind='M'))
    left, right = inputs['snapshots']
    fixture['pairs'].append(dict(pair_id='real-change', base_snapshot_id=left['snapshot_id'],
        head_snapshot_id=right['snapshot_id'], logical_path=right['notebook_path'], old_path=left['notebook_path'], kind='M'))
    fixture_path = work / 'real-fixture.json'
    save(fixture_path, fixture)
    output = work / 'consumer'
    children = report['children'] = []
    real_popen = subprocess.Popen

    class RecordedChild(real_popen):
        def __init__(self, argv, *args, **kwargs):
            self.record = dict(argv=list(map(str, argv)), cwd=str(kwargs.get('cwd', ROOT)),
                environment={k: v for k, v in os.environ.items() if k.startswith('SPUR_')},
                stream_prefix=f'child-{len(children):02}')
            children.append(self.record)
            super().__init__(argv, *args, **kwargs)
            self.record['pid'] = self.pid

        def communicate(self, *args, **kwargs):
            stdout, stderr = super().communicate(*args, **kwargs)
            prefix = self.record['stream_prefix']
            (work / (prefix + '.stdout')).write_text(stdout)
            (work / (prefix + '.stderr')).write_text(stderr)
            self.record.update(returncode=self.returncode, stdout_sha256=sha(stdout.encode()),
                               stderr_sha256=sha(stderr.encode()))
            return stdout, stderr

    def run(path=fixture_path, **kwargs):
        with patch.object(app.subprocess, 'Popen', RecordedChild):
            return app.run_consumer(path, output, package, expected_extractor_id=source_id, **kwargs)

    manifest = run()
    report['manifest'] = manifest
    docs = {}
    with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
        con.execute("LOAD '" + str(package).replace("'", "''") + "'")
        assert con.execute('SELECT spur_graph_info()').fetchone()[0]['extractor_id'] == source_id
        queries = ["spur_graph_info()", "spur_graph_symbols('a.rs','')",
            "spur_graph_symbol_changes('a.rs','a.rs','M',[],[])",
            "spur_graph_notebook_symbols('n.ipynb','{}')",
            "spur_graph_notebook_changes('n.ipynb',NULL,'M','{}','{}')"]
        report['schemas'] = {q: con.execute('DESCRIBE SELECT ' + q).fetchone()[1] for q in queries}
        report['documented_schema_sha256'] = check_documented_schemas(report['schemas'])
        report['registry'] = con.execute("SELECT function_name,parameter_types,description,examples "
            "FROM duckdb_functions() WHERE function_name LIKE 'spur_graph_notebook_%' ORDER BY function_name").fetchall()
        rows = con.execute('SELECT snapshot_id,blob_oid,document FROM read_parquet(?)',
            [str(output / manifest['tables']['notebook_explore']['path'])]).fetchall()
        checked, refs = 0, 0
        snapshots = report['snapshots'] = []
        for item, count, functions in zip(inputs['snapshots'], [46, 24], [17, 11]):
            sid = item['snapshot_id']
            matching = [(blob, doc) for snapshot, blob, doc in rows if snapshot == sid]
            assert len(matching) == 1
            blob, doc = matching[0]
            assert doc['contract_version'] == 1 and doc['source_sha256'] == item['source_sha256']
            raw_cells = json.loads(item['source'])['cells']
            assert len(doc['cells']) == len(raw_cells)
            for cell, raw in zip(doc['cells'], raw_cells):
                assert cell['id'] == raw['id'].strip()
                original = raw['source'] if isinstance(raw['source'], str) else ''.join(raw['source'])
                assert cell['source']['original'] == original
                assert cell['source']['original_sha256'] == sha(original.encode())
            symbols = [s for cell in doc['cells'] for s in cell['symbols']]
            assert len(symbols) == count
            actual_functions = [s for s in symbols if s['symbol']['symbol_kind'] == 'function']
            expected = [a for a in anchors if a['snapshot_id'] == sid]
            assert len(actual_functions) == len(expected) == functions
            for anchor in expected:
                matches = [s for s in actual_functions if s['symbol']['entity_name'] == anchor['entity_name']
                           and s['source_ref']['cell_id'] == anchor['cell_id']]
                assert len(matches) == 1
                assert all(matches[0]['source_ref'][k] == anchor[k]
                           for k in ('byte_start', 'byte_end', 'line_start', 'line_end'))
                checked += 1
            for symbol in symbols:
                assert resolve(doc, symbol) == app.read_notebook_source(output, sid, blob, symbol['source_ref'])
                refs += 1
            app.read_result(output, sid)
            docs[sid] = doc
            snapshots.append(dict(snapshot_id=sid, source_sha256=doc['source_sha256'], status=doc['status'],
                symbols=len(symbols), functions=len(actual_functions), document=doc))
        assert checked == 28 and refs == 70
        report.update(function_anchors_checked=checked, symbol_refs_checked=refs)
        reviews = con.execute('SELECT pair_id,result FROM read_parquet(?)',
            [str(output / manifest['tables']['notebook_review']['path'])]).fetchall()
        report['reviews'] = dict(reviews)
        for pair, result in reviews:
            if pair.endswith('-noop'):
                snapshot = pair.removesuffix('-noop')
                assert result['before'] == result['after'] == docs[snapshot]
                assert all(not result[k] for k in ('symbol_changes', 'cell_changes', 'document_changes'))
            else:
                assert result['symbol_changes'] and result['cell_changes']
            for change in result['symbol_changes']:
                for side, item in [('before', left), ('after', right)]:
                    symbol = change[side]
                    if symbol:
                        text = resolve(result[side], symbol)
                        assert text == app.read_notebook_source(output, item['snapshot_id'],
                            'sha256:' + item['source_sha256'], symbol['source_ref'])
        real = dict(reviews)['real-change']
        offset_only = report['offset_only'] = []
        def keyed(doc):
            return {(cell['id'], s['symbol']['entity_name'], s['symbol']['symbol_kind'],
                     s['symbol']['enclosing_scope']): s for cell in doc['cells'] for s in cell['symbols']}
        old, new = keyed(docs[left['snapshot_id']]), keyed(docs[right['snapshot_id']])
        for key in sorted(old.keys() & new.keys(), key=str):
            a, b = old[key], new[key]
            if resolve(docs[left['snapshot_id']], a) == resolve(docs[right['snapshot_id']], b) and any(
                    a['source_ref'][k] != b['source_ref'][k] for k in ('byte_start', 'byte_end', 'line_start', 'line_end')):
                assert not any(c['before'] and c['before']['id'] == a['id'] for c in real['symbol_changes'])
                offset_only.append(dict(cell_id=key[0], entity_name=key[1], before=a['source_ref'],
                    after=b['source_ref'], original_slice=resolve(docs[left['snapshot_id']], a)))
        assert {x['entity_name'] for x in offset_only} == {'PERSPECTIVE_VERSION', 'EvidencePort', 'esc', 'normalizeValue', 'tableRows'}
        report['counts'] = dict(symbol_changes=len(real['symbol_changes']), cell_changes=len(real['cell_changes']),
            kinds={k: sum(c['kind'] == k for c in real['symbol_changes']) for k in sorted({c['kind'] for c in real['symbol_changes']})})
        def extract(text):
            return con.execute("SELECT spur_graph_notebook_symbols('r.ipynb',?)", [text]).fetchone()[0]
        def compare(a, b):
            return con.execute("SELECT spur_graph_notebook_changes('r.ipynb',NULL,'M',?,?)", [a, b]).fetchone()[0]
        original = ':dep serde = "1" # café\r\nfn helper() -> i32 { 1 }\r\n'
        rust = extract(notebook(original))
        cell = rust['cells'][0]
        prepared = cell['source']
        assert rust['status'] == 'complete' and prepared['original'] == original
        assert prepared['policy'] == 'evcxr-leading-dep-mask-v1' and prepared['coordinate_map'] == 'identity'
        assert prepared['original_sha256'] == sha(original.encode())
        assert prepared['analysis_sha256'] == sha(prepared['analysis'].encode())
        raw, analysis = original.encode(), prepared['analysis'].encode()
        assert len(raw) == len(analysis)
        header_end = raw.index(b'\r\n')
        assert analysis[:header_end] == b' ' * header_end and analysis[header_end:] == raw[header_end:]
        helper = next(s for s in cell['symbols'] if s['symbol']['entity_name'] == 'helper')
        assert resolve(rust, helper).startswith('fn helper()') and helper['source_ref']['line_start'] == 2
        dependency = compare(notebook(original), notebook(original.replace('"1"', '"2"')))
        assert not dependency['symbol_changes']
        assert 'dependency_directives_changed' in {c['kind'] for c in dependency['cell_changes']}
        unsupported = {}
        for text in [':load /never/read/target.rs\nfn helper() {}\n', ':dep x = {\n version = "1"\n}\nfn helper() {}\n',
                     'fn helper() {}\n:dep x = "1"\n']:
            doc = extract(notebook(text))
            assert doc['status'] == 'partial' and doc['diagnostics']
            unsupported[text] = doc
        quoted = 'const S: &str = r#"\n:dep x = "1"\n"#;\nfn helper() {}\n'
        quoted_doc = extract(notebook(quoted))
        assert quoted_doc['cells'][0]['source']['analysis'] == quoted
        assert not quoted_doc['cells'][0]['source']['directives']
        duplicate = json.dumps(dict(metadata={}, cells=[dict(id=i, cell_type='code', metadata={'spur': {'code_type': 'python'}},
            source='def helper():\n    return 1\n') for i in ['a', 'b']]))
        noop = compare(duplicate, duplicate)
        assert noop['before'] == noop['after']
        assert len(noop['before']['cells']) == 2
        for cell in noop['before']['cells']:
            helpers = [s for s in cell['symbols'] if s['symbol']['symbol_kind'] == 'function']
            assert len(helpers) == 1 and helpers[0]['symbol']['entity_name'] == 'helper'
            assert resolve(noop['before'], helpers[0]).startswith('def helper():')
        assert all(not noop[k] for k in ('symbol_changes', 'cell_changes', 'document_changes'))
        report['rust'] = dict(document=rust, dependency_change=dependency, unsupported=unsupported, quoted=quoted_doc)
        report['duplicate_helper_noop'] = noop
    pointer = (output / 'current.json').read_bytes()
    immutable = {str(p.relative_to(output)): sha(p.read_bytes()) for p in output.rglob('*')
                 if p.is_file() and p.name != '.consumer.lock'}
    failures = report['publication_failures'] = []
    for fault in ('exit_after_candidate', 'interrupt_after_candidate', 'forged_source'):
        candidate = fixture_path
        kwargs = dict(fault=fault) if fault != 'forged_source' else {}
        if fault == 'forged_source':
            forged = copy.deepcopy(fixture)
            forged['snapshots'][0]['sources'][0]['source'] += ' '
            candidate = work / 'forged-fixture.json'
            save(candidate, forged)
        try:
            run(candidate, **kwargs)
            raise AssertionError('fault must not publish')
        except app.ChildFailure as error:
            assert error.returncode != 0
            if fault == 'forged_source':
                assert 'source byte identity mismatch' in error.stderr
            else:
                assert 'candidate written' in error.stdout
            try:
                os.kill(error.pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('child not reaped')
            failures.append(dict(fault=fault, pid=error.pid, returncode=error.returncode,
                                 stdout=error.stdout, stderr=error.stderr))
        assert (output / 'current.json').read_bytes() == pointer
        assert not list((output / 'runs').glob('.staging-*'))
        assert all(sha((output / p).read_bytes()) == h for p, h in immutable.items())
    second = run()
    assert second['run_id'] != manifest['run_id']
    for item in inputs['snapshots']:
        assert app.read_result(output, item['snapshot_id'], run_id=manifest['run_id'])['manifest'] == manifest
    report.update(immutable_before_failures=immutable, final_manifest=second,
                  pointer_preserved_on_failures=True, historical_readback=True)
    assert driver.verify_source(source_receipt) == source_id
    assert driver.approved_extension_sha(package, **receipt['release_trust']) == package_sha


def main():
    if not __debug__:
        raise RuntimeError('acceptance requires Python assertions enabled')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    work = args.out.resolve()
    work.mkdir(parents=True)
    report = dict(status='running', argv=sys.argv, cwd=str(Path.cwd()), children=[])
    try:
        accept(args.release.resolve(strict=True), work, report)
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
        raise
    else:
        report['status'] = 'passed'
        print(json.dumps({k: report[k] for k in ('status', 'extractor_id', 'package_sha256', 'counts',
              'function_anchors_checked', 'symbol_refs_checked', 'pointer_preserved_on_failures')}))
    finally:
        save(work / 'evidence.json', report)


if __name__ == '__main__':
    main()
