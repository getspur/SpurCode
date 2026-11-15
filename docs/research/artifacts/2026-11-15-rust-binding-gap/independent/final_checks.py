import hashlib
import json
from pathlib import Path
import subprocess
import duckdb

OUT=Path(__file__).resolve().parent
ROOT=Path('/Users/kevintruong/project/SpurCode')
ART=ROOT/'docs/research/artifacts/2026-11-15-rust-binding-gap'
SNAP=ROOT/'docs/research/artifacts/2026-11-15-spurcode-project-quality/snapshot.duckdb'
REV='61ba3d200439a541dce4c1617ca9277be7d438af'
WORKER=Path('/Users/kevintruong/project/SpurCode/.spur/worktrees/f67b5078-1f4b-4410-833d-8fda7487d02a')
c=duckdb.connect(':memory:',config={'threads':'2','memory_limit':'1GB','temp_directory':str(OUT/'spill')})
c.execute('ATTACH ? AS frozen (READ_ONLY)',[str(SNAP)])
schema=c.execute("SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('nodes','spans','sources','edges','inventory') ORDER BY table_name,ordinal_position").fetchall()
checks={'schema_first':schema}
tree=subprocess.check_output(['git','ls-tree','-r','-z',REV],cwd=ROOT)
entries={item.split(b'\t',1)[1].decode():item.split(b'\t',1)[0].split()[2].decode() for item in tree.split(b'\0') if item}
inventory=c.execute('SELECT path,git_blob_oid FROM frozen.main.inventory').fetchall()
assert all(entries[p]==oid for p,oid in inventory)
checks['inventory_git_revision']={'revision':REV,'inventory_rows':len(inventory),'matched_git_blob_oids':len(inventory)}
source_rows=[]
sources={}
for file,names in [('binding-analysis.json',['source_examples']),('owner-analysis.json',['exact_owner_hint_samples_limit_12','explicit_crate_paths_limit_12'])]:
    for receipt in json.loads((ART/file).read_text()):
        if receipt['name'] not in names:continue
        for row in receipt['rows']:
            path=row['logical_path']
            if path not in sources:
                sources[path]=c.execute('SELECT source_bytes FROM frozen.main.sources WHERE logical_path=?',[path]).fetchone()[0]
            start,end,line0,line1=c.execute("SELECT s.start_byte,s.end_byte,s.start_line,s.end_line FROM frozen.main.edges e JOIN frozen.main.nodes n ON n.logical_path=e.logical_path AND n.node_id=e.source_node_id JOIN frozen.main.spans s ON s.logical_path=n.logical_path AND s.span_id=n.source_span_id WHERE e.logical_path=? AND e.edge_id=?",[path,row['edge_id']]).fetchone()
            assert bytes.fromhex(row['caller_source_hex'])==sources[path][start:min(end,start+6000)]
            assert (row['start_line'],row['end_line'])==(line0,line1)
            source_rows.append({'receipt':receipt['name'],'path':path,'edge_id':row['edge_id'],'start_line':line0,'end_line':line1})
