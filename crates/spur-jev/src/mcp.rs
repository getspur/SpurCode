//! Brain-side MCP adapter for deterministic Jev pre-compilation.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spur_mcp::{ErrorCode, McpError, ToolCallContext, ToolDefinition, ToolModule, ToolResponse};

use crate::{
    battery::generate_routing_battery,
    client::{HttpJevTransport, JevError, JevTransport},
    compile::{compile_family, CompileError},
    gate::{AmbiguityReport, DecisionReview, Gate},
    provenance::JevProvenance,
    snapshot::CatalogSnapshot,
    wire::{Answer, JevRequest, Question, DEFAULT_MODEL},
};

const INVALID_PARAMS_CODE: i32 = -32602;
const METHOD_NOT_FOUND_CODE: i32 = -32601;
const INTERNAL_ERROR_CODE: i32 = -32603;
const TOOL_NAME: &str = "jev_compile";

/// Brain-side adapter that exposes Jev as a pre-compiler, never as a solver.
#[derive(Clone)]
pub struct JevMcpModule {
    snapshot: CatalogSnapshot,
    transport: Option<Arc<dyn JevTransport>>,
    unavailable_reason: Option<Arc<str>>,
}

impl JevMcpModule {
    /// Creates a live module with an injected transport.
    #[must_use]
    pub fn new<T>(snapshot: CatalogSnapshot, transport: T) -> Self
    where
        T: JevTransport + 'static,
    {
        Self {
            snapshot,
            transport: Some(Arc::new(transport)),
            unavailable_reason: None,
        }
    }

    /// Creates a list-only module whose calls return a typed unavailable error.
    #[must_use]
    pub fn catalog_only(snapshot: CatalogSnapshot) -> Self {
        Self {
            snapshot,
            transport: None,
            unavailable_reason: Some(Arc::from("JEV_API_KEY is not set")),
        }
    }

    /// Uses [`HttpJevTransport`] when `JEV_API_KEY` is present and falls back
    /// to the catalog-only adapter otherwise.
    #[must_use]
    pub fn from_env(snapshot: CatalogSnapshot) -> Self {
        match HttpJevTransport::from_env() {
            Ok(transport) => Self::new(snapshot, transport),
            Err(error) => Self {
                snapshot,
                transport: None,
                unavailable_reason: Some(Arc::from(error.to_string())),
            },
        }
    }

    fn live_transport(&self) -> Result<&dyn JevTransport, McpError> {
        self.transport.as_deref().ok_or_else(|| {
            McpError::new(
                ErrorCode(INTERNAL_ERROR_CODE),
                self.unavailable_reason
                    .as_deref()
                    .unwrap_or("Jev transport is unavailable")
                    .to_owned(),
                Some(json!({
                    "code": "solver_unavailable",
                    "service": "jev",
                    "required_env": "JEV_API_KEY"
                })),
            )
        })
    }

    async fn compile(&self, input: JevCompileRequest) -> Result<JevCompileResult, McpError> {
        if input.intent.trim().is_empty() {
            return Err(invalid_params("`intent` must not be empty"));
        }

        let data = normalize_data(input.data)?;
        let snapshot = filter_snapshot(&self.snapshot, input.family_hint.as_deref())?;
        let questions =
            generate_routing_battery(&input.intent, &data, &snapshot, snapshot.language_version)
                .map_err(|error| invalid_params(error.to_string()))?;
        let request = JevRequest {
            state: json!({
                "intent": input.intent,
                "data": data,
                "family_hint": input.family_hint,
            }),
            model: DEFAULT_MODEL.to_owned(),
            questions,
        };

        let response = self
            .live_transport()?
            .send(&request)
            .await
            .map_err(transport_error)?;
        validate_response(&request.questions, &response.answers)?;

        let review = DecisionReview::from_answers(&response.answers, &["data_complete"]);
        let provenance = JevProvenance::from_response(&response, &review);
        match review.gate {
            Gate::Open => {
                let compiled = compile_family(&response.answers, &snapshot, &request.state["data"])
                    .map_err(|error| compile_error(error, &provenance))?;
                Ok(JevCompileResult {
                    gate: GateStatus::Open,
                    ambiguity: None,
                    request: Some(compiled),
                    provenance,
                })
            }
            Gate::Blocked(ambiguity) => Ok(JevCompileResult {
                gate: GateStatus::Blocked,
                ambiguity: Some(ambiguity),
                request: None,
                provenance,
            }),
        }
    }
}

