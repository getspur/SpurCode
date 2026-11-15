import json, re, subprocess, hashlib
from pathlib import Path
import duckdb
R=Path(__file__).parent
W=Path('/Users/kevintruong/project/SpurCode/.spur/worktrees/357fa873-c27b-4a85-9f78-421190256640')
A=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
B=A/'build-4d32f19d4'
test=(W/'crates/spur-graph/tests/rust_macro_qualified_calls.rs').read_text()
fixtures=[]
def add(name,source): fixtures.append(dict(name=name,path='src/lib.rs',source=source))
for name in ['CONTROL','SCOPE_PROBES']:
 add(name,re.search(r'const '+name+r': &str = r#"(.*?)"#;',test,re.S).group(1))
add('parent_control',(A/'control.rs').read_text())
for name in ['local_constructor_and_bare_helper_keep_macro_heuristics','same_macro_keeps_distinct_qualifiers_and_foreign_leaf_unresolved','nested_token_trees_keep_each_call_scope','qualified_and_turbofish_paths_preserve_explicit_scope','multisegment_trivia_keeps_each_owner_and_unknown_scope']:
 add(name,re.search(r'fn '+name+r'\(\) \{\s+let source = r#"(.*?)"#;',test,re.S).group(1))
paths={
 'plain':'outer::local::deep()',
 'spaces':'outer :: local :: deep()',
 'internal_block':'outer /* qualifier */ :: local::deep()',
 'internal_line':'outer // qualifier\n :: local::deep()',
 'trailing_block':'outer::local /* qualifier */ ::deep()',
 'trailing_line':'outer::local // qualifier\n ::deep()',
 'all_comments':'outer /* a */ :: /* b */ local /* c */ :: /* d */ deep /* e */ ()',
}
for name,expr in paths.items():
 add('path_'+name,'mod outer { pub mod local { pub fn deep() -> u8 { 7 } } }\nfn caller() { let _ = vec!['+expr+']; }\n')
# Rerun the preceding review's exact 39 general token-shape inputs.
old=[json.loads(x) for x in Path('/private/tmp/bd-38yz-final-review.T3N0h9/input.jsonl').read_text().splitlines()]
old_expected=json.loads(Path('/private/tmp/bd-38yz-final-review.T3N0h9/final-probe-results.json').read_text())['cases']
for name,item in zip(old_expected,old):add('prior_'+name,item['source'])
# Independently stress keyword token boundaries, literal spelling and comments.
scopes=[
 ('trait_plain','<Type as Trait>','<Type as Trait>'),
 ('trait_comments','<Type/* a */as/* b */Trait>','<Type as Trait>'),
 ('trait_nested_comments','<Type/* outer /* nested */ end */as Trait>','<Type as Trait>'),
 ('raw_string','Const::<{ r#" a /* literal */ // literal "# }>','Const::<{ r#" a /* literal */ // literal "# }>'),
 ('string','Const::<{ " a /* literal */ // literal " }>','Const::<{ " a /* literal */ // literal " }>'),
 ('byte_string','Const::<{ b" a /* literal */ // literal " }>','Const::<{ b" a /* literal */ // literal " }>'),
 ('raw_byte_string','Const::<{ br##" a /* literal */ // literal "## }>','Const::<{ br##" a /* literal */ // literal "## }>'),
 ('char','Const::<{ \' \' }>','Const::<{ \' \' }>'),
 ('char_slash','Const::<{ \'/\' }>','Const::<{ \'/\' }>'),
 ('generic_paths','Const /* x */ :: < outer /* x */ :: local >','Const::< outer::local >'),
 ('leading',':: foreign /* q */ :: nested','::foreign::nested'),
]
for name,expr,expected in scopes:
 add('tokens_'+name,'fn helper() {}\nfn caller() { wrapper!('+expr+'::helper()); }\n')
# Token boundaries distinguish an actual trait qualifier from a joined identifier.
add('tokens_joined_identifier','fn helper() {} fn caller() { wrapper!(<TypeasTrait>::helper()); }')
items=[{k:x[k] for k in ['path','source']} for x in fixtures]
(R/'input.jsonl').write_text(''.join(json.dumps(x)+'\n' for x in items))
subprocess.run([str(B/'reference_file_facts'),str(R/'input.jsonl'),str(R/'native.jsonl')],check=True)
native=[json.loads(x) for x in (R/'native.jsonl').read_text().splitlines()]
def summary(f):
 nodes={n['node_id']:n for n in f['nodes']}
 parents={e['target_node_id']:e['source_node_id'] for e in f['edges'] if e['relation']=='contains'}
 return [dict(caller=nodes[e['source_node_id']]['label'],name=e['target_label'],scope=e.get('scope_text'),target=e.get('target_node_id'),owner=nodes.get(parents.get(e.get('target_node_id')),{}).get('label'),method=e.get('bind_method'),confidence=e['confidence'],score=e['confidence_score']) for e in f['edges'] if e['relation']=='calls']
c=duckdb.connect(config={'allow_unsigned_extensions':'true'})
c.execute("LOAD '"+str(B/'spur_graph.duckdb_extension')+"'")
rows={}
for item,n in zip(fixtures,native):
 f=c.execute('SELECT spur_graph_file_facts(?,?)',[item['path'],item['source']]).fetchone()[0]
 assert summary(n)==summary(f),item['name']
 rows[item['name']]=summary(n)
