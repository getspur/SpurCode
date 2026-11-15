import sys,json,subprocess
from pathlib import Path
import duckdb
R=Path(__file__).parent
A=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
version=sys.argv[1]
ext=A/'final-99760f547/spur_graph.duckdb_extension' if version=='final' else A.parent/'2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
prefix='mod outer { pub mod local { pub fn deep() -> u8 { 7 } } }\n'
cases={
 'plain':'outer::local::deep()',
 'spaces':'outer :: local :: deep()',
 'internal_block':'outer /* qualifier */ :: local::deep()',
 'internal_line':'outer // qualifier\n :: local::deep()',
 'trailing_block':'outer::local /* qualifier */ ::deep()',
 'trailing_line':'outer::local // qualifier\n ::deep()',
 'all_comments':'outer /* a */ :: /* b */ local /* c */ :: /* d */ deep /* e */ ()',
}
items=[{'path':'src/lib.rs','source':prefix+'fn caller() { let _ = vec!['+v+']; }\n'} for v in cases.values()]
inp=R/(version+'-path-input.jsonl');inp.write_text(''.join(json.dumps(x)+'\n' for x in items))
if version=='final':
 subprocess.run([str(A/'final-99760f547/reference_file_facts'),str(inp),str(R/'path-native.jsonl')],check=True)
c=duckdb.connect(config={'allow_unsigned_extensions':'true'})
c.execute("LOAD '"+str(ext)+"'")
rows={}
for name,item in zip(cases,items):
 f=c.execute('SELECT spur_graph_file_facts(?,?)',[item['path'],item['source']]).fetchone()[0]
 nodes={n['node_id']:n for n in f['nodes']}
 rows[name]=[e for e in f['edges'] if e['relation']=='calls' and nodes[e['source_node_id']]['label']=='caller']
 print(name,[(e['scope_text'],e['target_node_id'],e['bind_method']) for e in rows[name]])
(R/(version+'-path-results.json')).write_text(json.dumps(rows,indent=2)+'\n')
