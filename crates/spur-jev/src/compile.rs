//! Deterministic compilation from gated Jev answers to solver request JSON.

use std::collections::BTreeMap;

use serde_json::{json, Map, Number, Value};
use thiserror::Error;

use crate::{snapshot::CatalogSnapshot, wire::Answer};

/// Deterministic request-compilation failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CompileError {
    /// A required Jev answer is absent.
    #[error("missing required Jev answer `{question}`")]
    MissingAnswer {
        /// Missing question identifier.
        question: String,
    },
    /// A compile directive expected a choice answer.
    #[error("Jev answer `{question}` must be a choice")]
    ExpectedChoice {
        /// Question identifier with the wrong answer type.
        question: String,
    },
    /// A compile directive expected a noul answer.
    #[error("Jev answer `{question}` must be a noul")]
    ExpectedNoul {
        /// Question identifier with the wrong answer type.
        question: String,
    },
    /// The routed rule is not present in the supplied catalog snapshot.
    #[error("routed rule `{rule_id}` is absent from the catalog snapshot")]
    UnknownRule {
        /// Rule identifier selected by Jev.
        rule_id: String,
    },
    /// The routed mode is inconsistent with the declared unknowns.
    #[error("mode `{mode}` requires {required} unknowns, found {count}")]
    ModeUnknownsMismatch {
        /// Routed solve mode.
        mode: String,
        /// Number of declared unknowns.
        count: usize,
        /// Required count for this mode.
        required: usize,
    },
    /// A string choice could not be compiled as a signed integer.
    #[error("choice `{value}` for `{question}` is not a signed integer")]
    InvalidChoiceInteger {
        /// Question identifier used by the directive.
        question: String,
        /// Non-integer choice value.
        value: String,
    },
    /// A request template has the wrong JSON shape.
    #[error("invalid compile template at {path}: {message}")]
    InvalidTemplate {
        /// JSON-like location of the invalid value.
        path: String,
        /// Shape expected by the compiler.
        message: String,
    },
}

/// Compiles a catalog-family request without invoking the solver.
///
/// `data` may be a raw family payload or an envelope containing `subjects`,
/// `parameters`, `unknowns`, and an explicit `scene`/`facts` payload. Design
/// and accessibility families consume `scene`; every other family consumes
/// `facts`. The family itself is looked up only in `snapshot`.
pub fn compile_family(
    answers: &BTreeMap<String, Answer>,
    snapshot: &CatalogSnapshot,
    data: &Value,
) -> Result<Value, CompileError> {
    let rule_id = choice_value(answers, "route_rule")?;
    let mode = choice_value(answers, "solve_mode")?;
    let family = snapshot
        .rules
        .iter()
        .find(|card| card.rule_id == rule_id)
        .map(|card| card.family.as_str())
        .ok_or_else(|| CompileError::UnknownRule {
            rule_id: rule_id.to_owned(),
        })?;
    let data = resolve_value(data, answers, "$data")?;
    let data_object = data
        .as_object()
        .ok_or_else(|| invalid_template("$data", "family data must be an object"))?;

    let subjects = data_object
        .get("subjects")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    if !subjects.is_array() {
        return Err(invalid_template("$data.subjects", "expected an array"));
    }

    let mut rule = Map::from_iter([
        ("rule_id".to_owned(), Value::String(rule_id.to_owned())),
        ("subjects".to_owned(), subjects),
    ]);
    if let Some(parameters) = data_object.get("parameters") {
        rule.insert("parameters".to_owned(), parameters.clone());
    }

    let payload_key = if uses_scene(family) { "scene" } else { "facts" };
    let payload = data_object
        .get(payload_key)
        .cloned()
        .unwrap_or_else(|| raw_payload(data_object));
    let unknowns = data_object
        .get("unknowns")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    if !unknowns.is_array() {
        return Err(invalid_template("$data.unknowns", "expected an array"));
    }
    let count = unknowns.as_array().map_or(0, Vec::len);
    if mode == "verify" && count != 0 {
        return Err(CompileError::ModeUnknownsMismatch {
            mode: mode.to_owned(),
            count,
            required: 0,
        });
    }

    Ok(Value::Object(Map::from_iter([
        ("family".to_owned(), Value::String(family.to_owned())),
        ("mode".to_owned(), Value::String(mode.to_owned())),
        ("rules".to_owned(), Value::Array(vec![Value::Object(rule)])),
        (payload_key.to_owned(), payload),
        ("unknowns".to_owned(), unknowns),
    ])))
}

/// Compiles a generic B-prime request template without invoking the solver.
///
/// Templates may use the small deterministic directives `$choice`,
/// `$choice_int`, `$noul_at_least`, and `$noul_below`. Every constraint must
/// use the canonical wrapper with an `expr` field. A top-level enum-label
/// assertion is always rewritten to `eq(var, enum_label)` so the resulting
/// expression is Boolean.
pub fn compile_bprime(
    answers: &BTreeMap<String, Answer>,
    template: &Value,
) -> Result<Value, CompileError> {
    let resolved = resolve_value(template, answers, "$template")?;
    let template = resolved
        .as_object()
        .ok_or_else(|| invalid_template("$template", "request must be an object"))?;
    let vars = template
        .get("vars")
        .filter(|value| value.is_array())
        .cloned()
        .ok_or_else(|| invalid_template("$template.vars", "expected an array"))?;
    let constraints = template
        .get("constraints")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_template("$template.constraints", "expected an array"))?;

    let constraints = constraints
        .iter()
        .enumerate()
        .map(|(index, constraint)| compile_constraint(index, constraint))
        .collect::<Result<Vec<_>, _>>()?;
    let mut request = Map::from_iter([
        ("vars".to_owned(), vars),
        ("constraints".to_owned(), Value::Array(constraints)),
    ]);

    for key in [
        "objectives",
        "objective_priority",
        "max_solutions",
        "timeout_ms",
        "persist",
        "include_smt",
        "use_cache",
        "session_id",
        "session_op",
    ] {
        if let Some(value) = template.get(key) {
            request.insert(key.to_owned(), value.clone());
        }
    }

    Ok(Value::Object(request))
}

