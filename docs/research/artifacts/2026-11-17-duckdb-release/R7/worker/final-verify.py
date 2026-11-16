import hashlib,importlib.util,json,os,pathlib,re,subprocess,sys
ROOT=pathlib.Path.cwd();CRATE=ROOT/'crates/spur-graph-duckdb';E=pathlib.Path(__file__).resolve().parent;R=pathlib.Path('/private/tmp/spur-r7-65bb3605-release-1')
sha=lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
r=json.loads((R/'receipt.json').read_bytes());a=json.loads((E/'current-acceptance.json').read_bytes())
assert r['status']=='passed' and r['exit_code']==0 and all(c['returncode']==0 for c in r['commands'])
assert r['unittests']==dict(tests_run=206,failures=0,errors=0,skipped=0,expected_failures=0,unexpected_successes=0,skipped_tests=[],release_pass=True)
assert a['status']=='passed' and a['binary_sha256']==r['package']['binary']['sha256']==sha(a['package'])
assert a['headless_manifest']['artifact']==r['package']
assert all(sha(v['path'])==v['sha256'] for v in r['native_artifacts'].values())
assert sha(r['release_trust']['manifest'])==r['release_trust']['manifest_sha256']
sys.path.insert(0,str(CRATE/'scripts'))
from check_release import verify_source,approved_extension_sha
assert approved_extension_sha(a['package'],**r['release_trust'])==a['binary_sha256']
assert (R/'source-probe/spur_graph_source_identity.json').read_bytes()==(R/'source-probe-final/spur_graph_source_identity.json').read_bytes()
final_probe=E/'final-source-probe';final_probe.mkdir()
probe=r['native_artifacts']['build_script']['path'];argv=[probe]
with (E/'final-source-probe.stdout').open('x') as out,(E/'final-source-probe.stderr').open('x') as err:
 p=subprocess.run(argv,env=dict(os.environ,CARGO_MANIFEST_DIR=str(CRATE),OUT_DIR=str(final_probe)),stdout=out,stderr=err)
assert p.returncode==0
source=json.loads((final_probe/'spur_graph_source_identity.json').read_bytes())
assert verify_source(source)==r['release_trust']['source_identity']
assert source==json.loads((R/'source-probe/spur_graph_source_identity.json').read_bytes())
checks=[]
for path in [CRATE/'README.md',CRATE/'docs/release-contract.md']:
 text=path.read_text()
 assert text.count('```')%2==0
 for link in re.findall(r'\[[^\]]+\]\(([^)]+)\)',text):
  if not link.startswith(('http:','https:','#')):
   assert (path.parent/link.split('#')[0]).is_file(),(path,link)
 checks.append(dict(path=str(path.relative_to(ROOT)),sha256=sha(path),bytes=path.stat().st_size))
readme=(CRATE/'README.md').read_text();contract=(CRATE/'docs/release-contract.md').read_text()
rblocks=re.findall(r'```sh\n(.*?)```',readme,re.S);cblocks=re.findall(r'```sh\n(.*?)```',contract,re.S)
for name,content in [('readme-build',rblocks[0]),('readme-five-functions',rblocks[1]),('readme-headless',rblocks[2]),('contract-cache-demo',cblocks[0])]:
 assert (E/(name+'.sh')).read_text()==content
commands=json.loads((E/'documented-commands.json').read_bytes())
assert all(c['raw_exit']==0 for c in commands['commands'])
art=pathlib.Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s');pub=json.loads((art/'published.json').read_bytes())
assert sha(pub['database'])==pub['database_sha256']==a['database_sha256']
assert all(sha(p)==v for p,v in a['reference_hashes'].items())
solves={n:json.loads((E/(n+'.json')).read_bytes()) for n in ['solve-pre','solve-pre-nonpersisted','solve-pre-negative','solve-post','solve-post-nonpersisted','solve-post-negative']}
for name,expect in [('solve-pre-nonpersisted','pass'),('solve-pre-negative','fail'),('solve-post-nonpersisted','pass'),('solve-post-negative','fail')]:
 actual=json.loads(solves[name]['result']['content'][0]['text']);assert actual['outcome']==expect
for name in ['solve-pre','solve-post']:
 assert solves[name]['result']['isError'] and 'ArtifactCount' in solves[name]['result']['content'][0]['text']
checks_argv=['git','diff','--check'];check=subprocess.run(checks_argv,capture_output=True,text=True);assert check.returncode==0
changed=subprocess.check_output(['git','status','--porcelain=v1','--untracked-files=all'],text=True)
expected={'crates/spur-graph-duckdb/README.md','crates/spur-graph-duckdb/docs/release-contract.md'}
assert {line[3:] for line in changed.splitlines()}==expected,changed
predecessor_paths=['/private/tmp/spur-bd-4q5c-green/verification.json','/private/tmp/spur-r2-e5afa03c-evidence/receipt.json','/private/tmp/spur-r3-7cb6b619-evidence/receipt.json','/private/tmp/spur-r4-90b963ee-evidence/receipt.json','/private/tmp/spur-r5-9a79a2c4-evidence/verification.json','/private/tmp/spur-r6-4f646974-rework-evidence/verification.json']
report=dict(task='R7',issue='bd-r7f4',status='passed',source_head=r['source_head'],source_status_at_release_start=r['source_status'],owned_documents=checks,
 release_receipt=dict(path=str(R/'receipt.json'),sha256=sha(R/'receipt.json')),
 release_invocation=json.loads((E/'release-invocation.json').read_bytes()),release_phase_exits={c['phase']:c['returncode'] for c in r['commands']},
 package=r['package'],release_trust=r['release_trust'],runtime=r['runtime'],rustc=r['rustc'],native_artifacts=r['native_artifacts'],unittests=r['unittests'],remote_rust_tests=dict(passed=2,failed=0,exit=0),
 current_acceptance=dict(path=str(E/'current-acceptance.json'),sha256=sha(E/'current-acceptance.json'),command=json.loads((E/'current-acceptance-command.json').read_bytes())),
 documented_commands=commands,solves=solves,solver_audit=dict(comment_id='7190',path=str(E/'solver-audit.json')),
 predecessor_receipts=[dict(path=p,sha256=sha(p)) for p in predecessor_paths],
 final_source_probe=dict(argv=argv,raw_exit=p.returncode,sha256=sha(final_probe/'spur_graph_source_identity.json'),extractor_id=source['extractor_id'],input_count=len(source['inputs'])),
 diff_check=dict(argv=checks_argv,raw_exit=check.returncode),
 limitations=['R7 documentation/verification only: RED not applicable; predecessor authentic TDD retained.','Observed native acceptance osx_arm64 only; remote Linux Rust tests do not assert Linux packaging.','Hosted macos-15 CI configured but not executed.','Immutable corrected corpus read-only: real sample/control parity, not full reinjest.','Root owns final integration and notebook acceptance; worker notebook access denied; no bypass.','Unsigned local package; no public publish/tag/push.','Solver persistence quota 512/513 rejected; exact nonpersisted requests/results retained and audited.'])
(E/'verification.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(dict(status=report['status'],binary_sha256=report['package']['binary']['sha256'],source_identity=report['package']['source_identity'],python_tests=report['unittests']['tests_run'],source_inputs=len(source['inputs']),owned_documents=checks,raw_exit=0)))
