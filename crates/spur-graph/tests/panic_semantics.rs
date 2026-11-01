use serde_json::{json, Value};
use spur_graph::panic_evidence::{
    collect_panic_semantics, collect_rust_call_sites, PanicBuildConfig, PanicEvidenceSource,
    PanicSemanticFacts,
};
use spur_graph::{artifact_from_facts, build_facts, GraphIndexArtifact};

fn collect(
    source: &str,
    overflow: bool,
) -> anyhow::Result<(GraphIndexArtifact, PanicSemanticFacts)> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("src"))?;
    std::fs::write(dir.path().join("src/lib.rs"), source)?;
    let (raw, _) = build_facts(dir.path(), None)?;
    let graph = artifact_from_facts(&raw, dir.path())?;
    let build: PanicBuildConfig = serde_json::from_value(json!({
        "schema_version":"panic-build-config-v1", "graph_content_hash":graph.graph_content_hash,
        "target":"aarch64-apple-darwin", "edition":"2021", "cfg":[],
        "overflow_checks":overflow, "debug_assertions":true, "panic_strategy":"unwind",
        "rustc_verbose":"R5 explicitly declared source-model build", "codegen_flags":[], "features":[]
    }))?;
    let sources = [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: source.as_bytes(),
    }];
    let facts = collect_panic_semantics(&graph, &sources, &build)?;
    let calls = collect_rust_call_sites(&graph, &sources)?;
    facts.validate_against(&graph, &sources, &calls)?;
    facts.validate_canonical(&graph, &sources, &build)?;
    println!("R5_R3={}", json!({"source":source,"calls":calls}));
    Ok((graph, facts))
}

fn unwraps(f: &Value) -> Vec<&Value> {
    f["sites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["semantics"]["kind"] == "option_unwrap")
        .collect()
}

#[test]
fn collects_mutation_shadowing_and_reaching_phi_from_renamed_source() -> anyhow::Result<()> {
    for (name, local, spacing) in [("mutated", "value", " "), ("relabelled", "parcel", "\n  ")] {
        let source = format!("fn {name}(x:u8)->u8 {{{spacing}let mut {local}=Some(x);{spacing}if x==3 {{{local}=None;}}{spacing}if x==3 {{{local}.unwrap()}} else {{0}} }}");
        let (_, facts) = collect(&source, true)?;
        assert!(
            facts.unknown_obligations.is_empty(),
            "{:?}",
            facts.unknown_obligations
        );
        let f = serde_json::to_value(&facts)?;
        assert_eq!(unwraps(&f).len(), 1);
        assert!(facts.values.iter().any(|v| matches!(
            v.origin,
            spur_graph::panic_evidence::PanicValueOrigin::Phi { .. }
        )));
        assert!(facts.assignments.iter().any(|a| {
            source[a.context.location.byte_range[0]..a.context.location.byte_range[1]]
                .contains("None")
                && matches!(
                    a.context.guard,
                    spur_graph::panic_evidence::PanicGuard::When { .. }
                )
        }));
        println!(
            "R5_EXPORTED={}",
            json!({"name":name,"source":source,"facts":facts})
        );
    }
    let source = "fn shadows(x:u8)->u8 { let value=None::<u8>; { let value=Some(x); if x==7 {return value.unwrap();} } if x==8 {value.unwrap()} else {0} }";
    let (_, facts) = collect(source, true)?;
    let f = serde_json::to_value(&facts)?;
    let sites = unwraps(&f);
    assert_eq!(sites.len(), 2);
    assert_ne!(
        sites[0]["semantics"]["is_some"],
        sites[1]["semantics"]["is_some"]
    );
    assert!(facts.unknown_obligations.is_empty());
    println!(
        "R5_EXPORTED={}",
        json!({"name":"shadowed","source":source,"facts":facts})
    );
    Ok(())
}

#[test]
fn short_circuit_guard_is_local_and_unrelated_if_does_not_dominate() -> anyhow::Result<()> {
    for (name, tail, guarded) in [
        (
            "shortcircuit",
            "if x==0 && value.unwrap()==0 {1} else {0}",
            true,
        ),
        (
            "non_dominating",
            "if x==0 {let _=value.is_some();} value.unwrap()",
            false,
        ),
        (
            "or_guard",
            "if x!=0 || value.unwrap()==0 {1} else {0}",
            true,
        ),
    ] {
        let source =
            format!("fn {name}(x:u8)->u8 {{let value=if x==0 {{Some(x)}} else {{None}}; {tail} }}");
        let (_, facts) = collect(&source, true)?;
        let f = serde_json::to_value(&facts)?;
        let sites = unwraps(&f);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0]["context"]["guard"]["kind"] == "when", guarded);
        assert!(
            facts.unknown_obligations.is_empty(),
            "{:?}",
            facts.unknown_obligations
        );
        println!(
            "R5_EXPORTED={}",
            json!({"name":name,"source":source,"facts":facts})
        );
    }
    Ok(())
}