#[async_trait]
impl ToolModule for JevMcpModule {
    fn tools(&self) -> Vec<ToolDefinition> {
        vec![tool_definition()]
    }

    async fn call(
        &self,
        ctx: ToolCallContext<'_>,
        name: &str,
        args: Value,
    ) -> Result<ToolResponse, McpError> {
        if name != TOOL_NAME {
            return Err(McpError::new(
                ErrorCode(METHOD_NOT_FOUND_CODE),
                format!("Unknown tool: {name}"),
                None,
            ));
        }
        let input = serde_json::from_value(args)
            .map_err(|error| invalid_params(format!("invalid `{TOOL_NAME}` request: {error}")))?;
        let result = serde_json::to_value(self.compile(input).await?).map_err(|error| {
            McpError::internal_error(
                format!("could not serialize `{TOOL_NAME}` response: {error}"),
                None,
            )
        })?;
        Ok(ToolResponse::json_text(ctx.request_id_value(), result))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JevCompileRequest {
    intent: String,
    data: Value,
    #[serde(default)]
    family_hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum GateStatus {
    Open,
    Blocked,
}

#[derive(Debug, Serialize)]
struct JevCompileResult {
    gate: GateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    ambiguity: Option<AmbiguityReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<Value>,
    provenance: JevProvenance,
}

fn filter_snapshot(
    snapshot: &CatalogSnapshot,
    family_hint: Option<&str>,
) -> Result<CatalogSnapshot, McpError> {
    let Some(family_hint) = family_hint else {
        return Ok(snapshot.clone());
    };
    if family_hint.trim().is_empty() {
        return Err(invalid_params("`family_hint` must not be empty"));
    }
    let rules = snapshot
        .rules
        .iter()
        .filter(|rule| rule.family == family_hint)
        .cloned()
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return Err(invalid_params(format!(
            "unknown or unavailable solver family `{family_hint}`"
        )));
    }
    Ok(CatalogSnapshot {
        language_version: snapshot.language_version,
        rules,
    })
}

fn validate_response(
    questions: &BTreeMap<String, Question>,
    answers: &BTreeMap<String, Answer>,
) -> Result<(), McpError> {
    if answers.keys().ne(questions.keys()) {
        return Err(invalid_response(
            "answer identifiers do not exactly match the deterministic battery",
        ));
    }
    for (name, question) in questions {
        let answer = &answers[name];
        let valid = match (question, answer) {
            (Question::Noul { .. }, Answer::Noul { .. })
            | (Question::Score { .. }, Answer::Score { .. }) => true,
            (
                Question::Choice { criteria, .. },
                Answer::Choice {
                    choice,
                    probabilities,
                    ..
                },
            ) => {
                criteria.contains_key(choice)
                    && probabilities.keys().all(|key| criteria.contains_key(key))
            }
            _ => false,
        };
        if !valid {
            return Err(invalid_response(format!(
                "answer `{name}` does not match its question contract"
            )));
        }
    }
    Ok(())
}

fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_NAME.to_owned(),
        description: "Pre-compile natural-language intent and structured data into a confidence-gated solver request using the deterministic Jev battery. This tool never invokes Z3. Consume an open-gate request ONLY through solve_rule_spec and solve_rules for family requests, or solve_constraint_check and solve_constraints for generic requests; never execute or trust it directly."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "intent": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Natural-language constraint intent to route."
                },
                "data": {
                    "type": ["object", "string"],
                    "description": "A structured object, or a JSON string encoding one object (decoded once before interpretation). Put subjects, parameters, scene or facts, and unknowns at the top level."
                },
                "family_hint": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Optional exact solver-family hint used only to narrow routing candidates."
                }
            },
            "required": ["intent", "data"],
            "additionalProperties": false
        }),
    }
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::new(ErrorCode(INVALID_PARAMS_CODE), message.into(), None)
}

