//! T17: JSON-RPC dispatcher tests for `WorkerMcpServer`.
//!
//! Verifies that `tools/list` returns the curated worker-tool subset, that
//! `tools/call` routes by name to the freestanding handlers, that unknown
//! tool names produce `-32601`, and that batched JSON-RPC requests are
//! rejected at the transport decoder (per-element token attribution is
//! unsupported).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use duckdb::Connection;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::ServiceError,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use spur_acp::config::ContextServiceConfig;
use spur_acp::SpurEventBody;
use spur_core::handlers::{McpHandlerError, PlanResolver, WorkerCallContext};
use spur_core::plan::PlanState;
use spur_core::worker_server::{WorkerMcpDeps, WorkerMcpServer, WorkerSignalSink};
use spur_graph::{
    git_blob_oid, write_artifact_parquet, write_current_pointer, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphSymbolArtifact, NodeId,
    WriteOptions, GRAPH_INDEX_VERSION_TEMPORAL,
};
use spur_license::policy::PolicyResolver;
use spur_license::FeatureGate;
use spur_mcp::events::McpEventSink;
use spur_pm::{IssueCreate, PmService};
use tempfile::TempDir;
use tokio::sync::Mutex;

mod common;

struct CwdGuard {
    original: std::path::PathBuf,
}

static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl CwdGuard {
    fn enter(path: &Path) -> Self {
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

fn run_br(repo: &Path, args: &[&str]) {
    common::beads::run_br(repo, args)
        .unwrap_or_else(|err| panic!("test beads command {args:?} failed: {err}"));
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|error| panic!("run git {}: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_worker_fixture(repo: &Path) {
    run_git(repo, &["init", "-q"]);
    run_git(repo, &["config", "user.email", "test@spur"]);
    run_git(repo, &["config", "user.name", "SPUR Test"]);
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "worker fixture"]);
}

async fn pm_service_fixture(repo: &Path) -> Arc<PmService> {
    Arc::new(
        PmService::try_new(None, true, false, repo, None)
            .await
            .expect("PmService::try_new failed")
            .expect("expected Some(PmService)"),
    )
}

fn test_feature_gate() -> Arc<FeatureGate> {
    Arc::new(FeatureGate::new(PolicyResolver::embedded()))
}

struct NullSink;

impl McpEventSink for NullSink {
    fn emit(&self, _event: SpurEventBody) {}
}

/// Sink that panics on any emission — used to verify the disabled-gate
/// short-circuits before the handler is invoked.
struct PanicSink;

impl McpEventSink for PanicSink {
    fn emit(&self, _event: SpurEventBody) {
        panic!("emit must not be called when progress is disabled");
    }
    fn try_emit(&self, _event: SpurEventBody) -> Result<(), SpurEventBody> {
        panic!("try_emit must not be called when progress is disabled");
    }
}

/// Sink that simulates a full broadcast bus — `try_emit` always returns
/// `Err` so the handler must silently drop and still return success.
struct FullSink;

impl McpEventSink for FullSink {
    fn emit(&self, _event: SpurEventBody) {
        panic!("emit must not be called — try_emit should be used");
    }
    fn try_emit(&self, event: SpurEventBody) -> Result<(), SpurEventBody> {
        Err(event)
    }
}

/// Counting sink that records each event it receives. Used to verify the
/// happy path — when progress is enabled AND the bus has capacity, the
/// handler IS called and the event IS emitted.
struct CountingSink {
    count: AtomicUsize,
}

impl McpEventSink for CountingSink {
    fn emit(&self, _event: SpurEventBody) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    fn try_emit(&self, event: SpurEventBody) -> Result<(), SpurEventBody> {
        self.emit(event);
        Ok(())
    }
}

/// Sink that captures every event body it receives via `try_emit`.
struct RecordingSink {
    events: std::sync::Mutex<Vec<SpurEventBody>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl McpEventSink for RecordingSink {
    fn emit(&self, event: SpurEventBody) {
        self.events.lock().unwrap().push(event);
    }

    fn try_emit(&self, event: SpurEventBody) -> Result<(), SpurEventBody> {
        self.emit(event);
        Ok(())
    }
}

struct ForwardingWorkerSignalSink {
    funnel: Arc<dyn McpEventSink>,
}

#[async_trait]
impl WorkerSignalSink for ForwardingWorkerSignalSink {
    async fn report_signal(
        &self,
        _ctx: &WorkerCallContext,
        _args: Value,
    ) -> Result<Value, McpHandlerError> {
        Err(McpHandlerError::Unauthorized(
            "test signal sink is not licensed".into(),
        ))
    }

    async fn report_progress(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError> {
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| McpHandlerError::InvalidParams("missing message".into()))?
            .to_owned();
        let percent = args.get("percent").and_then(Value::as_f64);
        let _ = self.funnel.try_emit(SpurEventBody::WorkerReportProgress {
            delegation_id: ctx.delegation_id.clone(),
            message,
            percent,
        });
        Ok(json!({ "ok": true }))
    }
}

#[derive(Default)]
struct RecordingWorkerSignalSink {
    signal_calls: AtomicUsize,
    progress_calls: AtomicUsize,
}

#[async_trait]
impl WorkerSignalSink for RecordingWorkerSignalSink {
    async fn report_signal(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError> {
        self.signal_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "from_injected_signal_sink": true,
            "delegation_id": ctx.delegation_id,
            "task_id": args.get("task_id").and_then(Value::as_str),
        }))
    }

    async fn report_progress(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError> {
        self.progress_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "from_injected_progress_sink": true,
            "delegation_id": ctx.delegation_id,
            "message": args.get("message").and_then(Value::as_str),
        }))
    }
}

