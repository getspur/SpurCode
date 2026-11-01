use serde_json::{json, Value};
use spur_graph::panic_evidence::{
    collect_rust_call_sites, PanicBuildConfig, PanicEvidenceSource, PanicSemanticFacts,
    RustCallSiteEvidenceSet,
};
use spur_graph::{artifact_from_facts, build_facts, GraphIndexArtifact};

const SOURCE: &str = "fn plus(a: u8, b: u8) -> u8 { a + b }\nfn demo(flag: bool, x: u8) { let mut value = x; if flag { value = 1; } plus(value, 0); plus(value, 0); }\nfn other() {}\n";

fn sources() -> [PanicEvidenceSource<'static>; 1] {
    [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: SOURCE.as_bytes(),
    }]
}

fn fixture() -> anyhow::Result<(
    GraphIndexArtifact,
    RustCallSiteEvidenceSet,
    PanicSemanticFacts,
)> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir_all(root.path().join("src"))?;
    std::fs::write(root.path().join("src/lib.rs"), SOURCE)?;
    let (raw, _) = build_facts(root.path(), None)?;
    let graph = artifact_from_facts(&raw, root.path())?;
    let calls = collect_rust_call_sites(&graph, &sources())?;
    let build: PanicBuildConfig = serde_json::from_value(json!({
        "schema_version": "panic-build-config-v1", "graph_content_hash": graph.graph_content_hash,
        "target": "aarch64-apple-darwin", "edition": "2021", "cfg": [],
        "overflow_checks": true, "debug_assertions": true, "panic_strategy": "unwind",
        "rustc_verbose": "fixture rustc identity", "codegen_flags": [], "features": []
    }))?;
    let build_hash = build.content_hash()?;
    let demo = calls
        .bodies
        .iter()
        .find(|b| SOURCE[b.byte_range[0]..b.byte_range[1]].contains("let mut"))
        .unwrap();
    let loc = |range: [usize; 2], body: &str| json!({"body_id":body,"file_path":"src/lib.rs","source_oid":calls.sources[0].source_oid,"byte_range":range});
    let always = json!({"kind":"always"});
    let yes = json!({"kind":"when","expression_id":"e_flag","expected":true});
    let no = json!({"kind":"when","expression_id":"e_flag","expected":false});
    let ctx = |flow: &str, guard: &Value, text: &str| {
        let start = SOURCE[demo.byte_range[0]..demo.byte_range[1]]
            .find(text)
            .unwrap()
            + demo.byte_range[0];
        json!({"location":loc([start,start+text.len()],&demo.body_id),"flow_id":flow,"guard":guard})
    };
    let mut flow = Vec::new();
    let mut coverage = Vec::new();
    for b in &calls.bodies {
        let id = if b.body_id == demo.body_id {
            "f0".to_owned()
        } else {
            format!("entry:{}", b.body_id)
        };
        flow.push(json!({"flow_id":id,"location":loc(b.byte_range,&b.body_id),"order":0,"guard":always,"predecessors":[]}));
        coverage.push(json!({"body_id":b.body_id,"entry_flow_id":id,"unknown_obligations":[]}));
    }
    for (id, order, guard, pred, text) in [
        ("f1", 1, &yes, vec!["f0"], "value = 1"),
        ("f2", 2, &no, vec!["f0"], "if flag"),
        ("f3", 3, &always, vec!["f1", "f2"], "plus(value, 0)"),
        ("f4", 4, &always, vec!["f3"], "plus(value, 0)"),
        ("f5", 5, &always, vec!["f4"], "plus(value, 0)"),
    ] {
        flow.push(json!({"flow_id":id,"location":ctx(id,guard,text)["location"],"order":order,"guard":guard,"predecessors":pred}));
    }
    flow.sort_by_key(|f| {
        (
            f["location"]["body_id"].as_str().unwrap().to_owned(),
            f["order"].as_u64().unwrap(),
        )
    });
    coverage.sort_by_key(|b| b["body_id"].as_str().unwrap().to_owned());
    let u8_domain = json!({"kind":"u8","minimum":0,"maximum":255});
    let bool_domain = json!({"kind":"bool","values":[false,true]});
    let values = json!([
        {"value_id":"v_flag","binding_id":"flag","version":0,"sort":"bool","domain":bool_domain,"context":ctx("f0",&always,"flag"),"origin":{"kind":"parameter","index":0}},
        {"value_id":"v_x","binding_id":"x","version":0,"sort":"u8","domain":u8_domain,"context":ctx("f0",&always,"x"),"origin":{"kind":"parameter","index":1}},
        {"value_id":"v0","binding_id":"value","version":0,"sort":"u8","domain":u8_domain,"context":ctx("f0",&always,"x"),"origin":{"kind":"expression","expression_id":"e_x"}},
        {"value_id":"v1","binding_id":"value","version":1,"sort":"u8","domain":u8_domain,"context":ctx("f1",&yes,"value = 1"),"origin":{"kind":"assignment","assignment_id":"a1"}},
        {"value_id":"v2","binding_id":"value","version":2,"sort":"u8","domain":u8_domain,"context":ctx("f3",&always,"plus(value, 0)"),"origin":{"kind":"phi","inputs":[{"predecessor_flow_id":"f1","guard":yes,"value_id":"v1"},{"predecessor_flow_id":"f2","guard":no,"value_id":"v0"}]}}
    ]);
    let expressions = json!([
        {"expression_id":"e_flag","sort":"bool","domain":bool_domain,"context":ctx("f0",&always,"flag"),"derivation":{"kind":"value","value_id":"v_flag"}},
        {"expression_id":"e_one","sort":"u8","domain":{"kind":"u8","minimum":1,"maximum":1},"context":ctx("f1",&yes,"1"),"derivation":{"kind":"u8_literal","value":1}},
        {"expression_id":"e_value","sort":"u8","domain":u8_domain,"context":ctx("f4",&always,"value, 0"),"derivation":{"kind":"value","value_id":"v2"}},
        {"expression_id":"e_x","sort":"u8","domain":u8_domain,"context":ctx("f0",&always,"x"),"derivation":{"kind":"value","value_id":"v_x"}},
        {"expression_id":"e_zero","sort":"u8","domain":{"kind":"u8","minimum":0,"maximum":0},"context":ctx("f3",&always,"0"),"derivation":{"kind":"u8_literal","value":0}}
    ]);
    let mut sites = Vec::new();
    for (i, s) in calls.sites.iter().enumerate() {
        sites.push(json!({"site_id":s.site_id,"context":{"location":loc(s.byte_range,&s.body_id),"flow_id":format!("f{}",i+4),"guard":always},"semantics":{"kind":"primitive","contract_id":"plus-v1","arguments":["e_value","e_zero"]}}));
    }
    sites.sort_by_key(|s| s["site_id"].as_str().unwrap().to_owned());
    let mut values = values.as_array().unwrap().clone();
    values.sort_by_key(|v| v["value_id"].as_str().unwrap().to_owned());
    let facts: PanicSemanticFacts = serde_json::from_value(json!({
        "schema_version":"panic-semantic-facts-v1","graph_content_hash":graph.graph_content_hash,
        "build":build,"build_hash":build_hash,"sources":calls.sources,"bodies":calls.bodies,
        "body_coverage":coverage,"values":values,"expressions":expressions,"control_flow":flow,
        "assignments":[{"assignment_id":"a1","context":ctx("f1",&yes,"value = 1"),"target_value_id":"v1","expression_id":"e_one"}],
        "sites":sites,"primitive_contracts":[{"contract_id":"plus-v1","primitive":"u8_checked_add","build_hash":build_hash,
          "authority":{"kind":"bounded_source_model","reference":"R4 author-normalized fixture v1","sha256":"1111111111111111111111111111111111111111111111111111111111111111"}}],
        "unknown_obligations":[],"exploration":{"max_call_depth":1,"max_loop_iterations":1,"max_states":32,"frontiers":[]}
    }))?;
    Ok((graph, calls, facts))
}