#[test]
fn collects_checked_and_build_dependent_add_index_calls_and_literal_loop() -> anyhow::Result<()> {
    let cases = [
        ("checked", "fn f(x:u8)->u8 {x.checked_add(1).unwrap()}"),
        ("guarded_add", "fn f(x:u8)->u8 {if x<255 {x.checked_add(1).unwrap()} else {0}}"),
        ("add", "fn f(x:u8)->u8 {x+1}"),
        ("index", "fn f(x:u8)->u8 {[10u8,20,30][x as usize]}"),
        ("call", "fn renamed(n:u8)->u8 {let v=if n==0 {Some(n)} else {None};v.unwrap()} fn caller(x:u8)->u8 {renamed(x)}"),
        ("loop", "fn f(x:u8)->u8 {for step in 0u8..3 {if x==step && step==2 {return None::<u8>.unwrap();}} 0}"),
    ];
    for (name, source) in cases {
        let (_, facts) = collect(source, true)?;
        let f = serde_json::to_value(&facts)?;
        assert!(f["extraction"].is_object());
        match name {
            "checked" | "guarded_add" => assert_eq!(unwraps(&f).len(), 1),
            "add" | "index" => {
                assert!(!f["extraction"]["operations"].as_array().unwrap().is_empty())
            }
            "call" => assert!(f["sites"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["semantics"]["kind"] == "direct_call")),
            "loop" => assert_eq!(f["extraction"]["loops"].as_array().unwrap().len(), 1),
            _ => unreachable!(),
        }
        println!(
            "R5_EXPORTED={}",
            json!({"name":name,"source":source,"facts":facts})
        );
    }
    let source = "fn f(x:u8)->u8 {x+1}";
    let (_, on) = collect(source, true)?;
    let (_, off) = collect(source, false)?;
    assert_ne!(on.build_hash, off.build_hash);
    println!(
        "R5_EXPORTED={}",
        json!({"name":"add_off","source":source,"facts":off})
    );
    Ok(())
}

#[test]
fn unknown_effects_custom_dispatch_and_unsupported_syntax_remain_obligations() -> anyhow::Result<()>
{
    for source in [
        "struct Lookalike; impl Lookalike {fn unwrap(self)->u8 {7}} fn f()->u8 {Lookalike.unwrap()}",
        "fn f(x:u8)->u8 {let mut v=Some(x); malicious(&mut v); v.unwrap()}",
        "fn Some(x:u8)->u8 {x} fn f(x:u8)->u8 {Some(x).unwrap()}",
        "fn f(x:u8)->u8 {let Some=|a|a; Some(x).unwrap()}",
        "fn f(x:u8)->u8 {let v=x as i8; v.unwrap()}",
        "fn f(x:u8)->u8 {let (a,b)=(x,x); a}",
        "fn f(x:u8)->u8 {opaque!(x); x}",
    ] {
        let (_, facts) = collect(source, true)?;
        assert!(!facts.unknown_obligations.is_empty(), "accepted {source}");
        let f = serde_json::to_value(&facts)?;
        if source.contains("malicious") {assert_eq!(unwraps(&f).len(), 0);}
        println!("R5_UNSUPPORTED={}", json!({"source":source,"facts":facts}));
    }
    Ok(())
}