struct NullPlanResolver;

#[async_trait]
impl PlanResolver for NullPlanResolver {
    async fn load_or_project_plan(&self, plan_id: &str) -> Result<Arc<Mutex<PlanState>>, String> {
        Err(format!("test resolver: unknown plan_id '{plan_id}'"))
    }
}

fn test_deps(pm: Arc<PmService>) -> WorkerMcpDeps {
    test_deps_with_funnel(pm, Arc::new(NullSink))
}

fn test_deps_with_funnel(pm: Arc<PmService>, funnel: Arc<dyn McpEventSink>) -> WorkerMcpDeps {
    let worker_signal_sink = Arc::new(ForwardingWorkerSignalSink {
        funnel: Arc::clone(&funnel),
    });
    test_deps_with_funnel_and_signal_sink(pm, funnel, worker_signal_sink)
}

fn test_deps_with_funnel_and_signal_sink(
    pm: Arc<PmService>,
    funnel: Arc<dyn McpEventSink>,
    worker_signal_sink: Arc<dyn WorkerSignalSink>,
) -> WorkerMcpDeps {
    let feature_gate = test_feature_gate();
    let outcome_store: Arc<dyn spur_blob_store::OutcomeStore> =
        Arc::new(spur_blob_store::MemoryOutcomeStore::new());
    let worker_read_sink = Arc::new(spur_core::mcp::worker::WorkerReadMcpModule::new(
        spur_core::mcp::worker::WorkerReadMcpDeps {
            pm_service: Some(Arc::clone(&pm)),
            feature_gate: Arc::clone(&feature_gate),
            plan_resolver: Arc::new(NullPlanResolver),
            reconciler_outcomes: Arc::new(Mutex::new(
                spur_core::plan::outcomes::OutcomeStore::default(),
            )),
            outcome_store,
            repo_root: None,
        },
    ));
    WorkerMcpDeps {
        pm_service: pm,
        feature_gate,
        funnel,
        worker_signal_sink,
        worker_read_sink,
        repo_root: None,
    }
}

async fn test_server_with_real_pm() -> (TempDir, Arc<WorkerMcpServer>) {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let server = WorkerMcpServer::start("session-disp".into(), test_deps(pm))
        .await
        .expect("start must succeed");
    (dir, server)
}

async fn test_server_with_issue() -> (TempDir, Arc<WorkerMcpServer>, String) {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let issue_id = pm
        .create_issue(IssueCreate {
            title: "dispatch happy path".into(),
            description: Some("issue body".into()),
            issue_type: Some("task".into()),
            ..Default::default()
        })
        .await
        .expect("create issue");
    let server = WorkerMcpServer::start("session-disp".into(), test_deps(pm))
        .await
        .expect("start must succeed");
    (dir, server, issue_id)
}

async fn call_jsonrpc(
    server: &Arc<WorkerMcpServer>,
    token: &str,
    method: &str,
    params: Value,
) -> Value {
    let config =
        StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token.to_string());
    let client =
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
            .expect("rmcp client initialize");

    let response = match method {
        "tools/list" => match client.list_all_tools().await {
            Ok(tools) => serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "tools": tools }
            }),
            Err(error) => service_error_response(error),
        },
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .expect("params.name is required")
                .to_string();
            let mut request = CallToolRequestParams::new(tool_name);
            request.arguments = params.get("arguments").and_then(|v| v.as_object()).cloned();
            match client.call_tool(request).await {
                Ok(result) => {
                    let payload = result
                        .structured_content
                        .clone()
                        .unwrap_or_else(|| serde_json::to_value(result).expect("serialize result"));
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": payload
                    })
                }
                Err(error) => service_error_response(error),
            }
        }
        other => panic!("unsupported JSON-RPC method in test helper: {other}"),
    };

    drop(client);
    response
}