fn accepted(json: Value, graph: &GraphIndexArtifact, calls: &RustCallSiteEvidenceSet) -> bool {
    serde_json::from_value::<PanicSemanticFacts>(json)
        .is_ok_and(|f| f.validate_against(graph, &sources(), calls).is_ok())
}

#[test]
fn typed_contract_round_trip_with_versions_phi_and_order() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    facts.validate_against(&graph, &sources(), &calls)?;
    facts.validate_complete_against(&graph, &sources(), &calls)?;
    facts.validate_for_build(&facts.build)?;
    let bytes = serde_json::to_vec(&facts)?;
    let loaded: PanicSemanticFacts = serde_json::from_slice(&bytes)?;
    assert_eq!(loaded, facts);
    assert_eq!(serde_json::to_vec(&loaded)?, bytes);
    println!(
        "R4_EXPORTED={}",
        json!({"facts":facts,"call_sites":calls,"source":SOURCE,"graph_content_hash":graph.graph_content_hash})
    );
    Ok(())
}

#[test]
fn rejects_missing_domains_origin_presence_and_reused_ids() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    for relation in ["expressions", "values"] {
        for field in ["domain", "sort", "context"] {
            let mut bad = serde_json::to_value(&facts)?;
            bad[relation][0].as_object_mut().unwrap().remove(field);
            assert!(
                !accepted(bad.clone(), &graph, &calls),
                "missing {relation}.{field}"
            );
            println!(
                "R4_INVALID={}",
                json!({"name":format!("missing_{relation}_{field}"),"facts":bad,"accepted":false})
            );
        }
    }
    for value in [Value::Null, json!({"kind":"unknown"})] {
        let mut bad = serde_json::to_value(&facts)?;
        bad["values"][0]["origin"] = value;
        assert!(!accepted(bad, &graph, &calls));
    }
    let mut bad = serde_json::to_value(&facts)?;
    bad["values"][0].as_object_mut().unwrap().remove("origin");
    assert!(!accepted(bad.clone(), &graph, &calls));
    println!(
        "R4_INVALID={}",
        json!({"name":"missing_origin","facts":bad,"accepted":false})
    );
    for (relation, key) in [
        ("sites", "site_id"),
        ("values", "value_id"),
        ("expressions", "expression_id"),
        ("control_flow", "flow_id"),
    ] {
        let mut bad = serde_json::to_value(&facts)?;
        bad[relation][1][key] = bad[relation][0][key].clone();
        assert!(
            !accepted(bad.clone(), &graph, &calls),
            "duplicate {relation}"
        );
        println!(
            "R4_INVALID={}",
            json!({"name":format!("duplicate_{relation}"),"facts":bad,"accepted":false})
        );
    }
    Ok(())
}

