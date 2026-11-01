use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use spur_graph::panic_evidence::*;
use spur_solver::service::SolverService;
use spur_solver::types::{ConstraintExpr, ConstraintItem, ConstraintOp, SolveStatus};

#[allow(dead_code)]
#[path = "../examples/panic_evidence.rs"]
mod example;

const BOUNDS: &str = include_str!("fixtures/panic_analysis/bounds.rs");
const OVERFLOW: &str = include_str!("fixtures/panic_analysis/overflow.rs");
const CHECKED: &str = include_str!("fixtures/panic_analysis/checked.rs");

fn export(name: &str, value: &Value) -> anyhow::Result<()> {
    if let Some(dir) = std::env::var_os("R8_EVIDENCE_DIR") {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(Path::new(&dir).join(format!("{name}.json")), serde_json::to_vec_pretty(value)?)?;
    }
    Ok(())
}

fn compiler(overflow: bool) -> anyhow::Result<(String, String, Vec<PanicCfgAtom>)> {
    let version = Command::new("rustc").arg("-vV").output()?;
    anyhow::ensure!(version.status.success(), "rustc version failed");
    let verbose = String::from_utf8(version.stdout)?;
    let host = verbose.lines().find_map(|l| l.strip_prefix("host: ")).unwrap().to_owned();
    let cfg = Command::new("rustc").args(["--print", "cfg", "-C", &format!("overflow-checks={overflow}"), "-C", "debug-assertions=true", "-C", "panic=unwind"]).output()?;
    anyhow::ensure!(cfg.status.success(), "rustc cfg failed");
    let mut atoms = Vec::new();
    for line in String::from_utf8(cfg.stdout)?.lines() {
        let (key, value) = match line.split_once('=') {
            Some((k,v)) => (k.to_owned(), Some(serde_json::from_str(v)?)),
            None => (line.to_owned(), None),
        };
        atoms.push(PanicCfgAtom {key,value});
    }
    atoms.sort();
    Ok((verbose, host, atoms))
}

fn analyze_request(source: &str, overflow: bool) -> anyhow::Result<Value> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("src"))?;
    std::fs::write(dir.path().join("src/lib.rs"), source)?;
    let (facts, _) = spur_graph::build_facts(dir.path(), None)?;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path())?;
    let (rustc_verbose, target, cfg) = compiler(overflow)?;
    Ok(json!({"command":"analyze", "graph":graph,
        "sources":[{"file_path":"src/lib.rs", "text":source}],
        "build":{"schema_version":PANIC_BUILD_CONFIG_SCHEMA_VERSION,
            "graph_content_hash":graph.graph_content_hash,"target":target,"edition":"2021",
            "cfg":cfg,"overflow_checks":overflow,"debug_assertions":true,"panic_strategy":"unwind",
            "rustc_verbose":rustc_verbose,"codegen_flags":[],"features":[]}}))
}

fn solve_request(analyze: &Value, output: &Value, minimum: u16, maximum: u16) -> Value {
    let facts = &output["facts"];
    let body = &facts["bodies"][0]["body_id"];
    json!({"command":"solve", "graph":analyze["graph"],"sources":analyze["sources"],
        "build":analyze["build"],"facts":facts,
        "domains":[{"body_id":body,"parameter_index":0,"domain":{"kind":"u8","minimum":minimum,"maximum":maximum}}],
        "limits":{"entry_body_ids":[body],"expected_build_hash":facts["build_hash"],
            "max_call_depth":0,"max_loop_iterations":0,"max_states":4096,"timeout_ms":30000,"persist":true}})
}

fn replay_request(solve: &Value, report: &Value, payload: &str) -> Value {
    let mut request=solve.clone();
    request["command"]=json!("replay");
    request["report"]=report.clone();
    request["query_index"]=json!(0);
    request["expected_request_identity"]=report["queries"][0]["request_identity"].clone();
    request["expected_panic_payload"]=json!(payload);
    request["execution_timeout_ms"]=json!(30000);
    request
}