async fn call_tool_raw(
    server: &Arc<WorkerMcpServer>,
    token: &str,
    tool_name: &str,
    arguments: Value,
) -> CallToolResult {
    let config =
        StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token.to_string());
    let client =
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
            .expect("rmcp client initialize");

    let mut request = CallToolRequestParams::new(tool_name.to_string());
    request.arguments = arguments.as_object().cloned();
    let result = client.call_tool(request).await.expect("tool call succeeds");
    drop(client);
    result
}

fn structured_only(result: &CallToolResult) -> &Value {
    assert!(
        result.content.is_empty(),
        "worker tool results should not duplicate structured JSON in text content: {:?}",
        result.content
    );
    assert_eq!(result.is_error, Some(false));
    result
        .structured_content
        .as_ref()
        .expect("structured content present")
}

fn service_error_response(error: ServiceError) -> Value {
    let (code, message) = match &error {
        ServiceError::McpError(mcp_error) => {
            (i64::from(mcp_error.code.0), mcp_error.message.to_string())
        }
        _ => (-32603, error.to_string()),
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

#[tokio::test]
async fn tools_list_returns_curated_worker_tools_including_code_graph_reads() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(&server, &token, "tools/list", serde_json::json!({})).await;
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools array present");
    let expected = [
        "get_issue",
        "list_issues",
        "get_task_diff",
        "get_plan_status",
        "fetch_outcome_artifact",
        "code_symbol_search",
        "code_resolve",
        "code_file_symbols",
        "code_symbol_info",
        "code_read_symbol",
        "code_callers",
        "code_callees",
        "code_subgraph",
        "code_symbol_history",
        "doc_navigate",
        "knowledge_context_pack",
        "knowledge_context_pack_2",
        "query",
        "report_signal",
        "report_progress",
        "submit_review_verdict",
        "solve_rule_spec",
        "solve_rules",
        "solve_constraint_spec",
        "solve_constraint_check",
        "solve_constraints",
        "solve_smt",
        "get_solve_result",
        "skill_search",
        "skill_read",
        "skill_navigate",
    ];
    assert_eq!(
        tools.len(),
        expected.len(),
        "worker subset must expose exactly {} tools, got: {tools:?}",
        expected.len()
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.iter().all(|name| !name.starts_with("external_")),
        "unconfigured worker registry must omit external tools: {names:?}"
    );
    for expected in expected {
        assert!(
            names.contains(&expected),
            "missing curated tool: {expected}"
        );
    }
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn external_call_without_context_service_returns_clear_configuration_error() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-external-unconfigured", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        json!({
            "name": "external_code_read",
            "arguments": { "selector": "pkg:serde@1.0.0::serde::Deserialize" }
        }),
    )
    .await;

    assert_eq!(body["error"]["code"], -32603);
    assert_eq!(
        body["error"]["message"],
        "context service is not configured for worker MCP"
    );

    server.shutdown(Duration::from_secs(5)).await;
}

