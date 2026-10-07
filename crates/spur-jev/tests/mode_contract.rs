use std::collections::BTreeMap;

use serde_json::{json, Value};
use spur_jev::{
    compile::compile_family,
    snapshot::{CatalogSnapshot, RuleCard},
    wire::Answer,
};
use spur_solver::rules::execute::prepare;

fn answers(mode: &str, rule: &str) -> BTreeMap<String, Answer> {
    [("route_rule", rule), ("solve_mode", mode)]
        .into_iter()
        .map(|(question, value)| {
            (
                question.to_owned(),
                Answer::Choice {
                    choice: value.to_owned(),
                    probabilities: BTreeMap::from([(value.to_owned(), 1.0)]),
                    confidence: 1.0,
                },
            )
        })
        .collect()
}

fn snapshot(rule: &str, family: &str) -> CatalogSnapshot {
    CatalogSnapshot {
        language_version: 1,
        rules: vec![RuleCard {
            rule_id: rule.to_owned(),
            family: family.to_owned(),
            summary: "mode contract regression".to_owned(),
        }],
    }
}

#[test]
fn verification_rejects_declared_unknowns() {
    let rule = "layout.containment";
    let data = json!({
        "subjects": ["c"],
        "scene": {"nodes": {}},
        "unknowns": [{"kind": "rect", "node": "c"}]
    });

    assert!(compile_family(&answers("verify", rule), &snapshot(rule, "design"), &data).is_err());
}

#[test]
fn complete_model_synthesis_matches_solver_contract() {
    let rule = "scheduling.minimize_makespan";
    let mut data = json!({
        "subjects": ["a"],
        "parameters": {},
        "facts": {
            "horizon": 4,
            "jobs": {"a": {
                "release": 0,
                "deadline": 4,
                "durations": {"m1": 2},
                "eligible_machines": ["m1"],
                "demands": {"cpu": 1},
                "assignment": {"machine": "m1", "start": 0}
            }},
            "machines": {"m1": {"capacities": {"cpu": 1}}},
            "precedence": []
        }
    });
    let expected = json!({
        "family": "scheduling",
        "mode": "synthesize",
        "rules": [{"rule_id": rule, "subjects": ["a"], "parameters": {}}],
        "facts": data["facts"],
        "unknowns": []
    });
    // The solver introduces its own makespan variable for this objective.
    prepare(expected.clone()).expect("solver accepts synthesis of a complete schedule");

    for explicit_unknowns in [false, true] {
        if explicit_unknowns {
            data["unknowns"] = Value::Array(Vec::new());
        }
        let compiled = compile_family(
            &answers("synthesize", rule),
            &snapshot(rule, "scheduling"),
            &data,
        )
        .expect("preserve solver-supported synthesis without declared unknowns");
        assert_eq!(compiled, expected);
    }
}