#[test]
fn canonical_validation_rejects_manually_changed_conclusions() -> anyhow::Result<()> {
    let source = "fn f(x:u8)->u8 {let v=Some(x);v.unwrap()}";
    let (graph, facts) = collect(source, true)?;
    let mut forged = facts.clone();
    let index = forged
        .expressions
        .iter()
        .position(|e| {
            matches!(
                e.derivation,
                spur_graph::panic_evidence::PanicExpressionDerivation::BoolLiteral { value: true }
            )
        })
        .unwrap();
    forged.expressions[index].derivation =
        spur_graph::panic_evidence::PanicExpressionDerivation::BoolLiteral { value: false };
    forged.expressions[index].domain = spur_graph::panic_evidence::PanicValueDomain::Bool {
        values: vec![false],
    };
    let sources = [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: source.as_bytes(),
    }];
    // A self-consistent authored conclusion can pass structural checks while
    // disagreeing with source. Only recollection authenticates extraction.
    forged.validate_against(
        &graph,
        &sources,
        &collect_rust_call_sites(&graph, &sources)?,
    )?;
    assert!(forged
        .validate_canonical(
            &graph,
            &[PanicEvidenceSource {
                file_path: "src/lib.rs",
                bytes: source.as_bytes()
            }],
            &facts.build
        )
        .is_err());
    Ok(())
}

#[test]
fn unsupported_types_and_lexical_helper_shadows_cannot_supply_facts() -> anyhow::Result<()> {
    for source in [
        "fn f()->u8 {let a:u16=255; let _=a+1; 0}",
        "fn f()->u8 {let a=255; let _=a+1; 0}",
        "fn helper(x:u8)->u8 {x} fn f(x:u8)->u8 {fn helper(x:u8)->u8 {None::<u8>.unwrap()} helper(x)}",
    ] {
        let (_, facts) = collect(source, true)?;
        let f=serde_json::to_value(&facts)?;
        if source.contains("fn helper") {
            let caller=facts.bodies.iter().find(|b| source[b.byte_range[0]..b.byte_range[1]].contains("fn helper")).unwrap();
            assert!(facts.sites.iter().filter(|s|s.context.location.body_id==caller.body_id).all(|s|!matches!(s.semantics, spur_graph::panic_evidence::PanicSiteSemantics::DirectCall {..})));
        } else { assert!(!facts.unknown_obligations.is_empty(), "accepted unsupported source {source}"); }
        println!("R5_UNSUPPORTED={}",json!({"source":source,"facts":f}));
    }
    Ok(())
}

#[test]
fn collects_original_semantic_shapes_without_name_based_selection() -> anyhow::Result<()> {
    let source=include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/semantics/fixtures/lib.rs");
    for overflow in [true, false] {
        let (_, facts) = collect(source, overflow)?;
        for coverage in &facts.body_coverage {
            let body = facts
                .bodies
                .iter()
                .find(|b| b.body_id == coverage.body_id)
                .unwrap();
            let body_source = &source[body.byte_range[0]..body.byte_range[1]];
            if !body_source.contains("Lookalike")
                && !body_source.contains("for step")
                && body_source != "{ 7 }"
            {
                assert!(
                    coverage.unknown_obligations.is_empty(),
                    "{body_source}: {:?}",
                    coverage.unknown_obligations
                );
            }
        }
        println!(
            "R5_EXPORTED={}",
            json!({"name":if overflow {"corpus_on"} else {"corpus_off"},"source":source,"facts":facts})
        );
    }
    let source="fn f(flag:bool,x:u8)->u8 {let mut v=Some(x); if !flag {v=None;} if flag {v.unwrap()} else {0}}";
    let (_, facts) = collect(source, true)?;
    assert!(facts.unknown_obligations.is_empty());
    println!(
        "R5_EXPORTED={}",
        json!({"name":"bool_parameter","source":source,"facts":facts})
    );
    let source="fn f(x:u8)->u8 {let mut value=None::<u8>; {value=Some(x);let value=None::<u8>;let _=value;} value.unwrap()}";
    let (_, facts) = collect(source, true)?;
    assert!(facts.unknown_obligations.is_empty());
    println!(
        "R5_EXPORTED={}",
        json!({"name":"assignment_before_shadow","source":source,"facts":facts})
    );
    Ok(())
}