#[test]
fn worker_registry_lists_all_external_tools_when_context_service_configured() {
    let context_service = ContextServiceConfig {
        url: "https://context.example.test".to_owned(),
        ..ContextServiceConfig::default()
    };
    let registry = spur_core::mcp::worker_tool_registry_with_context_service(&context_service)
        .expect("configured worker registry should build");
    let external_names: std::collections::BTreeSet<_> = registry
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .filter(|name| name.starts_with("external_"))
        .collect();
    let expected = [
        "external_catalog",
        "external_code_search",
        "external_code_read",
        "external_code_callers",
        "external_code_callees",
        "external_knowledge_context",
        "external_index",
        "external_index_status",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    assert_eq!(external_names, expected);
}

#[tokio::test]
async fn tools_list_advertises_response_format_for_worker_code_graph_tools() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(&server, &token, "tools/list", json!({})).await;
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools array present");

    for tool_name in [
        "code_symbol_search",
        "code_file_symbols",
        "code_callers",
        "code_callees",
        "code_subgraph",
    ] {
        assert_worker_response_format_enum(tools, tool_name, &["full", "compact", "table"]);
    }
    assert_worker_response_format_enum(tools, "code_read_symbol", &["full", "compact", "source"]);

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn code_graph_worker_tools_do_not_duplicate_structured_payload_as_text_content() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    write_worker_graph_fixture(dir.path());
    server.register_delegation_worktree_root("d-no-duplicate".into(), dir.path().to_path_buf());
    let token = server.issue_token("d-no-duplicate", Duration::from_secs(60));

    let result = call_tool_raw(
        &server,
        &token,
        "code_file_symbols",
        serde_json::json!({ "file": "src/lib.rs" }),
    )
    .await;

    assert!(
        result.structured_content.is_some(),
        "worker code tools should keep machine-readable structured content"
    );
    assert!(
        result.content.is_empty(),
        "worker code tools should not duplicate structured JSON into text content: {result:?}"
    );
    assert_eq!(result.is_error, Some(false));

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn worker_query_tools_reject_injected_named_project_selectors() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-project-isolation", Duration::from_secs(60));

    for (tool_name, arguments) in [
        (
            "code_symbol_search",
            json!({"query": "anything", "project": "alpha"}),
        ),
        (
            "knowledge_context_pack",
            json!({"query": "anything", "project": "alpha"}),
        ),
        (
            "knowledge_context_pack_2",
            json!({"query": "anything", "project": "alpha"}),
        ),
        (
            "doc_navigate",
            json!({"query": "anything", "project": "alpha"}),
        ),
        ("query", json!({"query": "SELECT 1", "project": "alpha"})),
    ] {
        let body = call_jsonrpc(
            &server,
            &token,
            "tools/call",
            json!({"name": tool_name, "arguments": arguments}),
        )
        .await;
        assert_eq!(body["error"]["code"], -32602, "{tool_name}: {body:#}");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("not available on this MCP server")),
            "{tool_name}: {body:#}"
        );
    }

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn ordinary_worker_query_reads_the_assigned_worktree_analyst_database() {
    let (_server_dir, server) = test_server_with_real_pm().await;
    let worker_dir = TempDir::new().expect("worker tempdir");
    std::fs::create_dir_all(worker_dir.path().join(".spur"))
        .expect("create worker analyst directory");
    let db_path = worker_dir.path().join(".spur/analyst.duckdb");
    let conn = Connection::open(&db_path).expect("open worker analyst database");
    conn.execute_batch(
        "CREATE TABLE worker_identity (assigned_root VARCHAR); \
         INSERT INTO worker_identity VALUES ('assigned-worker-root');",
    )
    .expect("seed worker analyst database");
    drop(conn);

    server.register_delegation_worktree_root(
        "d-worker-query".into(),
        worker_dir.path().to_path_buf(),
    );
    let token = server.issue_token("d-worker-query", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        json!({
            "name": "query",
            "arguments": {
                "query": "SELECT assigned_root FROM worker_identity"
            }
        }),
    )
    .await;

    assert!(
        body.get("error").is_none(),
        "ordinary worker query should be routed: {body:#}"
    );
    assert_eq!(body["result"]["columns"], json!(["assigned_root"]));
    assert_eq!(body["result"]["rows"], json!([["assigned-worker-root"]]));
    assert_eq!(body["result"]["row_count"], 1);
    assert_eq!(body["result"]["db_path"], db_path.display().to_string());

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn ordinary_worker_query_fails_closed_without_an_assigned_worktree() {
    let (_server_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-worker-query-no-root", Duration::from_secs(60));

    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        json!({
            "name": "query",
            "arguments": { "query": "SELECT 1" }
        }),
    )
    .await;

    assert_eq!(body["error"]["code"], -32001, "{body:#}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("assigned delegation worktree")),
        "{body:#}"
    );

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn code_symbol_search_refreshes_dirty_worker_worktree_for_new_symbol() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    write_worker_graph_fixture(dir.path());
    commit_worker_fixture(dir.path());
    let source_path = dir.path().join("src/lib.rs");
    let mut source = std::fs::read_to_string(&source_path).expect("read fixture source");
    source.push_str("\npub fn worker_dirty_added_symbol() {}\n");
    std::fs::write(&source_path, source).expect("write dirty fixture source");

    server.register_delegation_worktree_root("d-dirty-search".into(), dir.path().to_path_buf());
    let token = server.issue_token("d-dirty-search", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        json!({
            "name": "code_symbol_search",
            "arguments": {
                "query": "worker_dirty_added_symbol",
                "mode": "exact",
                "limit": 20,
            },
        }),
    )
    .await;

    assert!(
        body.get("error").is_none(),
        "worker code_symbol_search should succeed, got: {body}"
    );
    let candidates = body["result"]["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("candidates array present: {body}"));
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["entity_name"] == "worker_dirty_added_symbol"),
        "dirty worker symbol should be visible through WorkerMcpServer: {body}"
    );
    assert_eq!(body["result"]["rebuild_status"], "fresh");

    server.shutdown(Duration::from_secs(5)).await;
}

