import json, hashlib, subprocess, sys
from pathlib import Path
import duckdb

ROOT=Path(__file__).parent
ART=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
BASE=ART.parent/'2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
FINAL=ART/'final-99760f547/spur_graph.duckdb_extension'
NATIVE=ART/'final-99760f547/reference_file_facts'

expressions={
 'vec_negative':'Vec::new()',
 'plain':'Factory::new()',
 'spaced':'Factory :: new()',
 'block_before':'Factory /* a */ :: new()',
 'block_after':'Factory :: /* a */ new()',
 'line_before':'Factory // a\n :: new()',
 'before_args':'Factory::new /* a */ ()',
 'before_args_line':'Factory::new // a\n ()',
 'bare':'helper()',
 'bare_comment':'helper /* a */ ()',
 'bare_turbofish':'generic::<u8>()',
 'bare_turbofish_comments':'generic /* a */ :: /* b */ <u8> /* c */ ()',
 'local_turbofish':'Factory::make::<u8>()',
 'local_turbofish_comments':'Factory /* a */ :: make /* b */ :: <u8> /* c */ ()',
 'external_turbofish':'Vec::<Option<u8>>::new()',
 'leading_foreign':'::foreign::helper()',
 'leading_bare':'::helper()',
 'qualified_trait':'<Factory as Make>::produce()',
 'qualified_trait_comments':'<Factory /* a */ as Make>::produce()',
 'qualified_trait_spaces':'< Factory as Make >::produce()',
 'nested_path':'crate::local::deep()',
 'nested_path_spaced':'crate :: local :: deep()',
 'nested_path_comment':'crate /* a */ :: local::deep()',
 'nested_path_line':'crate // a\n :: local::deep()',
 'relative_path':'local::deep()',
 'relative_path_space':'local :: deep()',
 'root_helper':'crate::helper()',
 'self_helper':'self::helper()',
 'root_constructor':'crate::Factory::new()',
 'root_constructor_spaces':'crate :: Factory :: new()',
 'root_constructor_comments':'crate /* a */ :: Factory :: new()',
 'mixed':'Vec::new(), Factory::new(), helper()',
 'nested':'{ [Vec::new(), (Factory::new(), helper())] }',
 'strings':'"Vec::new()", r###"Factory::new()"###, b"helper()", /* helper() */ helper()',
 'char_tokens':"'(', ')', helper()",
 'comparisons':'1 > 0, helper()',
 'closure':'(|| helper())()',
 'string_then_args':'"Factory::new" (helper())',
 'raw_identifier':'r#helper()',
}
prefix='''#![allow(dead_code)]
struct Factory;
impl Factory { fn new() -> Self { Self } fn make<T>() -> Self { Self } }
trait Make { fn produce() -> Self; }
impl Make for Factory { fn produce() -> Self { Self } }
fn helper() -> u8 { 7 }
fn generic<T>() -> u8 { 7 }
mod local { pub fn deep() -> u8 { 7 } }
'''
fixtures=[dict(path='src/lib.rs',source=prefix+'fn caller() { let _ = vec!['+expr+']; }\n') for expr in expressions.values()]
inp=ROOT/'input.jsonl'; inp.write_text(''.join(json.dumps(x)+'\n' for x in fixtures))
subprocess.run([str(NATIVE),str(inp),str(ROOT/'native.jsonl')],check=True)
native=[json.loads(x) for x in (ROOT/'native.jsonl').read_text().splitlines()]

def summary(facts):
 nodes={n['node_id']:n for n in facts['nodes']}
 parent={e['target_node_id']:e['source_node_id'] for e in facts['edges'] if e['relation']=='contains'}
 result=[]
 for e in facts['edges']:
  if e['relation']!='calls' or nodes[e['source_node_id']]['label']!='caller':continue
  t=nodes.get(e['target_node_id']); owner=nodes.get(parent.get(e['target_node_id']))
  result.append(dict(name=e['target_label'],scope=e.get('scope_text'),target=t['label'] if t else None,owner=owner['label'] if owner else None,method=e.get('bind_method'),confidence=e['confidence'],score=e['confidence_score']))
 return result

out={'version':duckdb.__version__,'artifacts':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [BASE,FINAL,NATIVE]},'cases':{}}
for label,ext in [(sys.argv[1], BASE if sys.argv[1]=='baseline' else FINAL)]:
 c=duckdb.connect(config={'allow_unsigned_extensions':'true'})
 c.execute("LOAD '"+str(ext)+"'")
 for i,(name,fixture) in enumerate(zip(expressions,fixtures)):
  f=c.execute('SELECT spur_graph_file_facts(?,?)',[fixture['path'],fixture['source']]).fetchone()[0]
  case=out['cases'].setdefault(name,{'expression':expressions[name]})
  case[label]=summary(f)
  if label=='final':
   # JSON serde spelling differs in IDs/enums, so compare the call summaries.
   case['native']=summary(native[i])
   case['native_sql_calls_match']=case['native']==case['final']
 c.close()
(ROOT/(sys.argv[1]+'-probe-results.json')).write_text(json.dumps(out,indent=2)+'\n')
for name,row in out['cases'].items():
 print(json.dumps({'case':name,**row},separators=(',',':')))
