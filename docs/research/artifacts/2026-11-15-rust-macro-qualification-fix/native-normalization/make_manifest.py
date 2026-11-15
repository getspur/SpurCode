from pathlib import Path
import hashlib,json,re,subprocess
E=Path(__file__).parent
base="99760f54763a2cb365f0223f4e7a2af7e0e09c2c"
changed=subprocess.check_output(["git","diff","--name-only",base,"HEAD"],text=True).splitlines()
assert changed==["crates/spur-graph/src/extract/languages.rs","crates/spur-graph/tests/rust_macro_qualified_calls.rs"],changed
runs=[]
for name in ["red","red-assertion","green-focused","red-corrected","green-corrected","green-related","green-lib-macro","fmt-complete","diff-check-complete"]:
    r=json.loads((E/(name+".json")).read_text()); log=Path(r["log"]).read_bytes()
    assert hashlib.sha256(log).hexdigest()==r["log_sha256"]
    r["test_results"]=[{"status":s,"passed":int(p),"failed":int(f),"ignored":int(i),"measured":int(m),"filtered":int(x)} for s,p,f,i,m,x in re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out",log.decode())]
    runs.append(r)
runmap={r["name"]:r for r in runs}
final_hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in map(Path,changed)}
for name in ["green-corrected","green-related","green-lib-macro","fmt-complete","diff-check-complete"]:
    assert runmap[name]["source_sha256"]==final_hashes,(name,runmap[name]["source_sha256"],final_hashes)
assert runmap["red-corrected"]["source_sha256"][changed[0]]=="30cc240dcb5ea74a4e9cc0d7a5b1acfe6cd43ae41160ed055a12673a63c16468"
assert runmap["red-corrected"]["source_sha256"][changed[1]]==final_hashes[changed[1]]
assert runmap["red-corrected"]["exit_code"]==101
assert runmap["red-corrected"]["test_results"]==[dict(status="FAILED",passed=14,failed=6,ignored=0,measured=0,filtered=0)]
for name in ["green-corrected","green-related","green-lib-macro","fmt-complete","diff-check-complete"]: assert runmap[name]["exit_code"]==0
assert runmap["green-corrected"]["test_results"][0]["passed"]==20
assert sum(x["passed"] for x in runmap["green-related"]["test_results"])==55
assert runmap["green-lib-macro"]["test_results"][0]["passed"]==2
current=json.loads((E/"final-path-results.json").read_text())
names={"plain":"multisegment_plain_path_keeps_local_target","spaces":"multisegment_spaced_path_keeps_local_target","internal_block":"multisegment_internal_block_comment_keeps_local_target","internal_line":"multisegment_internal_line_comment_keeps_local_target","trailing_block":"multisegment_trailing_block_comment_keeps_local_target","trailing_line":"multisegment_trailing_line_comment_keeps_local_target","all_comments":"multisegment_all_comments_keep_local_target"}
matrix=[]
red=Path(runmap["red-corrected"]["log"]).read_text();green=Path(runmap["green-corrected"]["log"]).read_text()
for variant,test in names.items():
    assert len(current[variant])==1
    edge=current[variant][0]
    assert f"test {test} ... ok" in green
    before="ok" if edge["target_node_id"] is not None else "FAILED"
    assert f"test {test} ... {before}" in red
    matrix.append({"variant":variant,"test":test,"source99760":{"scope_text":edge["scope_text"],"target_node_id":edge["target_node_id"]},"new_asserted":{"scope_text":"outer::local","target_declaration":"pub fn deep() -> u8 { 7 }","unique_edge":True,"bind_method":"macro_body_singleton","confidence_score":0.8,"bytes_filesystem_all_five_equal":True},"red_test":before,"green_test":"ok"})
(E/"variant-matrix.json").write_text(json.dumps(matrix,indent=2)+"\n")
patch=subprocess.check_output(["git","diff",base,"HEAD"]);(E/"repair.patch").write_bytes(patch)
status=subprocess.check_output(["git","status","--porcelain"],text=True)
assert not status,status
p=Path(changed[1]);assert p.read_bytes().startswith(subprocess.check_output(["git","show",base+":"+str(p)]))
receipts={"task_id":"bd-2vu9","root_issue":"bd-38yz","plan_id":"455c060a-0257-473e-94e4-e7ac14ea7419","delegation_id":"9fe5433f2fdc4e6b","base_commit":base,"test_commit":subprocess.check_output(["git","rev-parse","HEAD^"],text=True).strip(),"fix_commit":subprocess.check_output(["git","rev-parse","HEAD"],text=True).strip(),"changed_files":changed,"source_sha256":{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in map(Path,changed)},"patch":str(E/"repair.patch"),"patch_sha256":hashlib.sha256(patch).hexdigest(),"clean_worker_tree":True,"original_11_tests_byte_identical":True,"native_green_total":77,"source99760_variants_resolved":3,"new_variants_resolved":7,"variant_matrix":str(E/"variant-matrix.json"),"variant_matrix_sha256":hashlib.sha256((E/"variant-matrix.json").read_bytes()).hexdigest(),"runs":runs,"fixture_inputs":json.loads((E/"fixture-inputs.json").read_text()),"solve":"TDD-only per parent catalog: no syntax rule or invented bounded choice.","retained_failures":{"red":"Sandbox AWS endpoint unavailable; no tests ran; retried remote with network access.","green-focused":"All seven reviewer variants passed, but the mixed fixture wrongly expected bare deep to resolve among three candidates. Corrected unique helper control, amended test-only commit, restored exact baseline and repeated remote assertion RED before restoring production. No resolver change."},"baseline_caveat":"Supplied old-extension baseline-path-results.json has all_comments=[]; do not infer 7/7 old-extension bindings from that file. Matrix uses actual 99760 evidence and committed remote tests.","warnings":"Known unrelated dead-code warnings: integration builds as_bool and SymbolEmbedder::new; lib-test build file_oid_match_after_first_stat and as_bool. No repair warnings.","limits":["Parent owns rebuilt binary/extension, notebook SQL and frozen-file checks.","No main integration, approval, or issue closure.","New matrix records assertions from public BytesExtractor/filesystem tests, not a rebuilt SQL artifact."],"beads_audits_before_completion":["6802","6808","6809"]}
(E/"verification.json").write_text(json.dumps(receipts,indent=2)+"\n")
print(json.dumps({k:receipts[k] for k in ["test_commit","fix_commit","native_green_total","source99760_variants_resolved","new_variants_resolved","clean_worker_tree","source_sha256","patch_sha256"]},indent=2))
print("verification_sha256",hashlib.sha256((E/"verification.json").read_bytes()).hexdigest())
