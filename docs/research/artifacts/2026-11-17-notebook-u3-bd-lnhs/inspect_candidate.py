"""Read-only native identity/schema/real-fixture handoff for U4/U5."""
import hashlib
import json
import os
from pathlib import Path
import sys

import _duckdb
import duckdb

ROOT = Path(__file__).resolve().parents[4]

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def main():
    assert duckdb.__version__ == '2.0.0'
    package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
    baseline = Path('/private/tmp/spur-notebook-u3-bd-lnhs-baseline/spur_graph.duckdb_extension')
    assert sha(package) != sha(baseline), 'old binary cannot establish GREEN'
    with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
        con.execute("LOAD '" + str(package).replace("'", "''") + "'")
        report = dict(package=str(package), package_sha256=sha(package), baseline_sha256=sha(baseline),
                      python=sys.executable, python_version=sys.version, native_engine=_duckdb.__file__,
                      engine_sha256=sha(_duckdb.__file__), version=con.execute('SELECT version()').fetchone()[0],
                      platform=con.execute('PRAGMA platform').fetchone()[0],
                      info=con.execute('SELECT spur_graph_info()').fetchone()[0])
        report['functions'] = [dict(zip(['name', 'parameters', 'parameter_types', 'return_type', 'description', 'examples'], row))
                               for row in con.execute("SELECT function_name, parameters, parameter_types, return_type, description, examples "
                                                       "FROM duckdb_functions() WHERE function_name LIKE 'spur_graph_%' ORDER BY function_name").fetchall()]
        queries = dict(symbols="spur_graph_notebook_symbols('n.ipynb','{\"cells\":[] }')",
                       changes="spur_graph_notebook_changes('n.ipynb',NULL,'A',NULL,'{\"cells\":[]}')",
                       ordinary_symbols="spur_graph_symbols('x.rs','')",
                       ordinary_changes="spur_graph_symbol_changes('x.rs','x.rs','M',[],[])", info='spur_graph_info()')
        report['describe'] = {key: con.execute('DESCRIBE SELECT ' + query).fetchone()[1] for key, query in queries.items()}
        fixture = ROOT / 'docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json'
        report['fixture_sha256'] = sha(fixture)
        snapshots = json.loads(fixture.read_text())['snapshots']
        report['snapshots'] = []
        for s in snapshots:
            doc = con.execute('SELECT spur_graph_notebook_symbols(?,?)', [s['notebook_path'], s['source']]).fetchone()[0]
            assert doc['source_sha256'] == s['source_sha256']
            report['snapshots'].append(dict(snapshot_id=s['snapshot_id'], path=doc['path'], source_sha256=doc['source_sha256'],
                                           status=doc['status'], cells=len(doc['cells']),
                                           symbols=sum(len(c['symbols']) for c in doc['cells']),
                                           functions=[dict(cell_id=c['id'], id=x['id'], name=x['symbol']['entity_name'], source_ref=x['source_ref'])
                                                      for c in doc['cells'] for x in c['symbols'] if x['symbol']['symbol_kind'] == 'function']))
        comparison = con.execute('SELECT spur_graph_notebook_changes(?,NULL,\'M\',?,?)',
                                 [snapshots[1]['notebook_path'], snapshots[0]['source'], snapshots[1]['source']]).fetchone()[0]
        report['comparison'] = dict(status=comparison['status'], symbol_changes=len(comparison['symbol_changes']),
                                    cell_changes=len(comparison['cell_changes']), document_changes=len(comparison['document_changes']),
                                    kinds={k: sum(c['kind'] == k for c in comparison['symbol_changes']) for k in ['added', 'modified', 'deleted', 'renamed']})
        print(json.dumps(report, indent=2))

if __name__ == '__main__':
    main()
