//! Worker MCP dispatch coverage for context-service-owned `external_*` tools.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use rmcp::{
    model::CallToolRequestParams,
    service::ServiceError,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use spur_acp::{config::ContextServiceConfig, SpurEventBody};
use spur_core::handlers::{McpHandlerError, PlanResolver, WorkerCallContext};
use spur_core::plan::PlanState;
use spur_core::worker_server::{WorkerMcpDeps, WorkerMcpServer, WorkerSignalSink};
use spur_license::policy::PolicyResolver;
use spur_license::FeatureGate;
use spur_mcp::events::McpEventSink;
use spur_pm::PmService;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[allow(dead_code)]
mod common;

struct NullEventSink;

impl McpEventSink for NullEventSink {
    fn emit(&self, _event: SpurEventBody) {}
}

struct NullWorkerSignalSink;

#[async_trait]
impl WorkerSignalSink for NullWorkerSignalSink {
    async fn report_signal(
        &self,
        _ctx: &WorkerCallContext,
        _args: Value,
    ) -> Result<Value, McpHandlerError> {
        Ok(json!({ "ok": true }))
    }

    async fn report_progress(
        &self,
        _ctx: &WorkerCallContext,
        _args: Value,
    ) -> Result<Value, McpHandlerError> {
        Ok(json!({ "ok": true }))
    }
}

struct NullPlanResolver;

#[async_trait]
impl PlanResolver for NullPlanResolver {
    async fn load_or_project_plan(&self, plan_id: &str) -> Result<Arc<Mutex<PlanState>>, String> {
        Err(format!("test resolver: unknown plan_id '{plan_id}'"))
    }
}

fn run_br(repo: &Path, args: &[&str]) {
    common::beads::run_br(repo, args)
        .unwrap_or_else(|error| panic!("test beads command {args:?} failed: {error}"));
}

async fn test_server(
    context_service_config: ContextServiceConfig,
) -> (TempDir, Arc<WorkerMcpServer>) {
    let dir = TempDir::new().expect("tempdir");
    run_br(dir.path(), &["init"]);
    let pm_service = Arc::new(
        PmService::try_new(None, true, false, dir.path(), None)
            .await
            .expect("PmService::try_new failed")
            .expect("expected Some(PmService)"),
    );
    let feature_gate = Arc::new(FeatureGate::new(PolicyResolver::embedded()));
    let outcome_store: Arc<dyn spur_blob_store::OutcomeStore> =
        Arc::new(spur_blob_store::MemoryOutcomeStore::new());
    let worker_read_sink = Arc::new(spur_core::mcp::worker::WorkerReadMcpModule::new(
        spur_core::mcp::worker::WorkerReadMcpDeps {
            pm_service: Some(Arc::clone(&pm_service)),
            feature_gate: Arc::clone(&feature_gate),
            plan_resolver: Arc::new(NullPlanResolver),
            reconciler_outcomes: Arc::new(Mutex::new(
                spur_core::plan::outcomes::OutcomeStore::default(),
            )),
            outcome_store,
            repo_root: None,
        },
    ));
    let server = WorkerMcpServer::start_with_context_service_config(
        "session-external-dispatch".into(),
        WorkerMcpDeps {
            pm_service,
            feature_gate,
            funnel: Arc::new(NullEventSink),
            worker_signal_sink: Arc::new(NullWorkerSignalSink),
            worker_read_sink,
            repo_root: None,
        },
        context_service_config,
    )
    .await
    .expect("start worker MCP server");
    (dir, server)
}

#[derive(Clone, Default)]
struct ContextStubState {
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn context_stub(
    State(state): State<ContextStubState>,
    Json(request): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.requests.lock().await.push(request.clone());
    match request.get("tool").and_then(Value::as_str) {
        Some("external_code_read") => (
            StatusCode::OK,
            Json(json!({
                "symbol": {
                    "selector": "pkg:serde@1.0.0::serde::Deserialize",
                    "source": "pub trait Deserialize<'de>: Sized {}"
                }
            })),
        ),
        Some("external_index") => (
            StatusCode::OK,
            Json(json!({ "job_id": "index-job-1", "status": "queued" })),
        ),
        Some("external_index_status") => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "temporarily unavailable" })),
        ),
        Some(other) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unexpected tool: {other}") })),
        ),
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing tool" })),
        ),
    }
}

