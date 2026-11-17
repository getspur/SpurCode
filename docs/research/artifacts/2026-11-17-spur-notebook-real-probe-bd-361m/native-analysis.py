import ast, collections, hashlib, importlib.util, json, pathlib, sys
import duckdb
root = pathlib.Path(sys.argv[1])
core = pathlib.Path(sys.argv[2])
package = pathlib.Path(sys.argv[3])
spec = importlib.util.spec_from_file_location('probe_consumer', core / 'crates/spur-graph-duckdb/examples/app_consumer.py')
consumer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(consumer)
manifest = json.loads((root / 'consumer/current.json').read_text())
freeze = json.loads((root / 'freeze.json').read_text())
sha = lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
def dump(name, value):
    (root / name).write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')
def jsonl(name, rows):
    (root / name).write_text(''.join(json.dumps(x, ensure_ascii=False) + '\n' for x in rows))
def records(cursor):
    names = [x[0] for x in cursor.description]
    return [dict(zip(names, row)) for row in cursor.fetchall()]
def quoted(path):
    return "'" + str(path).replace("'", "''") + "'"
for sid in manifest['snapshots']:
    assert consumer.read_result(root / 'consumer', sid)['snapshot_id'] == sid
con = duckdb.connect(config={'allow_unsigned_extensions': True})
assert con.execute('SELECT version()').fetchone()[0] == 'v2.0.0'
con.execute('LOAD ' + quoted(package))
assert con.execute('SELECT spur_graph_info()').fetchone()[0]['extractor_id'] == manifest['extractor_id']
typed = {}
for name, item in manifest['tables'].items():
    path = root / 'consumer' / item['path']
    assert sha(path) == item['sha256'], name
    actual_schema = [[r[0], r[1]] for r in con.execute('DESCRIBE SELECT * FROM read_parquet(' + quoted(path) + ')').fetchall()]
    actual_rows = con.execute('SELECT count(*) FROM read_parquet(' + quoted(path) + ')').fetchone()[0]
    assert actual_schema == item['schema'] and actual_rows == item['row_count'], name
    con.execute('CREATE VIEW ' + name + ' AS SELECT * FROM read_parquet(' + quoted(path) + ')')
    typed[name] = {'row_count': actual_rows, 'schema_matches': True, 'sha256': item['sha256']}
sources = records(con.execute('SELECT * FROM sources'))
cache = records(con.execute('SELECT blob_oid, logical_path, extractor_sha, status FROM file_facts_cache'))
key = lambda row: tuple(row[k] for k in ('blob_oid','logical_path','extractor_sha'))
status = {key(r): r['status'] for r in cache}
symbols, parents, flat, anchor_rows, ast_rows = {}, [], [], [], []
source_by_pair = {(r['snapshot_id'],r['logical_path']):r for r in sources}
for source in sources:
    raw = source['source'].encode()
    assert 'sha256:' + hashlib.sha256(raw).hexdigest() == source['blob_oid']
    k = key(source)
    if status[k] not in consumer.SUCCESS:
        parents.append({'snapshot_id':source['snapshot_id'],'logical_path':source['logical_path'],'status':status[k],'symbol_count':None})
        continue
    if k not in symbols:
        symbols[k] = con.execute('SELECT spur_graph_symbols(?,?)', [source['logical_path'], source['source']]).fetchone()[0]
    syms = symbols[k]
    parents.append({'snapshot_id':source['snapshot_id'],'logical_path':source['logical_path'],'status':status[k],'symbol_count':len(syms)})
    for symbol in syms:
        flat.append({'snapshot_id':source['snapshot_id'],'blob_oid':source['blob_oid'],
            'logical_path':source['logical_path'],'extractor_sha':source['extractor_sha'],**symbol})
        start,end = symbol['byte_start'],symbol['byte_end']
        in_bounds = 0 <= start <= end <= len(raw)
        span = raw[start:end] if in_bounds else b''
        try:
            text = span.decode()
            utf8 = True
        except UnicodeDecodeError:
            text, utf8 = '', False
        anchor_rows.append({'snapshot_id':source['snapshot_id'],'logical_path':source['logical_path'],
            'entity_name':symbol['entity_name'],'symbol_kind':symbol['symbol_kind'],
            'byte_start':start,'byte_end':end,'in_bounds':in_bounds,'utf8_boundary':utf8,
            'name_in_span':symbol['entity_name'] in text,
            'start_line_matches':raw[:start].count(b'\n')+1 == symbol['line_start']})
    if source['logical_path'].endswith('.py'):
        for node in ast.walk(ast.parse(source['source'])):
            if isinstance(node, (ast.FunctionDef,ast.AsyncFunctionDef,ast.ClassDef)):
                matches = [s for s in syms if s['entity_name']==node.name
                    and s['symbol_kind'] in ('function','method','class')
                    and s['line_start'] <= node.lineno and s['line_end'] >= node.end_lineno]
                ast_rows.append({'snapshot_id':source['snapshot_id'],'logical_path':source['logical_path'],
                    'name':node.name,'ast_line_start':node.lineno,'ast_line_end':node.end_lineno,
                    'matching_native_definitions':len(matches)})