#[test]
fn cfg_and_parse_uncertainty_report_collection_obligations() -> anyhow::Result<()> {
    for source in [
        "#[cfg(feature=\"hidden\")] fn f()->u8 {None::<u8>.unwrap()}",
        "fn broken( {",
    ] {
        let error = collect(source, true).unwrap_err();
        assert!(
            error
                .downcast_ref::<spur_graph::panic_evidence::PanicCollectionObligation>()
                .is_some(),
            "{error:#}"
        );
    }
    Ok(())
}

#[test]
fn primitive_type_shadows_and_unconstrained_option_payloads_are_unknown() -> anyhow::Result<()> {
    for source in [
        "type u8=u16; fn f(x:u8)->u8 {x+1}",
        "fn f()->u8 {let v=Some(255); let _=v.unwrap()+1; 0}",
    ] {
        let (_, facts) = collect(source, true)?;
        assert!(!facts.unknown_obligations.is_empty(), "accepted {source}");
    }
    Ok(())
}

#[test]
fn typed_local_aliases_and_reversed_option_phi_remain_supported() -> anyhow::Result<()> {
    for (name, source) in [
        (
            "reversed_phi",
            "fn f(x:u8)->u8 {let maybe=if x==0 {None} else {Some(x)};maybe.unwrap()}",
        ),
        ("typed_local", "fn f(x:u8)->u8 {let a:u8=1;let b=a;b+x}"),
    ] {
        let (_, facts) = collect(source, true)?;
        assert!(
            facts.unknown_obligations.is_empty(),
            "{source}: {:?}",
            facts.unknown_obligations
        );
        println!(
            "R5_EXPORTED={}",
            json!({"name":name,"source":source,"facts":facts})
        );
    }
    Ok(())
}

// Root review counterexamples: each program compiles as ordinary Rust. Unknown
// means this lane refuses certification, never a claim that execution is safe.
const R5_REVIEW_CASES: &[(&str, &str)] = &[
    ("raw_primitive_alias", "type r#u8 = u16; fn f(x:u8)->u8 {x+1}"),
    ("macro_primitive_alias", "macro_rules! wider { () => { type u8 = u16; }; } wider!(); fn f(x:u8)->u8 {x+1}"),
    ("default_integer_condition", "fn f()->u8 {if 255+1==0 {1} else {0}}"),
    ("default_array_elements", "fn f(x:u8)->u8 {let array=[255,0]; if x==0 {let _=array[x as usize]+1;} 0}"),
    ("comment_type_witness", "fn f()->u8 {let value=Some(/*::<u8>*/255); let _=value.unwrap()+1; 0}"),
    ("default_loop_induction", "fn f()->u8 {for step in 0..3 {let _=step+255;} 0}"),
    ("tuple_constructor_shadow", "fn helper(x:u8)->u8 {/*root*/None::<u8>.unwrap()} fn caller(x:u8)->u8 {/*caller*/let _=helper(x); struct helper(u8); 0}"),
    ("raw_function_shadow", "fn helper(x:u8)->u8 {/*root*/x} fn caller(x:u8)->u8 {/*caller*/helper(x); fn r#helper(x:u8)->u8 {None::<u8>.unwrap()} 0}"),
];

fn review_unknown(name: &str, source: &str) -> anyhow::Result<()> {
    let (_, facts) = collect(source, true)?;
    println!(
        "R5_REWORK={}",
        json!({"name":name,"source":source,"facts":facts,"unsupported":true})
    );
    assert!(
        facts
            .unknown_obligations
            .iter()
            .any(|u| u.kind != spur_graph::panic_evidence::PanicUnknownKind::LoopBound),
        "{name}: unproven type or lexical authority must retain an obligation"
    );
    Ok(())
}

