import collections, hashlib, importlib.util, json, os, pathlib, re, subprocess, sys
import duckdb
root,core,package = map(pathlib.Path,sys.argv[1:4])
spec=importlib.util.spec_from_file_location('probe_consumer',core/'crates/spur-graph-duckdb/examples/app_consumer.py')
consumer=importlib.util.module_from_spec(spec); spec.loader.exec_module(consumer)
sha=lambda raw: hashlib.sha256(raw).hexdigest()
def read(path): return json.loads(path.read_text())
def write(path,value): path.write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n')
def quote(path): return "'"+str(path).replace("'","''")+"'"
def rows(cursor): return [dict(zip([x[0] for x in cursor.description],r)) for r in cursor.fetchall()]
manifest=read(root/'consumer/current.json')
freeze=read(root/'freeze.json')
manifest_sha=sha((root/'consumer/current.json').read_bytes())
repo=pathlib.Path(freeze['repository'])
head_sid='git:'+freeze['head_commit']
base_sid='git:'+freeze['base_commit']
def scope(note,requested_snapshot):
    return {'requested_snapshot':requested_snapshot,
            'state':'current' if requested_snapshot==note['snapshot_id'] else 'historical',
            'eligible_as_current':requested_snapshot==note['snapshot_id']}
def verify_note():
    note=read(root/'review-note.json'); receipt=read(root/'review-note-receipt.json')
    assert sha((root/'review-note.json').read_bytes())==receipt['sha256']
    verified=consumer.read_result(root/'consumer',note['snapshot_id'])['manifest']
    assert manifest_sha==note['publication_manifest_sha256']==receipt['publication_manifest_sha256']
    assert verified['extractor_id']==note['extractor_id']
    raw=subprocess.check_output(['git','cat-file','blob',note['source']['git_blob_oid']],cwd=repo)
    assert sha(raw)==note['source']['source_sha256']
    start,end=note['source']['byte_range']
    assert raw[start:end].decode()==note['excerpt']
    assert [raw[:start].count(b'\n')+1,raw[:end].count(b'\n')+1]==note['source']['line_range']
    con=duckdb.connect()
    review=root/'consumer'/verified['tables']['review']['path']
    found=con.execute('SELECT count(*) FROM read_parquet('+quote(review)+') r, unnest(r.result.changes) u(d) '
        'WHERE pair_id=? AND d.stable_symbol_id=? AND d.symbol.entity_name=?',
        [note['pair_id'],note['symbol_id'],note['entity_name']]).fetchone()[0]
    con.close()
    assert found==1
    return {'status':'verified','note_sha256':receipt['sha256'],'snapshot_id':note['snapshot_id'],
            'git_blob_verified':True,'source_excerpt_verified':True,'matching_delta_rows':found,
            'context_checks':[scope(note,head_sid),scope(note,base_sid),
                              scope(note,'git:'+freeze['repo_head_at_freeze'])]}
if '--verify-note' in sys.argv:
    print(json.dumps(verify_note(),indent=2)); sys.exit(0)
con=duckdb.connect()
assert con.execute('SELECT version()').fetchone()[0]=='v2.0.0'
for name,item in manifest['tables'].items():
    con.execute('CREATE VIEW '+name+' AS SELECT * FROM read_parquet('+quote(root/'consumer'/item['path'])+')')
path='app_gallery/code-graph-workbench/server/tools/query.py'
delta=con.execute('SELECT pair_id,d FROM review,unnest(result.changes)u(d) WHERE logical_path=? '
                  "AND d.symbol.entity_name='wb_query'",[path]).fetchone()
source=rows(con.execute('SELECT * FROM sources WHERE snapshot_id=? AND logical_path=?',[head_sid,path]))[0]
inventory=[json.loads(l) for l in (root/'inventory.jsonl').read_text().splitlines()]
git_source=next(r for r in inventory if r['snapshot_id']==head_sid and r['logical_path']==path)
symbol=delta[1]['symbol']; raw=source['source'].encode()
start,end=symbol['byte_start'],symbol['byte_end']
query=read(root/'query-record.json')
patch=subprocess.check_output(['git','diff','--no-ext-diff','--unified=3',
                               freeze['base_commit'],freeze['head_commit'],'--',path],cwd=repo)
(root/'endpoint.patch').write_bytes(patch)
note={
 'format_version':1,'kind':'probe_candidate_note','origin':'agent_probe','status':'candidate',
 'claim':'The new wb_query endpoint returns four panels directly. Confirm their snapshot and coverage identity before presenting a combined review result.',
 'review_decision':'not_reviewed',
 'analysis_id':'probe:'+sha((freeze['fixture_sha256']+manifest_sha).encode()),
 'publication_manifest_sha256':manifest_sha,'fixture_sha256':freeze['fixture_sha256'],
 'snapshot_id':head_sid,'commit_oid':freeze['head_commit'],'extractor_id':manifest['extractor_id'],
 'facts_schema_version':manifest['facts_schema_version'],'artifact_sha256':manifest['artifact']['binary']['sha256'],
 'pair_id':delta[0],'symbol_id':delta[1]['stable_symbol_id'],'entity_name':'wb_query',
 'source':{'logical_path':path,'git_blob_oid':git_source['git_blob_oid'],
           'source_sha256':git_source['source_sha256'],'source_id':source['blob_oid'],
           'byte_range':[start,end],'line_range':[symbol['line_start'],symbol['line_end']],
           'coordinate_space':'git_blob_utf8'},
 'excerpt':raw[start:end].decode(),'git_diff_sha256':sha(patch),
 'git_hunk_header':next(line for line in patch.decode().splitlines() if line.startswith('@@')),
 'result_ref':{'table':'review','table_sha256':manifest['tables']['review']['sha256'],'pair_id':delta[0]},
 'query_ref':{k:query[k] for k in ['notebook_path','cell_id','cell_version','cell_etag']} |
             {'source_sha256':sha(query['source'].encode())},
 'scope_note':'Candidate evidence only; no production note API, human decision or remote PR publication is implemented.'}
