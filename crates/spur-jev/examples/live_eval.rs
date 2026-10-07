//! Live in-session evaluation: `jev_compile` (real Jev transport) -> real Z3.
//!
//! Run with a live key and local build:
//! ```sh
//!   set -a; source .env; set +a
//!   SPUR_REMOTE=0 SPUR_JEV_ENABLED=1 scripts/spur-cargo run -p spur-jev --example live_eval
//! ```
//!
//! Two code paths, both production entry points:
//!   A. MCP tool path: `jev_compile` -> `solve_rules` (three family cases + one ambiguity probe)
//!   B. Library path: typed battery -> gate -> `compile_bprime` -> check -> `solve_constraints`
//! All intents are fresh paraphrases; none reuse the recorded POC fixtures.

use std::sync::Arc;

use serde_json::{json, Value};
use spur_jev::client::{HttpJevTransport, JevTransport as _};
use spur_jev::gate::DecisionReview;
use spur_jev::mcp::JevMcpModule;
use spur_jev::snapshot::CatalogSnapshot;
use spur_jev::wire::{JevRequest, Question};
use spur_mcp::{ServerKind, ToolAuthority, ToolCallContext, ToolRegistry};
use spur_solver::{mcp::SolverMcpModule, service::SolverService};

fn real_snapshot() -> CatalogSnapshot {
    let catalog = spur_solver::rules::manifest_registry();
    let executable = spur_solver::rules::manifest_executable_rule_ids();
    let rules = catalog
        .rules()
        .iter()
        .filter(|rule| executable.iter().any(|id| id == rule.id()))
        .map(|rule| {
            let serialized = serde_json::to_value(rule).unwrap_or_default();
            let summary = serialized
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_else(|| rule.primitive())
                .to_owned();
            spur_jev::snapshot::RuleCard {
                rule_id: rule.id().to_owned(),
                family: rule.family().to_owned(),
                summary,
            }
        })
        .collect();
    CatalogSnapshot {
        language_version: catalog.schema_version(),
        rules,
    }
}

fn context() -> ToolCallContext<'static> {
    ToolCallContext::new(ServerKind::Brain, ToolAuthority::Brain, None, None)
}

fn result_json(response: &spur_mcp::JsonRpcResponse) -> Value {
    assert!(
        response.error.is_none(),
        "unexpected MCP error: {:?}",
        response.error
    );
    let text = response
        .result
        .as_ref()
        .and_then(|result| result["content"][0]["text"].as_str())
        .expect("MCP JSON text result");
    serde_json::from_str(text).expect("parse MCP JSON text")
}

async fn jev_compile(registry: &ToolRegistry, intent: &str, data: Value) -> Value {
    result_json(
        &registry
            .call_json_tool(
                context(),
                "jev_compile",
                json!({ "intent": intent, "data": data }),
            )
            .await,
    )
}

async fn solve_rules(registry: &ToolRegistry, request: Value) -> Value {
    result_json(
        &registry
            .call_json_tool(context(), "solve_rules", request)
            .await,
    )
}

async fn case_family(
    registry: &ToolRegistry,
    name: &str,
    intent: &str,
    data: Value,
    expect: &str,
) -> Value {
    let started = std::time::Instant::now();
    let compiled = jev_compile(registry, intent, data).await;
    let gate = compiled["gate"].as_str().unwrap_or("?");
    let weakest = compiled["provenance"]["weakest"].as_f64();
    let route = compiled["provenance"]["answers"]["route_rule"]["choice"]
        .as_str()
        .unwrap_or("(none)")
        .to_owned();
    let solved = if gate == "open" {
        solve_rules(registry, compiled["request"].clone()).await
    } else {
        json!({"status": "not-solved"})
    };
    println!(
        "{}",
        json!({
            "case": name,
            "routed": route,
            "gate": gate,
            "weakest": weakest,
            "ambiguity_present": compiled.get("ambiguity").is_some(),
            "solve_status": solved.get("status").cloned().unwrap_or_else(|| json!("blocked")),
            "outcome": solved.get("outcome").cloned().unwrap_or(json!(null)),
            "diagnostic": solved["rule_results"][0].get("diagnostic").cloned().unwrap_or(json!(null)),
            "expect": expect,
            "wall_ms": started.elapsed().as_millis() as u64,
        })
    );
    compiled
}