macro_rules! review_regression {
    ($test:ident, $index:expr) => {
        #[test]
        fn $test() -> anyhow::Result<()> {
            let (name, source) = R5_REVIEW_CASES[$index];
            review_unknown(name, source)
        }
    };
}
review_regression!(review_raw_primitive_alias, 0);
review_regression!(review_macro_primitive_alias, 1);
review_regression!(review_default_integer_condition, 2);
review_regression!(review_default_array_elements, 3);
review_regression!(review_comment_type_witness, 4);
review_regression!(review_default_loop_induction, 5);

fn review_shadow(index: usize) -> anyhow::Result<()> {
    for (name, source) in &R5_REVIEW_CASES[index..index + 1] {
        let (_, facts) = collect(source, true)?;
        let outer = facts
            .bodies
            .iter()
            .find(|b| source[b.byte_range[0]..b.byte_range[1]].contains("/*root*/"))
            .unwrap();
        let caller = facts
            .bodies
            .iter()
            .find(|b| source[b.byte_range[0]..b.byte_range[1]].contains("/*caller*/"))
            .unwrap();
        assert!(facts.sites.iter().filter(|s|s.context.location.body_id == caller.body_id).all(|s| !matches!(&s.semantics,
            spur_graph::panic_evidence::PanicSiteSemantics::DirectCall{callee_body_id,..} if callee_body_id == &outer.body_id)),
            "{name}: hoisted lexical value item must not resolve to outer helper");
        review_unknown(name, source)?;
    }
    Ok(())
}

#[test]
fn review_tuple_constructor_shadow() -> anyhow::Result<()> {
    review_shadow(6)
}
#[test]
fn review_raw_function_shadow() -> anyhow::Result<()> {
    review_shadow(7)
}

#[test]
fn review_range_comments_cannot_change_iteration_membership() -> anyhow::Result<()> {
    let source =
        "fn f()->u8 {for step in 0u8 /* ..= */ .. 0 {let _=step+255; None::<u8>.unwrap();} 0}";
    let (_, facts) = collect(source, true)?;
    println!(
        "R5_REWORK={}",
        json!({"name":"comment_range","source":source,"facts":facts,"unsupported":false})
    );
    let loops = &facts.extraction.as_ref().unwrap().loops;
    assert_eq!(loops.len(), 1);
    assert!(
        !loops[0].inclusive,
        "comment changed exclusive range into inclusive range"
    );
    Ok(())
}

#[test]
fn review_renamed_whitespace_and_namespace_adversaries() -> anyhow::Result<()> {
    for (name, source) in [
        ("raw_alias_spaced", "type r#u8 = u16; fn renamed( parcel : r#u8 ) -> r#u8 { parcel + 1 }"),
        ("macro_alias_renamed", "macro_rules! enlarge { () => { type r#u8 = u16; }; } enlarge ! (); fn renamed(n:u8)->u8 { n + 1 }"),
        ("default_condition_spaced", "fn renamed()->u8 { if ( 255 + 1 ) == 0 { 1 } else { 0 } }"),
        ("default_array_renamed", "fn renamed(n:u8)->u8 { let values = [ 255, 0 ]; if n == 0 { let _ = values[n as usize] + 1; } 0 }"),
        ("comment_witness_renamed", "fn renamed()->u8 { let parcel = Some( /* ::<u8> */ 255 ); let _ = parcel.unwrap() + 1; 0 }"),
        ("default_loop_renamed", "fn renamed()->u8 { for counter in 0 .. 3 { let _ = counter + 255; } 0 }"),
        ("raw_const_shadow", "fn target(x:u8)->u8 {x} fn caller(x:u8)->u8 {let _=target(x); const r#target:fn(u8)->u8 = |_| None::<u8>.unwrap(); 0}"),
        ("raw_tuple_shadow", "fn target(x:u8)->u8 {None::<u8>.unwrap()} fn caller(x:u8)->u8 {let _=r#target(x); struct r#target(u8); 0}"),
        ("unit_constructor_shadow", "fn target()->u8 {None::<u8>.unwrap()} fn caller()->u8 {let _=target; struct target; 0}"),
        ("wide_suffix", "fn f()->u8 {let _=255u16+1; 0}"),
    ] {
        review_unknown(name, source)?;
    }
    Ok(())
}