#[test]
fn rejects_stale_bindings_wrong_existing_body_and_detached_guards() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    let base = serde_json::to_value(&facts)?;
    let other = calls
        .bodies
        .iter()
        .find(|b| b.body_id != facts.sites[0].context.location.body_id)
        .unwrap();
    let mutations = [
        (
            "wrong_existing_body",
            "/sites/0/context/location/body_id",
            json!(other.body_id),
        ),
        (
            "stale_source",
            "/sites/0/context/location/source_oid",
            json!("stale"),
        ),
        ("stale_build", "/build/overflow_checks", json!(false)),
        ("stale_graph", "/graph_content_hash", json!("stale")),
        ("sort_mismatch", "/expressions/0/sort", json!("u8")),
        (
            "out_of_range",
            "/values/0/domain",
            json!({"kind":"u8","minimum":0,"maximum":256}),
        ),
        (
            "detached_guard",
            "/sites/0/context/guard",
            json!({"kind":"when","expression_id":"e_flag","expected":true}),
        ),
        (
            "missing_guard",
            "/sites/0/context/guard",
            json!({"kind":"when","expression_id":"missing","expected":true}),
        ),
        (
            "wrong_span",
            "/sites/0/context/location/byte_range",
            json!([0, 1]),
        ),
        (
            "wrong_origin_sort",
            "/values/3/origin",
            json!({"kind":"expression","expression_id":"e_x"}),
        ),
        ("phi_missing_arm", "/values/2/origin/inputs", json!([])),
        (
            "stale_authority",
            "/primitive_contracts/0/build_hash",
            json!("stale"),
        ),
        (
            "missing_authority",
            "/primitive_contracts/0/authority/sha256",
            json!(""),
        ),
    ];
    for (name, path, value) in mutations {
        let mut bad = base.clone();
        *bad.pointer_mut(path).unwrap() = value;
        assert!(!accepted(bad.clone(), &graph, &calls), "accepted {name}");
        println!(
            "R4_INVALID={}",
            json!({"name":name,"facts":bad,"accepted":false})
        );
    }
    let changed = SOURCE.to_owned() + "\n";
    assert!(facts
        .validate_against(
            &graph,
            &[PanicEvidenceSource {
                file_path: "src/lib.rs",
                bytes: changed.as_bytes()
            }],
            &calls
        )
        .is_err());
    let mut expected = facts.build.clone();
    expected.overflow_checks = false;
    assert!(facts.validate_for_build(&expected).is_err());
    // Self-consistency alone cannot authenticate a caller's expected build.
    let mut rehashed = facts.clone();
    rehashed.build = expected;
    rehashed.build_hash = rehashed.build.content_hash()?;
    assert!(rehashed.validate_for_build(&facts.build).is_err());
    Ok(())
}