fn assert_worker_response_format_enum(tools: &[Value], tool_name: &str, expected: &[&str]) {
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == tool_name)
        .unwrap_or_else(|| panic!("worker tools/list missing {tool_name}"));
    let schema = &tool["inputSchema"]["properties"]["response_format"];
    assert!(
        schema.is_object(),
        "{tool_name} must define response_format in worker tools/list inputSchema: {tool}"
    );
    let schema = resolve_worker_schema(&tool["inputSchema"], schema);
    assert_eq!(
        schema["type"], "string",
        "{tool_name}.response_format must be a string schema"
    );
    let actual = schema["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("{tool_name}.response_format must define enum values"))
        .iter()
        .map(|value| value.as_str().expect("enum entries are strings"))
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "{tool_name}.response_format enum drift in worker tools/list"
    );
}

fn resolve_worker_schema<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return schema_ref_target(root, reference);
    }

    for key in ["anyOf", "oneOf"] {
        if let Some(options) = schema.get(key).and_then(Value::as_array) {
            if let Some(option) = options
                .iter()
                .find(|option| option.get("type").and_then(Value::as_str) != Some("null"))
            {
                return resolve_worker_schema(root, option);
            }
        }
    }

    schema
}

fn schema_ref_target<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let Some(def_name) = reference
        .strip_prefix("#/$defs/")
        .or_else(|| reference.strip_prefix("#/definitions/"))
    else {
        panic!("unsupported schema ref: {reference}");
    };
    &root["$defs"][def_name]
}

#[tokio::test]
async fn tools_call_code_graph_metadata_tools_are_reachable() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    write_worker_graph_fixture(dir.path());
    let _cwd = CwdGuard::enter(dir.path());
    let token = server.issue_token("d-1", Duration::from_secs(60));

    for (tool, arguments, expected_key) in [
        (
            "code_resolve",
            serde_json::json!({ "selector": "launch_order" }),
            "candidates",
        ),
        (
            "code_file_symbols",
            serde_json::json!({ "file": "src/lib.rs" }),
            "symbols",
        ),
        (
            "code_symbol_info",
            serde_json::json!({ "selector": "launch_order" }),
            "symbol",
        ),
    ] {
        let body = call_jsonrpc(
            &server,
            &token,
            "tools/call",
            serde_json::json!({"name": tool, "arguments": arguments}),
        )
        .await;
        assert!(
            body["result"].get(expected_key).is_some(),
            "{tool} should return result.{expected_key}, got: {body}"
        );
    }

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn tools_call_code_file_symbols_omitted_response_format_stays_full() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    write_worker_graph_fixture(dir.path());
    let _cwd = CwdGuard::enter(dir.path());
    let token = server.issue_token("d-1", Duration::from_secs(60));

    let result = call_tool_raw(
        &server,
        &token,
        "code_file_symbols",
        json!({ "file": "src/lib.rs" }),
    )
    .await;
    let body = structured_only(&result);

    assert!(body.get("response_format").is_none());
    assert!(
        body.get("files").is_none(),
        "full response is not table-shaped"
    );
    assert!(
        body.get("graph_content_hash").is_some(),
        "omitted response_format must keep full metadata: {body}"
    );
    let symbol = &body["symbols"][0];
    assert_eq!(symbol["id"], "symbol-launch");
    assert_eq!(symbol["file_path"], "src/lib.rs");
    assert_eq!(symbol["uri"], "graph://symbol/symbol-launch");

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn tools_call_code_file_symbols_compact_and_table_formats_are_structured_only() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    write_worker_graph_fixture(dir.path());
    let _cwd = CwdGuard::enter(dir.path());
    let token = server.issue_token("d-1", Duration::from_secs(60));

    let compact = call_tool_raw(
        &server,
        &token,
        "code_file_symbols",
        json!({ "file": "src/lib.rs", "response_format": "compact" }),
    )
    .await;
    let compact = structured_only(&compact);
    assert!(compact.get("response_format").is_none());
    assert!(
        compact.get("graph_content_hash").is_none(),
        "compact response should omit full metadata defaults: {compact}"
    );
    assert_eq!(compact["symbols"][0]["id"], "symbol-launch");
    assert_eq!(compact["symbols"][0]["file_path"], "src/lib.rs");

    let table = call_tool_raw(
        &server,
        &token,
        "code_file_symbols",
        json!({ "file": "src/lib.rs", "response_format": "table" }),
    )
    .await;
    let table = structured_only(&table);
    assert_eq!(table["response_format"], "table");
    assert!(
        table.get("graph_content_hash").is_none(),
        "table response should omit full metadata defaults: {table}"
    );
    assert_eq!(table["files"], json!(["src/lib.rs"]));
    assert_eq!(
        table["symbols"]["cols"],
        json!([
            "id",
            "entity_name",
            "qualified_name",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
            "enclosing_scope"
        ])
    );
    assert_eq!(table["symbols"]["rows"][0][0], "symbol-launch");
    assert_eq!(table["symbols"]["rows"][0][3], 0);

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn tools_call_code_read_symbol_source_format_is_structured_only() {
    let _cwd_lock = CWD_LOCK.lock().expect("cwd lock");
    let (dir, server) = test_server_with_real_pm().await;
    let content_oid = write_worker_graph_fixture(dir.path());
    let _cwd = CwdGuard::enter(dir.path());
    let token = server.issue_token("d-1", Duration::from_secs(60));

    let result = call_tool_raw(
        &server,
        &token,
        "code_read_symbol",
        json!({
            "stable_symbol_id": "symbol-launch",
            "response_format": "source"
        }),
    )
    .await;
    let body = structured_only(&result);

    assert_eq!(
        body,
        &json!({
            "id": "symbol-launch",
            "name": "launch_order",
            "file": "src/lib.rs",
            "range": { "start": 1, "end": 3 },
            "source": WORKER_LAUNCH_SOURCE,
            "file_oid": content_oid,
        })
    );

    server.shutdown(Duration::from_secs(5)).await;
}