c.close()
(R/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
def one(case,caller,name,scope,owner=None,resolved=None):
 matches=[x for x in rows[case] if (x['caller'],x['name'],x['scope'])==(caller,name,scope)]
 assert len(matches)==1,(case,caller,name,scope,matches,rows[case])
 x=matches[0]
 if owner is not None:assert x['owner']==owner,(case,x,owner)
 if resolved is not None:assert (x['target'] is not None)==resolved,(case,x,resolved)
 if resolved and case!='CONTROL':assert (x['method'],x['confidence'])==('macro_body_singleton','heuristic') and abs(x['score']-0.8)<1e-6,(case,x)
 return x
one('CONTROL','observed_macro','new','Vec',resolved=False)
one('CONTROL','observed_helper','helper',None,resolved=True)
assert re.search(r'const CONTROL: &str = r#"(.*?)"#;',test,re.S).group(1)==(A.parent/'2026-11-15-rust-local-targets/name-collision-control.rs').read_text()
for name in ['plain','spaced','block_before_separator','block_after_separator','line_before_separator','via_self']:
 one('SCOPE_PROBES',name,'new','Self' if name=='via_self' else 'Factory',owner='Factory',resolved=True)
for name in paths:
 one('path_'+name,'caller','deep','outer::local',owner='local',resolved=True)
 assert len(rows['path_'+name])==1
for case in ['local_constructor_and_bare_helper_keep_macro_heuristics','nested_token_trees_keep_each_call_scope']:
 one(case,'caller','new','LocalFactory',owner='LocalFactory',resolved=True)
 one(case,'caller','helper',None,resolved=True)
 if case.startswith('nested'):
  one(case,'caller','new','Vec',resolved=False)
  one(case,'caller','helper','foreign',resolved=False)
  assert len(rows[case])==4
mixed='same_macro_keeps_distinct_qualifiers_and_foreign_leaf_unresolved'
for owner in ['Alpha','Beta']:one(mixed,'caller','new',owner,owner=owner,resolved=True)
one(mixed,'caller','helper',None,resolved=True)
one(mixed,'caller','helper','foreign',resolved=False)
assert len(rows[mixed])==4
mixed='multisegment_trivia_keeps_each_owner_and_unknown_scope'
one(mixed,'caller','deep','outer::local',resolved=True)
one(mixed,'caller','deep','other::local',resolved=True)
a=one(mixed,'caller','deep','outer::local');b=one(mixed,'caller','deep','other::local')
assert a['target']!=b['target']
one(mixed,'caller','helper','foreign::local',resolved=False)
one(mixed,'caller','helper',None,resolved=True)
one(mixed,'caller','new','Vec',resolved=False)
assert len(rows[mixed])==5
case='qualified_and_turbofish_paths_preserve_explicit_scope'
for scope in ['::foreign','foreign::nested']:one(case,'caller','helper',scope,resolved=False)
one(case,'caller','new','Vec::<u8>',resolved=False)
for caller in ['caller','generic_only','generic_nested']:one(case,caller,'make','LocalFactory',owner='LocalFactory',resolved=True)
for caller in ['caller','generic_only']:one(case,caller,'make','foreign',resolved=False)
one(case,'generic_nested','new','Vec::<Option<u8>>',resolved=False)
for name,expr,scope in scopes:
 one('tokens_'+name,'caller','helper',scope,resolved=False)
 assert len(rows['tokens_'+name])==1
one('tokens_joined_identifier','caller','helper','<TypeasTrait>',resolved=False)
# All prior token-shape semantics must remain, apart from path trivia normalization.
changed=[]
for name,expected in old_expected.items():
 before=[{k:x.get(k) for k in ['name','scope','target','owner','method','confidence','score']} for x in expected['final']]
 after=[{k:x.get(k) for k in ['name','scope','target','owner','method','confidence','score']} for x in rows['prior_'+name] if x['caller']=='caller']
 # Prior target is the label, not the native numeric ID.
 for x in after:
  if x['target'] is not None:x['target']=x['name']
 if before!=after:changed.append(dict(case=name,before=before,after=after))
(R/'prior-differences.json').write_text(json.dumps(changed,indent=2)+'\n')
assert {x['case'] for x in changed}=={'qualified_trait_comments','nested_path_spaced','nested_path_comment','nested_path_line','root_constructor_spaces','root_constructor_comments'},changed
one('prior_qualified_trait_comments','caller','produce','<Factory as Make>',owner='Make for Factory',resolved=True)
assert one('prior_qualified_trait_comments','caller','produce','<Factory as Make>')['target']==one('prior_qualified_trait','caller','produce','<Factory as Make>')['target']
out=dict(fixtures=len(fixtures),native_sql_call_parity=True,path_variants=7,single_qualifier_controls=6,original_vec_retained_unresolved=True,local_helper=True,mixed_nested_turbofish=True,token_literal_cases=len(scopes)+1,prior_shape_cases=len(old),prior_differences=changed,duckdb_version=duckdb.__version__)
(R/'probe-summary.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out,indent=2))
