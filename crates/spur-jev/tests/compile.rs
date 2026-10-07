use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use spur_jev::{
    compile::{compile_bprime, compile_family, CompileError},
    snapshot::{CatalogSnapshot, RuleCard},
    wire::Answer,
};

fn choice(value: &str, confidence: f64) -> Answer {
    Answer::Choice {
        choice: value.to_owned(),
        probabilities: BTreeMap::from([(value.to_owned(), 1.0)]),
        confidence,
    }
}

fn report() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/jev_poc_results/phase2_report.json"
    )))
    .expect("phase-2 report")
}

#[test]
fn enum_assertions_are_boolean_eq_expressions() {
    let answers = BTreeMap::from([("required_label".to_owned(), choice("large", 1.0))]);
    let template = json!({
        "vars": [{"type": "enum", "name": "tier", "values": ["small", "large"]}],
        "constraints": [{
            "id": "tier_required",
            "expr": {
                "kind": "enum_label",
                "var": "tier",
                "label": {"$choice": "required_label"}
            }
        }]
    });

    let compiled = compile_bprime(&answers, &template).expect("compile B-prime");

    assert_eq!(
        compiled["constraints"][0]["expr"],
        json!({
            "kind": "op",
            "op": "eq",
            "args": [
                {"kind": "var", "name": "tier"},
                {"kind": "enum_label", "var": "tier", "label": "large"}
            ]
        })
    );
}

#[test]
fn recorded_family_cases_compile_with_deep_equal_parity() {
    let report = report();

    for row in report["suites"]["BC"].as_array().expect("BC rows") {
        let expected = &row["compiled"];
        let rule_id = row["routed"].as_str().expect("routed rule");
        let family = expected["family"].as_str().expect("family");
        let answers = BTreeMap::from([
            (
                "route_rule".to_owned(),
                choice(
                    rule_id,
                    row["route_conf"].as_f64().expect("route confidence"),
                ),
            ),
            (
                "solve_mode".to_owned(),
                choice(
                    row["mode"].as_str().expect("mode"),
                    row["mode_conf"].as_f64().expect("mode confidence"),
                ),
            ),
            (
                "data_complete".to_owned(),
                Answer::Noul {
                    noul: row["data_complete"].as_f64().expect("data completeness"),
                },
            ),
        ]);
        let snapshot = CatalogSnapshot {
            language_version: 1,
            rules: vec![RuleCard {
                rule_id: rule_id.to_owned(),
                family: family.to_owned(),
                summary: "recorded fixture".to_owned(),
            }],
        };

        let rule = &expected["rules"][0];
        let payload_key = if expected.get("scene").is_some() {
            "scene"
        } else {
            "facts"
        };
        let mut data = Map::from_iter([
            ("subjects".to_owned(), rule["subjects"].clone()),
            ("parameters".to_owned(), rule["parameters"].clone()),
            ("unknowns".to_owned(), expected["unknowns"].clone()),
            (payload_key.to_owned(), expected[payload_key].clone()),
        ]);
        if let Some(dimension) = row.get("dimension") {
            data.insert("selected_dimension".to_owned(), dimension.clone());
        }

        let compiled = compile_family(&answers, &snapshot, &Value::Object(data))
            .unwrap_or_else(|error| panic!("{}: {error}", row["case"]));
        assert_eq!(compiled, *expected, "{}", row["case"]);
    }
}

#[test]
fn verification_unknowns_mismatch_is_a_typed_error() {
    let answers = BTreeMap::from([
        ("route_rule".to_owned(), choice("layout.containment", 1.0)),
        ("solve_mode".to_owned(), choice("verify", 1.0)),
    ]);
    let snapshot = CatalogSnapshot {
        language_version: 1,
        rules: vec![RuleCard {
            rule_id: "layout.containment".to_owned(),
            family: "design".to_owned(),
            summary: "s".to_owned(),
        }],
    };
    let data = json!({"subjects": ["c"], "scene": {"nodes": {}}, "unknowns": [{"kind": "rect", "node": "c"}]});

    let err = compile_family(&answers, &snapshot, &data).expect_err("verify with unknowns");
    assert!(
        matches!(err, CompileError::ModeUnknownsMismatch { mode, count: 1, required: 0 } if mode == "verify")
    );
}