fn compile_constraint(index: usize, constraint: &Value) -> Result<Value, CompileError> {
    let path = format!("$template.constraints[{index}]");
    let constraint = constraint
        .as_object()
        .ok_or_else(|| invalid_template(&path, "constraint must be a wrapped object"))?;
    let expr = constraint
        .get("expr")
        .cloned()
        .ok_or_else(|| invalid_template(format!("{path}.expr"), "missing expression"))?;

    let mut compiled = Map::new();
    for key in ["id", "group", "soft", "weight"] {
        if let Some(value) = constraint.get(key) {
            compiled.insert(key.to_owned(), value.clone());
        }
    }
    compiled.insert("expr".to_owned(), wrap_enum_assertion(expr, &path)?);
    Ok(Value::Object(compiled))
}

fn wrap_enum_assertion(expr: Value, path: &str) -> Result<Value, CompileError> {
    let Some(object) = expr.as_object() else {
        return Ok(expr);
    };
    if object.get("kind").and_then(Value::as_str) != Some("enum_label") {
        return Ok(expr);
    }
    let variable = object
        .get("var")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_template(format!("{path}.expr.var"), "expected a string"))?;

    Ok(json!({
        "kind": "op",
        "op": "eq",
        "args": [
            {"kind": "var", "name": variable},
            expr
        ]
    }))
}

fn resolve_value(
    value: &Value,
    answers: &BTreeMap<String, Answer>,
    path: &str,
) -> Result<Value, CompileError> {
    let Value::Object(object) = value else {
        return match value {
            Value::Array(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| resolve_value(value, answers, &format!("{path}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            scalar => Ok(scalar.clone()),
        };
    };

    if object.len() == 1 {
        if let Some(question) = object.get("$choice") {
            let question = directive_question(question, path, "$choice")?;
            return Ok(Value::String(choice_value(answers, question)?.to_owned()));
        }
        if let Some(question) = object.get("$choice_int") {
            let question = directive_question(question, path, "$choice_int")?;
            let value = choice_value(answers, question)?;
            let value = value.parse::<i64>().map_err(|_parse_error| {
                CompileError::InvalidChoiceInteger {
                    question: question.to_owned(),
                    value: value.to_owned(),
                }
            })?;
            return Ok(Value::Number(Number::from(value)));
        }
        if let Some(spec) = object.get("$noul_at_least") {
            return resolve_noul_predicate(spec, answers, path, |value, threshold| {
                value >= threshold
            });
        }
        if let Some(spec) = object.get("$noul_below") {
            return resolve_noul_predicate(spec, answers, path, |value, threshold| {
                value < threshold
            });
        }
        if let Some(key) = object.keys().find(|key| key.starts_with('$')) {
            return Err(invalid_template(
                path,
                format!("unknown compile directive `{key}`"),
            ));
        }
    }

    object
        .iter()
        .map(|(key, value)| {
            resolve_value(value, answers, &format!("{path}.{key}"))
                .map(|value| (key.clone(), value))
        })
        .collect::<Result<Map<_, _>, _>>()
        .map(Value::Object)
}

fn resolve_noul_predicate(
    spec: &Value,
    answers: &BTreeMap<String, Answer>,
    path: &str,
    predicate: impl FnOnce(f64, f64) -> bool,
) -> Result<Value, CompileError> {
    let spec = spec
        .as_object()
        .ok_or_else(|| invalid_template(path, "noul directive must be an object"))?;
    let question = spec
        .get("question")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_template(format!("{path}.question"), "expected a string"))?;
    let threshold = spec
        .get("threshold")
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid_template(format!("{path}.threshold"), "expected a number"))?;
    let value = match required_answer(answers, question)? {
        Answer::Noul { noul } => *noul,
        Answer::Choice { .. } | Answer::Score { .. } => {
            return Err(CompileError::ExpectedNoul {
                question: question.to_owned(),
            });
        }
    };
    Ok(Value::Bool(predicate(value, threshold)))
}

fn directive_question<'a>(
    value: &'a Value,
    path: &str,
    directive: &str,
) -> Result<&'a str, CompileError> {
    value
        .as_str()
        .ok_or_else(|| invalid_template(path, format!("{directive} must name a question")))
}

fn required_answer<'a>(
    answers: &'a BTreeMap<String, Answer>,
    question: &str,
) -> Result<&'a Answer, CompileError> {
    answers
        .get(question)
        .ok_or_else(|| CompileError::MissingAnswer {
            question: question.to_owned(),
        })
}

fn choice_value<'a>(
    answers: &'a BTreeMap<String, Answer>,
    question: &str,
) -> Result<&'a str, CompileError> {
    match required_answer(answers, question)? {
        Answer::Choice { choice, .. } => Ok(choice),
        Answer::Noul { .. } | Answer::Score { .. } => Err(CompileError::ExpectedChoice {
            question: question.to_owned(),
        }),
    }
}

fn uses_scene(family: &str) -> bool {
    matches!(family, "accessibility" | "design")
}

fn raw_payload(data: &Map<String, Value>) -> Value {
    Value::Object(
        data.iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "subjects" | "parameters" | "unknowns" | "scene" | "facts"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn invalid_template(path: impl Into<String>, message: impl Into<String>) -> CompileError {
    CompileError::InvalidTemplate {
        path: path.into(),
        message: message.into(),
    }
}
