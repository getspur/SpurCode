import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import duckdb

OUT=Path(__file__).resolve().parent
ROOT=Path('/Users/kevintruong/project/SpurCode')
ART=ROOT/'docs/research/artifacts/2026-11-15-rust-binding-gap'
SNAP=ROOT/'docs/research/artifacts/2026-11-15-spurcode-project-quality/snapshot.duckdb'
EXT=ROOT/'docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
REV='61ba3d200439a541dce4c1617ca9277be7d438af'
receipts=[]
commands=[]
checks={}
def save():
    (OUT/'compiler-audit.json').write_text(json.dumps(dict(checks=checks,receipts=receipts,commands=commands),indent=2)+'\n')
def cmd(args,overrides=None,success=True):
    overrides={'RUSTUP_TOOLCHAIN':'1.94.1',**(overrides or {})}
    run=subprocess.run(args,cwd=OUT,capture_output=True,text=True,env={**os.environ,**overrides},timeout=60)
    commands.append(dict(args=args,cwd=str(OUT),environment_overrides=overrides,returncode=run.returncode,stdout=run.stdout,stderr=run.stderr))
    save()
    if success: assert run.returncode==0,run.stderr
    return run
c=duckdb.connect(':memory:',config={'allow_unsigned_extensions':'true','threads':'2','memory_limit':'1GB','temp_directory':str(OUT/'spill')})
c.execute('ATTACH ? AS frozen (READ_ONLY)',[str(SNAP)])
def q(name,sql,params=None):
    result=c.execute(sql,params or [])
    rows=[dict(zip([d[0] for d in result.description],r)) for r in result.fetchall()]
    receipts.append(dict(name=name,sql=sql,params=params or [],rows=rows))
    save()
    return rows
q('schema_first',"SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('nodes','spans','sources','macro_invocations') ORDER BY table_name,ordinal_position")
q('functions_schema',"SELECT column_name,data_type FROM information_schema.columns WHERE false")
q('runtime','SELECT version() AS version')
q('functions_before',"SELECT function_name FROM duckdb_functions() WHERE starts_with(function_name,'spur_graph') ORDER BY 1")
c.execute("LOAD '"+str(EXT)+"'")
q('functions_after',"SELECT function_name,function_type FROM duckdb_functions() WHERE starts_with(function_name,'spur_graph') ORDER BY 1")
nodes=q('frozen_pieces',"SELECT n.logical_path,n.node_id,n.label,n.kind,p.start_byte,p.end_byte,p.start_line,p.end_line FROM frozen.main.nodes n JOIN frozen.main.spans p ON p.logical_path=n.logical_path AND p.span_id=n.source_span_id WHERE (n.logical_path='crates/spur-graph/src/schema.rs' AND n.label='RelationKind' AND n.kind='enum') OR (n.logical_path='crates/spur-graph-duckdb/src/file_facts.rs' AND n.label IN ('labels','NativeLabel')) ORDER BY n.logical_path,n.node_id")
file_bytes={}
for path in sorted({n['logical_path'] for n in nodes}):
    raw=c.execute('SELECT source_bytes FROM frozen.main.sources WHERE logical_path=?',[path]).fetchone()[0]
    gitraw=subprocess.check_output(['git','show',REV+':'+path],cwd=ROOT)
    assert raw==gitraw
    file_bytes[path]=raw