jsonl('symbol-parents.jsonl', parents)
jsonl('symbol-rows.jsonl', flat)
jsonl('symbol-anchor-checks.jsonl', anchor_rows)
jsonl('python-ast-checks.jsonl', ast_rows)
for name, file in [('probe_symbols','symbol-rows.jsonl'),('probe_symbol_parents','symbol-parents.jsonl'),
                   ('probe_anchor_checks','symbol-anchor-checks.jsonl'),('probe_ast_checks','python-ast-checks.jsonl')]:
    con.execute('CREATE TABLE ' + name + ' AS SELECT * FROM read_json_auto(' + quoted(root / file) + ', format=''newline_delimited'')')
    con.execute('COPY ' + name + ' TO ' + quoted(root / (name + '.parquet')) + ' (FORMAT PARQUET)')
review_checks = []
for pair in records(con.execute('SELECT * FROM review')):
    old = source_by_pair.get((pair['base_snapshot_id'],pair['old_path']))
    new = source_by_pair.get((pair['head_snapshot_id'],pair['logical_path']))
    before = symbols.get(key(old),[]) if old else []
    after = symbols.get(key(new),[]) if new else []
    for delta in (pair['result'] or {}).get('changes',[]):
        target = delta['symbol']
        def candidates(syms):
            return [s for s in syms if all(s.get(field)==target.get(field)
                for field in ('entity_name','symbol_kind','enclosing_scope'))]
        b,a = candidates(before),candidates(after)
        row = {'pair_id':pair['pair_id'],'logical_path':pair['logical_path'],
               'entity_name':target['entity_name'],'symbol_kind':target['symbol_kind'],
               'native_change_kind':delta['change_kind'],'before_matches':len(b),'after_matches':len(a),
               'source_span_equal':None,'coordinates_changed':None,'native_tokens_equal':None}
        if delta['change_kind']=='modified' and len(b)==len(a)==1:
            oldraw,newraw = old['source'].encode(),new['source'].encode()
            row['source_span_equal'] = oldraw[b[0]['byte_start']:b[0]['byte_end']] == newraw[a[0]['byte_start']:a[0]['byte_end']]
            row['coordinates_changed'] = any(b[0][f]!=a[0][f] for f in ('byte_start','byte_end','line_start','line_end'))
            row['native_tokens_equal'] = b[0]['tokens']==a[0]['tokens']
            row['span_classification'] = ('unchanged_span_moved' if row['source_span_equal'] and row['coordinates_changed']
                else 'unchanged_span' if row['source_span_equal'] else 'source_span_changed')
        else:
            row['span_classification'] = delta['change_kind'] if delta['change_kind']!='modified' else 'nonunique_match'
        review_checks.append(row)
jsonl('review-span-checks.jsonl',review_checks)
con.execute('CREATE TABLE probe_review_checks AS SELECT * FROM read_json_auto(' + quoted(root / 'review-span-checks.jsonl') + ', format=''newline_delimited'')')
con.execute('COPY probe_review_checks TO ' + quoted(root / 'probe_review_checks.parquet') + ' (FORMAT PARQUET)')
cell_rows=[]
for line in (root / 'notebook-cell-sources.jsonl').read_text().splitlines():
    cell=json.loads(line)
    syms=con.execute('SELECT spur_graph_symbols(?,?)',[cell['logical_path'],cell['source']]).fetchone()[0]
    cell_rows.append({k:cell[k] for k in ['snapshot_role','snapshot_id','parent_path','cell_id','language','logical_path']} |
                     {'source_sha256':hashlib.sha256(cell['source'].encode()).hexdigest(),'symbol_count':len(syms),
                      'function_names':[s['entity_name'] for s in syms if s['symbol_kind'] in ('function','method')]})
jsonl('notebook-cell-symbols.jsonl',cell_rows)
con.execute('CREATE TABLE probe_notebook_cells AS SELECT * FROM read_json_auto(' + quoted(root / 'notebook-cell-symbols.jsonl') + ', format=''newline_delimited'')')
con.execute('COPY probe_notebook_cells TO ' + quoted(root / 'probe_notebook_cells.parquet') + ' (FORMAT PARQUET)')
exports={}
for path in root.glob('probe_*.parquet'):
    exports[path.stem]={'path':path.name,'sha256':sha(path),'row_count':con.execute('SELECT count(*) FROM read_parquet('+quoted(path)+')').fetchone()[0]}
report={'status':'complete','runtime':duckdb.__version__,'manifest_sha256':sha(root/'consumer/current.json'),
        'fixture_sha256':sha(root/'fixture.json'),'typed_tables':typed,'fresh_snapshots_reopened':list(manifest['snapshots']),
        'symbol_parent_rows':len(parents),'symbols':len(flat),
        'symbol_range_out_of_bounds':sum(not x['in_bounds'] for x in anchor_rows),
        'symbol_utf8_boundary_failures':sum(not x['utf8_boundary'] for x in anchor_rows),
        'python_ast_definitions':len(ast_rows),'python_ast_unmatched':sum(x['matching_native_definitions']==0 for x in ast_rows),
        'delta_rows':len(review_checks),'delta_span_classifications':dict(collections.Counter(x['span_classification'] for x in review_checks)),
        'notebook_cell_symbols':cell_rows,'exports':exports,
        'limits':['AST cross-check covers named Python definitions only; it does not establish repository semantics.',
                  'Source-span classification is a probe comparison for unique name/kind/scope matches, not a replacement change matcher.',
                  'Standalone code-cell witnesses do not implement cell-aware notebook matching or source navigation.']}
dump('analysis.json',report)
con.close()
print(json.dumps({k:report[k] for k in ['status','runtime','symbol_parent_rows','symbols',
 'symbol_range_out_of_bounds','symbol_utf8_boundary_failures','python_ast_definitions','python_ast_unmatched',
 'delta_rows','delta_span_classifications','notebook_cell_symbols']},indent=2))