#[tokio::main]
async fn main() {
    let snapshot = real_snapshot();
    println!(
        "{}",
        json!({"snapshot_rules": snapshot.rules.len(), "language_version": snapshot.language_version})
    );

    // --- Path A: MCP tool path with the live transport ----------------------
    let transport = match HttpJevTransport::from_env() {
        Ok(transport) => transport,
        Err(error) => {
            eprintln!("live transport unavailable: {error}");
            std::process::exit(2);
        }
    };
    let registry = ToolRegistry::builder()
        .with(JevMcpModule::new(snapshot.clone(), transport))
        .expect("register live Jev module")
        .with(SolverMcpModule::new(Arc::new(SolverService::new())))
        .expect("register solver module")
        .build();

    // A1: fresh non_overlap numbers, expected FAIL (gap 10 < required 12).
    let toast = json!({
        "subjects": ["toast", "cta"],
        "parameters": {"minimum_gap": 12},
        "scene": {
            "viewport": {"width": 390, "height": 844},
            "nodes": {
                "toast": {"rect": {"x": 0, "y": 0, "width": 20, "height": 20}},
                "cta": {"rect": {"x": 30, "y": 0, "width": 20, "height": 20}}
            }
        },
        "unknowns": []
    });
    case_family(
        &registry,
        "A1_non_overlap_gap_violation",
        "The toast occupies x 0..20 and the call-to-action button x 30..50 on the same row. Keep at least 12 units of clearance between the two elements and audit the current row.",
        toast,
        "fail + design.overlap",
    )
    .await;

    // A2: three jobs on one cpu-slot machine, expected PASS (fresh job count).
    let machine = json!({
        "subjects": ["m1"],
        "parameters": {},
        "facts": {
            "horizon": 6,
            "jobs": {
                "alpha": {"release": 0, "deadline": 6, "durations": {"m1": 2}, "eligible_machines": ["m1"], "demands": {"cpu": 1}, "assignment": {"machine": "m1", "start": 0}},
                "beta": {"release": 0, "deadline": 6, "durations": {"m1": 2}, "eligible_machines": ["m1"], "demands": {"cpu": 1}, "assignment": {"machine": "m1", "start": 2}},
                "gamma": {"release": 0, "deadline": 6, "durations": {"m1": 2}, "eligible_machines": ["m1"], "demands": {"cpu": 1}, "assignment": {"machine": "m1", "start": 4}}
            },
            "machines": {"m1": {"capacities": {"cpu": 1}}},
            "precedence": []
        },
        "unknowns": []
    });
    case_family(
        &registry,
        "A2_cumulative_three_jobs",
        "Machine m1 exposes a single cpu slot across a 6-tick horizon. Jobs alpha, beta, and gamma each occupy two consecutive ticks at starts 0, 2, and 4 and each need one cpu. Verify the machine never runs two jobs at once.",
        machine,
        "pass",
    )
    .await;

    // A3: duplicate key snapshot, expected FAIL via live negative detection.
    let records = json!({
        "subjects": ["session_key"],
        "parameters": {},
        "facts": {
            "relations": {
                "sessions": {
                    "fields": {"key": {"kind": "integer", "minimum": 0, "maximum": 100}},
                    "rows": {
                        "first": {"active": true, "cells": {"key": {"present": true, "value": 7}}},
                        "second": {"active": true, "cells": {"key": {"present": true, "value": 7}}}
                    }
                }
            },
            "unique_constraints": {"session_key": {"relation": "sessions", "fields": ["key"]}},
            "foreign_keys": {}, "cardinality_constraints": {}, "value_ranges": {},
            "conditional_requirements": {}, "aggregate_balances": {}, "consistency_relations": {},
            "temporal_constraints": {}
        },
        "unknowns": []
    });
    case_family(
        &registry,
        "A3_unique_duplicate_key",
        "Two active session rows both carry key 7 in the sessions snapshot. Active rows must never share a complete key; audit the snapshot.",
        records,
        "fail",
    )
    .await;

    // A4: vague intent, expected BLOCKED gate, no solve.
    case_family(
        &registry,
        "A4_vague_blocked",
        "Make sure the layout generally looks right and feels good.",
        json!({}),
        "gate=blocked, no request",
    )
    .await;

    // --- Path B: library path, generic B-prime with live Jev decisions ------
    let questions: std::collections::BTreeMap<String, Question> = std::collections::BTreeMap::from(
        [
            (
                "cap_is_soft".to_owned(),
                Question::Noul {
                    instructions: Some(json!({
                        "sentence": "Prefer, when possible, keeping the retry budget at or below 3. This is a preference, not a hard requirement.",
                        "question": "Is 'retry budget at or below 3' a SOFT preference (violable at a cost) rather than a hard requirement?"
                    })),
                    criteria: None,
                },
            ),
            (
                "prefer_minimal".to_owned(),
                Question::Choice {
                    instructions: Some(json!({
                        "sentence": "Report the smallest legal retry budget.",
                        "question": "Which objective matches the sentence?"
                    })),
                    criteria: std::collections::BTreeMap::from([
                        (
                            "minimize".to_owned(),
                            json!("Minimize the budget variable."),
                        ),
                        (
                            "maximize".to_owned(),
                            json!("Maximize the budget variable."),
                        ),
                        ("none".to_owned(), json!("No objective needed.")),
                    ]),
                },
            ),
        ],
    );
    let request = JevRequest {
        state: json!({
            "intent": "The retry budget r is an integer from 0 to 7. Hard requirement: doubled r must reach at least 5. Preference when possible: keep r at or below 3. Report the smallest legal budget.",
            "domain": {"r": [0, 7]},
        }),
        model: spur_jev::wire::DEFAULT_MODEL.to_owned(),
        questions,
    };
    let started = std::time::Instant::now();
    let transport = HttpJevTransport::from_env().expect("live transport for path B");
    let response = transport.send(&request).await.expect("live Jev call");
    let review = DecisionReview::from_answers(&response.answers, &["cap_is_soft"]);
    let template = json!({
        "vars": [{"type": "int_range", "name": "r", "min": 0, "max": 7}],
        "constraints": [
            {"id": "doubled_floor", "expr": {"kind": "op", "op": "ge",
                "args": [{"kind": "op", "op": "mul", "args": [{"kind": "int", "value": 2}, {"kind": "var", "name": "r"}]}, {"kind": "int", "value": 5}]}},
            {"id": "cap", "soft": {"$noul_at_least": {"question": "cap_is_soft", "threshold": 0.5}}, "weight": 1,
             "expr": {"kind": "op", "op": "le", "args": [{"kind": "var", "name": "r"}, {"kind": "int", "value": 3}]}}
        ],
        "objectives": [{"op": {"$choice": "prefer_minimal"}, "expr": {"kind": "var", "name": "r"}}]
    });
    let compiled =
        spur_jev::compile::compile_bprime(&response.answers, &template).expect("compile B-prime");
    let validated =
        spur_solver::constraint_spec::parse_and_validate(compiled).expect("validate B-prime");
    let solved = SolverService::new()
        .solve_constraints(validated)
        .await
        .expect("solve B-prime");
    let solved = serde_json::to_value(&solved).expect("serialize solve");
    println!(
        "{}",
        json!({
            "case": "B1_retry_budget_bprime",
            "served_model": response.model,
            "decisions": {
                "cap_is_soft": response.answers["cap_is_soft"],
                "objective": response.answers["prefer_minimal"],
            },
            "weakest": review.weakest,
            "gate": if matches!(review.gate, spur_jev::gate::Gate::Open) { "open" } else { "blocked" },
            "solve_status": solved["status"],
            "model": solved["model"],
            "termination": solved["optimization"]["termination"],
            "objective_value": solved["optimization"]["solutions"][0]["objectives"][0]["value"],
            "soft_satisfied": solved["optimization"]["solutions"][0]["soft_constraints"],
            "usage": response.usage,
            "wall_ms": started.elapsed().as_millis() as u64,
        })
    );
}