write(root/'review-note.json',note)
write(root/'review-note-receipt.json',{'sha256':sha((root/'review-note.json').read_bytes()),
                                    'publication_manifest_sha256':manifest_sha})
# Prove notebook graph ranges address the containing decoded cell, not raw JSON.
cell_data={(r['snapshot_id'],r['cell_id']):r for r in
           (json.loads(l) for l in (root/'notebook-cell-sources.jsonl').read_text().splitlines())}
cell_checks=[]
for src in rows(con.execute("SELECT * FROM sources WHERE logical_path LIKE '%.ipynb'")):
    params=[src[k] for k in ('blob_oid','logical_path','extractor_sha')]
    nodes=rows(con.execute('SELECT * FROM file_facts_nodes WHERE blob_oid=? AND logical_path=? AND extractor_sha=?',params))
    edges=rows(con.execute('SELECT * FROM file_facts_edges WHERE blob_oid=? AND logical_path=? AND extractor_sha=?',params))
    spans={r['span_id']:r for r in rows(con.execute('SELECT * FROM file_facts_spans WHERE blob_oid=? AND logical_path=? AND extractor_sha=?',params))}
    node_map={r['node_id']:r for r in nodes}
    for node in nodes:
        if node['kind']!='function': continue
        owners=[node_map[e['source_node_id']] for e in edges if e['relation']=='contains'
                and e['target_node_id']==node['node_id'] and node_map[e['source_node_id']]['kind']=='cell']
        assert len(owners)==1
        cell_id=owners[0]['label'].removeprefix('cell://')
        cell=cell_data[(src['snapshot_id'],cell_id)]
        span=spans[node['source_span_id']]
        a,b=span['start_byte'],span['end_byte']; decoded=cell['source'].encode()
        assert 0<=a<=b<=len(decoded) and node['label'] in decoded[a:b].decode()
        assert [decoded[:a].count(b'\n')+1,decoded[:b].count(b'\n')+1]==[span['start_line'],span['end_line']]
        cell_checks.append({'snapshot_id':src['snapshot_id'],'logical_path':src['logical_path'],
            'cell_id':cell_id,'entity_name':node['label'],'coordinate_space':'decoded_code_cell_utf8',
            'byte_start':a,'byte_end':b,'line_start':span['start_line'],'line_end':span['end_line'],
            'raw_json_slice_differs':src['source'].encode()[a:b]!=decoded[a:b]})
(root/'notebook-cell-anchors.jsonl').write_text(''.join(json.dumps(x)+'\n' for x in cell_checks))
con.execute('CREATE TABLE lifecycle_cell_anchors AS SELECT * FROM read_json_auto('+quote(root/'notebook-cell-anchors.jsonl')+')')
con.execute('COPY lifecycle_cell_anchors TO '+quote(root/'lifecycle_cell_anchors.parquet')+' (FORMAT PARQUET)')
con.close()
# Real isolated child dies after producing a candidate; prior publication must remain byte-identical.
before=(root/'consumer/current.json').read_bytes()
hashes={item['path']:sha((root/'consumer'/item['path']).read_bytes()) for item in manifest['tables'].values()}
hashes[manifest['snapshot_manifest']['path']]=sha((root/'consumer'/manifest['snapshot_manifest']['path']).read_bytes())
fault=None
try:
    consumer.run_consumer(root/'fixture.json',root/'consumer',package,
                          expected_extractor_id=manifest['extractor_id'],fault='exit_after_candidate')
    raise AssertionError('expected native child exit after candidate')
except consumer.ChildFailure as error:
    assert error.returncode==23
    fault={'returncode':error.returncode,'pid':error.pid,'stdout':error.stdout,'stderr':error.stderr}
assert (root/'consumer/current.json').read_bytes()==before
assert all(sha((root/'consumer'/path).read_bytes())==value for path,value in hashes.items())
assert not list((root/'consumer/runs').glob('.staging-*'))
try: os.kill(fault['pid'],0); child_visible=True
except ProcessLookupError: child_visible=False
assert not child_visible
reopen=subprocess.run([sys.executable,str(pathlib.Path(__file__)),str(root),str(core),str(package),'--verify-note'],
                      capture_output=True,text=True)
(root/'note-reopen.stdout').write_text(reopen.stdout)
(root/'note-reopen.stderr').write_text(reopen.stderr)
assert reopen.returncode==0,reopen.stderr
reopened=json.loads(reopen.stdout)
assert [r['eligible_as_current'] for r in reopened['context_checks']]==[True,False,False]
report={'status':'passed','runtime':duckdb.__version__,'publication_manifest_sha256':manifest_sha,
 'note_reopen':reopened,'fault_after_candidate':fault,'prior_pointer_unchanged':True,
 'published_files_unchanged':True,'validated_published_files':len(hashes),'native_child_reaped':True,
 'staging_clean':True,'notebook_function_cell_anchors':len(cell_checks),
 'notebook_raw_json_slices_differ':sum(r['raw_json_slice_differs'] for r in cell_checks),
 'notebook_cell_anchor_export':{'path':'lifecycle_cell_anchors.parquet',
   'sha256':sha((root/'lifecycle_cell_anchors.parquet').read_bytes()),'rows':len(cell_checks)},
 'scope':'Headless probe artifacts/context checks; no production UI, note service, automatic stale-state guard or remote publication.'}
write(root/'lifecycle.json',report)
print(json.dumps(report,indent=2))