checks['all_36_saved_source_sample_spans_equal_frozen']=source_rows
assert len(source_rows)==36
managed=json.loads((ART/'mcp-function-managed-runtime.json').read_text())
membership=json.loads((ART/'mcp-membership.json').read_text())
assert managed['engine_version']=='v1.5.5' and managed['status']=='complete' and managed['complete'] is True and managed['matches']==[]
assert membership['target']==managed['target'] and membership['total']==0
graph=json.loads((ART/'mcp-graph-catalog.json').read_text())
onager=json.loads((ART/'mcp-onager-catalog.json').read_text())
compiler=json.loads((ART/'mcp-compiler-catalog.json').read_text())
assert graph['next_cursor'] is not None
assert next(x for x in graph['items'] if x['package_name']=='duckgql')['availability']['state']=='available'
assert next(x for x in graph['items'] if x['package_name']=='duckpgq')['availability']['state']=='unavailable'
assert onager['items'][0]['package_name']=='onager' and onager['items'][0]['availability']['state']=='available'
assert {x['package_name'] for x in compiler['items']}=={'ducktinycc','luajit'}
assert compiler['next_cursor'] is None
checks['managed_runtime_and_bounded_catalog_receipts_verified']=True
nb=json.loads((ART/'notebook-evaluation.json').read_text())
outputs=''.join(o.get('text','') for o in nb['outputs'] if o.get('name')=='stdout').strip()
assert json.loads(outputs)['complete'] is True and json.loads(outputs)['receipts']==28
assert len(json.loads((ART/'binding-analysis.json').read_text()))==28
checks['saved_notebook_stdout_matches_complete_28_receipts']=True
cells=json.loads((ART/'notebook-cells.json').read_text())['cells']
for script in ['binding_pilot.py','evaluate_gap.py','owner_diagnostics.py']:
    cell=next(x for x in cells if script in x.get('source',''))
    assert cell['metadata']['spur']['code_type']=='rust'
    assert cell['status']=='success'
    assert '/private/tmp/spur-graph-duckdb-v2-20261114/bin/python' in cell['source']
checks['saved_rust_cell_wrappers_verified']=[{'id':x['id'],'version':x['version'],'exec_count':x['exec_count']} for x in cells]
checks['limitations']=['Live notebook MCP returned worker not authorized to read this notebook or cell; no raw notebook access used; notebook and managed runtime evaluated from saved receipts only.','Live code_read_symbol exceeded its 15000ms deadline. Native seam receipt was independently matched verbatim to frozen source bytes with the same file_oid.','Catalog graph results are paginated; conclusions bounded to returned cards and managed 1.5.5/osx_arm64 availability, not native2 compatibility.','No full workspace build, expansion, accuracy/precision/recall or throughput test.','Initial temp-directory compiler lookup found default 1.96.0 and failed a version assertion before compilation. Preserved under initial-default-toolchain-*; reproduction explicitly selected already-installed 1.94.1.']
c.close()
before=json.loads((OUT/'input-hashes-before.json').read_text())
after={p:hashlib.file_digest(Path(p).open('rb'),'sha256').hexdigest() for p in before}
changes={p:{'before':old,'after':after[p]} for p,old in before.items() if old!=after[p]}
expected_changes=json.loads((OUT/'observed-parent-concurrent-changes.json').read_text())
assert changes==expected_changes
checks['concurrent_parent_changes_re_reviewed']=changes
checks['changes_interpretation']='Concurrent parent modifications, not audit writes. Latest report adds a file-local extraction seam paragraph, independently checked against immutable source; runner adds execution-provenance receipt; notebook receipts refresh execution identity with unchanged result values.'
(OUT/'input-hashes-after.json').write_text(json.dumps(after,indent=2)+'\n')
checks['input_file_count']=len(after)
checks['unchanged_input_file_count']=len(after)-len(changes)
checks['current_parent_file_hashes']={str(p):hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in sorted(ART.iterdir()) if p.is_file()}
provenance=json.loads((ART/'execution-provenance.json').read_text())
assert provenance['script_sha256']==checks['current_parent_file_hashes'][str(ART/'evaluate_gap.py')]
assert provenance['snapshot_sha256']==after[str(SNAP)]
assert provenance['source_revision']==REV and provenance['duckdb_version']=='2.0.0'
checks['latest_execution_provenance_matches']=True
checks['snapshot_sha256']=after[str(SNAP)]
ext=ROOT/'docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
checks['extension_sha256']=after[str(ext)]
checks['worker_git_status']=subprocess.check_output(['git','status','--short','--untracked-files=all'],cwd=WORKER,text=True)
checks['worker_git_diff']=subprocess.check_output(['git','diff','--stat','HEAD'],cwd=WORKER,text=True)
assert checks['worker_git_status']==checks['worker_git_diff']==''
checks['all_assertions_passed']=True
(OUT/'final-checks.json').write_text(json.dumps(checks,indent=2)+'\n')
print(json.dumps({k:v for k,v in checks.items() if k not in ['schema_first','all_36_saved_source_sample_spans_equal_frozen']},indent=2))