checks['frozen_sources_equal_git_revision']={p:hashlib.sha256(raw).hexdigest() for p,raw in file_bytes.items()}
pieces={n['label']:file_bytes[n['logical_path']][n['start_byte']:n['end_byte']].decode() for n in nodes}
invocations=q('frozen_labels_invocations',"SELECT logical_path,byte_range,line_range,resolution FROM frozen.main.macro_invocations WHERE logical_path='crates/spur-graph-duckdb/src/file_facts.rs' AND normalized_macro_path='labels' ORDER BY byte_range[1]")
inv=invocations[0]
start,end=inv['byte_range']
source=file_bytes[inv['logical_path']]
invocation=source[start:end].decode()
assert source[end:end+1]==b';' and end==4967
slots=re.findall(r'(\w+)\s*=>\s*"([^"]+)"',invocation)
assert len(slots)==14
checks['source_piece_sha256']={key:hashlib.sha256(text.encode()).hexdigest() for key,text in pieces.items()}
checks['invocation_sha256']=hashlib.sha256(invocation.encode()).hexdigest()
checks['semicolon_byte_offset']=end
main='fn main() {\n'+''.join('println!("{}", RelationKind::'+variant+'.label());\n' for variant,label in slots)+'}'
fixture='\n\n'.join([pieces['RelationKind'],pieces['NativeLabel'],pieces['labels'],invocation+';',main])
assert fixture==(ART/'macro-control.rs').read_text()
(OUT/'macro-control.rs').write_text(fixture)
bad='\n\n'.join([pieces['RelationKind'],pieces['NativeLabel'],pieces['labels'],invocation,main])
(OUT/'macro-missing-semicolon.rs').write_text(bad)
receiver=(ART/'receiver-control.rs').read_text()
(OUT/'receiver-control.rs').write_text(receiver)
expect=json.loads((ART/'compiler-expectations.json').read_text())
saved=json.loads((ART/'compiler-controls.json').read_text())
assert expect==saved['expectations']
version=cmd(['rustc','--version','--verbose'])
checks['compiler_version_matches_saved']=version.stdout==next(r['stdout'] for r in saved['commands'] if r['args'][:2]==['rustc','--version'])
assert checks['compiler_version_matches_saved']
cmd(['rustc','--edition=2021','--crate-name','receiver_control','--emit=mir',str(OUT/'receiver-control.rs'),'-o',str(OUT/'receiver-control.mir')])
cmd(['rustc','--edition=2021','--crate-name','receiver_control',str(OUT/'receiver-control.rs'),'-o',str(OUT/'receiver-control')])
receiver_run=cmd([str(OUT/'receiver-control')])
assert receiver_run.stdout==expect['receiver_stdout']
mir=(OUT/'receiver-control.mir').read_text()
observed=re.search(r'fn observed\(.*?(?=\nfn expected)',mir,re.S).group(0)
expected=re.search(r'fn expected\(.*?(?=\nfn main)',mir,re.S).group(0)
assert 'Vec::<u8>::len' in observed and 'Domain::len' not in observed
assert 'Domain::len' in expected and 'Vec::<u8>::len' not in expected
parent_mir=(ART/'receiver-control.mir').read_text()
assert mir.replace(str(OUT),str(ART))==parent_mir
checks['receiver_stdout']=receiver_run.stdout
checks['mir_function_scoped_targets']={'observed':'Vec::<u8>::len','expected':'Domain::len'}
failed=cmd(['rustc','--edition=2021','--crate-name','macro_missing_semicolon','-Zunpretty=expanded',str(OUT/'macro-missing-semicolon.rs')],{'RUSTC_BOOTSTRAP':'1'},success=False)
assert failed.returncode!=0 and 'followed by a semicolon' in failed.stderr
expanded=cmd(['rustc','--edition=2021','--crate-name','macro_control','-Zunpretty=expanded',str(OUT/'macro-control.rs')],{'RUSTC_BOOTSTRAP':'1'}).stdout
(OUT/'macro-expanded.rs').write_text(expanded)
assert expanded==(ART/'macro-expanded.rs').read_text()
cmd(['rustc','--edition=2021','--crate-name','macro_control',str(OUT/'macro-control.rs'),'-o',str(OUT/'macro-control')])
labels=cmd([str(OUT/'macro-control')]).stdout
assert labels==''.join(label+'\n' for variant,label in slots)
checks['macro_runtime_labels']=labels.splitlines()
checks['macro_source_equals_parent']=True
checks['macro_expanded_equals_parent']=True
recorded={r['name']:r for r in json.loads((ART/'binding-analysis.json').read_text())}
summaries={}
for name,text in [('receiver-control',receiver),('macro-control',fixture),('macro-expanded',expanded)]:
    facts=q(name+'_full_extraction','SELECT spur_graph_file_facts(?,?) AS facts',[name+'.rs',text])[0]['facts']
    parent=recorded[name+'_extraction']
    assert parent['params']==[name+'.rs',text]
    assert facts==parent['rows'][0]['facts']
    spans={s['span_id']:s for s in facts['spans']}
    def body(n):
        s=spans[n['source_span_id']]
        return text.encode()[s['start_byte']:s['end_byte']].decode()
    bodies=[dict(node=n,source=body(n)) for n in facts['nodes'] if n['label']=='label' and n['kind'] in ('method','function') and 'match self' in body(n)]
    calls=[e for e in facts['edges'] if e['relation']=='calls' and e['target_label']=='len']
    summaries[name]=dict(nodes=len(facts['nodes']),edges=len(facts['edges']),macro_invocations=len(facts['macro_invocations']),generated_bodies=bodies,len_calls=calls,full_receipt_equals_parent=True)
assert len(summaries['macro-control']['generated_bodies'])==expect['generated_label_bodies_before']==0
assert len(summaries['macro-expanded']['generated_bodies'])==expect['generated_label_bodies_after']==1
assert len(summaries['receiver-control']['len_calls'])==2
assert all(e['target_node_id'] is None for e in summaries['receiver-control']['len_calls'])
assert {e['scope_text'] for e in summaries['receiver-control']['len_calls']}=={'Vec<u8>','Domain'}
checks['control_extractions']=summaries
checks['all_assertions_passed']=True
save()
c.close()
print(json.dumps(checks,indent=2))