#[test]
fn explicit_uncertainty_and_frontiers_survive_json_and_block_completeness() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    for kind in [
        "opaque",
        "mutation",
        "cfg",
        "parse",
        "callee",
        "call_depth",
        "loop_bound",
        "state_budget",
    ] {
        let mut json = serde_json::to_value(&facts)?;
        let location = json["sites"][0]["context"]["location"].clone();
        json["unknown_obligations"] = json!([{"obligation_id":"u0","kind":kind,"location":location,"detail":"explicit unresolved fixture obligation"}]);
        json["sites"][0]["semantics"] = json!({"kind":"unknown","obligation_id":"u0"});
        let body = json["sites"][0]["context"]["location"]["body_id"].clone();
        for b in json["body_coverage"].as_array_mut().unwrap() {
            if b["body_id"] == body {
                b["unknown_obligations"] = json!(["u0"]);
            }
        }
        if matches!(kind, "call_depth" | "loop_bound" | "state_budget") {
            json["exploration"]["frontiers"] = json!([{"frontier_id":"front0","kind":kind,"context":json["sites"][0]["context"],"obligation_id":"u0"}]);
        }
        let loaded: PanicSemanticFacts = serde_json::from_value(json.clone())?;
        loaded.validate_against(&graph, &sources(), &calls)?;
        assert!(loaded
            .validate_complete_against(&graph, &sources(), &calls)
            .is_err());
        assert_eq!(serde_json::to_value(&loaded)?, json);
        let mut lost = json.clone();
        lost["unknown_obligations"] = json!([]);
        assert!(!accepted(lost, &graph, &calls));
        println!(
            "R4_UNCERTAIN={}",
            json!({"kind":kind,"facts":loaded,"complete":false})
        );
    }
    Ok(())
}