const WORKER_LAUNCH_SOURCE: &str = "pub fn launch_order() {\n    parse_order();\n}\n";
const WORKER_GRAPH_SOURCE: &str =
    "pub fn launch_order() {\n    parse_order();\n}\n\npub fn parse_order() {}\n";

fn write_worker_graph_fixture(worktree: &Path) -> String {
    let source_path = worktree.join("src/lib.rs");
    std::fs::create_dir_all(source_path.parent().expect("source parent"))
        .expect("create fixture source dir");
    std::fs::write(&source_path, WORKER_GRAPH_SOURCE).expect("write fixture source");
    let content_oid = git_blob_oid(WORKER_GRAPH_SOURCE.as_bytes());
    let artifact = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "worker-test".to_string(),
        graph_content_hash: "worker-hash".to_string(),
        file_manifests: vec![GraphFileManifestEntry {
            stable_file_id: "file-src-lib".to_string(),
            path: "src/lib.rs".to_string(),
            content_oid: content_oid.clone(),
            node_ids: Vec::new(),
        }],
        files: vec![GraphFileArtifact {
            stable_file_id: "file-src-lib".to_string(),
            file_path: "src/lib.rs".to_string(),
        }],
        file_node_ids: vec![NodeId(1)],
        symbols: vec![GraphSymbolArtifact {
            stable_symbol_id: "symbol-launch".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [0, 8],
            line_range: [1, 3],
            entity_name: "launch_order".to_string(),
            qualified_name: "launch_order".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "hash-symbol-launch".to_string(),
            enclosing_scope: None,
        }],
        symbol_node_ids: vec![NodeId(1_001)],
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    };
    let artifact_base = worktree.join(".spur/graph");
    let written = write_artifact_parquet(
        &artifact,
        &artifact_base,
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    write_current_pointer(worktree, &written).expect("write CURRENT pointer");
    content_oid
}

// ─── T23: per-delegation summary event emission ───────────────────────────