#[test]
fn review_grounded_positives_and_structural_type_tokens() -> anyhow::Result<()> {
    for (name, source) in [
        ("scalar", "fn f(x:u8)->u8 {x+1}"),
        (
            "helper",
            "fn helper(x:u8)->u8 {x} fn caller(x:u8)->u8 {helper(x)}",
        ),
        (
            "option",
            "fn f(x:u8)->u8 {let value=Some(x); value.unwrap()}",
        ),
        (
            "array",
            "fn f(x:u8)->u8 {let array=[255u8,0]; if x==0 {let _=array[x as usize]+1;} 0}",
        ),
        (
            "comment_generic",
            "fn f()->u8 {let parcel=Some :: /*type*/ < u8 > (255); let _=parcel.unwrap()+1; 0}",
        ),
        (
            "comment_annotation",
            "fn f(x:u8)->u8 {let parcel:Option /*type*/ <u8> = Some(x); parcel.unwrap()}",
        ),
        (
            "typed_array",
            "fn f(x:u8)->u8 {let array:[u8;2]=[255,0]; if x==0 {let _=array[x as usize]+1;} 0}",
        ),
        (
            "typed_loop",
            "fn f()->u8 {for step in 0u8..3 {let _=step+255;} 0}",
        ),
        (
            "typed_loop_end",
            "fn f()->u8 {for step in 0..3u8 {let _=step+255;} 0}",
        ),
        ("raw_local", "fn f(r#x:u8)->u8 {let r#parcel=x; parcel+1}"),
        (
            "raw_helper",
            "fn r#target(r#n:u8)->u8 {n+1} fn caller(x:u8)->u8 {target(x)}",
        ),
        (
            "guarded",
            "fn f(x:u8)->u8 {if x<255 && x+1>0 {x+1} else {0}}",
        ),
    ] {
        let (_, facts) = collect(source, true)?;
        println!(
            "R5_REWORK={}",
            json!({"name":name,"source":source,"facts":facts,"unsupported":false})
        );
        assert!(
            facts
                .unknown_obligations
                .iter()
                .all(|u| u.kind == spur_graph::panic_evidence::PanicUnknownKind::LoopBound),
            "lost supported source {name}: {:?}",
            facts.unknown_obligations
        );
    }
    Ok(())
}

const R5_DISPATCH_CASES: &[(&str, &str)] = &[
    ("module_hidden_panic", "fn helper(x:u8)->u8{/*root*/x} mod nested {fn helper(x:u8)->u8{None::<u8>.unwrap()} pub fn caller(x:u8)->u8{/*caller*/helper(x)}}"),
    ("module_false_panic", "fn helper(x:u8)->u8{/*root*/None::<u8>.unwrap()} mod nested {fn helper(x:u8)->u8{x} pub fn caller(x:u8)->u8{/*caller*/helper(x)}}"),
    ("nested_module_hidden_panic", "fn task(x:u8)->u8{/*root*/x} mod outer { pub mod inner {fn task(x:u8)->u8{None::<u8>.unwrap()} pub fn caller(x:u8)->u8{/*caller*/task(x)}}}"),
    ("trait_hidden_panic", "trait Flag {fn is_some(self)->bool;} impl Flag for Option<u8>{fn is_some(self)->bool{true}} fn f(x:u8)->u8{/*caller*/let value:Option<u8>=None; if value.is_some(){value.unwrap()}else{x}}"),
    ("trait_false_panic", "trait Flag {fn is_some(self)->bool;} impl Flag for Option<u8>{fn is_some(self)->bool{false}} fn f(x:u8)->u8{/*caller*/let value=Some(x); if value.is_some(){None::<u8>.unwrap()}else{0}}"),
];