#[test]
fn rejects_absence_reordering_cycles_and_incompatible_build_fields() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    for relation in ["sources", "bodies", "body_coverage", "sites"] {
        let mut bad = serde_json::to_value(&facts)?;
        bad[relation].as_array_mut().unwrap().pop();
        assert!(!accepted(bad, &graph, &calls), "omitted {relation}");
    }
    for relation in [
        "body_coverage",
        "values",
        "expressions",
        "control_flow",
        "sites",
    ] {
        let mut bad = serde_json::to_value(&facts)?;
        bad[relation].as_array_mut().unwrap().reverse();
        assert!(!accepted(bad, &graph, &calls), "unordered {relation}");
    }
    for field in [
        "target",
        "edition",
        "cfg",
        "overflow_checks",
        "debug_assertions",
        "panic_strategy",
    ] {
        let mut build = serde_json::to_value(&facts.build)?;
        build.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<PanicBuildConfig>(build).is_err(),
            "defaulted {field}"
        );
    }
    for (path, val) in [
        ("/schema_version", json!("future")),
        (
            "/expressions/0/derivation",
            json!({"kind":"not","operand":"e_flag"}),
        ),
        ("/control_flow/0/predecessors", json!(["f5"])),
        ("/expressions/0/domain", json!({"kind":"bool","values":[]})),
        ("/exploration/max_states", json!(0)),
    ] {
        let mut bad = serde_json::to_value(&facts)?;
        *bad.pointer_mut(path).unwrap() = val;
        assert!(!accepted(bad, &graph, &calls), "accepted {path}");
    }
    let mut forged = serde_json::to_value(&facts)?;
    forged["complete"] = json!(true);
    assert!(!accepted(forged, &graph, &calls));
    Ok(())
}

fn binding_extension(
    facts: &PanicSemanticFacts,
    binding: &str,
    sort: &str,
    origin: &str,
) -> anyhow::Result<PanicSemanticFacts> {
    let mut data = serde_json::to_value(facts)?;
    let context = facts
        .sites
        .iter()
        .max_by_key(|s| &s.context.flow_id)
        .unwrap()
        .context
        .clone();
    let expression = if sort == "bool" { "e_flag" } else { "e_zero" };
    let domain = if sort == "bool" {
        json!({"kind":"bool","values":[false,true]})
    } else {
        json!({"kind":"u8","minimum":0,"maximum":0})
    };
    let origin = match origin {
        "expression" => json!({"kind":"expression","expression_id":expression}),
        "assignment" => {
            data["assignments"].as_array_mut().unwrap().push(json!({
                "assignment_id":"a3","context":context,
                "target_value_id":"v3","expression_id":expression
            }));
            json!({"kind":"assignment","assignment_id":"a3"})
        }
        "unknown" => {
            data["unknown_obligations"] = json!([{
                "obligation_id":"u_binding","kind":"mutation",
                "location":context.location,"detail":"unresolved new binding version"
            }]);
            for body in data["body_coverage"].as_array_mut().unwrap() {
                if body["body_id"] == context.location.body_id {
                    body["unknown_obligations"] = json!(["u_binding"]);
                }
            }
            json!({"kind":"unknown","obligation_id":"u_binding"})
        }
        _ => unreachable!(),
    };
    let values = data["values"].as_array_mut().unwrap();
    values.push(json!({
        "value_id":"v3","binding_id":binding,"version":3,
        "sort":sort,"domain":domain,"context":context,"origin":origin
    }));
    values.sort_by_key(|v| v["value_id"].as_str().unwrap().to_owned());
    Ok(serde_json::from_value(data)?)
}

fn assert_binding_sort_changes_rejected(origin: &str) -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    let mut accepted_changes = Vec::new();
    for (binding, sort, direction) in [
        ("value", "bool", "u8_to_bool"),
        ("flag", "u8", "bool_to_u8"),
    ] {
        // Same row and derivation with a distinct binding is a valid shadow.
        // This isolates the cross-version invariant from local origin typing.
        let shadow = binding_extension(&facts, &format!("{binding}_shadow"), sort, origin)?;
        shadow.validate_against(&graph, &sources(), &calls)?;
        let bad = binding_extension(&facts, binding, sort, origin)?;
        let result = bad.validate_against(&graph, &sources(), &calls);
        println!(
            "R4_BINDING_CASE={}",
            json!({
                "name":format!("{direction}_{origin}"),"facts":bad,"accepted":result.is_ok()
            })
        );
        match result {
            Ok(()) => accepted_changes.push(direction),
            Err(error) => assert!(error.to_string().contains("binding sort")),
        }
    }
    assert!(
        accepted_changes.is_empty(),
        "accepted {accepted_changes:?} from {origin} in one lexical binding"
    );
    Ok(())
}

