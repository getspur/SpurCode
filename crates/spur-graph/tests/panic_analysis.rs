use std::sync::Arc;

use serde_json::json;
use spur_graph::panic_evidence::*;
use spur_graph::{artifact_from_facts, build_facts};
use spur_solver::process::{
    ProcessError, ProcessFuture, ProcessOutcome, ProcessOutput, ProcessRequest, ProcessRunner,
};
use spur_solver::service::SolverService;
use spur_solver::types::{ConstraintExpr, ConstraintItem, ConstraintOp, ModelValue, SolveStatus};

fn facts(source: &str, overflow: bool) -> anyhow::Result<PanicSemanticFacts> {
    normalized_facts(source, overflow, |_| {})
}

fn normalized_facts(
    source: &str,
    overflow: bool,
    normalize: impl FnOnce(&mut PanicSemanticFacts),
) -> anyhow::Result<PanicSemanticFacts> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("src"))?;
    std::fs::write(dir.path().join("src/lib.rs"), source)?;
    let (raw, _) = build_facts(dir.path(), None)?;
    let graph = artifact_from_facts(&raw, dir.path())?;
    let build: PanicBuildConfig = serde_json::from_value(json!({
        "schema_version":"panic-build-config-v1", "graph_content_hash":graph.graph_content_hash,
        "target":"aarch64-apple-darwin", "edition":"2021", "cfg":[],
        "overflow_checks":overflow, "debug_assertions":true, "panic_strategy":"unwind",
        "rustc_verbose":"R6 explicitly declared bool/u8 source model", "codegen_flags":[], "features":[]
    }))?;
    let sources = [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: source.as_bytes(),
    }];
    let mut f = collect_panic_semantics(&graph, &sources, &build)?;
    f.validate_canonical(&graph, &sources, &build)?;
    normalize(&mut f);
    f.validate_against(
        &graph,
        &sources,
        &collect_rust_call_sites(&graph, &sources)?,
    )?;
    Ok(f)
}

fn limits(
    f: &PanicSemanticFacts,
    _entry: &str,
    depth: u32,
    iterations: u32,
) -> PanicAnalysisLimits {
    // Every fixture places its entry function last; IDs come from R3 facts.
    let entry_id = f
        .bodies
        .iter()
        .max_by_key(|b| b.byte_range[0])
        .map(|b| b.body_id.clone());
    PanicAnalysisLimits {
        entry_body_ids: vec![entry_id.expect("entry body")],
        expected_build_hash: f.build_hash.clone(),
        max_call_depth: depth,
        max_loop_iterations: iterations,
        max_states: spur_solver::types::MAX_CONSTRAINTS as u32,
        timeout_ms: 30_000,
        persist: true,
    }
}

async fn run(
    name: &str,
    source: &str,
    overflow: bool,
    depth: u32,
    iterations: u32,
) -> anyhow::Result<PanicAnalysisReport> {
    let f = facts(source, overflow)?;
    let storage = tempfile::tempdir()?;
    let service = SolverService::new().with_repo_root(storage.path());
    let l = limits(&f, "f", depth, iterations);
    let report = analyze_panic_facts(&service, &f, &l).await?;
    for query in &report.queries {
        query.request.validate()?;
        let raw = query.response.as_ref().expect("real Z3 response");
        let receipt = service.get_solve_result(raw.solve_id.as_deref().expect("persisted"))?;
        assert_eq!(serde_json::to_value(&query.request)?, receipt.request);
    }
    println!(
        "R6_EXPORTED={}",
        json!({"name":name,"source":source,"facts":f,"report":report})
    );
    Ok(report)
}