#[test]
fn json_analyze_collects_canonical_facts_and_keeps_legacy_execute() -> anyhow::Result<()> {
    let request=analyze_request(BOUNDS,true)?;
    let output=example::execute(request.clone());
    assert!(output.is_ok(), "analyze must be wired: {output:?}");
    let output=output?;
    assert_eq!(output["status"],"collected");
    let facts:PanicSemanticFacts=serde_json::from_value(output["facts"].clone())?;
    let graph=serde_json::from_value(request["graph"].clone())?;
    let build=serde_json::from_value(request["build"].clone())?;
    facts.validate_canonical(&graph,&[PanicEvidenceSource{file_path:"src/lib.rs",bytes:BOUNDS.as_bytes()}],&build)?;
    assert!(example::execute(json!({"command":"collect","graph":request["graph"],"sources":request["sources"]}))?["method_sites"].is_object());
    export("analyze",&json!({"request":request,"output":output}))?;
    Ok(())
}

#[tokio::test]
async fn real_bounds_overflow_and_option_witnesses_replay_exact_payloads() -> anyhow::Result<()> {
    let storage=tempfile::tempdir()?;
    let service=SolverService::new().with_repo_root(storage.path());
    for overflow in [false,true] {
        for (name,source,input,payload) in [
            ("bounds",BOUNDS,3,"index out of bounds: the len is 3 but the index is 3"),
            ("overflow",OVERFLOW,255,"attempt to add with overflow"),
            ("checked",CHECKED,255,"called `Option::unwrap()` on a `None` value"),
        ] {
            let analyze=analyze_request(source,overflow)?;
            let collected=example::execute(analyze.clone())?;
            let solve=solve_request(&analyze,&collected,input,input);
            let output=example::execute_async(solve.clone(),&service).await?;
            let report=&output["report"];
            let q=&report["queries"][0];
            let id=q["response"]["solve_id"].as_str().unwrap();
            let reload=service.get_solve_result(id)?;
            assert_eq!(reload.request,q["request"]);
            let candidate=!(name=="overflow" && !overflow);
            assert_eq!(q["outcome"],if candidate {"candidate_witness"} else {"no_counterexample_within_complete_scope"});
            let replay=example::execute_async(replay_request(&solve,report,payload),&service).await;
            if candidate {
                assert_eq!(q["candidate"]["runtime_reproduced"],false);
                let replay=replay?;
                assert_eq!(replay["replay"]["runtime_reproduced"],true);
                assert_eq!(replay["replay"]["actual_panic_payload"],payload);
                assert_eq!(replay["replay"]["request_identity"],q["request_identity"]);
                assert_eq!(replay["replay"]["build_hash"],report["build_hash"]);
                assert_eq!(replay["replay"]["sources"],report["sources"]);
                export(&format!("{name}-{overflow}"),&json!({"analyze":analyze,"solve":solve,"output":output,"reload":reload,"replay":replay}))?;
            } else {
                assert!(replay.is_err(),"UNSAT is never a runtime reproduction");
                export(&format!("{name}-{overflow}"),&json!({"analyze":analyze,"solve":solve,"output":output,"reload":reload}))?;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn missing_facts_domains_and_changed_identities_fail_closed() -> anyhow::Result<()> {
    let storage=tempfile::tempdir()?;
    let service=SolverService::new().with_repo_root(storage.path());
    let analyze=analyze_request(OVERFLOW,true)?;
    let output=example::execute(analyze.clone())?;
    let solve=solve_request(&analyze,&output,255,255);
    let solved=example::execute_async(solve.clone(),&service).await?;
    let replay=replay_request(&solve,&solved["report"],"attempt to add with overflow");
    let mut results=vec![];
    for (pointer,value) in [
        ("/facts/extraction",Value::Null),("/facts/sites",json!([])),
        ("/facts/control_flow",json!([])),("/facts/body_coverage",json!([])),
        ("/sources/0/text",json!("fn probe(value:u8)->u8{value}")),
        ("/build/overflow_checks",json!(false)),("/limits/expected_build_hash",json!("changed")),
        ("/domains",json!([])),("/domains/0/domain/maximum",json!(256)),
    ] {
        let mut changed=solve.clone(); *changed.pointer_mut(pointer).unwrap()=value;
        let result=example::execute_async(changed,&service).await;
        assert!(result.is_err(),"accepted changed {pointer}");
        results.push(json!({"pointer":pointer,"rejected":true}));
    }
    for (pointer,value) in [
        ("/expected_request_identity",json!("changed")),
        ("/report/build_hash",json!("changed")),
        ("/report/sources/0/source_oid",json!("changed")),
        ("/report/queries/0/request_digest",json!("changed")),
        ("/report/queries/0/receipt_binding",json!("changed")),
        ("/report/queries/0/candidate/runtime_reproduced",json!(true)),
        ("/report/queries/0/candidate/inputs/input_0",json!(254)),
    ] {
        let mut changed=replay.clone(); *changed.pointer_mut(pointer).unwrap()=value;
        assert!(example::execute_async(changed,&service).await.is_err(),"accepted replay {pointer}");
        results.push(json!({"pointer":pointer,"rejected":true}));
    }
    let mut mismatch=replay;
    mismatch["expected_panic_payload"]=json!("wrong payload");
    let mismatch=example::execute_async(mismatch,&service).await?;
    assert_eq!(mismatch["replay"]["runtime_reproduced"],false);
    assert_eq!(mismatch["replay"]["outcome"],"panic_payload_mismatch");
    export("negatives",&json!({"results":results,"mismatch":mismatch}))?;
    Ok(())
}

#[tokio::test]
async fn held_out_names_shadows_cfg_and_opaque_calls_preserve_uncertainty() -> anyhow::Result<()> {
    let storage=tempfile::tempdir()?;
    let service=SolverService::new().with_repo_root(storage.path());
    for (name,source,unknown) in [
        ("shadow",include_str!("fixtures/panic_analysis/shadow.rs"),false),
        ("assertion",include_str!("fixtures/panic_analysis/assertion.rs"),true),
        ("opaque",include_str!("fixtures/panic_analysis/opaque.rs"),true),
        ("trait","trait Hidden {fn is_some(&self)->bool;} impl Hidden for Option<u8>{fn is_some(&self)->bool{None::<u8>.unwrap();true}} fn renamed(x:u8)->u8{let v=Some(x);if v.is_some(){0}else{0}}",true),
    ] {
        let analyze=analyze_request(source,true)?;
        let collected=example::execute(analyze.clone())?;
        let mut solve=solve_request(&analyze,&collected,8,8);
        // The entry is the last top-level function, regardless of held-out spelling.
        let bodies=collected["facts"]["bodies"].as_array().unwrap();
        let id=bodies.iter().max_by_key(|b|b["byte_range"][0].as_u64()).unwrap()["body_id"].clone();
        solve["limits"]["entry_body_ids"]=json!([id]);
        solve["domains"][0]["body_id"]=id;
        let result=example::execute_async(solve.clone(),&service).await?;
        assert_eq!(result["report"]["queries"][0]["outcome"],if unknown {"incomplete_semantics"} else {"candidate_witness"});
        export(name,&json!({"analyze":analyze,"solve":solve,"output":result}))?;
    }
    let request=analyze_request(include_str!("fixtures/panic_analysis/cfg.rs"),true)?;
    let result=example::execute(request.clone())?;
    assert_eq!(result["status"],"unknown");
    assert_eq!(result["obligation"]["kind"],"cfg");
    assert!(result.get("facts").is_none());
    export("cfg",&json!({"request":request,"output":result}))?;
    Ok(())
}

#[tokio::test]
async fn domains_and_loop_call_state_bounds_never_become_general_safety() -> anyhow::Result<()> {
    let service=SolverService::new();
    for (name,source,depth,iterations,states,expected) in [
        ("bounded",OVERFLOW,0,0,4096,"no_counterexample_within_complete_scope"),
        ("state",OVERFLOW,0,0,1,"incomplete_semantics"),
        ("call","fn helper(x:u8)->u8{x.checked_add(1).unwrap()} fn probe(x:u8)->u8{helper(x)}",0,0,4096,"incomplete_semantics"),
        ("loop","fn probe(x:u8)->u8{for y in 0u8..3 {if x==y && y==2{return None::<u8>.unwrap();}} 0}",0,2,4096,"incomplete_semantics"),
    ] {
        let analyze=analyze_request(source,true)?;
        let collected=example::execute(analyze.clone())?;
        let mut solve=solve_request(&analyze,&collected,0,254);
        let body=collected["facts"]["bodies"].as_array().unwrap().iter().max_by_key(|b|b["byte_range"][0].as_u64()).unwrap()["body_id"].clone();
        solve["limits"]["entry_body_ids"]=json!([body]); solve["domains"][0]["body_id"]=body;
        solve["limits"]["max_call_depth"]=json!(depth);
        solve["limits"]["max_loop_iterations"]=json!(iterations);
        solve["limits"]["max_states"]=json!(states);
        solve["limits"]["persist"]=json!(false);
        let output=example::execute_async(solve.clone(),&service).await?;
        assert_eq!(output["report"]["queries"][0]["outcome"],expected,"{name}");
        export(name,&json!({"analyze":analyze,"solve":solve,"output":output}))?;
    }
    Ok(())
}

#[tokio::test]
async fn enumerate_every_u8_with_real_solver_and_ordinary_rustc() -> anyhow::Result<()> {
    let service=SolverService::new();
    for overflow in [false,true] {
        for (name,source) in [("bounds",BOUNDS),("overflow",OVERFLOW),("checked",CHECKED)] {
            let analyze=analyze_request(source,overflow)?;
            let collected=example::execute(analyze.clone())?;
            let mut solve=solve_request(&analyze,&collected,0,255);
            solve["limits"]["persist"]=json!(false);
            let solved=example::execute_async(solve,&service).await?;
            let query:PanicAnalysisQuery=serde_json::from_value(solved["report"]["queries"][0].clone())?;
            let dir=tempfile::tempdir()?;
            std::fs::write(dir.path().join("fixture.rs"),source)?;
            let harness="include!(\"fixture.rs\"); fn main(){std::panic::set_hook(Box::new(|_|{}));for x in 0..=255u8{if std::panic::catch_unwind(||probe(x)).is_err(){println!(\"{}\",x);}}}";
            std::fs::write(dir.path().join("main.rs"),harness)?;
            let binary=dir.path().join("native");
            let compiled=Command::new("rustc").arg(dir.path().join("main.rs")).args(["--edition=2021","-C","panic=unwind","-C","debug-assertions=true","-C",&format!("overflow-checks={overflow}"),"-o"]).arg(&binary).output()?;
            assert!(compiled.status.success(),"{}",String::from_utf8_lossy(&compiled.stderr));
            let native=Command::new(&binary).output()?;
            assert!(native.status.success());
            let native:Vec<u8>=String::from_utf8(native.stdout)?.lines().map(|l|l.parse().unwrap()).collect();
            let expected:Vec<u8>=(0..=255u8).filter(|&x|match name{"bounds"=>[11u8,22,33].get(x as usize).is_none(),"overflow"=>overflow && x.overflowing_add(1).1,_=>x.checked_add(1).is_none()}).collect();
            assert_eq!(native,expected);
            let mut observed=vec![];
            for x in 0..=255u8 {
                let mut request=query.request.clone();
                request.constraints.push(ConstraintItem::Bare(ConstraintExpr::Op{op:ConstraintOp::Eq,args:vec![ConstraintExpr::Var{name:query.inputs[0].variable.clone()},ConstraintExpr::Int{value:x.into()}]}));
                let result=service.solve_constraints(request).await?;
                assert_eq!(result.status,if expected.contains(&x){SolveStatus::Sat}else{SolveStatus::Unsat},"{name} overflow={overflow} x={x}");
                if result.status==SolveStatus::Sat {observed.push(x);}
            }
            export(&format!("enumerated-{name}-{overflow}"),&json!({"source":source,"build":analyze["build"],"request":query.request,"expected":expected,"native":native,"solver":observed,"inputs":256}))?;
        }
    }
    Ok(())
}