#[test]
fn rejects_binding_sort_changes_from_expressions() -> anyhow::Result<()> {
    assert_binding_sort_changes_rejected("expression")
}

#[test]
fn rejects_binding_sort_changes_from_assignments() -> anyhow::Result<()> {
    assert_binding_sort_changes_rejected("assignment")
}

#[test]
fn rejects_binding_sort_changes_from_unknown_origins() -> anyhow::Result<()> {
    assert_binding_sort_changes_rejected("unknown")
}

#[test]
fn rejects_binding_sort_changes_involving_parameter_and_phi_versions() -> anyhow::Result<()> {
    let (graph, calls, mut facts) = fixture()?;
    // Keep the U8 expression/assignment/phi versions distinct and ordered,
    // leaving version zero for the individually valid Bool parameter origin.
    for value in &mut facts.values {
        if value.binding_id == "value" {
            value.version += 1;
        }
    }
    facts.validate_against(&graph, &sources(), &calls)?;
    facts
        .values
        .iter_mut()
        .find(|v| v.value_id == "v_flag")
        .unwrap()
        .binding_id = "value".into();
    let result = facts.validate_against(&graph, &sources(), &calls);
    println!(
        "R4_BINDING_CASE={}",
        json!({
            "name":"parameter_and_phi","facts":facts,"accepted":result.is_ok()
        })
    );
    assert!(
        result.is_err(),
        "accepted Bool parameter with U8 versions of the same binding"
    );
    assert!(result.unwrap_err().to_string().contains("binding sort"));
    Ok(())
}

#[test]
fn accepts_repeated_binding_sorts_narrowing_and_distinct_shadows() -> anyhow::Result<()> {
    let (graph, calls, facts) = fixture()?;
    for (name, binding, sort) in [
        ("same_u8_narrowed", "value", "u8"),
        ("same_bool", "flag", "bool"),
        ("distinct_shadow", "value_shadow", "bool"),
    ] {
        let extended = binding_extension(&facts, binding, sort, "expression")?;
        extended.validate_complete_against(&graph, &sources(), &calls)?;
        let bytes = serde_json::to_vec(&extended)?;
        let loaded: PanicSemanticFacts = serde_json::from_slice(&bytes)?;
        assert_eq!(loaded, extended);
        println!(
            "R4_BINDING_CASE={}",
            json!({"name":name,"facts":loaded,"accepted":true})
        );
    }
    let mut duplicate = binding_extension(&facts, "value", "u8", "expression")?;
    duplicate
        .values
        .iter_mut()
        .find(|v| v.value_id == "v3")
        .unwrap()
        .version = 2;
    assert!(duplicate
        .validate_against(&graph, &sources(), &calls)
        .is_err());
    println!(
        "R4_BINDING_CASE={}",
        json!({"name":"duplicate_binding_version","facts":duplicate,"accepted":false})
    );
    Ok(())
}

#[test]
fn binding_sort_namespace_is_body_local() -> anyhow::Result<()> {
    let (graph, calls, mut facts) = fixture()?;
    let body = calls
        .bodies
        .iter()
        .find(|b| &SOURCE[b.byte_range[0]..b.byte_range[1]] == "{}")
        .unwrap();
    let flow = facts
        .control_flow
        .iter()
        .find(|f| f.location.body_id == body.body_id)
        .unwrap();
    facts.values.push(serde_json::from_value(json!({
        "value_id":"v_other","binding_id":"value","version":0,
        "sort":"bool","domain":{"kind":"bool","values":[false,true]},
        "context":{"location":flow.location,"flow_id":flow.flow_id,"guard":flow.guard},
        "origin":{"kind":"parameter","index":0}
    }))?);
    facts.values.sort_by(|a, b| a.value_id.cmp(&b.value_id));
    facts.validate_complete_against(&graph, &sources(), &calls)?;
    println!(
        "R4_BINDING_CASE={}",
        json!({"name":"body_local_namespace","facts":facts,"accepted":true})
    );
    Ok(())
}
