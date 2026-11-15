import json, subprocess, re
from pathlib import Path
import duckdb
R=Path(__file__).parent
A=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
files=['control.rs','scope-probes.rs']
items=[{'path':p,'source':(A/p).read_text()} for p in files]
test=Path('/private/tmp/bd-38yz-build-20261115/crates/spur-graph/tests/rust_macro_qualified_calls.rs').read_text()
for name in ['same_macro_keeps_distinct_qualifiers_and_foreign_leaf_unresolved','nested_token_trees_keep_each_call_scope','qualified_and_turbofish_paths_preserve_explicit_scope']:
 source=re.search(r'fn '+name+r'\(\) \{\s+let source = r#"(.*?)"#;',test,re.S).group(1)
 items.append({'path':name+'.rs','source':source})
inp=R/'controls-input.jsonl';inp.write_text(''.join(json.dumps(x)+'\n' for x in items))
subprocess.run([str(A/'final-99760f547/reference_file_facts'),str(inp),str(R/'controls-native.jsonl')],check=True)
native=[json.loads(x) for x in (R/'controls-native.jsonl').read_text().splitlines()]
c=duckdb.connect(config={'allow_unsigned_extensions':'true'})
c.execute("LOAD '"+str(A/'final-99760f547/spur_graph.duckdb_extension')+"'")
def calls(f):
 nodes={n['node_id']:n for n in f['nodes']}
 parents={e['target_node_id']:e['source_node_id'] for e in f['edges'] if e['relation']=='contains'}
 return [dict(caller=nodes[e['source_node_id']]['label'],name=e['target_label'],scope=e.get('scope_text'),target=e.get('target_node_id'),owner=nodes.get(parents.get(e.get('target_node_id')),{}).get('label'),bind_method=e.get('bind_method'),confidence=e['confidence'],confidence_score=e['confidence_score']) for e in f['edges'] if e['relation']=='calls']
rows={}
for fixture,n in zip(items,native):
 f=c.execute('SELECT spur_graph_file_facts(?,?)',[fixture['path'],fixture['source']]).fetchone()[0]
 assert calls(n)==calls(f)
 rows[fixture['path']]=calls(f)
control=rows['control.rs']
negative=[x for x in control if x['caller']=='observed_macro' and x['name']=='new']
assert len(negative)==1 and negative[0]['scope']=='Vec' and negative[0]['target'] is None
local=[x for x in control if x['caller']=='observed_local_macro' and x['name']=='new']
assert local and all(x['owner']=='LocalFactory' and x['bind_method']=='macro_body_singleton' for x in local)
helper=[x for x in control if x['caller']=='observed_mixed_macro' and x['name']=='helper']
assert helper and all(x['target'] is not None for x in helper)
scope=rows['scope-probes.rs']
names=['plain','spaced','block_before_separator','block_after_separator','line_before_separator','via_self']
assert all(any(x['caller']==name and x['name']=='new' and x['owner']=='Factory' for x in scope) for name in names)
mixed=rows['same_macro_keeps_distinct_qualifiers_and_foreign_leaf_unresolved.rs']
assert [(x['scope'],x['owner']) for x in mixed if x['name']=='new']==[('Alpha','Alpha'),('Beta','Beta')]
assert any(x['scope']=='foreign' and x['target'] is None for x in mixed)
assert any(x['name']=='helper' and x['scope'] is None and x['target'] is not None for x in mixed)
assert len(mixed)==4
(R/'exact-controls-results.json').write_text(json.dumps(rows,indent=2)+'\n')
print(json.dumps({'compiler_backed_negative_retained_unresolved':True,'local_constructor':True,'mixed_helper':True,'scope_controls':6,'native_sql_call_parity':True}))
