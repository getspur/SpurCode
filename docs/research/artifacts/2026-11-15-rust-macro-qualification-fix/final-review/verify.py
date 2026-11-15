import json,hashlib,subprocess,re
from pathlib import Path
R=Path(__file__).parent
W=Path('/Users/kevintruong/project/SpurCode/.spur/worktrees/357fa873-c27b-4a85-9f78-421190256640')
A=Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix')
BASE='18ee71a2c5d02414d4617f964604d28c03c04ee6'
HEAD='fbb3cdbe47ca10b8364b1668ed6747beef27ff9a'
sha=lambda b:hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git','-C',str(W),*args])
def oid(x):return git('rev-parse',x).decode().strip()
def source(c,p):return git('show',c+':'+p)
prod=['crates/spur-graph/src/extract/languages.rs','crates/spur-graph/queries/rust/spur-edges.scm']
test='crates/spur-graph/tests/rust_macro_qualified_calls.rs'
assert oid('HEAD')==HEAD
assert oid('HEAD^{tree}')==oid('4d32f19d4^{tree}')
files=git('diff','--name-only',BASE,HEAD).decode().splitlines()
assert set(files)==set(prod+[test]),files
out=dict(base=BASE,head=HEAD,tree=oid('HEAD^{tree}'),ordered_final_commit=oid('4d32f19d4'),ordered_final_tree=oid('4d32f19d4^{tree}'),files=files)
diff=git('diff','--binary',BASE,HEAD)
(R/'reviewed.diff').write_bytes(diff)
out['diff_sha256']=sha(diff)
out['diff_stat']=git('diff','--stat',BASE,HEAD).decode()
out['source_sha256']={p:sha(source(HEAD,p)) for p in files}
out['source_git_blob_oid']={p:oid(HEAD+':'+p) for p in files}
commits=['08fed2401','3427630ba','9a05457df','ee4c7da1','99760f547','102ff51ee7','4d32f19d4']
out['commit_chain']={x:git('show','-s','--format=%H %P %s',x).decode().strip() for x in commits}
for c in ['08fed2401','3427630ba','ee4c7da1','102ff51ee7']:
 assert git('diff-tree','--no-commit-id','--name-only','-r',c).decode().splitlines()==[test]
 for p in prod:assert source(c,p)==source(c+'^',p),(c,p)
for fix in ['9a05457df','99760f547','4d32f19d4']:assert source(fix,test)==source(fix+'^',test)
assert source('102ff51ee7',test).startswith(source('99760f547',test))
for c in ['645330c2','5fa07540']:
 assert oid(c+'^{tree}')==oid(('ee4c7da1' if c=='645330c2' else '99760f547')+'^{tree}')
out['tdd_test_only_production_unchanged']=True
out['fix_commits_tests_unchanged']=True
out['original_11_tests_byte_identical']=True
out['artifact_hash_maps']={}
for phase in ['native-initial','native-trivia','native-normalization','initial-review']:
 m=json.loads((A/phase/'sha256.json').read_text())
 for p,h in m.items():assert sha((A/phase/p).read_bytes())==h,(phase,p)
 out['artifact_hash_maps'][phase]=len(m)
out['runs']=[]
for phase,manifest in [('native-initial','commands.json'),('native-trivia','verification.json'),('native-normalization','verification.json')]:
 v=json.loads((A/phase/manifest).read_text())
 for r in v['runs']:
  p=A/phase/Path(r['log']).name
  assert sha(p.read_bytes())==r['log_sha256'],p
  if 'command_sha256' in r:assert sha(r['command'].encode())==r['command_sha256']
  results=re.findall(r'test result: ([^.]+)\. (\d+) passed; (\d+) failed;',p.read_text())
  if r['exit_code']==0 and r['name'].startswith('green'):assert results and all(x[0]=='ok' and x[2]=='0' for x in results)
  out['runs'].append(dict(phase=phase,name=r['name'],exit=r['exit_code'],results=results,log_sha256=sha(p.read_bytes())))
 if phase!='native-initial':
  final='99760f547' if phase=='native-trivia' else '4d32f19d4'
  for p,h in v['source_sha256'].items():assert sha(source(final,p))==h
  for r in v['runs']:
   if phase=='native-trivia' or r['name'] in ['red-corrected','green-corrected','green-related','green-lib-macro','fmt-complete','diff-check-complete']:
    for p,h in r.get('source_sha256',{}).items():
     c='102ff51ee7' if r['name']=='red-corrected' else final
     assert sha(source(c,p))==h,(phase,r['name'],p)
  if phase=='native-normalization':
   for r in v['runs']:
    if r['name'] in ['red','red-assertion','green-focused']:
     for p,h in r['source_sha256'].items():
      c='4d32f19d4' if r['name']=='green-focused' and p in prod else 'e48b43b3'
      assert sha(source(c,p))==h
