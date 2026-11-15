"""Freeze a bounded real-source corpus and requests from the declared Git tree."""
import hashlib
import json
from pathlib import Path
import subprocess

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
COMMIT = 'b5e1246cc9706f9938bcff6f6a4233d8a6c12199'
PATHS = [
    'crates/spur-graph-duckdb/src/lib.rs',
    'crates/spur-graph-duckdb/src/file_facts.rs',
    'crates/spur-graph/src/extract/tree_sitter.rs',
    'crates/spur-graph/src/extract/mod.rs',
    'crates/spur-graph/src/extract/languages.rs',
    'crates/spur-graph/src/schema.rs',
]


def save_once(name, value):
    data = (json.dumps(value, indent=2) + '\n').encode()
    path = OUT / name
    if path.exists():
        assert path.read_bytes() == data, f'Existing frozen artifact differs: {path}'
    else:
        path.write_bytes(data)


files, hashes = [], []
for name in PATHS:
    data = subprocess.check_output(['git', 'show', COMMIT + ':' + name], cwd=ROOT)
    files.append({'path': name, 'source': data.decode('utf-8')})
    hashes.append({'path': name, 'bytes': len(data),
                   'sha256': hashlib.sha256(data).hexdigest(),
                   'git_blob_oid': hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()})

requests = []


def add(case_id, tool, args):
    requests.append({'case_id': case_id, 'tool': tool, 'args': args})


lib, facts, tree = PATHS[:3]
add('real-search-exact-function', 'code_symbol_search',
    {'query': 'source_offset', 'mode': 'exact', 'file': lib, 'symbol_kind': 'function', 'limit': 10})
add('real-search-prefix', 'code_symbol_search',
    {'query': 'read_', 'mode': 'prefix', 'file': lib, 'limit': 20})
add('real-search-substring-struct', 'code_symbol_search',
    {'query': 'Symbol', 'mode': 'substring', 'file': lib, 'symbol_kind': 'struct', 'limit': 20})
add('real-search-glob', 'code_symbol_search',
    {'query': 'register', 'mode': 'exact', 'file_glob': 'crates/spur-graph-duckdb/src/*.rs', 'limit': 20})
add('real-search-limited', 'code_symbol_search',
    {'query': 'Symbol', 'mode': 'substring', 'file': lib, 'limit': 1})
add('real-search-empty', 'code_symbol_search',
    {'query': '__retrieval_parity_no_such_symbol__', 'mode': 'exact', 'limit': 20})
for name, path in [('source_offset', lib), ('register', facts), ('extract_file_facts', tree)]:
    for context in [0, 2]:
        add('real-read-' + name + '-' + str(context), 'code_read_symbol',
            {'path': path, 'name': name, 'context_lines': context, 'response_format': 'source'})
    for direction in ['callers', 'callees']:
        for unresolved in [False, True]:
            add('real-' + direction + '-' + name + '-' + str(unresolved).lower(), 'code_' + direction,
                {'selector': path + '::' + name, 'include_unresolved': unresolved, 'response_format': 'full'})
add('real-read-missing', 'code_read_symbol',
    {'path': lib, 'name': '__retrieval_parity_no_such_symbol__', 'context_lines': 0, 'response_format': 'source'})

save_once('real-cases.json', {'files': files, 'requests': requests})
save_once('real-source-origin.json', {
    'repository': 'SpurCode', 'commit': COMMIT,
    'scope': 'Six-file standalone frozen source cohort; both pipelines receive every file. Not the whole original project.',
    'files': hashes, 'request_count': len(requests),
})
print(json.dumps({'files': len(files), 'bytes': sum(f['bytes'] for f in hashes),
                  'requests': len(requests), 'commit': COMMIT}))