fn dispatch_unknown(name: &str, source: &str) -> anyhow::Result<()> {
    use spur_graph::panic_evidence::{PanicSiteSemantics, PanicUnknownKind};
    let (_, facts) = collect(source, true)?;
    let caller = facts
        .bodies
        .iter()
        .filter(|b| source[b.byte_range[0]..b.byte_range[1]].contains("/*caller*/"))
        .min_by_key(|b| b.byte_range[1] - b.byte_range[0])
        .unwrap();
    println!(
        "R5_DISPATCH={}",
        json!({"name":name,"source":source,"facts":facts,"body_id":caller.body_id,"unsupported":true})
    );
    let sites: Vec<_> = facts
        .sites
        .iter()
        .filter(|s| s.context.location.body_id == caller.body_id)
        .collect();
    // Refusal is attached to the affected invocation, not a different method body.
    assert!(
        sites.iter().any(
            |s| matches!(&s.semantics, PanicSiteSemantics::Unknown {obligation_id}
        if facts.unknown_obligations.iter().any(|u| u.obligation_id == *obligation_id
            && u.kind == PanicUnknownKind::Callee && u.location == s.context.location))
        ),
        "{name}: affected caller/site needs a Callee obligation"
    );
    assert!(
        sites.iter().all(|s| !matches!(
            &s.semantics,
            PanicSiteSemantics::DirectCall { .. } | PanicSiteSemantics::OptionIsSome { .. }
        )),
        "{name}: unsupported dispatch must not select root helper or Option presence"
    );
    Ok(())
}

macro_rules! dispatch_regression {
    ($test:ident, $index:expr) => {
        #[test]
        fn $test() -> anyhow::Result<()> {
            let (name, source) = R5_DISPATCH_CASES[$index];
            dispatch_unknown(name, source)
        }
    };
}
dispatch_regression!(dispatch_module_hidden_panic, 0);
dispatch_regression!(dispatch_module_false_panic, 1);
dispatch_regression!(dispatch_nested_module_hidden_panic, 2);
dispatch_regression!(dispatch_trait_hidden_panic, 3);
dispatch_regression!(dispatch_trait_false_panic, 4);

#[test]
fn dispatch_contextual_argument_cannot_borrow_wrong_helper_type() -> anyhow::Result<()> {
    for source in [
        "fn work(n:u8)->u8{n} mod renamed {fn work(n:i32)->u8{0} pub fn caller(n:u8)->u8{/*caller*/work(255+1)}}",
        "fn work(n:u8)->u8{n} fn owner()->u8{fn work(n:i32)->u8{0} fn caller(n:u8)->u8{/*caller*/work(255+1)} caller(0)}",
    ] {
        let (_, facts) = collect(source, true)?;
        let caller = facts.bodies.iter().filter(|b| source[b.byte_range[0]..b.byte_range[1]].contains("/*caller*/"))
            .min_by_key(|b| b.byte_range[1] - b.byte_range[0]).unwrap();
        assert!(facts.extraction.as_ref().unwrap().operations.iter().all(|o| o.context.location.body_id != caller.body_id),
            "unsupported lexical helper supplied a false u8 argument type");
    }
    Ok(())
}