# Exact original native source identity and TDD patch content.
v=json.loads((A/'native-initial/result.json').read_text())
for p,x in v['modified_files'].items():assert sha(source('9a05457df',p))==x['sha256']
assert git('diff',BASE,'9a05457df')==(A/'native-initial/changes.patch').read_bytes()
assert git('diff','645330c2','5fa07540')==(A/'native-trivia/green-focused.diff').read_bytes()
assert git('diff','99760f547','4d32f19d4')==(A/'native-normalization/repair.patch').read_bytes()
strength=json.loads((A/'native-initial/red-strengthened.json').read_text())
assert sha(source('3427630ba',test))==strength['test_sha256']
assert 'left: Some(NodeId(4))' in (A/'native-initial/red-strengthened.log').read_text()
# Binary is tied to exactly the reviewed tree by retained source and artifact manifest.
v=json.loads((A/'build-4d32f19d4/build-manifest.json').read_text())
assert oid(v['source_commit']+'^{tree}')==out['tree']
for p,h in v['source_sha256'].items():assert sha((W/p).read_bytes())==h,p
for p,h in v['artifact_sha256'].items():assert sha((A/'build-4d32f19d4'/p).read_bytes())==h,p
out['current_binary_manifest_verified']=v
# Confirm preceding important finding against its retained native binary, same new inputs.
subprocess.run([str(A/'final-99760f547/reference_file_facts'),str(R/'input.jsonl'),str(R/'before-native.jsonl')],check=True)
before=[json.loads(s) for s in (R/'before-native.jsonl').read_text().splitlines()]
after=[json.loads(s) for s in (R/'native.jsonl').read_text().splitlines()]
path_names=['plain','spaces','internal_block','internal_line','trailing_block','trailing_line','all_comments']
comparison=[]
for name,f,g in zip(path_names,before[8:15],after[8:15]):
 def calls(x):return [e for e in x['edges'] if e['relation']=='calls']
 b=calls(f);a=calls(g)
 assert len(a)==1 and a[0]['target_node_id'] is not None
 assert len(b)==1 and (b[0]['target_node_id'] is not None)==(name in ['plain','trailing_block','trailing_line'])
 comparison.append(dict(case=name,before=b,after=a))
out['prior_important_finding_reproduced_then_resolved']=comparison
out['prior_review_completed_outcome']=json.loads(Path('/private/tmp/bd-38yz-final-review.T3N0h9/result.json').read_text())['findings']
out['initial_review_findings']=json.loads((A/'initial-review/result.json').read_text())['findings']
assert len(out['prior_review_completed_outcome']['important'])==1
out['unchanged_resolver_and_adapter']=all(source(BASE,p)==source(HEAD,p) for p in ['crates/spur-graph/src/extract/tree_sitter.rs','crates/spur-graph-duckdb/src/file_facts.rs','crates/spur-graph-duckdb/src/lib.rs'])
assert out['unchanged_resolver_and_adapter']
out['fresh_fmt']={'command':'scripts/spur-cargo fmt --package spur-graph -- --check','exit':0}
out['diff_check_exit']=subprocess.run(['git','-C',str(W),'diff','--check',BASE,HEAD],capture_output=True).returncode
assert out['diff_check_exit']==0
out['worker_status']=git('status','--porcelain=v1','-uall').decode()
assert not out['worker_status']
assert not git('diff') and not git('diff','--cached')
(R/'verification.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps({k:v for k,v in out.items() if k not in ['prior_important_finding_reproduced_then_resolved','current_binary_manifest_verified','initial_review_findings','prior_review_completed_outcome','runs']},indent=2))
print('All retained TDD source/command/log/patch and artifact hashes verified; previous important finding reproduced on 99760 native and resolved on reviewed native.')