fn normalize_data(data: Value) -> Result<Value, McpError> {
    let data = if let Value::String(encoded) = data {
        serde_json::from_str(&encoded).map_err(|error| {
            invalid_data(
                "invalid_json",
                "string",
                format!("`data` contains invalid JSON: {error}"),
                json!({"line": error.line(), "column": error.column()}),
            )
        })?
    } else {
        data
    };
    if !data.is_object() {
        let actual_type = match &data {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => unreachable!("objects passed the shape check"),
        };
        return Err(invalid_data(
            "expected_object",
            actual_type,
            format!("`data` must resolve to a JSON object; received {actual_type}"),
            json!({}),
        ));
    }
    Ok(data)
}

fn invalid_data(reason: &str, actual_type: &str, message: String, details: Value) -> McpError {
    McpError::new(
        ErrorCode(INVALID_PARAMS_CODE),
        message,
        Some(json!({
            "code": "invalid_jev_data",
            "stage": "input",
            "reason": reason,
            "path": "$.data",
            "expected_type": "object",
            "actual_type": actual_type,
            "hint": "Pass data as a JSON object or a JSON string encoding one object. Do not double-encode it.",
            "details": details,
        })),
    )
}

fn compile_error(error: CompileError, provenance: &JevProvenance) -> McpError {
    // Routing answers have already passed validate_response. Template directive
    // references and type errors therefore belong to the caller's data.
    let (reason, path, hint, details) = match &error {
        CompileError::InvalidTemplate { path, message } => (
            "invalid_template",
            path.replacen("$data", "$.data", 1),
            "Supply the expected value at this path. Keep subjects, parameters, scene or facts, and unknowns at the top level of data.",
            json!({"message": message}),
        ),
        CompileError::ModeUnknownsMismatch { mode, count, required } => (
            "mode_unknowns_mismatch",
            "$.data.unknowns".to_owned(),
            "Verification requires complete facts and no unknowns. Remove unknowns, or request synthesis if values should be solved.",
            json!({"mode": mode, "actual_count": count, "required_count": required}),
        ),
        CompileError::MissingAnswer { question } => (
            "missing_answer",
            "$.data".to_owned(),
            "Reference an existing routing-battery answer in template directives: route_rule, solve_mode, or data_complete.",
            json!({"question": question}),
        ),
        CompileError::ExpectedChoice { question } => (
            "expected_choice",
            "$.data".to_owned(),
            "Use a choice answer (route_rule or solve_mode) for a $choice directive.",
            json!({"question": question}),
        ),
        CompileError::ExpectedNoul { question } => (
            "expected_noul",
            "$.data".to_owned(),
            "Use a noul answer (data_complete) for a $noul_at_least or $noul_below directive.",
            json!({"question": question}),
        ),
        CompileError::InvalidChoiceInteger { question, value } => (
            "invalid_choice_integer",
            "$.data".to_owned(),
            "Use $choice for a string answer. $choice_int requires a choice containing a signed integer.",
            json!({"question": question, "value": value}),
        ),
        CompileError::UnknownRule { rule_id } => (
            "unknown_rule",
            "$.provenance.answers.route_rule.choice".to_owned(),
            "Refresh the solver catalog and retry; the selected rule is absent from the compilation snapshot.",
            json!({"rule_id": rule_id}),
        ),
    };
    let response_error = matches!(&error, CompileError::UnknownRule { .. });
    let choice = |question: &str| match provenance.answers.get(question) {
        Some(Answer::Choice { choice, .. }) => Some(choice.as_str()),
        _ => None,
    };
    McpError::new(
        ErrorCode(if response_error {
            INTERNAL_ERROR_CODE
        } else {
            INVALID_PARAMS_CODE
        }),
        error.to_string(),
        Some(json!({
            "code": if response_error { "invalid_jev_response" } else { "invalid_jev_compile" },
            "stage": "compile",
            "reason": reason,
            "path": path,
            "hint": hint,
            "details": details,
            "rule_id": choice("route_rule"),
            "mode": choice("solve_mode"),
            "provenance": provenance,
        })),
    )
}

fn invalid_response(message: impl Into<String>) -> McpError {
    McpError::new(
        ErrorCode(INTERNAL_ERROR_CODE),
        message.into(),
        Some(json!({"code": "invalid_jev_response"})),
    )
}