#[test]
fn family_is_taken_from_snapshot_metadata_not_rule_prefix() {
    let answers = BTreeMap::from([
        (
            "route_rule".to_owned(),
            choice("scheduling.looks_like_a_family", 1.0),
        ),
        ("solve_mode".to_owned(), choice("verify", 1.0)),
    ]);
    let snapshot = CatalogSnapshot {
        language_version: 1,
        rules: vec![RuleCard {
            rule_id: "scheduling.looks_like_a_family".to_owned(),
            family: "design".to_owned(),
            summary: "metadata wins".to_owned(),
        }],
    };
    let data = json!({
        "subjects": ["child", "parent"],
        "parameters": {"padding": 4},
        "scene": {"nodes": {}},
        "unknowns": []
    });

    let compiled = compile_family(&answers, &snapshot, &data).expect("compile family");

    assert_eq!(compiled["family"], "design");
    assert_eq!(compiled["scene"], data["scene"]);
    assert_eq!(compiled["rules"][0]["parameters"], data["parameters"]);
    assert_eq!(compiled["unknowns"], data["unknowns"]);
    assert!(compiled.get("facts").is_none());
}

#[test]
fn recorded_bprime_cases_compile_with_deep_equal_parity_after_d1_repair() {
    let report = report();

    for row in report["suites"]["D"].as_array().expect("D rows") {
        let case = row["case"].as_str().expect("case");
        let answers = bprime_answers(row);
        let mut template = row["compiled"].clone();
        let mut expected = row["compiled"].clone();

        match case {
            "D1_enum_label" => {
                template["vars"][0]["type"] = json!({"$choice": "x_kind"});
                template["vars"][1]["type"] = json!({"$choice": "y_kind"});
                template["constraints"][1]["expr"]["label"] = json!({"$choice": "required_label"});
                let label = expected["constraints"][1]["expr"].clone();
                expected["constraints"][1]["expr"] = json!({
                    "kind": "op",
                    "op": "eq",
                    "args": [
                        {"kind": "var", "name": "tier"},
                        label
                    ]
                });
            }
            "D2_hard_soft_objective" => {
                template["constraints"][0]["soft"] = json!({
                    "$noul_below": {"question": "floor_is_hard", "threshold": 0.5}
                });
                template["constraints"][1]["soft"] = json!({
                    "$noul_at_least": {"question": "cap_is_soft", "threshold": 0.5}
                });
                template["objectives"][0]["op"] = json!({"$choice": "objective"});
            }
            "D3_multi_constraint" => {
                template["constraints"][0]["soft"] = json!({
                    "$noul_below": {"question": "sum_is_hard", "threshold": 0.5}
                });
                template["constraints"][1]["soft"] = json!({
                    "$noul_below": {"question": "distinct_is_hard", "threshold": 0.5}
                });
                template["constraints"][2]["soft"] = json!({
                    "$noul_at_least": {"question": "pref_is_soft", "threshold": 0.5}
                });
                template["constraints"][2]["weight"] = json!({"$choice_int": "pref_weight"});
            }
            other => panic!("unhandled recorded case {other}"),
        }

        let compiled =
            compile_bprime(&answers, &template).unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(compiled, expected, "{case}");
    }
}

fn bprime_answers(row: &Value) -> BTreeMap<String, Answer> {
    let decisions = row["decisions"].as_object().expect("decisions");
    let confidences = row["confidences"].as_object().expect("confidences");
    decisions
        .iter()
        .map(|(question, decision)| {
            let answer = if let Some(value) = decision.as_str() {
                choice(
                    value,
                    confidences[question].as_f64().expect("choice confidence"),
                )
            } else {
                Answer::Noul {
                    noul: decision.as_f64().expect("noul decision"),
                }
            };
            (question.clone(), answer)
        })
        .collect()
}