#[tokio::test]
async fn configured_worker_proxies_external_tools_with_structured_results_and_errors() {
    let stub_state = ContextStubState::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind context stub");
    let stub_addr = listener.local_addr().expect("context stub addr");
    let stub_app = Router::new()
        // The proxy treats `ContextServiceConfig::url` as a service origin and
        // posts context tools directly to `{origin}/mcp/code` (knowledge tools
        // would hit `{origin}/mcp/knowledge`, unused by this test).
        .route("/mcp/code", post(context_stub))
        .with_state(stub_state.clone());
    let stub = tokio::spawn(async move {
        axum::serve(listener, stub_app)
            .await
            .expect("serve context stub");
    });
    let (_dir, server) = test_server(ContextServiceConfig {
        url: format!("http://{stub_addr}"),
        ..ContextServiceConfig::default()
    })
    .await;
    let token = server.issue_token("delegation-external", Duration::from_secs(60));
    let transport = StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token);
    let client =
        ().serve(StreamableHttpClientTransport::from_config(transport))
            .await
            .expect("rmcp client initialize");

    let names: Vec<_> = client
        .list_all_tools()
        .await
        .expect("list tools")
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .filter(|name| name.starts_with("external_"))
        .collect();
    assert_eq!(
        names.len(),
        8,
        "configured worker advertises external tools"
    );

    let mut read = CallToolRequestParams::new("external_code_read");
    read.arguments = Some(
        json!({
            "selector": "pkg:serde@1.0.0::serde::Deserialize",
            "project": "serde"
        })
        .as_object()
        .expect("object")
        .clone(),
    );
    let read_result = client
        .call_tool(read)
        .await
        .expect("external read succeeds");
    assert!(read_result.content.is_empty());
    assert_eq!(read_result.is_error, Some(false));
    assert_eq!(
        read_result.structured_content,
        Some(json!({
            "symbol": {
                "selector": "pkg:serde@1.0.0::serde::Deserialize",
                "source": "pub trait Deserialize<'de>: Sized {}"
            }
        }))
    );

    let mut index = CallToolRequestParams::new("external_index");
    index.arguments = Some(
        json!({ "package": "serde", "version": "1.0.0" })
            .as_object()
            .expect("object")
            .clone(),
    );
    let index_result = client
        .call_tool(index)
        .await
        .expect("external index succeeds");
    assert!(index_result.content.is_empty());
    assert_eq!(index_result.is_error, Some(false));
    assert_eq!(
        index_result.structured_content,
        Some(json!({ "job_id": "index-job-1", "status": "queued" }))
    );

    let mut list_issues = CallToolRequestParams::new("list_issues");
    list_issues.arguments = Some(json!({ "limit": 10 }).as_object().expect("object").clone());
    let list_result = client
        .call_tool(list_issues)
        .await
        .expect("existing worker tool still routes");
    assert_eq!(list_result.is_error, Some(false));
    assert!(list_result.structured_content.is_some());

    let mut status = CallToolRequestParams::new("external_index_status");
    status.arguments = Some(
        json!({ "job_id": "index-job-1" })
            .as_object()
            .expect("object")
            .clone(),
    );
    let error = client
        .call_tool(status)
        .await
        .expect_err("upstream HTTP failure must be an MCP error");
    match error {
        ServiceError::McpError(error) => {
            assert_eq!(error.code.0, -32603);
            assert!(
                error.message.contains("context service HTTP 503"),
                "unexpected error: {}",
                error.message
            );
        }
        other => panic!("expected MCP error, got {other}"),
    }

    assert_eq!(
        *stub_state.requests.lock().await,
        vec![
            json!({
                "tool": "external_code_read",
                "args": {
                    "selector": "pkg:serde@1.0.0::serde::Deserialize",
                    "project": "serde"
                }
            }),
            json!({
                "tool": "external_index",
                "args": { "package": "serde", "version": "1.0.0" }
            }),
            json!({
                "tool": "external_index_status",
                "args": { "job_id": "index-job-1" }
            }),
        ]
    );

    drop(client);
    server.shutdown(Duration::from_secs(5)).await;
    stub.abort();
}