fn transport_error(error: JevError) -> McpError {
    let code = if matches!(error, JevError::MissingApiKey) {
        "solver_unavailable"
    } else {
        "jev_transport"
    };
    McpError::new(
        ErrorCode(INTERNAL_ERROR_CODE),
        error.to_string(),
        Some(json!({"code": code, "service": "jev"})),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        ffi::OsString,
        process::Command,
        sync::{Arc, Mutex},
    };

    use serde_json::{json, Value};
    use spur_mcp::{ServerKind, ToolAuthority, ToolCallContext, ToolRegistry};
    use spur_solver::{mcp::SolverMcpModule, service::SolverService};

    use super::JevMcpModule;
    use crate::{
        client::{JevError, JevTransport, MockTransport},
        snapshot::{CatalogSnapshot, RuleCard},
        wire::{Answer, JevRequest, JevResponse, Usage},
    };

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

    fn z3_available() -> bool {
        let binary = std::env::var_os("SPUR_Z3_BIN").unwrap_or_else(|| OsString::from("z3"));
        Command::new(binary)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn snapshot() -> CatalogSnapshot {
        CatalogSnapshot {
            language_version: 1,
            rules: vec![RuleCard {
                rule_id: "layout.containment".to_owned(),
                family: "design".to_owned(),
                summary: "Keep one axis-aligned rectangle inside another.".to_owned(),
            }],
        }
    }

    fn recorded_response() -> JevResponse {
        JevResponse {
            model: "jev-1.13.0".to_owned(),
            answers: BTreeMap::from([
                ("data_complete".to_owned(), Answer::Noul { noul: 0.95 }),
                (
                    "route_rule".to_owned(),
                    Answer::Choice {
                        choice: "layout.containment".to_owned(),
                        probabilities: BTreeMap::from([("layout.containment".to_owned(), 1.0)]),
                        confidence: 1.0,
                    },
                ),
                (
                    "solve_mode".to_owned(),
                    Answer::Choice {
                        choice: "verify".to_owned(),
                        probabilities: BTreeMap::from([("verify".to_owned(), 1.0)]),
                        confidence: 1.0,
                    },
                ),
            ]),
            usage: Usage {
                input_tokens: 829,
                output_tokens: 136,
            },
        }
    }

    fn recorded_data() -> Value {
        json!({
            "subjects": ["child", "parent"],
            "scene": {
                "viewport": {"width": 390, "height": 844},
                "nodes": {
                    "parent": {"rect": {"x": 0, "y": 0, "width": 100, "height": 100}},
                    "child": {"rect": {"x": 76, "y": 0, "width": 24, "height": 24}}
                }
            },
            "unknowns": []
        })
    }

    struct RecordingTransport(Arc<Mutex<Vec<JevRequest>>>);

    #[async_trait::async_trait]
    impl JevTransport for RecordingTransport {
        async fn send(&self, request: &JevRequest) -> Result<JevResponse, JevError> {
            self.0.lock().expect("record request").push(request.clone());
            Ok(recorded_response())
        }
    }

    #[tokio::test]
    async fn json_encoded_object_matches_object_before_jev_transport() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::new(
                snapshot(),
                RecordingTransport(Arc::clone(&requests)),
            ))
            .expect("register Jev module")
            .build();
        let mut results = Vec::new();
        for data in [recorded_data(), json!(recorded_data().to_string())] {
            results.push(result_json(
                &registry
                    .call_json_tool(
                        context(),
                        "jev_compile",
                        json!({"intent": "verify containment", "data": data}),
                    )
                    .await,
            ));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[1]["gate"], "open");
        let requests = requests.lock().expect("read requests");
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[1].state["data"], recorded_data());
    }

    #[tokio::test]
    async fn malformed_data_json_reports_parse_location_before_transport() {
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::catalog_only(snapshot()))
            .expect("register Jev module")
            .build();
        let response = registry
            .call_json_tool(
                context(),
                "jev_compile",
                json!({"intent": "verify containment", "data": "{\n  \"subjects\": [\n}"}),
            )
            .await;
        let error = response.error.expect("malformed JSON must fail");
        assert_eq!(error.code, i64::from(super::INVALID_PARAMS_CODE));
        let details = error.data.expect("structured input error");
        assert_eq!(details["code"], "invalid_jev_data");
        assert_eq!(details["reason"], "invalid_json");
        assert_eq!(details["stage"], "input");
        assert_eq!(details["path"], "$.data");
        assert_eq!(details["actual_type"], "string");
        assert!(details["details"]["line"].as_u64().is_some_and(|n| n > 0));
        assert!(details["details"]["column"].as_u64().is_some());
        assert!(details["hint"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn non_object_data_reports_actual_type_without_recursive_decoding() {
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::catalog_only(snapshot()))
            .expect("register Jev module")
            .build();
        for (data, actual_type) in [
            (Value::Null, "null"),
            (json!(true), "boolean"),
            (json!(1), "number"),
            (json!([]), "array"),
            (json!("null"), "null"),
            (json!("true"), "boolean"),
            (json!("1"), "number"),
            (json!("[]"), "array"),
            (
                json!(json!(recorded_data().to_string()).to_string()),
                "string",
            ),
        ] {
            let response = registry
                .call_json_tool(
                    context(),
                    "jev_compile",
                    json!({"intent": "verify containment", "data": data}),
                )
                .await;
            let error = response.error.expect("non-object data must fail");
            assert_eq!(error.code, i64::from(super::INVALID_PARAMS_CODE));
            let details = error.data.expect("structured type error");
            assert_eq!(details["code"], "invalid_jev_data");
            assert_eq!(details["reason"], "expected_object");
            assert_eq!(details["stage"], "input");
            assert_eq!(details["path"], "$.data");
            assert_eq!(details["expected_type"], "object");
            assert_eq!(details["actual_type"], actual_type);
            assert!(details["hint"].as_str().is_some_and(|s| !s.is_empty()));
        }
    }

    #[tokio::test]
    async fn compile_input_errors_report_binding_path_rule_and_repair_hint() {
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::new(
                snapshot(),
                MockTransport::new(recorded_response()),
            ))
            .expect("register Jev module")
            .build();
        for (field, value, reason, path) in [
            (
                "subjects",
                json!("child"),
                "invalid_template",
                "$.data.subjects",
            ),
            (
                "unknowns",
                json!([{"kind": "rect", "node": "child"}]),
                "mode_unknowns_mismatch",
                "$.data.unknowns",
            ),
            (
                "parameters",
                json!({"padding": {"$choice": "missing_question"}}),
                "missing_answer",
                "$.data",
            ),
            (
                "parameters",
                json!({"padding": {"$choice": "data_complete"}}),
                "expected_choice",
                "$.data",
            ),
            (
                "parameters",
                json!({"padding": {"$choice_int": "solve_mode"}}),
                "invalid_choice_integer",
                "$.data",
            ),
            (
                "parameters",
                json!({"padding": {"$noul_at_least": {"question": "solve_mode", "threshold": 0.5}}}),
                "expected_noul",
                "$.data",
            ),
        ] {
            let mut data = recorded_data();
            data[field] = value;
            let response = registry
                .call_json_tool(
                    context(),
                    "jev_compile",
                    json!({"intent": "verify containment", "data": data}),
                )
                .await;
            let error = response.error.expect("invalid binding must fail");
            assert_eq!(error.code, i64::from(super::INVALID_PARAMS_CODE));
            let details = error.data.expect("structured compile error");
            assert_eq!(details["code"], "invalid_jev_compile");
            assert_eq!(details["reason"], reason);
            assert_eq!(details["stage"], "compile");
            assert_eq!(details["path"], path);
            assert_eq!(details["rule_id"], "layout.containment");
            assert_eq!(details["mode"], "verify");
            assert_eq!(details["provenance"]["served_model_version"], "jev-1.13.0");
            assert_eq!(details["provenance"]["gate"]["status"], "open");
            assert!(details["hint"].as_str().is_some_and(|s| !s.is_empty()));
        }
    }

    #[tokio::test]
    async fn malformed_jev_answers_remain_internal_response_errors() {
        let mut response = recorded_response();
        response.answers.remove("route_rule");
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::new(snapshot(), MockTransport::new(response)))
            .expect("register Jev module")
            .build();
        let response = registry
            .call_json_tool(
                context(),
                "jev_compile",
                json!({"intent": "verify containment", "data": recorded_data()}),
            )
            .await;
        let error = response.error.expect("incomplete Jev response must fail");
        assert_eq!(error.code, i64::from(super::INTERNAL_ERROR_CODE));
        assert_eq!(
            error.data.expect("response error")["code"],
            "invalid_jev_response"
        );
    }

    #[test]
    fn data_schema_documents_object_and_json_string_inputs() {
        let schema = super::tool_definition().input_schema;
        assert_eq!(
            schema["properties"]["data"]["type"],
            json!(["object", "string"])
        );
    }

    fn generic_containment_preflight() -> Value {
        json!({
            "vars": [
                {"type": "int_range", "name": "child_x", "min": 0, "max": 390},
                {"type": "int_range", "name": "child_width", "min": 0, "max": 390},
                {"type": "int_range", "name": "parent_width", "min": 0, "max": 390}
            ],
            "constraints": [
                {"id": "recorded_child_x", "expr": {"kind": "op", "op": "eq", "args": [
                    {"kind": "var", "name": "child_x"}, {"kind": "int", "value": 76}
                ]}},
                {"id": "recorded_child_width", "expr": {"kind": "op", "op": "eq", "args": [
                    {"kind": "var", "name": "child_width"}, {"kind": "int", "value": 24}
                ]}},
                {"id": "recorded_parent_width", "expr": {"kind": "op", "op": "eq", "args": [
                    {"kind": "var", "name": "parent_width"}, {"kind": "int", "value": 100}
                ]}},
                {"id": "contained_right_edge", "expr": {"kind": "op", "op": "le", "args": [
                    {"kind": "op", "op": "add", "args": [
                        {"kind": "var", "name": "child_x"},
                        {"kind": "var", "name": "child_width"}
                    ]},
                    {"kind": "var", "name": "parent_width"}
                ]}}
            ],
            "persist": false
        })
    }

    #[tokio::test]
    async fn recorded_containment_replays_from_jev_compile_through_solver_tools() {
        if !z3_available() {
            eprintln!("skipping Jev MCP containment replay: Z3 binary is unavailable");
            return;
        }

        let registry = ToolRegistry::builder()
            .with(JevMcpModule::new(
                snapshot(),
                MockTransport::new(recorded_response()),
            ))
            .expect("register Jev module")
            .with(SolverMcpModule::new(Arc::new(SolverService::new())))
            .expect("register solver module")
            .build();

        let compiled = result_json(
            &registry
                .call_json_tool(
                    context(),
                    "jev_compile",
                    json!({
                        "intent": "verify the child remains inside its parent",
                        "data": recorded_data(),
                        "family_hint": "design"
                    }),
                )
                .await,
        );
        assert_eq!(compiled["gate"], "open");
        assert_eq!(compiled["provenance"]["served_model_version"], "jev-1.13.0");
        assert_eq!(compiled["request"]["family"], "design");
        assert_eq!(
            compiled["request"]["rules"][0]["rule_id"],
            "layout.containment"
        );

        let checked = result_json(
            &registry
                .call_json_tool(
                    context(),
                    "solve_constraint_check",
                    generic_containment_preflight(),
                )
                .await,
        );
        assert_eq!(checked["valid"], true);

        let solved = result_json(
            &registry
                .call_json_tool(context(), "solve_rules", compiled["request"].clone())
                .await,
        );
        assert_eq!(solved["status"], "sat");
        assert_eq!(solved["outcome"], "pass");
    }

    #[tokio::test]
    async fn catalog_only_module_returns_typed_unavailable_error() {
        let registry = ToolRegistry::builder()
            .with(JevMcpModule::catalog_only(snapshot()))
            .expect("register Jev catalog module")
            .build();
        let response = registry
            .call_json_tool(
                context(),
                "jev_compile",
                json!({"intent": "check containment", "data": recorded_data()}),
            )
            .await;
        let error = response.error.expect("catalog-only call must fail");
        assert_eq!(error.code, i64::from(super::INTERNAL_ERROR_CODE));
        assert_eq!(
            error.data.as_ref().and_then(|data| data["code"].as_str()),
            Some("solver_unavailable")
        );
    }
}