#[tokio::test]
async fn dispatcher_drop_emits_summary_event_with_correct_counts() {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let sink = Arc::new(RecordingSink::new());
    let server = WorkerMcpServer::start(
        "session-summary".into(),
        test_deps_with_funnel(Arc::clone(&pm), Arc::clone(&sink) as Arc<dyn McpEventSink>),
    )
    .await
    .expect("start must succeed");

    server.register_delegation(
        "d-summary".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: false,
        },
    );
    let token = server.issue_token("d-summary", Duration::from_secs(60));

    // Create an issue so both read and write calls succeed.
    let issue_id = pm
        .create_issue(spur_pm::IssueCreate {
            title: "summary test".into(),
            description: Some("body".into()),
            issue_type: Some("task".into()),
            ..Default::default()
        })
        .await
        .expect("create issue");

    // One read tool call (get_issue)
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "get_issue", "arguments": {"id": &issue_id}}),
    )
    .await;
    assert!(
        body.get("result").is_some(),
        "get_issue should succeed, got: {body}"
    );

    // A second tool call so summary event reports two distinct tools.
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({
            "name": "list_issues",
            "arguments": {}
        }),
    )
    .await;
    assert!(
        body.get("result").is_some(),
        "list_issues should succeed, got: {body}"
    );

    // Complete the delegation
    server.complete_delegation("d-summary", "success");

    {
        let events = sink.events.lock().unwrap();
        let summaries: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SpurEventBody::WorkerMcpDelegationSummary { .. }))
            .collect();
        assert_eq!(
            summaries.len(),
            1,
            "expected exactly one summary event, got: {events:?}"
        );

        if let SpurEventBody::WorkerMcpDelegationSummary {
            delegation_id,
            brain_session_id,
            calls_total,
            calls_by_tool,
            errors,
            ..
        } = &summaries[0]
        {
            assert_eq!(delegation_id, "d-summary");
            assert_eq!(brain_session_id, "session-summary");
            assert_eq!(
                *calls_total, 2,
                "expected 2 tool calls (get_issue + list_issues)"
            );
            assert_eq!(calls_by_tool.get("get_issue"), Some(&1));
            assert_eq!(calls_by_tool.get("list_issues"), Some(&1));
            assert_eq!(*errors, 0, "no calls returned errors");
        } else {
            panic!(
                "expected WorkerMcpDelegationSummary, got: {:?}",
                summaries[0]
            );
        }
    }

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn report_progress_disabled_returns_success_without_calling_handler() {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let server = WorkerMcpServer::start(
        "session-disp".into(),
        test_deps_with_funnel(pm, Arc::new(PanicSink)),
    )
    .await
    .expect("start must succeed");
    server.register_delegation(
        "d-1".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: false,
        },
    );
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "report_progress", "arguments": {"message": "hi"}}),
    )
    .await;
    assert_eq!(
        body["result"]["ok"].as_bool(),
        Some(true),
        "should return success when progress is disabled, got: {body}"
    );
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn report_progress_full_bus_silently_drops_and_returns_success() {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let server = WorkerMcpServer::start(
        "session-disp".into(),
        test_deps_with_funnel(pm, Arc::new(FullSink)),
    )
    .await
    .expect("start must succeed");
    server.register_delegation(
        "d-1".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: true,
        },
    );
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "report_progress", "arguments": {"message": "hi"}}),
    )
    .await;
    assert_eq!(
        body["result"]["ok"].as_bool(),
        Some(true),
        "should return success even when bus is full, got: {body}"
    );
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn report_progress_enabled_with_capacity_emits_event() {
    let sink = Arc::new(CountingSink {
        count: AtomicUsize::new(0),
    });
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let server = WorkerMcpServer::start(
        "session-disp".into(),
        test_deps_with_funnel(pm, sink.clone()),
    )
    .await
    .expect("start must succeed");
    server.register_delegation(
        "d-1".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: true,
        },
    );
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "report_progress", "arguments": {"message": "hi"}}),
    )
    .await;
    assert_eq!(body["result"]["ok"].as_bool(), Some(true));
    assert_eq!(
        sink.count.load(Ordering::SeqCst),
        1,
        "event must be emitted exactly once"
    );
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn worker_signal_tools_route_through_injected_sink() {
    let worker_signal_sink = Arc::new(RecordingWorkerSignalSink::default());
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm = pm_service_fixture(dir.path()).await;
    let server = WorkerMcpServer::start(
        "session-disp".into(),
        test_deps_with_funnel_and_signal_sink(
            pm,
            Arc::new(NullSink),
            Arc::<RecordingWorkerSignalSink>::clone(&worker_signal_sink)
                as Arc<dyn WorkerSignalSink>,
        ),
    )
    .await
    .expect("start must succeed");
    server.register_delegation(
        "d-injected".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: true,
        },
    );
    let token = server.issue_token("d-injected", Duration::from_secs(60));

    let signal = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({
            "name": "report_signal",
            "arguments": {
                "task_id": "issue-injected",
                "signal": {
                    "kind": "scope_drift",
                    "signal_id": "550e8400-e29b-41d4-a716-446655441111",
                    "severity": 0.5,
                    "reason": "dispatch-routing-test",
                    "estimated_subtasks": 2
                }
            }
        }),
    )
    .await;
    assert_eq!(signal["result"]["from_injected_signal_sink"], true);
    assert_eq!(signal["result"]["delegation_id"], "d-injected");
    assert_eq!(signal["result"]["task_id"], "issue-injected");

    let progress = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "report_progress", "arguments": {"message": "working"}}),
    )
    .await;
    assert_eq!(progress["result"]["from_injected_progress_sink"], true);
    assert_eq!(progress["result"]["delegation_id"], "d-injected");
    assert_eq!(progress["result"]["message"], "working");
    assert_eq!(
        worker_signal_sink.signal_calls.load(Ordering::SeqCst),
        1,
        "report_signal must call the injected sink exactly once"
    );
    assert_eq!(
        worker_signal_sink.progress_calls.load(Ordering::SeqCst),
        1,
        "report_progress must call the injected sink exactly once"
    );

    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn tools_call_get_issue_routes_to_handler() {
    let (_dir, server, issue_id) = test_server_with_issue().await;
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "get_issue", "arguments": {"id": issue_id}}),
    )
    .await;
    assert_eq!(
        body["result"]["id"].as_str(),
        Some(issue_id.as_str()),
        "dispatcher should return raw issue JSON, got: {body}"
    );
    assert_eq!(body["result"]["body"].as_str(), Some("issue body"));
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn tools_call_brain_only_tool_returns_authorization_error() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let body = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "delegate_to_worker", "arguments": {}}),
    )
    .await;
    assert_eq!(
        body["error"]["code"].as_i64(),
        Some(-32001),
        "brain-only worker tool should be rejected as authorization failure, got: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("not authorized")),
        "authorization denial should explain worker authority, got: {body}"
    );
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn json_rpc_batched_request_rejected() {
    let (_dir, server) = test_server_with_real_pm().await;
    let token = server.issue_token("d-1", Duration::from_secs(60));
    let client = reqwest::Client::new();
    let initialize = client
        .post(server.url())
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "spur-worker-dispatch-test",
                    "version": "0.0.0"
                }
            }
        }))
        .send()
        .await
        .expect("initialize request");
    let session_id = initialize
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("initialize response must carry mcp-session-id")
        .to_string();

    // rmcp's typed client API cannot encode batched JSON-RPC payload arrays,
    // so this negative wire-contract assertion must send raw JSON over HTTP.
    let response = client
        .post(server.url())
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .header("mcp-session-id", session_id)
        .bearer_auth(token)
        .json(&serde_json::json!([
            {"jsonrpc":"2.0","method":"tools/list","id":1},
            {"jsonrpc":"2.0","method":"tools/list","id":2}
        ]))
        .send()
        .await
        .expect("request");
    let status = response.status();
    let body = response.text().await.expect("response body");
    assert_eq!(
        status,
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "batches must be rejected as malformed transport requests, got {status}: {body}"
    );
    assert!(
        body.contains("JsonRpcMessage"),
        "batch rejection should identify malformed JSON-RPC payload, got: {body}"
    );
    server.shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn curated_tool_calls_route_and_deserialize_params() {
    let (_dir, server, issue_id) = test_server_with_issue().await;
    server.register_delegation(
        "d-1".into(),
        spur_core::worker_server::DelegationContext {
            enable_worker_progress: true,
        },
    );
    let token = server.issue_token("d-1", Duration::from_secs(60));

    let list_issues = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "list_issues", "arguments": {"limit": 10}}),
    )
    .await;
    assert!(
        list_issues.get("error").is_none() || list_issues["error"].is_null(),
        "list_issues should deserialize + route, got: {list_issues}"
    );
    assert!(
        list_issues["result"].as_array().is_some_and(|issues| issues
            .iter()
            .any(|issue| issue["id"].as_str() == Some(&issue_id))),
        "list_issues should include seeded issue {issue_id}, got: {list_issues}"
    );

    let get_plan_status = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "get_plan_status", "arguments": {"plan_id": "missing-plan"}}),
    )
    .await;
    assert_eq!(
        get_plan_status["error"]["code"].as_i64(),
        Some(-32602),
        "unknown plan_id should hit get_plan_status handler and return invalid params"
    );

    let get_task_diff = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({
            "name": "get_task_diff",
            "arguments": {"plan_id": "missing-plan", "task_id": &issue_id}
        }),
    )
    .await;
    assert_eq!(
        get_task_diff["error"]["code"].as_i64(),
        Some(-32602),
        "unknown plan_id should hit get_task_diff handler and return invalid params"
    );

    let fetch_outcome = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "fetch_outcome_artifact", "arguments": {"delegation_id": "d-1"}}),
    )
    .await;
    assert!(
        matches!(
            fetch_outcome["error"]["code"].as_i64(),
            Some(-32004) | Some(-32001)
        ),
        "fetch_outcome_artifact should route and return domain error, got: {fetch_outcome}"
    );

    let report_signal = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({
            "name": "report_signal",
            "arguments": {
                "task_id": &issue_id,
                "signal": {
                    "kind": "scope_drift",
                    "signal_id": "550e8400-e29b-41d4-a716-446655440999",
                    "severity": 0.5,
                    "reason": "dispatch-routing-test",
                    "estimated_subtasks": 2
                }
            }
        }),
    )
    .await;
    let signal_error = report_signal["error"]["code"].as_i64();
    let signal_recorded = report_signal["result"]["recorded"].as_bool();
    assert!(
        signal_error == Some(-32001) || signal_recorded == Some(true),
        "report_signal should route + deserialize (either gated or recorded), got: {report_signal}"
    );

    let report_progress = call_jsonrpc(
        &server,
        &token,
        "tools/call",
        serde_json::json!({"name": "report_progress", "arguments": {"message": "working"}}),
    )
    .await;
    assert_eq!(
        report_progress["result"]["ok"].as_bool(),
        Some(true),
        "report_progress should deserialize + route, got: {report_progress}"
    );

    server.shutdown(Duration::from_secs(5)).await;
}
