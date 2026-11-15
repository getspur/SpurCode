"""Parent-only complete-corpus equality gate; never supplies facts to the candidate."""
import argparse
import importlib.util
import json
from pathlib import Path

ART = Path(__file__).resolve().parent
loader = importlib.util.spec_from_file_location('candidate', ART / 'candidate-approved.py')
parity = importlib.util.module_from_spec(loader)
loader.loader.exec_module(parity)


def unique_map(rows, path_key, blob_key):
    result = {row[path_key]: row[blob_key] for row in rows}
    if len(result) != len(rows):
        raise ValueError('Duplicate paths in corpus manifest')
    return result


def require_equal(expected, actual, name):
    if expected != actual:
        raise ValueError(f'{name}: complete path/blob mismatch: expected={expected!r}, actual={actual!r}')


def validate(corpus, comparison=None):
    import duckdb
    spec, inputs = parity.shared_inputs(corpus)
    native = json.loads((corpus / '.retrieval/native/provenance.json').read_text())
    expected = unique_map(inputs['sources'], 'path', 'git_blob_oid')
    require_equal(expected, unique_map(native['indexed_files'], 'path', 'content_oid'), 'native indexed files')
    require_equal(expected, unique_map(native['sources'], 'path', 'git_blob_oid'), 'native input files')
    assert native['head_oid'] == inputs['head_oid']
    assert native['spec_sha256'] == inputs['spec_sha256']
    for row in native['artifact_files']:
        raw = (corpus / row['path']).read_bytes()
        assert len(raw) == row['bytes'] and parity.sha256(raw) == row['sha256'], row['path']
    files = corpus / '.spur/graph' / native['graph_content_hash'] / 'files.parquet'
    with duckdb.connect() as connection:
        actual = connection.execute('SELECT file_path FROM read_parquet(?) ORDER BY file_path', [str(files)]).fetchall()
    assert [r[0] for r in actual] == sorted(expected), 'Actual native Parquet corpus differs'
    receipt = {'status': 'passed', 'inputs': inputs, 'native_graph_hash': native['graph_content_hash'],
               'indexed_path_blob_equality': True, 'actual_native_parquet_paths_equal': True,
               'all_native_artifacts_hash_verified': True,
               'purpose': 'Reject discovery/ignore-policy gaps before candidate run; parent-only metadata validation.'}
    if comparison:
        report = json.loads((comparison / 'report.json').read_text())
        provenance = json.loads((comparison / 'provenance.json').read_text())
        require_equal(inputs, report['inputs'], 'report inputs')
        require_equal(inputs, provenance['inputs'], 'comparison provenance inputs')
        sql = provenance['sql_ready']
        require_equal(expected, unique_map(sql['sources'], 'path', 'git_blob_oid'), 'SQL indexed files')
        require_equal(inputs['sources'], sql['sources'], 'SQL complete bytes/hash manifest')
        assert sql['counts']['facts'] == len(expected)
        assert sql['counts']['sources'] == len(expected)
        assert sql['head_oid'] == inputs['head_oid']
        assert sql['spec_sha256'] == inputs['spec_sha256']
        assert report['summary']['native_validation_failures'] == 0
        assert report['summary']['candidate_source_failures'] == 0
        receipt['sql_complete_path_blob_equality'] = True
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus', required=True, type=Path)
    parser.add_argument('--comparison', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    # Meaningful fault injection for the review finding: one skipped source must fail.
    try:
        require_equal({'src/lib.rs': 'a', 'target/extra.rs': 'b'}, {'src/lib.rs': 'a'}, 'skip fault')
    except ValueError:
        pass
    else:
        raise AssertionError('Complete-corpus gate missed skipped target source')
    receipt = validate(args.corpus.resolve(), args.comparison.resolve() if args.comparison else None)
    receipt['skipped_source_fault_rejected'] = True
    parity.write_json(args.out, receipt)
    print(json.dumps({'status': receipt['status'], 'files': len(receipt['inputs']['sources']),
                      'graph_hash': receipt['native_graph_hash'], 'sql_checked': bool(args.comparison)}))