#[test]
fn dispatch_module_variants_and_contextual_type_authority() -> anyhow::Result<()> {
    for (name, source) in [
        ("module_raw_renamed", "fn r#work(n:u8)->u8{n} mod renamed {fn work(n:u8)->u8{None::<u8>.unwrap()} pub fn caller( parcel : u8 )->u8 { /*caller*/ r#work ( parcel ) }}"),
        ("nested_module_false_panic", "fn work(n:u8)->u8{None::<u8>.unwrap()} mod renamed {pub mod deep {fn r#work(n:u8)->u8{n} pub fn caller(parcel:u8)->u8{/*caller*/work(parcel)}}}"),
        ("module_no_local_helper", "fn work(n:u8)->u8{n} mod renamed {pub fn caller(n:u8)->u8{/*caller*/super::work(n)}}"),
        ("module_argument_type", "fn work(n:u8)->u8{n} mod renamed {fn work(n:i32)->u8{0} pub fn caller(n:u8)->u8{/*caller*/work(255+1)}}"),
        ("nested_function_argument_type", "fn work(n:u8)->u8{n} fn owner()->u8{fn work(n:i32)->u8{0} fn caller(n:u8)->u8{/*caller*/work(255+1)} caller(0)}"),
    ] {
        dispatch_unknown(name, source)?;
        let (_, facts) = collect(source, true)?;
        if name.ends_with("argument_type") {
            let caller = facts.bodies.iter().filter(|b| source[b.byte_range[0]..b.byte_range[1]].contains("/*caller*/"))
                .min_by_key(|b| b.byte_range[1] - b.byte_range[0]).unwrap();
            assert!(facts.extraction.as_ref().unwrap().operations.iter().all(|o| o.context.location.body_id != caller.body_id),
                "{name}: an unsupported lexical target cannot supply u8 argument authority");
        }
    }
    Ok(())
}

#[test]
fn dispatch_trait_default_blanket_and_lexical_variants() -> anyhow::Result<()> {
    for (name, source) in [
        ("trait_default", "trait Probe:Sized {fn is_some(self)->bool{true}} impl Probe for Option<u8>{} fn f(x:u8)->u8{/*caller*/let parcel:Option<u8>=None; if parcel.is_some(){parcel.unwrap()}else{x}}"),
        ("trait_blanket_option", "trait Probe {fn is_some(self)->bool;} impl<T> Probe for Option<T>{fn is_some(self)->bool{true}} fn f(x:u8)->u8{/*caller*/let parcel:Option<u8>=None; if parcel.is_some(){parcel.unwrap()}else{x}}"),
        ("trait_blanket_all", "trait Probe {fn is_some(self)->bool;} impl<T> Probe for T{fn is_some(self)->bool{false}} fn f(x:u8)->u8{/*caller*/let parcel=Some(x); if parcel.is_some(){None::<u8>.unwrap()}else{0}}"),
        ("trait_raw_renamed", "trait Probe {fn r#is_some(self)->bool;} impl Probe for Option<u8>{fn r#is_some(self)->bool{true}} fn renamed(parcel:u8)->u8{/*caller*/let item:Option<u8>=None; if item . r#is_some () {item.unwrap()}else{parcel}}"),
        ("trait_local", "fn f(x:u8)->u8{/*caller*/let parcel:Option<u8>=None; return if parcel.is_some(){parcel.unwrap()}else{x}; trait Probe{fn is_some(self)->bool;} impl Probe for Option<u8>{fn is_some(self)->bool{true}}}"),
        ("trait_nested_module", "mod outer {pub mod inner {trait Probe{fn is_some(self)->bool;} impl Probe for Option<u8>{fn is_some(self)->bool{true}} pub fn caller(x:u8)->u8{/*caller*/let parcel:Option<u8>=None; if parcel.is_some(){parcel.unwrap()}else{x}}}}"),
    ] { dispatch_unknown(name, source)?; }
    Ok(())
}

#[test]
fn dispatch_unambiguous_root_and_option_positives() -> anyhow::Result<()> {
    for (name, source) in [
        ("root_typed_argument", "fn work(n:u8)->u8{n} fn caller(x:u8)->u8{work(254+1)}"),
        ("root_helper_option", "fn work(n:u8)->u8{n+1} fn caller(x:u8)->u8{work(x)}"),
        ("option_is_some", "fn f(x:u8)->u8{let parcel=if x==0{Some(x)}else{None}; if parcel.is_some(){parcel.unwrap()}else{0}}"),
        ("option_is_some_raw", "fn renamed(x:u8)->u8{let parcel=Some(x); if parcel.r#is_some(){parcel.unwrap()}else{0}}"),
    ] {
        let (_, facts) = collect(source, true)?;
        println!("R5_DISPATCH={}", json!({"name":name,"source":source,"facts":facts,"unsupported":false}));
        assert!(facts.unknown_obligations.is_empty(), "{name}: {:?}", facts.unknown_obligations);
    }
    Ok(())
}