fn assert_result(
    report: &PanicAnalysisReport,
    expected: PanicAnalysisOutcome,
    witness: Option<i64>,
) {
    let q = &report.queries[0];
    assert_eq!(q.outcome, expected, "{report:#?}");
    if expected == PanicAnalysisOutcome::CandidateWitness {
        assert!(q.model_validation.as_ref().unwrap().valid);
        assert!(!q.candidate.as_ref().unwrap().runtime_reproduced);
        if let Some(x) = witness {
            assert_eq!(
                q.response.as_ref().unwrap().model.as_ref().unwrap()[&q.inputs[0].variable],
                ModelValue::Int(x)
            );
        }
    }
    if expected == PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope {
        assert!(q.obligations.is_empty());
        assert_eq!(q.response.as_ref().unwrap().status, SolveStatus::Unsat);
    }
}

#[tokio::test]
async fn guards_mutation_shadowing_and_arithmetic_use_real_z3() -> anyhow::Result<()> {
    use PanicAnalysisOutcome::{
        CandidateWitness as Sat, NoCounterexampleWithinCompleteScope as Unsat,
    };
    for (name, src, overflow, expected, x) in [
        ("safe_option", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x!=3 {v.unwrap()} else {0}}", true, Unsat, None),
        ("changed_option", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x==3 {v.unwrap()} else {0}}", true, Sat, Some(3)),
        ("mutation", "fn f(x:u8)->u8 {let mut v=Some(x); if x==3 {v=None;} if x==3 {v.unwrap()} else {0}}", true, Sat, Some(3)),
        ("shadow", "fn f(x:u8)->u8 {let v=None::<u8>; {let v=Some(x); if x==7 {return v.unwrap();}} if x==8 {v.unwrap()} else {0}}", true, Sat, Some(8)),
        ("checked", "fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true, Sat, Some(255)),
        ("checked_safe", "fn f(x:u8)->u8 {if x<255 {x.checked_add(1).unwrap()} else {0}}", true, Unsat, None),
        ("add_on", "fn f(x:u8)->u8 {x+1}", true, Sat, Some(255)),
        ("add_off", "fn f(x:u8)->u8 {x+1}", false, Unsat, None),
        ("index_bad", "fn f(x:u8)->u8 {[10u8,20,30][x as usize]}", true, Sat, None),
        ("index_safe", "fn f(x:u8)->u8 {if x<3 {[10u8,20,30][x as usize]} else {0}}", true, Unsat, None),
        ("short_circuit", "fn f(x:u8)->bool {let v=if x==3 {None} else {Some(x)}; x!=3 && v.unwrap()==0}", true, Unsat, None),
        ("non_dominating", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x!=3 {let y=v.is_some();} v.unwrap()}", true, Sat, Some(3)),
    ] {
        assert_result(&run(name, src, overflow, 0, 0).await?, expected, x);
    }
    Ok(())
}

#[tokio::test]
async fn helpers_substitute_actual_arguments_and_preserve_call_frontiers() -> anyhow::Result<()> {
    let src = "fn helper(n:u8)->u8 {let v=if n==4 {None} else {Some(n)}; v.unwrap()} fn f(x:u8)->u8 {if x<255 {helper(x+1)} else {0}}";
    assert_result(
        &run("helper_depth0", src, true, 0, 0).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );
    assert_result(
        &run("helper_depth1", src, true, 1, 0).await?,
        PanicAnalysisOutcome::CandidateWitness,
        Some(3),
    );
    let safe = "fn helper(n:u8)->u8 {let v=if n==4 {None} else {Some(n)}; v.unwrap()} fn f(x:u8)->u8 {helper(0)}";
    assert_result(
        &run("helper_safe", safe, true, 1, 0).await?,
        PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope,
        None,
    );
    Ok(())
}

#[tokio::test]
async fn literal_loop_prefix_and_sufficient_bound_are_distinct() -> anyhow::Result<()> {
    let src = "fn f(x:u8)->u8 {for step in 0u8..3 {if x==step && step==2 {return None::<u8>.unwrap();}} 0}";
    assert_result(
        &run("loop_prefix", src, true, 0, 2).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );
    assert_result(
        &run("loop_witness", src, true, 0, 3).await?,
        PanicAnalysisOutcome::CandidateWitness,
        Some(2),
    );
    let safe = "fn f(x:u8)->u8 {for step in 0u8..3 {if x==step && step==4 {return None::<u8>.unwrap();}} 0}";
    assert_result(
        &run("loop_complete", safe, true, 0, 3).await?,
        PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope,
        None,
    );
    Ok(())
}

#[derive(Debug)]
struct Controlled(&'static str);
impl ProcessRunner for Controlled {
    fn run(&self, request: ProcessRequest) -> ProcessFuture<'_> {
        Box::pin(async move {
            match self.0 {
                "timeout" => Ok(ProcessOutcome::TimedOut),
                "unavailable" => Err(ProcessError::SolverUnavailable {
                    message: "controlled unavailable".into(),
                }),
                text => {
                    let text = if text == "invalid" {
                        let values = request
                            .smt()
                            .lines()
                            .filter_map(|line| {
                                let declaration =
                                    line.strip_prefix("(declare-const ")?.strip_suffix(')')?;
                                let (symbol, sort) = declaration.split_once(' ')?;
                                Some(format!(
                                    "({symbol} {})",
                                    if sort == "Bool" { "false" } else { "0" }
                                ))
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("sat\n({values})\n")
                    } else {
                        text.to_owned()
                    };
                    Ok(ProcessOutcome::Completed(ProcessOutput {
                        success: true,
                        exit_code: Some(0),
                        stdout: text.into_bytes(),
                        stderr: vec![],
                    }))
                }
            }
        })
    }
}

#[tokio::test]
async fn operational_and_invalid_model_branches_never_become_safety() -> anyhow::Result<()> {
    let f = facts("fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true)?;
    let mut l = limits(&f, "f", 0, 0);
    l.persist = false;
    for (output, outcome, status) in [
        (
            "unknown\n",
            PanicAnalysisOutcome::Unknown,
            Some(SolveStatus::Unknown),
        ),
        (
            "timeout",
            PanicAnalysisOutcome::Timeout,
            Some(SolveStatus::Timeout),
        ),
        ("unavailable", PanicAnalysisOutcome::OperationalError, None),
        (
            "broken output",
            PanicAnalysisOutcome::OperationalError,
            Some(SolveStatus::Error),
        ),
        // Syntactically valid, domain-valid model violating the panic constraint.
        (
            "invalid",
            PanicAnalysisOutcome::InvalidModel,
            Some(SolveStatus::Sat),
        ),
    ] {
        let service = SolverService::with_runner(Arc::new(Controlled(output)));
        let report = analyze_panic_facts(&service, &f, &l).await?;
        let q = &report.queries[0];
        assert_eq!(q.outcome, outcome, "{report:#?}");
        assert_eq!(q.response.as_ref().map(|r| r.status.clone()), status);
        assert!(q.candidate.is_none());
        println!(
            "R6_OPERATIONAL={}",
            json!({"control":output,"report":report})
        );
    }
    Ok(())
}

#[tokio::test]
async fn limits_unknowns_ordering_and_identity_are_checked() -> anyhow::Result<()> {
    let f = facts("fn f(x:u8)->u8 {external(); None::<u8>.unwrap()}", true)?;
    let service = SolverService::new();
    let mut l = limits(&f, "f", 0, 0);
    l.persist = false;
    let report = analyze_panic_facts(&service, &f, &l).await?;
    assert_result(&report, PanicAnalysisOutcome::IncompleteSemantics, None);
    l.expected_build_hash.push('x');
    assert!(analyze_panic_facts(&service, &f, &l).await.is_err());
    let f = facts("fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true)?;
    l = limits(&f, "f", 0, 0);
    l.persist = false;
    l.max_states = 1;
    assert_result(
        &analyze_panic_facts(&service, &f, &l).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );
    let mut damaged = f.clone();
    damaged.build_hash.push('x');
    assert!(analyze_panic_facts(&service, &damaged, &l).await.is_err());
    // A malformed domain must not become an apparent complete safety result.
    let mut damaged = f.clone();
    damaged.values[0].domain = PanicValueDomain::U8 {
        minimum: 0,
        maximum: 256,
    };
    assert!(analyze_panic_facts(&service, &damaged, &l).await.is_err());
    Ok(())
}

#[tokio::test]
async fn missing_relations_and_ambiguous_event_order_remain_incomplete() -> anyhow::Result<()> {
    let service = SolverService::new();
    // Structurally valid R4 facts can lack the optional R5 extraction extension.
    // A plain add then lacks its implicit panic operation; its result alone
    // cannot prove absence of overflow. Checked-add has separate Option semantics.
    let f = normalized_facts("fn f(x:u8)->u8 {x+1}", true, |f| f.extraction = None)?;
    let mut l = limits(&f, "f", 0, 0);
    l.persist = false;
    assert_result(
        &analyze_panic_facts(&service, &f, &l).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );

    let f = normalized_facts(
        "fn helper(x:u8)->u8 {x} fn f(x:u8)->u8 {helper(x).checked_add(1).unwrap()}",
        true,
        |f| f.extraction = None,
    )?;
    let mut l = limits(&f, "f", 1, 0);
    l.persist = false;
    assert_result(
        &analyze_panic_facts(&service, &f, &l).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );

    // R4 validates a DAG, but cannot infer an order between two effects in one
    // node. Preserve the missing order rather than use site ID ordering.
    let f = normalized_facts(
        "fn f(x:u8)->u8 {let a=None::<u8>.unwrap(); None::<u8>.unwrap()}",
        true,
        |f| {
            let latest = f
                .sites
                .iter()
                .filter(|s| matches!(s.semantics, PanicSiteSemantics::OptionUnwrap { .. }))
                .max_by_key(|s| {
                    f.control_flow
                        .iter()
                        .find(|c| c.flow_id == s.context.flow_id)
                        .unwrap()
                        .order
                })
                .unwrap()
                .context
                .flow_id
                .clone();
            for site in &mut f.sites {
                if matches!(site.semantics, PanicSiteSemantics::OptionUnwrap { .. }) {
                    site.context.flow_id = latest.clone();
                }
            }
        },
    )?;
    let mut l = limits(&f, "f", 0, 0);
    l.persist = false;
    assert_result(
        &analyze_panic_facts(&service, &f, &l).await?,
        PanicAnalysisOutcome::IncompleteSemantics,
        None,
    );
    Ok(())
}

#[tokio::test]
async fn ordered_panics_wrapping_values_and_helper_returns_keep_their_inputs() -> anyhow::Result<()>
{
    let src = "fn f(x:u8)->u8 {let y=x+1; if y==0 {None::<u8>.unwrap()} else {y}}";
    assert_result(
        &run("wrapping_then_unwrap", src, false, 0, 0).await?,
        PanicAnalysisOutcome::CandidateWitness,
        Some(255),
    );
    let src = "fn f(x:u8)->u8 {let y=x.checked_add(1).unwrap(); if x==255 {None::<u8>.unwrap()} else {y}}";
    let report = run("ordered_panics", src, true, 0, 0).await?;
    assert_result(&report, PanicAnalysisOutcome::CandidateWitness, Some(255));
    let q = &report.queries[0];
    assert_eq!(
        q.candidate.as_ref().unwrap().event_ids,
        vec![q.events[0].event_id.clone()]
    );
    let src = "fn helper(n:u8)->u8 {if n==0 {return 7;} 9} fn f(x:u8)->u8 {let y=helper(x); if y==7 {None::<u8>.unwrap()} else {0}}";
    assert_result(
        &run("helper_return", src, true, 1, 0).await?,
        PanicAnalysisOutcome::CandidateWitness,
        Some(0),
    );
    Ok(())
}

#[tokio::test]
async fn exact_bool_and_u8_domains_and_identity_bindings_are_retained() -> anyhow::Result<()> {
    let storage = tempfile::tempdir()?;
    let service = SolverService::new().with_repo_root(storage.path());
    for (minimum, maximum, outcome) in [
        (
            0,
            254,
            PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope,
        ),
        (255, 255, PanicAnalysisOutcome::CandidateWitness),
    ] {
        let f = normalized_facts("fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true, |f| {
            f.values
                .iter_mut()
                .find(|v| matches!(v.origin, PanicValueOrigin::Parameter { .. }))
                .unwrap()
                .domain = PanicValueDomain::U8 { minimum, maximum };
        })?;
        let mut l = limits(&f, "f", 0, 0);
        l.persist = true;
        let r = analyze_panic_facts(&service, &f, &l).await?;
        assert_result(&r, outcome, None);
        assert_eq!(
            r.queries[0].inputs[0].domain,
            PanicValueDomain::U8 { minimum, maximum }
        );
        println!(
            "R6_EXPORTED={}",
            json!({"name":format!("u8_domain_{minimum}_{maximum}"),"source":"fn f(x:u8)->u8 {x.checked_add(1).unwrap()}","facts":f,"report":r})
        );
    }
    for values in [vec![false], vec![true], vec![false, true]] {
        let f = normalized_facts(
            "fn f(flag:bool)->u8 {if flag {None::<u8>.unwrap()} else {0}}",
            true,
            |f| {
                f.values
                    .iter_mut()
                    .find(|v| matches!(v.origin, PanicValueOrigin::Parameter { .. }))
                    .unwrap()
                    .domain = PanicValueDomain::Bool {
                    values: values.clone(),
                };
            },
        )?;
        let mut l = limits(&f, "f", 0, 0);
        l.persist = true;
        let r = analyze_panic_facts(&service, &f, &l).await?;
        assert_result(
            &r,
            if values.contains(&true) {
                PanicAnalysisOutcome::CandidateWitness
            } else {
                PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope
            },
            None,
        );
        println!(
            "R6_EXPORTED={}",
            json!({"name":format!("bool_domain_{values:?}"),"source":"fn f(flag:bool)->u8 {if flag {None::<u8>.unwrap()} else {0}}","facts":f,"report":r})
        );
    }
    let f = facts("fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true)?;
    let mut l = limits(&f, "f", 0, 0);
    l.persist = true;
    let original = analyze_panic_facts(&service, &f, &l).await?;
    for changed in [
        facts("fn f(n:u8)->u8 {n.checked_add(1).unwrap()}", true)?,
        facts("fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", false)?,
    ] {
        let mut next = limits(&changed, "f", 0, 0);
        next.persist = true;
        let r = analyze_panic_facts(&service, &changed, &next).await?;
        assert_eq!(original.queries[0].request, r.queries[0].request);
        assert_ne!(
            original.queries[0].request_identity,
            r.queries[0].request_identity
        );
    }
    l.max_states -= 1;
    let changed = analyze_panic_facts(&service, &f, &l).await?;
    assert_eq!(original.queries[0].request, changed.queries[0].request);
    assert_ne!(
        original.queries[0].request_identity,
        changed.queries[0].request_identity
    );
    Ok(())
}

#[tokio::test]
async fn exhaust_u8_against_independent_native_predicates() -> anyhow::Result<()> {
    if std::env::var_os("SPUR_R6_EXHAUST").is_none() {
        return Ok(());
    }
    type Oracle = fn(u8) -> bool;
    let cases: &[(&str, &str, bool, u32, u32, Oracle)] = &[
        ("checked", "fn f(x:u8)->u8 {x.checked_add(1).unwrap()}", true, 0, 0, |x|x.checked_add(1).is_none()),
        ("checked_safe", "fn f(x:u8)->u8 {if x<255 {x.checked_add(1).unwrap()} else {0}}", true, 0, 0, |x|x<255 && x.checked_add(1).is_none()),
        ("add_on", "fn f(x:u8)->u8 {x+1}", true, 0, 0, |x|x.overflowing_add(1).1),
        ("add_off", "fn f(x:u8)->u8 {x+1}", false, 0, 0, |_|false),
        ("index", "fn f(x:u8)->u8 {[10u8,20,30][x as usize]}", true, 0, 0, |x|[10u8,20,30].get(x as usize).is_none()),
        ("index_safe", "fn f(x:u8)->u8 {if x<3 {[10u8,20,30][x as usize]} else {0}}", true, 0, 0, |x|x<3 && [10u8,20,30].get(x as usize).is_none()),
        ("mutation", "fn f(x:u8)->u8 {let mut v=Some(x);if x==3 {v=None;} if x==3 {v.unwrap()} else {0}}", true, 0, 0, |x|{let mut v=Some(x);if x==3 {v=None;} x==3 && v.is_none()}),
        ("shadow", "fn f(x:u8)->u8 {let v=None::<u8>;{let v=Some(x);if x==7 {return v.unwrap();}} if x==8 {v.unwrap()} else {0}}", true, 0, 0, |x|x==8),
        ("safe_option", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x!=3 {v.unwrap()} else {0}}", true, 0, 0, |x|x!=3 && (if x==3 {None} else {Some(x)}).is_none()),
        ("changed_option", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x==3 {v.unwrap()} else {0}}", true, 0, 0, |x|x==3),
        ("short_circuit", "fn f(x:u8)->bool {let v=if x==3 {None} else {Some(x)}; x!=3 && v.unwrap()==0}", true, 0, 0, |x|x!=3 && (if x==3 {None} else {Some(x)}).is_none()),
        ("non_dominating", "fn f(x:u8)->u8 {let v=if x==3 {None} else {Some(x)}; if x!=3 {let y=v.is_some();} v.unwrap()}", true, 0, 0, |x|x==3),
        ("helper", "fn helper(n:u8)->u8 {let v=if n==4 {None} else {Some(n)}; v.unwrap()} fn f(x:u8)->u8 {if x<255 {helper(x+1)} else {0}}", true, 1, 0, |x|x<255 && x+1==4),
        ("helper_safe", "fn helper(n:u8)->u8 {let v=if n==4 {None} else {Some(n)}; v.unwrap()} fn f(x:u8)->u8 {helper(0)}", true, 1, 0, |_|false),
        ("loop", "fn f(x:u8)->u8 {for step in 0u8..3 {if x==step && step==2 {return None::<u8>.unwrap();}} 0}", true, 0, 3, |x|(0u8..3).any(|step|x==step && step==2)),
        ("loop_safe", "fn f(x:u8)->u8 {for step in 0u8..3 {if x==step && step==4 {return None::<u8>.unwrap();}} 0}", true, 0, 3, |x|(0u8..3).any(|step|x==step && step==4)),
        ("wrapping", "fn f(x:u8)->u8 {let y=x+1; if y==0 {None::<u8>.unwrap()} else {y}}", false, 0, 0, |x|x.wrapping_add(1)==0),
    ];
    let service = SolverService::new();
    for (name, src, overflow, depth, iterations, oracle) in cases {
        let f = facts(src, *overflow)?;
        let mut l = limits(&f, "f", *depth, *iterations);
        l.persist = false;
        let report = analyze_panic_facts(&service, &f, &l).await?;
        let q = &report.queries[0];
        let mut panics = vec![];
        for x in 0..=255u8 {
            let mut request = q.request.clone();
            request
                .constraints
                .push(ConstraintItem::Bare(ConstraintExpr::Op {
                    op: ConstraintOp::Eq,
                    args: vec![
                        ConstraintExpr::Var {
                            name: q.inputs[0].variable.clone(),
                        },
                        ConstraintExpr::Int { value: x.into() },
                    ],
                }));
            let r = service.solve_constraints(request).await?;
            let expected = if oracle(x) {
                SolveStatus::Sat
            } else {
                SolveStatus::Unsat
            };
            assert_eq!(r.status, expected, "{name} x={x}");
            if expected == SolveStatus::Sat {
                panics.push(x);
            }
        }
        println!(
            "R6_EXHAUST={}",
            json!({"name":name,"inputs":256,"panic_inputs":panics})
        );
    }
    Ok(())
}
