import json,hashlib,re,subprocess
from pathlib import Path
R=Path(__file__).parent
W=Path('/private/tmp/bd-38yz-build-20261115')
A=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
BASE='18ee71a2c5d02414d4617f964604d28c03c04ee6'
FINAL='99760f54763a2cb365f0223f4e7a2af7e0e09c2c'
sha=lambda b:hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git','-C',str(W),*args])
def oid(s):return git('rev-parse',s).decode().strip()
out={}
out['base']=BASE;out['head']=oid('HEAD');assert out['head']==FINAL
out['tree']=oid(FINAL+'^{tree}')
out['tree_equivalence']={x:oid(x+'^{tree}') for x in ['99760f547','5fa07540','fd4fc0c8','9a05457df','d373f11bb','ee4c7da1','645330c2']}
assert out['tree_equivalence']['99760f547']==out['tree_equivalence']['5fa07540']==out['tree_equivalence']['fd4fc0c8']
assert out['tree_equivalence']['9a05457df']==out['tree_equivalence']['d373f11bb']
assert out['tree_equivalence']['ee4c7da1']==out['tree_equivalence']['645330c2']
out['commit_chain']={x:git('show','-s','--format=%H %P %s',x).decode().strip() for x in ['08fed2401','3427630ba','9a05457df','645330c2','5fa07540','ee4c7da1','99760f547']}
out['test_only_commits']={x:git('diff-tree','--no-commit-id','--name-only','-r',x).decode().splitlines() for x in ['08fed2401','3427630ba','645330c2','ee4c7da1']}
for files in out['test_only_commits'].values():assert files==['crates/spur-graph/tests/rust_macro_qualified_calls.rs']
prod=['crates/spur-graph/src/extract/languages.rs','crates/spur-graph/queries/rust/spur-edges.scm']
for c in ['08fed2401','3427630ba']:
 for f in prod:assert git('show',c+':'+f)==git('show',BASE+':'+f)
for f in prod:assert git('show','645330c2:'+f)==git('show','9a05457df:'+f)
test_path='crates/spur-graph/tests/rust_macro_qualified_calls.rs'
assert git('show','9a05457df:'+test_path)==git('show','3427630ba:'+test_path)
assert git('show','5fa07540:'+test_path)==git('show','645330c2:'+test_path)
out['test_first_production_bytes_unchanged']=True
diff=git('diff','--binary',BASE,FINAL)
(R/'reviewed.diff').write_bytes(diff)
out['full_diff_sha256']=sha(diff)
out['diff_stat']=git('diff','--stat',BASE,FINAL).decode()
out['diff_check']=subprocess.run(['git','-C',str(W),'diff','--check',BASE,FINAL],capture_output=True,text=True).returncode
out['artifact_hash_maps']={}
for name in ['native-initial','native-trivia']:
 m=json.loads((A/name/'sha256.json').read_text())
 errors={p:(expected,sha((A/name/p).read_bytes())) for p,expected in m.items() if expected!=sha((A/name/p).read_bytes())}
 assert not errors,errors
 out['artifact_hash_maps'][name]={'verified_files':len(m),'mismatches':errors}
out['runs']=[]
for name,manifest in [('native-initial','commands.json'),('native-trivia','verification.json')]:
 v=json.loads((A/name/manifest).read_text())
 for row in v['runs']:
  p=A/name/Path(row['log']).name
  digest=sha(p.read_bytes());assert digest==row['log_sha256']
  command=row.get('command')
  if 'command_sha256' in row:assert sha(command.encode())==row['command_sha256']
  results=re.findall(r'test result: ([^.]+)\. (\d+) passed; (\d+) failed;',p.read_text())
  out['runs'].append({'phase':name,'name':row['name'],'exit':row['exit_code'],'log_sha256':digest,'test_results':results})
  if row['exit_code']==0 and row['name'].startswith('green'):assert results and all(r[0]=='ok' and r[2]=='0' for r in results)
src='crates/spur-graph/tests/rust_macro_qualified_calls.rs'
strength=json.loads((A/'native-initial/red-strengthened.json').read_text())
assert sha(git('show','3427630ba:'+src))==strength['test_sha256']
test=git('show',FINAL+':'+src).decode()
control=re.search(r'const CONTROL: &str = r#"(.*?)"#;',test,re.S).group(1)
scope=re.search(r'const SCOPE_PROBES: &str = r#"(.*?)"#;',test,re.S).group(1)
assert sha(control.encode())==strength['control_sha256']
assert control==(A.parent/'2026-11-15-rust-local-targets/name-collision-control.rs').read_text()
assert scope==(A/'scope-probes.rs').read_text()
out['inline_fixtures_exact']=True
out['source_sha256']={f:sha(git('show',FINAL+':'+f)) for f in prod+[src]}
manifest=json.loads((A/'final-99760f547/build-manifest.json').read_text())
assert manifest['source_commit']==FINAL
for f,h in manifest['source_sha256'].items():assert h==sha(git('show',FINAL+':'+f))
for f,h in manifest['artifact_sha256'].items():assert h==sha((A/'final-99760f547'/f).read_bytes())
out['final_artifact_manifest_hashes_verified']=True
v=json.loads((A/'native-trivia/verification.json').read_text())
for f,h in v['source_sha256'].items():assert h==sha(git('show',FINAL+':'+f))
for row in v['runs']:
 for f,h in row.get('source_sha256',{}).items():assert h==sha(git('show',FINAL+':'+f))
assert git('diff','645330c2','5fa07540')==(A/'native-trivia/green-focused.diff').read_bytes()
out['green_source_bytes_match_final']=True
for name in ['native-initial','native-trivia']:
 x=json.loads((A/name/'completion-audit.json').read_text())
 y=json.loads((A/name/'completion-audit-readback.json').read_text()) if name=='native-trivia' else x['readback']
 x=x if name=='native-trivia' else x['first']
 a=x['structuredContent'];b=y['structuredContent']
 assert a['recorded'] and b['recorded'] and b['idempotent'] and a['audit_id']==b['audit_id'] and a['comment_id']==b['comment_id']
 assert a['audit']['evidence']['kind']=='completion'
 out[name+'_completion_audit']={'task_id':a['task_id'],'audit_id':a['audit_id'],'comment_id':a['comment_id'],'idempotent_readback':True}
out['build_log_tests']=re.findall(r'test result: ([^.]+)\. (\d+) passed; (\d+) failed;', (A/'final-99760f547/extension-tests.log').read_text())
out['worker_status']=subprocess.check_output(['git','status','--porcelain=v1'],text=True)
out['parent_status']=git('status','--porcelain=v1').decode()
assert not out['worker_status'] and not out['parent_status']
(R/'verification.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out,indent=2))
