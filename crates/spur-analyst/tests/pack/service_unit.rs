#![allow(clippy::needless_raw_string_hashes)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

use super::*;
use crate::embedding::sidecar_service::{EmbedService, SPUR_EMBED_SOCKET_ENV};
#[cfg(feature = "embed")]
use crate::embedding::{embed_model_cell, embed_with_ready_model, EmbedModelCell};
use crate::embedding::{
    reset_auto_sidecar_probe_for_test, set_analyst_embed_mode_for_test, warm_embed_model,
    AnalystEmbedMode, EmbeddingRuntime,
};
use crate::{KnowledgeCandidate, KnowledgeQueryIntent, KnowledgeSearchScope};
use duckdb::Connection;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;

use crate::db::sql::{sql_escape_literal, sql_escape_path};
use spur_graph::store::{write_sections_dataset, CODE_SYMBOLS_PARQUET, SECTIONS_PARQUET};
use spur_graph::{
    artifact_from_facts, build_facts, embedding_query_text_for_model, write_artifact_parquet,
    write_current_pointer, EmbeddingModelSelection, EMBEDDING_VECTOR_DIMENSIONS,
};

const INIT_SEARCH_SQL: &str = include_str!("../../../spur-context/analyst/init_search.sql");
static ENV_LOCK: Mutex<()> = Mutex::new(());
static ASYNC_ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

struct HybridConfidenceFixture {
    _temp_dir: tempfile::TempDir,
    db_path: PathBuf,
    query_vec: Vec<f32>,
}

struct OverlayPackFixture {
    _temp_dir: tempfile::TempDir,
    repo: PathBuf,
    base_hash: String,
}

async fn run_knowledge_context_pack(args: &Value) -> Result<Value, McpHandlerError> {
    let request = KnowledgeContextPackRequest::parse(args)?;
    knowledge_context_pack(request).await
}

async fn run_knowledge_context_pack_2(args: &Value) -> Result<Value, McpHandlerError> {
    let request = KnowledgeContextPackV2Request::parse(args)?;
    knowledge_context_pack_2(request).await
}

#[test]
fn hybrid_confidence_thresholds_match_bge_base_scores() {
    assert_eq!(
        confidence_score_thresholds(Some("hybrid-code")),
        (0.80, 0.55)
    );
}

#[tokio::test]
async fn analyst_db_path_falls_back_to_parent_repo_db_for_spur_worker_worktree() {
    let _lock = async_env_lock().await;
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let repo_spur = repo_dir.path().join(".spur");
    let worker_dir = repo_spur.join("worktrees").join("worker-1");
    fs::create_dir_all(&worker_dir).expect("create worker dir");
    fs::write(repo_spur.join("analyst.duckdb"), b"db").expect("write repo analyst db");

    let selected =
        spur_graph::mcp::with_worktree_root_for_request(worker_dir, async { analyst_db_path() })
            .await
            .expect("analyst db path");

    assert_eq!(selected, repo_spur.join("analyst.duckdb"));
}

fn test_embedding(first_value: f32) -> [f32; EMBEDDING_VECTOR_DIMENSIONS] {
    let mut embedding = [0.0; EMBEDDING_VECTOR_DIMENSIONS];
    embedding[0] = first_value;
    embedding
}

fn test_embedding_vec(first_value: f32) -> Vec<f32> {
    test_embedding(first_value).to_vec()
}

async fn embed_query(query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
    EmbeddingRuntime::global().embed_query(query).await
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn set_env_var_for_test(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> EnvVarGuard {
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    EnvVarGuard { key, previous }
}

struct StubSidecar {
    _temp_dir: tempfile::TempDir,
    socket_path: PathBuf,
    task: JoinHandle<()>,
}

impl Drop for StubSidecar {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn wait_for_socket(socket_path: &Path) {
    for _ in 0..100 {
        match UnixStream::connect(socket_path).await {
            Ok(_) => return,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!(
                "unexpected error while waiting for socket {}: {error}",
                socket_path.display()
            ),
        }
    }

    panic!("timed out waiting for socket {}", socket_path.display());
}

async fn start_stub_embed_sidecar<E>(
    ready: impl Fn() -> bool + Send + Sync + 'static,
    embedder: E,
) -> StubSidecar
where
    E: Fn(Vec<String>) -> Result<Vec<Vec<f32>>, String> + Send + Sync + 'static,
{
    let temp_dir = tempfile::tempdir().expect("sidecar tempdir");
    let socket_path = temp_dir.path().join("embed.sock");
    let service = EmbedService::new("stub-model", ready, embedder);
    let serve_path = socket_path.clone();
    let task = tokio::spawn(async move {
        let _ = service.serve_socket(serve_path).await;
    });
    wait_for_socket(&socket_path).await;

    StubSidecar {
        _temp_dir: temp_dir,
        socket_path,
        task,
    }
}

async fn start_raw_embed_sidecar(
    response_line: Option<String>,
    response_delay: Duration,
    request_count: Arc<std::sync::atomic::AtomicUsize>,
) -> StubSidecar {
    let temp_dir = tempfile::tempdir().expect("sidecar tempdir");
    let socket_path = temp_dir.path().join("embed.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind raw sidecar");
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .expect("set raw sidecar socket permissions");
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let response_line = response_line.clone();
            let request_count = Arc::clone(&request_count);
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut lines = BufReader::new(reader).lines();
                if lines.next_line().await.ok().flatten().is_none() {
                    return;
                }
                request_count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(response_delay).await;
                if let Some(line) = response_line {
                    let _ = writer.write_all(line.as_bytes()).await;
                    let _ = writer.write_all(b"\n").await;
                }
            });
        }
    });
    wait_for_socket(&socket_path).await;

    StubSidecar {
        _temp_dir: temp_dir,
        socket_path,
        task,
    }
}

#[derive(Clone, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedTraceEvent>>>,
}

impl TraceCapture {
    fn subscriber(&self) -> CaptureSubscriber {
        CaptureSubscriber {
            events: Arc::clone(&self.events),
        }
    }

    fn contains_warning(&self, needle: &str) -> bool {
        self.events
            .lock()
            .expect("trace events lock")
            .iter()
            .any(|event| event.level == "WARN" && event.fields.contains(needle))
    }
}

struct CapturedTraceEvent {
    level: &'static str,
    fields: String,
}

struct CaptureSubscriber {
    events: Arc<Mutex<Vec<CapturedTraceEvent>>>,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace events lock")
            .push(CapturedTraceEvent {
                level: event.metadata().level().as_str(),
                fields: visitor.fields,
            });
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct TraceFieldVisitor {
    fields: String,
}

impl tracing::field::Visit for TraceFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(&format!("{}={value:?}", field.name()));
    }
}

#[cfg(feature = "embed")]
#[test]
fn embed_model_cell_selection_uses_one_distinct_cell_per_model() {
    let nomic = embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15);
    let nomic_again = embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15);
    let coderank = embed_model_cell(EmbeddingModelSelection::CodeRankEmbed);
    let jina = embed_model_cell(EmbeddingModelSelection::JinaEmbeddingsV2BaseCode);

    assert!(std::ptr::eq(nomic, nomic_again));
    assert!(!std::ptr::eq(nomic, coderank));
    assert!(!std::ptr::eq(nomic, jina));
    assert!(!std::ptr::eq(coderank, jina));
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn embedding_runtime_facade_can_embed_query() {
    let _lock = async_env_lock().await;
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Off);

    assert!(
        EmbeddingRuntime::global()
            .embed_query("ranking beacon")
            .await
            .is_none(),
        "off mode should make the shared embedding runtime skip vector search"
    );
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn off_embed_mode_never_starts_in_process_model_load() {
    let _lock = async_env_lock().await;
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Off);
    let model_cell = embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15);

    assert!(!model_cell.is_ready(), "test assumes model has not loaded");
    assert!(
        !model_cell.is_loading_for_test(),
        "test assumes no previous load is running"
    );

    let runtime = crate::embedding::OwnedEmbeddingRuntime::new();
    warm_embed_model(&runtime).await.expect("owned warm");
    runtime.shutdown().await.expect("joined warm");

    assert!(
        !model_cell.is_ready(),
        "off mode must not warm the in-process model"
    );
    assert!(
        !model_cell.is_loading_for_test(),
        "off mode must not mark the model cell as loading"
    );

    assert!(embed_query("ranking beacon").await.is_none());
    assert!(
        !model_cell.is_ready(),
        "off mode query must not load the in-process model"
    );
    assert!(
        !model_cell.is_loading_for_test(),
        "off mode query must not start a background load"
    );
}

#[tokio::test]
async fn sidecar_embed_mode_returns_query_vector_without_double_transforming_text() {
    let _lock = async_env_lock().await;
    let captured_texts = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&captured_texts);
    let sidecar = start_stub_embed_sidecar(
        || true,
        move |texts| {
            *captured.lock().expect("captured sidecar texts") = texts;
            Ok(vec![test_embedding_vec(0.25)])
        },
    )
    .await;
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &sidecar.socket_path);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Sidecar);

    let embedding = embed_query("ranking beacon")
        .await
        .expect("sidecar mode should return the sidecar vector");

    assert_eq!(embedding[0], 0.25);
    let texts = captured_texts.lock().expect("captured sidecar texts");
    assert_eq!(
        texts.as_slice(),
        [embedding_query_text_for_model(
            "ranking beacon",
            EmbeddingModelSelection::NomicEmbedTextV15
        )
        .into_owned()],
        "the client must send the raw query and let the sidecar apply the model transform once"
    );
}

#[tokio::test]
async fn sidecar_embed_mode_absent_socket_falls_back_quickly() {
    let _lock = async_env_lock().await;
    let temp_dir = tempfile::tempdir().expect("socket tempdir");
    let missing_socket = temp_dir.path().join("missing.sock");
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &missing_socket);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Sidecar);

    let started = std::time::Instant::now();
    let embedding = embed_query("ranking beacon").await;
    let elapsed = started.elapsed();

    assert!(embedding.is_none());
    assert!(
        elapsed < Duration::from_millis(500),
        "absent sidecar socket should not wait for the full inference budget, elapsed={elapsed:?}"
    );
}

#[tokio::test]
async fn sidecar_embed_mode_times_out_the_whole_round_trip() {
    let _lock = async_env_lock().await;
    let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sidecar = start_raw_embed_sidecar(
        None,
        Duration::from_millis(5_000),
        Arc::clone(&request_count),
    )
    .await;
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &sidecar.socket_path);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Sidecar);

    let started = std::time::Instant::now();
    let embedding = embed_query("ranking beacon").await;
    let elapsed = started.elapsed();

    assert!(embedding.is_none());
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "sidecar client should connect and send one request before timing out"
    );
    assert!(
        elapsed >= Duration::from_millis(1_000) && elapsed < Duration::from_millis(2_500),
        "sidecar timeout should budget the connect+write+read round trip, elapsed={elapsed:?}"
    );
}

#[tokio::test]
async fn sidecar_embed_mode_dimension_mismatch_falls_back_to_bm25() {
    let _lock = async_env_lock().await;
    let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = serde_json::json!({
        "v": 1,
        "id": "query",
        "vectors": [[0.0, 1.0, 2.0]]
    })
    .to_string();
    let sidecar =
        start_raw_embed_sidecar(Some(response), Duration::ZERO, Arc::clone(&request_count)).await;
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &sidecar.socket_path);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Sidecar);

    let embedding = embed_query("ranking beacon").await;

    assert!(embedding.is_none());
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "dimension mismatch should be detected from the sidecar response"
    );
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn auto_embed_mode_uses_reachable_sidecar_without_in_process_load() {
    let _lock = async_env_lock().await;
    reset_auto_sidecar_probe_for_test();
    let sidecar = start_stub_embed_sidecar(|| true, |_| Ok(vec![test_embedding_vec(0.5)])).await;
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &sidecar.socket_path);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Auto);
    let model_cell = embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15);
    let _permit = model_cell
        .begin_load()
        .expect("test should hold the in-process load gate");

    let embedding = embed_query("ranking beacon")
        .await
        .expect("auto mode should use the reachable sidecar");

    assert_eq!(embedding[0], 0.5);
    assert!(
        !model_cell.is_ready(),
        "auto sidecar query must not initialize the in-process model"
    );
    assert!(
        model_cell.is_loading_for_test(),
        "the only in-process load state should be the permit held by this test"
    );
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn auto_warm_embed_model_pings_reachable_sidecar_without_starting_in_process_load() {
    let _lock = async_env_lock().await;
    reset_auto_sidecar_probe_for_test();
    let pinged = Arc::new(AtomicBool::new(false));
    let pinged_by_ready = Arc::clone(&pinged);
    let sidecar = start_stub_embed_sidecar(
        move || {
            pinged_by_ready.store(true, Ordering::SeqCst);
            true
        },
        |_| Ok(vec![test_embedding_vec(0.5)]),
    )
    .await;
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &sidecar.socket_path);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Auto);
    let model_cell = embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15);
    let _permit = model_cell
        .begin_load()
        .expect("test should hold the in-process load gate");

    let runtime = crate::embedding::OwnedEmbeddingRuntime::new();
    warm_embed_model(&runtime).await.expect("owned warm");
    runtime.shutdown().await.expect("joined warm");
    for _ in 0..20 {
        if pinged.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert!(
        pinged.load(Ordering::SeqCst),
        "auto warm should probe the sidecar before considering in-process warm-up"
    );
    assert!(
        !model_cell.is_ready(),
        "auto warm with a reachable sidecar must not initialize the in-process model"
    );
    assert!(
        model_cell.is_loading_for_test(),
        "the only in-process load state should be the permit held by this test"
    );
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn auto_embed_mode_without_sidecar_uses_supervised_helper() {
    let _lock = async_env_lock().await;
    reset_auto_sidecar_probe_for_test();
    let temp_dir = tempfile::tempdir().expect("socket tempdir");
    let missing_socket = temp_dir.path().join("missing.sock");
    let _socket_guard = set_env_var_for_test(SPUR_EMBED_SOCKET_ENV, &missing_socket);
    let _mode_guard = set_analyst_embed_mode_for_test(AnalystEmbedMode::Auto);
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
    let log = temp_dir.path().join("worker.log");
    let config = json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let _config = set_env_var_for_test("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string());
    let _log = set_env_var_for_test("SPUR_GRAPH_TEST_WORKER_LOG", &log);
    let runtime = crate::embedding::OwnedEmbeddingRuntime::new();
    assert!(runtime.embed_query("ranking beacon").await.is_some());
    runtime.shutdown().await.expect("joined fallback helper");
    let events = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        events.lines().filter(|line| *line == "service").count(),
        1,
        "{events}"
    );
    assert_eq!(events.lines().last(), Some("shutdown"), "{events}");
}

#[test]
fn unknown_embed_mode_falls_back_to_auto_and_warns() {
    let captured = TraceCapture::default();

    let mode = tracing::subscriber::with_default(captured.subscriber(), || {
        AnalystEmbedMode::parse_env_value("mystery-mode")
    });

    assert_eq!(mode, AnalystEmbedMode::Auto);
    assert!(
        captured.contains_warning("unknown analyst embed mode")
            && captured.contains_warning("mystery-mode")
            && captured.contains_warning("SPUR_ANALYST_EMBED_MODE"),
        "unknown mode should emit a warning with the bad value and env var"
    );
}

#[cfg(feature = "embed")]
#[test]
fn embed_model_cell_retries_after_transient_load_failure() {
    let cell = EmbedModelCell::<u32>::new();
    let mut attempts = 0;

    assert!(cell
        .load_if_idle(|| {
            attempts += 1;
            None
        })
        .is_none());
    assert_eq!(attempts, 1);

    let model = cell
        .load_if_idle(|| {
            attempts += 1;
            Some(7)
        })
        .expect("second load should succeed");
    assert_eq!(
        *model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        7
    );
    assert_eq!(attempts, 2);

    let model = cell
        .load_if_idle(|| {
            attempts += 1;
            Some(9)
        })
        .expect("ready model should be reused");
    assert_eq!(
        *model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        7
    );
    assert_eq!(attempts, 2, "ready model should not be reloaded");
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn embed_with_ready_model_falls_back_while_load_in_progress() {
    let cell = EmbedModelCell::<u32>::new();
    let _permit = cell.begin_load().expect("load should begin");
    let inference_called = Arc::new(AtomicBool::new(false));
    let called = Arc::clone(&inference_called);

    let result = embed_with_ready_model(&cell, "query", Duration::from_millis(25), move |_, _| {
        called.store(true, Ordering::SeqCst);
        Some(test_embedding(1.0))
    })
    .await;

    assert!(result.is_none());
    assert!(
        !inference_called.load(Ordering::SeqCst),
        "inference must not run while the model is still loading"
    );
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn embed_with_ready_model_times_out_inference_only() {
    let cell = EmbedModelCell::<u32>::new();
    cell.load_if_idle(|| Some(42))
        .expect("test model should load");

    let result = embed_with_ready_model(&cell, "query", Duration::from_millis(10), move |_, _| {
        std::thread::sleep(Duration::from_millis(100));
        Some(test_embedding(1.0))
    })
    .await;

    assert!(result.is_none());
}

fn candidate(stable_symbol_id: Option<&str>, title: &str, score: f64) -> KnowledgeCandidate {
    KnowledgeCandidate {
        kind: "code".into(),
        title: title.into(),
        file_path: "crates/spur-mcp/src/lib.rs".into(),
        stable_symbol_id: stable_symbol_id.map(str::to_string),
        symbol_kind: Some("function".into()),
        score,
        signal: None,
        neighbor_kind: None,
        edge_bind_method: None,
        grounding: "test".into(),
    }
}

fn doc_candidate(stable_symbol_id: Option<&str>, title: &str, score: f64) -> KnowledgeCandidate {
    KnowledgeCandidate {
        kind: "doc".into(),
        title: title.into(),
        file_path: "docs/context.md".into(),
        stable_symbol_id: stable_symbol_id.map(str::to_string),
        symbol_kind: Some("section".into()),
        score,
        signal: None,
        neighbor_kind: None,
        edge_bind_method: None,
        grounding: "test-doc".into(),
    }
}

fn minimal_analyst_db_with_meta() -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = temp_dir.path().join("analyst.duckdb");
    let conn = Connection::open(&db_path).expect("open fixture db");
    conn.execute_batch(
        r#"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('fixture-hash');
        "#,
    )
    .expect("create fixture meta");
    drop(conn);
    (temp_dir, db_path)
}

fn analyst_db_with_path_budget_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp_dir, db_path) = minimal_analyst_db_with_meta();
    let conn = Connection::open(&db_path).expect("open fixture db");
    conn.execute_batch(
        r#"
        CREATE TABLE edges (
            source_stable_id VARCHAR,
            target_stable_id VARCHAR,
            relation VARCHAR,
            edge_kind VARCHAR,
            confidence VARCHAR,
            bind_method VARCHAR
        );
        INSERT INTO edges VALUES
            ('sym-source', 'sym-connected', 'calls', 'calls', 'syntax_exact', 'singleton');
        "#,
    )
    .expect("create path budget fixture");
    drop(conn);
    (temp_dir, db_path)
}

fn analyst_db_with_graph_reasoning_views() -> (tempfile::TempDir, PathBuf) {
    let (temp_dir, db_path) = minimal_analyst_db_with_meta();
    let conn = Connection::open(&db_path).expect("open fixture db");
    conn.execute_batch(
        r#"
        CREATE TABLE v_symbol_scorecard (
            stable_symbol_id VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            file_path VARCHAR,
            pagerank DOUBLE,
            in_degree BIGINT,
            out_degree BIGINT,
            callers BIGINT,
            importers BIGINT,
            inbound_total BIGINT,
            churn_90d BIGINT,
            last_touched TIMESTAMP,
            blast_radius_score DOUBLE,
            posture VARCHAR
        );
        INSERT INTO v_symbol_scorecard VALUES
            ('sym-one', 'symbol_one', 'fixture::symbol_one', 'function', 'src/one.rs',
             0.42, 7, 3, 5, 1, 6, 9, TIMESTAMP '2026-06-17 12:00:00', 0.91, 'active'),
            ('sym-two', 'symbol_two', 'fixture::symbol_two', 'function', 'src/two.rs',
             0.21, 2, 1, 1, 0, 1, 0, NULL, 0.12, 'stable');

        CREATE TABLE v_symbol_component (
            stable_symbol_id VARCHAR,
            component_id BIGINT,
            component_size BIGINT
        );
        INSERT INTO v_symbol_component VALUES
            ('sym-one', 10, 4),
            ('sym-two', 10, 4);

        CREATE TABLE v_symbol_community (
            stable_symbol_id VARCHAR,
            community_id BIGINT
        );
        INSERT INTO v_symbol_community VALUES
            ('sym-one', 20),
            ('sym-two', 20);

        CREATE TABLE v_graph_metrics (
            calls_edges BIGINT,
            connected_nodes BIGINT,
            components BIGINT,
            largest_component BIGINT,
            communities BIGINT,
            density DOUBLE
        );
        INSERT INTO v_graph_metrics VALUES (12, 6, 1, 6, 2, 0.18);
        "#,
    )
    .expect("create graph reasoning fixture views");
    drop(conn);
    (temp_dir, db_path)
}

fn git(worktree: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()
        .unwrap_or_else(|error| panic!("run git {}: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout UTF-8")
}

fn commit_fixture(worktree: &Path) {
    git(worktree, &["init", "-q"]);
    git(worktree, &["config", "user.email", "test@spur"]);
    git(worktree, &["config", "user.name", "SPUR Test"]);
    git(worktree, &["add", "."]);
    git(worktree, &["commit", "-m", "fixture"]);
}

fn write_graph_artifact_for_test(worktree: &Path, artifact: &spur_graph::GraphIndexArtifact) {
    let artifact_dir = worktree.join(".spur/graph/test-artifact.parquet");
    let written = write_artifact_parquet(
        artifact,
        &artifact_dir,
        spur_graph::WriteOptions::default(),
        Vec::new(),
    )
    .expect("write graph artifact");
    write_current_pointer(worktree, &written).expect("write graph CURRENT pointer");
}

fn write_minimal_graph_fixture(worktree: &Path, source: &str) {
    fs::create_dir_all(worktree.join("src")).expect("create src dir");
    fs::write(
        worktree.join("Cargo.toml"),
        "[package]\nname = \"kcp-graph-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write fixture manifest");
    fs::write(worktree.join("src/lib.rs"), source).expect("write fixture source");
}

fn kcp2_fixture_repo(include_graph_reasoning_views: bool) -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let repo = temp_dir.path().join("repo");
    fs::create_dir_all(repo.join(".spur")).expect("create .spur");
    let db_path = repo.join(".spur").join("analyst.duckdb");
    let conn = Connection::open(&db_path).expect("open fixture db");
    configure_fixture_extension_directory(&conn);
    conn.execute_batch("INSTALL fts; LOAD fts; INSTALL icu; LOAD icu;")
        .expect("load fixture extensions");
    conn.execute_batch(
        r#"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('kcp2-fixture-hash');

        CREATE TABLE sections_search (
            stable_symbol_id VARCHAR,
            qualified_name VARCHAR,
            file_path VARCHAR,
            heading_level INTEGER,
            content_hash VARCHAR,
            body_text VARCHAR,
            embedding FLOAT[768]
        );
        INSERT INTO sections_search (
            stable_symbol_id, qualified_name, file_path, heading_level, content_hash, body_text
        ) VALUES
            ('doc-dispatch', 'Dispatch Approval Reading Path', 'docs/dispatch.md', 2, 'doc-hash',
             'dispatch approval evidence reading path');

        CREATE TABLE symbol_text (
            stable_symbol_id VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            file_path VARCHAR,
            symbol_kind VARCHAR,
            doc_text VARCHAR,
            embedding FLOAT[768]
        );
        INSERT INTO symbol_text (
            stable_symbol_id, entity_name, qualified_name, file_path, symbol_kind, doc_text
        ) VALUES
            ('sym-dispatch', 'dispatch_plan', 'fixture::dispatch_plan',
             'src/dispatch.rs', 'function', 'dispatch approval evidence entry point'),
            ('sym-review', 'review_approval', 'fixture::review_approval',
             'src/review.rs', 'function', 'dispatch approval evidence review path');

        CREATE TABLE v_symbol_scorecard (
            stable_symbol_id VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            file_path VARCHAR,
            pagerank DOUBLE,
            in_degree BIGINT,
            out_degree BIGINT,
            callers BIGINT,
            importers BIGINT,
            inbound_total BIGINT,
            churn_90d BIGINT,
            last_touched TIMESTAMP,
            blast_radius_score DOUBLE,
            posture VARCHAR
        );
        INSERT INTO v_symbol_scorecard VALUES
            ('sym-dispatch', 'dispatch_plan', 'fixture::dispatch_plan', 'function', 'src/dispatch.rs',
             0.42, 7, 3, 11, 2, 13, 9, TIMESTAMP '2026-06-17 12:00:00', 0.91, 'load-bearing wall'),
            ('sym-review', 'review_approval', 'fixture::review_approval', 'function', 'src/review.rs',
             0.21, 2, 1, 3, 0, 3, 1, TIMESTAMP '2026-06-16 09:30:00', 0.33, 'stable');

        CREATE TABLE v_symbol_inbound (
            stable_symbol_id VARCHAR,
            callers BIGINT
        );
        INSERT INTO v_symbol_inbound VALUES
            ('sym-dispatch', 11),
            ('sym-review', 3);
        "#,
    )
    .expect("create kcp2 candidate fixture schema");
    if include_graph_reasoning_views {
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                stable_symbol_id VARCHAR,
                node_id BIGINT,
                file_path VARCHAR,
                entity_name VARCHAR,
                qualified_name VARCHAR,
                symbol_kind VARCHAR
            );
            INSERT INTO nodes VALUES
                ('sym-dispatch', 1, 'src/dispatch.rs', 'dispatch_plan', 'fixture::dispatch_plan', 'function'),
                ('sym-review', 2, 'src/review.rs', 'review_approval', 'fixture::review_approval', 'function');

            CREATE TABLE edges (
                source_stable_id VARCHAR,
                target_stable_id VARCHAR,
                src_id BIGINT,
                dst_id BIGINT,
                target_label VARCHAR,
                relation VARCHAR,
                confidence VARCHAR,
                confidence_score FLOAT,
                edge_kind VARCHAR,
                bind_method VARCHAR
            );
            INSERT INTO edges VALUES
                ('sym-dispatch', 'sym-review', 1, 2, 'review_approval', 'calls', 'syntax_exact', 1.0, 'calls', 'singleton');

            CREATE TABLE v_symbol_component (
                stable_symbol_id VARCHAR,
                component_id BIGINT,
                component_size BIGINT
            );
            INSERT INTO v_symbol_component VALUES
                ('sym-dispatch', 10, 2),
                ('sym-review', 10, 2);

            CREATE TABLE v_symbol_community (
                stable_symbol_id VARCHAR,
                community_id BIGINT
            );
            INSERT INTO v_symbol_community VALUES
                ('sym-dispatch', 20),
                ('sym-review', 20);

            CREATE TABLE v_graph_metrics (
                calls_edges BIGINT,
                connected_nodes BIGINT,
                components BIGINT,
                largest_component BIGINT,
                communities BIGINT,
                density DOUBLE
            );
            INSERT INTO v_graph_metrics VALUES (1, 2, 1, 2, 1, 0.5);
            "#,
        )
        .expect("create kcp2 graph reasoning fixture schema");
    }
    conn.execute_batch(
        r#"
        PRAGMA create_fts_index('main.sections_search', 'stable_symbol_id', 'body_text', overwrite=1, stemmer='porter');
        PRAGMA create_fts_index('main.symbol_text', 'stable_symbol_id', 'doc_text', overwrite=1);
        "#,
    )
    .expect("create kcp2 fixture fts indexes");
    let macro_sql = context_candidate_macro_sql();
    conn.execute_batch(&macro_sql)
        .expect("define kcp2 fixture context search macro");
    drop(conn);
    (temp_dir, repo)
}

fn kcp2_overlay_fixture_repo(force_delta_failure: bool) -> OverlayPackFixture {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let repo = temp_dir.path().join("repo");
    write_minimal_graph_fixture(
        &repo,
        r#"
pub fn dispatch_approval_evidence() -> &'static str {
"base"
}
"#,
    );
    fs::create_dir_all(repo.join(".spur")).expect("create .spur");

    let facts = build_facts(&repo, None).expect("build base graph facts").0;
    let artifact = artifact_from_facts(&facts, &repo).expect("build base graph artifact");
    let base_hash = artifact.graph_content_hash.clone();
    write_graph_artifact_for_test(&repo, &artifact);
    commit_fixture(&repo);

    let artifact_dir = repo.join(".spur/graph/test-artifact.parquet");
    let db_path = repo.join(".spur").join("analyst.duckdb");
    seed_overlay_pack_analyst_db(&db_path, &artifact_dir, &base_hash);

    fs::write(
        repo.join("src/lib.rs"),
        r#"
pub fn dispatch_approval_evidence() -> &'static str {
"dirty"
}
"#,
    )
    .expect("dirty fixture source");

    if force_delta_failure {
        fs::write(repo.join(".spur/analyst-overlays"), b"not a directory")
            .expect("force delta output path failure");
    }

    OverlayPackFixture {
        _temp_dir: temp_dir,
        repo,
        base_hash,
    }
}

fn seed_overlay_pack_analyst_db(db_path: &Path, artifact_dir: &Path, graph_hash: &str) {
    let conn = Connection::open(db_path).expect("open overlay pack fixture db");
    configure_fixture_extension_directory(&conn);
    conn.execute_batch("INSTALL fts; LOAD fts; INSTALL icu; LOAD icu;")
        .expect("load overlay pack fixture extensions");
    let artifact_dir = sql_escape_path(artifact_dir);
    let graph_hash = sql_escape_literal(graph_hash);
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('{graph_hash}');

        CREATE OR REPLACE TABLE node_dense_id_map AS
        WITH referenced_ids AS (
          SELECT stable_symbol_id FROM read_parquet('{artifact_dir}/nodes.parquet')
          UNION
          SELECT source_stable_id AS stable_symbol_id FROM read_parquet('{artifact_dir}/edges.parquet')
          UNION
          SELECT target_stable_id FROM read_parquet('{artifact_dir}/edges.parquet')
          UNION
          SELECT source_stable_id FROM read_parquet('{artifact_dir}/edges_by_dst.parquet')
          UNION
          SELECT target_stable_id FROM read_parquet('{artifact_dir}/edges_by_dst.parquet')
          UNION
          SELECT source_stable_id FROM read_parquet('{artifact_dir}/edges_unresolved.parquet')
        )
        SELECT
          stable_symbol_id,
          ROW_NUMBER() OVER (ORDER BY stable_symbol_id) AS dense_id
        FROM (
          SELECT DISTINCT stable_symbol_id
          FROM referenced_ids
          WHERE stable_symbol_id IS NOT NULL
        );

        CREATE OR REPLACE VIEW nodes AS
        SELECT n.* REPLACE (m.dense_id AS node_id)
        FROM read_parquet('{artifact_dir}/nodes.parquet') n
        JOIN node_dense_id_map m USING (stable_symbol_id);

        CREATE OR REPLACE VIEW edges AS
        SELECT e.* REPLACE (
          s.dense_id AS src_id,
          d.dense_id AS dst_id
        )
        FROM read_parquet('{artifact_dir}/edges.parquet') e
        JOIN node_dense_id_map s ON s.stable_symbol_id = e.source_stable_id
        JOIN node_dense_id_map d ON d.stable_symbol_id = e.target_stable_id;

        CREATE OR REPLACE VIEW edges_by_dst AS
        SELECT e.* REPLACE (
          s.dense_id AS src_id,
          d.dense_id AS dst_id
        )
        FROM read_parquet('{artifact_dir}/edges_by_dst.parquet') e
        JOIN node_dense_id_map s ON s.stable_symbol_id = e.source_stable_id
        JOIN node_dense_id_map d ON d.stable_symbol_id = e.target_stable_id;

        CREATE OR REPLACE VIEW edges_unresolved AS
        SELECT e.* REPLACE (s.dense_id AS src_id)
        FROM read_parquet('{artifact_dir}/edges_unresolved.parquet') e
        JOIN node_dense_id_map s ON s.stable_symbol_id = e.source_stable_id;

        CREATE OR REPLACE VIEW files AS
        SELECT *
        FROM read_parquet('{artifact_dir}/files.parquet');

        CREATE OR REPLACE VIEW file_manifests AS
        SELECT *
        FROM read_parquet('{artifact_dir}/file_manifests.parquet');

        CREATE OR REPLACE VIEW tombstones AS
        SELECT *
        FROM read_parquet('{artifact_dir}/tombstones.parquet');

        CREATE TABLE sections_search (
            stable_symbol_id VARCHAR,
            qualified_name VARCHAR,
            file_path VARCHAR,
            heading_level INTEGER,
            content_hash VARCHAR,
            body_text VARCHAR,
            embedding FLOAT[768]
        );

        CREATE TABLE symbol_text AS
        SELECT stable_symbol_id,
               entity_name,
               qualified_name,
               file_path,
               symbol_kind,
               entity_name || ' dispatch approval evidence' AS doc_text,
               CAST(NULL AS FLOAT[768]) AS embedding
        FROM nodes
        WHERE symbol_kind = 'function';

        CREATE TABLE v_symbol_scorecard AS
        SELECT stable_symbol_id,
               entity_name,
               qualified_name,
               symbol_kind,
               file_path,
               0.42::DOUBLE AS pagerank,
               0::BIGINT AS in_degree,
               0::BIGINT AS out_degree,
               0::BIGINT AS callers,
               0::BIGINT AS importers,
               0::BIGINT AS inbound_total,
               0::BIGINT AS churn_90d,
               NULL::TIMESTAMP AS last_touched,
               0.0::DOUBLE AS blast_radius_score,
               'fixture' AS posture
        FROM symbol_text;

        CREATE TABLE v_symbol_inbound AS
        SELECT stable_symbol_id, 0::BIGINT AS callers
        FROM symbol_text;
        "#
    ))
    .expect("create overlay pack fixture schema");
    conn.execute_batch(
        r#"
        PRAGMA create_fts_index('main.sections_search', 'stable_symbol_id', 'body_text', overwrite=1, stemmer='porter');
        PRAGMA create_fts_index('main.symbol_text', 'stable_symbol_id', 'doc_text', overwrite=1);
        "#,
    )
    .expect("create overlay pack fixture fts indexes");
    let macro_sql = context_candidate_macro_sql();
    conn.execute_batch(&macro_sql)
        .expect("define overlay pack fixture context search macro");
    drop(conn);
}

fn context_candidate_macro_sql() -> String {
    INIT_SEARCH_SQL
        .split("CREATE OR REPLACE MACRO search_context_candidates(q, requested_scope, intent) AS TABLE")
        .nth(1)
        .and_then(|rest| rest.split("-- Graph-augmented:").next())
        .map(|body| {
            let start = "CREATE OR REPLACE MACRO search_context_candidates(q, requested_scope, intent) AS TABLE";
            format!("{start}{body}")
        })
        .expect("context candidate macro should be present in init_search.sql")
}

fn context_candidate_macro_sql_with_artifact_dir(artifact_dir: &Path) -> String {
    context_candidate_macro_sql().replace(
        "__SPUR_GRAPH_ARTIFACT_DIR__",
        &sql_escape_path(artifact_dir),
    )
}

fn semantic_query_vec() -> Vec<f32> {
    let mut query_vec = vec![0.0; EMBEDDING_VECTOR_DIMENSIONS];
    query_vec[0] = 1.0;
    query_vec
}

fn format_query_vec_sql(query_vec: &[f32]) -> String {
    let values = query_vec
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]::FLOAT[{EMBEDDING_VECTOR_DIMENSIONS}]")
}

fn seed_section_vectors(
    conn: &Connection,
    semantic_rows: &[(&str, &[f32])],
    lexical_rows: &[(&str, &[f32])],
) {
    if semantic_rows.is_empty() && lexical_rows.is_empty() {
        return;
    }
    let overrides = semantic_rows
        .iter()
        .chain(lexical_rows.iter())
        .map(|(file_path, vector)| {
            format!(
                "('{}', {})",
                file_path.replace('\'', "''"),
                format_query_vec_sql(vector)
            )
        })
        .collect::<Vec<_>>();
    let sql = format!(
        r#"
        UPDATE sections_search
        SET embedding = o.vector
        FROM (
            SELECT col0 AS file_path, col1 AS vector
            FROM (VALUES {})
        ) AS o
        WHERE sections_search.file_path = o.file_path
           OR sections_search.stable_symbol_id = o.file_path;
        "#,
        overrides.join(",\n                  ")
    );
    conn.execute_batch(&sql)
        .expect("seed fixture section vectors");
}

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().expect("env lock")
}

async fn async_env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ASYNC_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

#[cfg(feature = "embed")]
struct EmbedQueryDisableGuard {
    previous: bool,
}

#[cfg(feature = "embed")]
impl Drop for EmbedQueryDisableGuard {
    fn drop(&mut self) {
        crate::embedding::set_embed_query_disabled_for_test(self.previous);
    }
}

#[cfg(feature = "embed")]
fn disable_embed_query_for_test() -> EmbedQueryDisableGuard {
    let previous = crate::embedding::set_embed_query_disabled_for_test(true);
    EmbedQueryDisableGuard { previous }
}

#[cfg(not(feature = "embed"))]
fn disable_embed_query_for_test() {}

fn parse_vector_json_to_f32(raw: &str) -> Vec<f32> {
    serde_json::from_str::<Vec<f64>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|value| value as f32)
        .collect()
}

fn build_hybrid_confidence_fixture() -> HybridConfidenceFixture {
    let _lock = env_lock();

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let root = temp_dir.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::create_dir_all(root.join("docs")).expect("create docs dir");
    fs::write(
        root.join("src/hybrid.rs"),
        r#"
pub fn ranking_beacon_router() {
// Ranking beacon: this symbol intentionally repeats the target query phrase.
let ranking_beacon = "ranking beacon ranking beacon ranking beacon";
println!("{ranking_beacon}");
}

pub fn lexical_signal_anchor() {
println!("lexical fallback utility");
}
"#,
    )
    .expect("write strong hybrid code");
    fs::write(
        root.join("docs/strong_hybrid.md"),
        "# Strong Hybrid\n\nranking beacon ranking beacon ranking beacon.\n",
    )
    .expect("write strong hybrid doc");
    fs::write(
        root.join("docs/lexical_hybrid.md"),
        "# Lexical Rival\n\nranking beacon appears often ranking beacon.\n",
    )
    .expect("write lexical rival doc");
    fs::write(
        root.join("docs/weak_hybrid.md"),
        "# Weak Only\n\nprivate lexical-only weakness signal.\n",
    )
    .expect("write weak-only doc");

    let facts = build_facts(&root, None).expect("build fixture facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("build fixture artifact");
    let artifact_dir = temp_dir.path().join("artifact");
    write_sections_dataset(&artifact, &root, &artifact_dir).expect("write Lance sidecar");

    let db_path = temp_dir.path().join("analyst.duckdb");
    let conn = Connection::open(&db_path).expect("open fixture db");
    configure_fixture_extension_directory(&conn);
    conn.execute_batch("INSTALL fts; LOAD fts; INSTALL icu; LOAD icu;")
        .expect("load fixture extensions");
    let sections_parquet = sql_escape_path(&artifact_dir.join(SECTIONS_PARQUET));
    let symbols_parquet = sql_escape_path(&artifact_dir.join(CODE_SYMBOLS_PARQUET));
    let mut symbol_row_stmt = conn
        .prepare(&format!(
            "
        SELECT stable_symbol_id
        FROM read_parquet('{symbols_parquet}')
        WHERE file_path = 'src/hybrid.rs'
        ORDER BY stable_symbol_id
        LIMIT 1
        "
        ))
        .expect("query code symbol id");
    let strong_symbol_id: String = symbol_row_stmt
        .query_row([], |row| row.get(0))
        .expect("query strong symbol id");
    let mut symbol_vec_stmt = conn
        .prepare(&format!(
            "SELECT to_json(vector) FROM read_parquet('{symbols_parquet}') WHERE stable_symbol_id = ? LIMIT 1"
        ))
        .expect("query strong symbol vector");
    let symbol_vector_json = symbol_vec_stmt
        .query_row([&strong_symbol_id], |row| {
            row.get::<usize, Option<String>>(0)
        })
        .expect("query code symbol vector");
    let query_vec = symbol_vector_json
        .and_then(|value| (!value.is_empty()).then_some(value))
        .map(|value| parse_vector_json_to_f32(&value))
        .filter(|query_vec| query_vec.len() == EMBEDDING_VECTOR_DIMENSIONS)
        .unwrap_or_else(semantic_query_vec);
    conn.execute_batch(
        &r#"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('hybrid-fixture-hash');

        CREATE TABLE sections_search AS
        SELECT stable_symbol_id, qualified_name, file_path, heading_level, content_hash, body_text,
               CAST(NULL AS FLOAT[768]) AS embedding
        FROM read_parquet('__SECTIONS_PARQUET__');

        CREATE TABLE symbol_text AS
        SELECT stable_symbol_id,
               entity_name,
               qualified_name,
               file_path,
               symbol_kind,
               embed_text AS doc_text,
               vector::FLOAT[768] AS embedding
        FROM read_parquet('__SYMBOLS_PARQUET__');

        CREATE TABLE v_symbol_scorecard AS
        SELECT stable_symbol_id,
               entity_name,
               file_path,
               symbol_kind,
               0.01 AS pagerank,
               3::BIGINT AS churn_90d,
               'stable' AS posture,
               1::BIGINT AS component_size,
               2::BIGINT AS callers
        FROM symbol_text;

        CREATE TABLE v_symbol_inbound AS
        SELECT stable_symbol_id, 1::BIGINT AS callers
        FROM symbol_text;
        "#
        .replace("__SECTIONS_PARQUET__", &sections_parquet)
        .replace("__SYMBOLS_PARQUET__", &symbols_parquet),
    )
    .expect("create fixture schema");
    conn.execute_batch(
        r#"
        PRAGMA create_fts_index('main.sections_search', 'stable_symbol_id', 'body_text', overwrite=1, stemmer='porter');
        PRAGMA create_fts_index('main.symbol_text', 'stable_symbol_id', 'doc_text', overwrite=1);
        "#,
    )
    .expect("create fixture fts indexes");
    seed_section_vectors(
        &conn,
        &[("docs/strong_hybrid.md", query_vec.as_slice())],
        &[],
    );
    let macro_sql = context_candidate_macro_sql_with_artifact_dir(&artifact_dir);
    conn.execute_batch(&macro_sql)
        .expect("define search context macro");
    drop(conn);

    HybridConfidenceFixture {
        _temp_dir: temp_dir,
        db_path,
        query_vec,
    }
}

fn configure_fixture_extension_directory(conn: &Connection) {
    if let Ok(directory) = std::env::var("SPUR_DUCKDB_EXTENSION_DIR") {
        let escaped = directory.replace('\'', "''");
        conn.execute_batch(&format!("SET extension_directory = '{escaped}'"))
            .expect("set isolated fixture extension directory");
    }
}

#[test]
fn pack_response_helpers_are_split_into_pack_modules() {
    let pack_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pack");
    for module in [
        "response.rs",
        "evidence.rs",
        "caveats.rs",
        "next_tools.rs",
        "staleness.rs",
    ] {
        assert!(
            pack_dir.join(module).exists(),
            "missing pack response module {module}"
        );
    }
}

#[test]
fn recommended_next_tools_are_intent_adaptive() {
    let primary = vec![json!({
        "stable_symbol_id": "graph://symbol/sym-1",
        "file": "crates/spur-mcp/src/lib.rs"
    })];
    let docs = vec![json!({
        "kind": "doc",
        "stable_symbol_id": "doc-1",
        "file": "docs/context.md"
    })];

    let debug_tools = recommended_next_tools(KnowledgeIntent::Debug, &primary, &[]);
    assert_eq!(
        debug_tools
            .iter()
            .map(|tool| tool["tool"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        vec!["code_read_symbol", "code_symbol_history", "code_subgraph"]
    );
    assert_eq!(debug_tools[2]["radius"], 2);

    let review_tools = recommended_next_tools(KnowledgeIntent::Review, &primary, &[]);
    assert_eq!(
        review_tools
            .iter()
            .map(|tool| tool["tool"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        vec!["code_read_symbol", "code_callers"]
    );

    let plan_tools = recommended_next_tools(KnowledgeIntent::Plan, &primary, &docs);
    assert_eq!(
        plan_tools
            .iter()
            .map(|tool| tool["tool"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        vec!["code_read_symbol", "doc_navigate", "code_file_symbols"]
    );
    assert_eq!(plan_tools[0]["stable_symbol_id"], "doc-1");
    assert_eq!(plan_tools[1]["root"], "doc-1");
    assert_eq!(plan_tools[2]["file"], "crates/spur-mcp/src/lib.rs");

    let fallback = recommended_next_tools(KnowledgeIntent::Debug, &[], &[]);
    assert_eq!(fallback[0]["tool"], "code_semantic_search");
}

#[test]
fn code_next_tools_are_intent_adaptive() {
    let tools = |intent| {
        code_next_tools(intent)
            .into_iter()
            .map(|tool| tool["tool"].as_str().expect("tool name").to_owned())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        tools(KnowledgeIntent::Debug),
        vec!["code_read_symbol", "code_symbol_history"]
    );
    assert_eq!(
        tools(KnowledgeIntent::Review),
        vec!["code_read_symbol", "code_callers"]
    );
    assert_eq!(
        tools(KnowledgeIntent::Plan),
        vec!["code_read_symbol", "code_file_symbols"]
    );
    assert_eq!(tools(KnowledgeIntent::Explain), vec!["code_read_symbol"]);
}

#[test]
fn doc_next_tools_prefer_code_read_symbol_and_omit_tree_hop_for_leaves() {
    let with_unknown_children = doc_next_tools("doc-1", None);
    assert_eq!(
        with_unknown_children
            .iter()
            .map(|tool| tool["tool"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        vec!["code_read_symbol", "doc_navigate"]
    );
    assert_eq!(with_unknown_children[0]["stable_symbol_id"], "doc-1");
    assert_eq!(with_unknown_children[1]["root"], "doc-1");

    let leaf = doc_next_tools("doc-leaf", Some(0));
    assert_eq!(leaf.len(), 1);
    assert_eq!(leaf[0]["tool"], "code_read_symbol");
    assert_eq!(leaf[0]["stable_symbol_id"], "doc-leaf");

    let parent = doc_next_tools("doc-parent", Some(3));
    assert_eq!(
        parent
            .iter()
            .map(|tool| tool["tool"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        vec!["code_read_symbol", "doc_navigate"]
    );
}

#[tokio::test]
async fn knowledge_context_pack_missing_analyst_db_returns_structured_unavailable() {
    let _lock = async_env_lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".spur")).expect("create .spur");

    let result = spur_graph::mcp::with_worktree_root_for_request(repo.clone(), async {
        run_knowledge_context_pack(&json!({ "query": "semantic search" })).await
    })
    .await
    .expect("structured unavailable response");

    assert_eq!(result["query"], "semantic search");
    assert_eq!(result["intent"], "explain");
    assert_eq!(result["scope"], "all");
    assert_eq!(result["answerable"], false);
    assert_eq!(result["confidence"], "low");
    assert_eq!(result["graph_content_hash"], Value::Null);
    assert_eq!(result["staleness"]["available"], false);
    assert_eq!(result["error"]["code"], "analyst_unavailable");
    assert!(result["error"]["message"]
        .as_str()
        .expect("error message")
        .contains(".spur/analyst.duckdb"));
}

#[tokio::test]
async fn knowledge_context_pack_2_missing_analyst_db_respects_compact_format() {
    let _lock = async_env_lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".spur")).expect("create .spur");

    let result = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        run_knowledge_context_pack_2(&json!({ "query": "semantic search" })).await
    })
    .await
    .expect("structured unavailable response");

    assert_eq!(result["staleness"]["available"], false);
    assert_eq!(result["error"]["code"], "analyst_unavailable");
    assert!(result.get("risk_scorecard").is_none());
    assert!(result.get("community_context").is_none());
    assert!(result.get("temporal_context").is_none());
    assert_eq!(result["caveats"][0]["code"], "analyst_unavailable");
}

#[test]
fn path_budget_plan_caps_targets_without_shrinking_per_target_limit() {
    const MAX_PATHS: usize = 4;
    let plan = path_budget_plan(6, MAX_PATHS);

    assert_eq!(plan.target_cap, MAX_PATHS);
    assert_eq!(plan.per_target_max_paths, MAX_PATHS);

    let target_outcomes = [
        "no_path",
        "path_found",
        "no_path",
        "path_found",
        "path_found",
    ];
    let processed_limits = target_outcomes
        .iter()
        .take(plan.target_cap)
        .map(|_| plan.per_target_max_paths)
        .collect::<Vec<_>>();
    assert_eq!(
        processed_limits,
        vec![MAX_PATHS; MAX_PATHS],
        "target outcomes must not feed back into per-target path limits"
    );

    let smaller_target_set = path_budget_plan(2, MAX_PATHS);
    assert_eq!(smaller_target_set.target_cap, 2);
    assert_eq!(smaller_target_set.per_target_max_paths, MAX_PATHS);
}

#[test]
fn collect_graph_paths_keeps_full_per_target_limit_after_no_path() {
    let (_temp_dir, db_path) = analyst_db_with_path_budget_fixture();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "intent": "review",
        "graph_reasoning": {
            "paths": true,
            "max_path_hops": 2,
            "max_paths": 2
        }
    }))
    .expect("request");
    let mut sections = GraphReasoningSections::default();
    let code_symbol_ids = vec![
        "sym-source".to_owned(),
        "sym-disconnected".to_owned(),
        "sym-connected".to_owned(),
        "sym-late".to_owned(),
    ];

    collect_graph_paths(&db_path, &request, &code_symbol_ids, &mut sections);

    assert_eq!(
        sections.graph_paths.len(),
        2,
        "processed targets should be capped by max_paths"
    );
    assert_eq!(
        sections
            .graph_paths
            .iter()
            .map(|path| path["target_stable_id"].as_str().expect("target id"))
            .collect::<Vec<_>>(),
        vec!["sym-disconnected", "sym-connected"]
    );
    assert_eq!(sections.graph_paths[0]["status"], "no_path");
    assert_eq!(sections.graph_paths[1]["status"], "path_found");
    assert_eq!(
        sections
            .graph_paths
            .iter()
            .map(|path| path["max_paths"].as_u64().expect("max paths"))
            .collect::<Vec<_>>(),
        vec![2, 2],
        "a disconnected target must not shrink the later target's path limit"
    );
}

#[test]
fn collect_graph_paths_dedupes_repeated_no_path_caveats_for_source() {
    let (_temp_dir, db_path) = analyst_db_with_path_budget_fixture();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "intent": "review",
        "graph_reasoning": {
            "paths": true,
            "max_path_hops": 2,
            "max_paths": 3
        }
    }))
    .expect("request");
    let mut sections = GraphReasoningSections::default();
    let code_symbol_ids = vec![
        "sym-source".to_owned(),
        "sym-disconnected-one".to_owned(),
        "sym-disconnected-two".to_owned(),
    ];

    collect_graph_paths(&db_path, &request, &code_symbol_ids, &mut sections);

    let graph_path_caveats = sections
        .caveats
        .iter()
        .filter(|caveat| caveat["code"] == "graph_path_unavailable")
        .collect::<Vec<_>>();
    assert_eq!(
        graph_path_caveats.len(),
        1,
        "identical no_path caveats for one source should collapse"
    );
    assert_eq!(
        graph_path_caveats[0]["message"],
        "no undirected path found within 2 hops"
    );
    assert_eq!(graph_path_caveats[0]["stable_symbol_id"], "sym-source");
}

#[tokio::test]
async fn knowledge_context_pack_v1_response_omits_v2_sections() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search"
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![candidate(Some("sym-one"), "symbol_one", 7.0)],
    };

    let pack = pack_query_result(&request, result).await;

    assert!(pack.get("graph_paths").is_none());
    assert!(pack.get("risk_scorecard").is_none());
    assert!(pack.get("community_context").is_none());
    assert!(pack.get("temporal_context").is_none());
    assert!(pack.get("caveats").is_none());
}

#[tokio::test]
async fn knowledge_context_pack_uses_single_connection_for_candidate_queries() {
    let _lock = async_env_lock().await;
    let _embed_guard = disable_embed_query_for_test();
    let (_temp_dir, repo) = kcp2_fixture_repo(true);
    let db_path = repo.join(".spur").join("analyst.duckdb");
    crate::db::connection::reset_analyst_connection_open_count_for_test(&db_path);

    let pack = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        run_knowledge_context_pack(&json!({
            "query": "dispatch approval evidence",
            "intent": "change",
            "scope": "all",
            "limit": 5
        }))
        .await
    })
    .await
    .expect("v1 fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert_eq!(
        crate::db::connection::analyst_connection_open_count_for_test(&db_path),
        1,
        "v1 candidate and graph retrieval should share one analyst connection"
    );
}

#[tokio::test]
async fn knowledge_context_pack_2_uses_single_connection_for_pack_request() {
    let _lock = async_env_lock().await;
    let _embed_guard = disable_embed_query_for_test();
    let (_temp_dir, repo) = kcp2_fixture_repo(true);
    let db_path = repo.join(".spur").join("analyst.duckdb");
    crate::db::connection::reset_analyst_connection_open_count_for_test(&db_path);

    let pack = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        run_knowledge_context_pack_2(&json!({
            "query": "dispatch approval evidence",
            "intent": "review",
            "scope": "all",
            "limit": 5,
            "graph_reasoning": {
                "paths": true,
                "communities": true,
                "risk": true,
                "max_path_hops": 2,
                "max_paths": 1
            }
        }))
        .await
    })
    .await
    .expect("kcp2 fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert_eq!(
        crate::db::connection::analyst_connection_open_count_for_test(&db_path),
        1,
        "v2 candidates, paths, and symbol enrichment should share one analyst connection"
    );
}

#[test]
fn open_pack_connection_reuses_one_connection_for_concurrent_calls_to_same_db_path() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    crate::db::connection::reset_analyst_connection_open_count_for_test(&db_path);

    let db_path = Arc::new(db_path);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let db_path = Arc::clone(&db_path);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let conn = open_pack_connection(&db_path, "knowledge_context_pack")
                    .expect("open pooled pack connection");
                let conn = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let hash: String = conn
                    .query_row("SELECT graph_content_hash FROM _meta", [], |row| row.get(0))
                    .expect("read fixture hash");
                assert_eq!(hash, "fixture-hash");
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("pack thread");
    }

    assert_eq!(
        crate::db::connection::analyst_connection_open_count_for_test(&db_path),
        1,
        "pack service should pool one read-only connection per analyst DB path"
    );
}

#[tokio::test]
async fn knowledge_context_pack_2_preserves_v1_fields_and_adds_empty_v2_sections_when_disabled() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "change",
        "scope": "code",
        "limit": 4,
        "include_tests": false,
        "max_symbol_bodies": 0,
        "graph_reasoning": {
            "paths": false,
            "communities": false,
            "risk": false
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-one"), "symbol_one", 7.5),
            doc_candidate(Some("doc-one"), "Context Doc", 3.0),
        ],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("fixture-hash".into()),
            response_file_oids_match: Some(true),
            impacts: Vec::new(),
        },
        &db_path,
    )
    .await;

    assert_eq!(pack["query"], "semantic search");
    assert_eq!(pack["intent"], "change");
    assert_eq!(pack["scope"], "code");
    assert_eq!(pack["graph_content_hash"], "fixture-hash");
    assert_eq!(
        pack["staleness"]["analyst_graph_content_hash"],
        "fixture-hash"
    );
    assert_eq!(pack["candidates"]["total"], 2);
    assert_eq!(pack["candidates"]["returned_primary"], 1);
    assert_eq!(pack["candidates"]["returned_supporting_docs"], 1);
    assert_eq!(
        pack["primary_evidence"][0]["stable_symbol_id"],
        "graph://symbol/sym-one"
    );
    assert_eq!(pack["supporting_docs"][0]["stable_symbol_id"], "doc-one");
    assert_eq!(
        pack["recommended_next_tools"][0]["selector"],
        "graph://symbol/sym-one"
    );
    assert_eq!(pack["graph_paths"], json!([]));
    assert_eq!(pack["risk_scorecard"], json!([]));
    assert_eq!(pack["community_context"], json!([]));
    assert_eq!(pack["temporal_context"], json!([]));
    assert_eq!(pack["caveats"], json!([]));
    assert_eq!(pack["staleness"]["delta_applied"], false);
    assert_eq!(pack["staleness"]["algo_as_of"], "fixture-hash");
}

#[tokio::test]
async fn knowledge_context_pack_2_staleness_reports_overlay_session_state() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "graph_reasoning": {
            "paths": false,
            "communities": false,
            "risk": false
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![candidate(Some("sym-one"), "symbol_one", 7.5)],
    };
    let exact_context = ExactGraphContext {
        graph_content_hash: Some("fixture-hash".into()),
        response_file_oids_match: Some(true),
        impacts: Vec::new(),
    };

    let delta_pack = pack_query_result_v2_with_graph_sections_and_staleness(
        &request,
        result.clone(),
        exact_context.clone(),
        GraphReasoningSections::default(),
        PackStaleness {
            delta_applied: true,
            algo_as_of: Some("fixture-hash".to_owned()),
        },
    )
    .await;

    assert_eq!(delta_pack["staleness"]["delta_applied"], true);
    assert_eq!(delta_pack["staleness"]["algo_as_of"], "fixture-hash");

    let degraded_pack = pack_query_result_v2_with_graph_sections_and_staleness(
        &request,
        result,
        exact_context,
        GraphReasoningSections::default(),
        PackStaleness {
            delta_applied: false,
            algo_as_of: Some("fixture-hash".to_owned()),
        },
    )
    .await;

    assert_eq!(degraded_pack["staleness"]["delta_applied"], false);
    assert_eq!(degraded_pack["staleness"]["algo_as_of"], "fixture-hash");
}

#[tokio::test]
async fn knowledge_context_pack_2_reports_overlay_staleness_end_to_end() {
    let _lock = async_env_lock().await;
    let _embed_guard = disable_embed_query_for_test();

    let happy = kcp2_overlay_fixture_repo(false);
    let happy_pack = spur_graph::mcp::with_worktree_root_for_request(happy.repo.clone(), async {
        run_knowledge_context_pack_2(&json!({
            "query": "dispatch approval evidence",
            "response_format": "full",
            "intent": "review",
            "scope": "code",
            "limit": 5,
            "graph_reasoning": {
                "paths": false,
                "communities": false,
                "risk": false
            }
        }))
        .await
    })
    .await
    .expect("happy overlay pack response");

    assert!(happy_pack.get("error").is_none(), "{happy_pack:#}");
    assert_eq!(happy_pack["staleness"]["available"], true);
    assert_eq!(happy_pack["staleness"]["delta_applied"], true);
    assert_eq!(happy_pack["staleness"]["algo_as_of"], happy.base_hash);

    let degraded = kcp2_overlay_fixture_repo(true);
    let degraded_pack =
        spur_graph::mcp::with_worktree_root_for_request(degraded.repo.clone(), async {
            run_knowledge_context_pack_2(&json!({
                "query": "dispatch approval evidence",
                "response_format": "full",
                "intent": "review",
                "scope": "code",
                "limit": 5,
                "graph_reasoning": {
                    "paths": false,
                    "communities": false,
                    "risk": false
                }
            }))
            .await
        })
        .await
        .expect("degraded overlay pack response");

    assert!(degraded_pack.get("error").is_none(), "{degraded_pack:#}");
    assert_eq!(degraded_pack["staleness"]["available"], true);
    assert_eq!(degraded_pack["staleness"]["delta_applied"], false);
    assert_eq!(degraded_pack["staleness"]["algo_as_of"], degraded.base_hash);
}

#[tokio::test]
async fn knowledge_context_pack_2_missing_graph_views_return_caveats_not_error() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "limit": 2,
        "graph_reasoning": {
            "paths": true,
            "communities": true,
            "risk": true,
            "max_path_hops": 2,
            "max_paths": 1
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-one"), "symbol_one", 8.0),
            candidate(Some("sym-two"), "symbol_two", 7.0),
        ],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext::default(),
        &db_path,
    )
    .await;

    assert!(pack.get("error").is_none(), "v2 graph failures are caveats");
    let caveat_codes = pack["caveats"]
        .as_array()
        .expect("caveats")
        .iter()
        .filter_map(|caveat| caveat["code"].as_str())
        .collect::<Vec<_>>();
    assert!(caveat_codes.contains(&"scorecard_unavailable"));
    assert!(caveat_codes.contains(&"community_unavailable"));
    assert!(caveat_codes.contains(&"graph_metrics_unavailable"));
    assert!(caveat_codes.contains(&"graph_path_unavailable"));
    assert_eq!(pack["risk_scorecard"][0]["status"], "unavailable");
    assert_eq!(pack["community_context"][0]["status"], "unavailable");
    assert_eq!(pack["graph_paths"][0]["status"], "unavailable");
    assert_eq!(pack["graph_paths"][0]["rows"][0]["status"], "unavailable");
}

#[tokio::test]
async fn knowledge_context_pack_2_returns_temporal_context_from_scorecard_when_available() {
    let (_temp_dir, db_path) = analyst_db_with_graph_reasoning_views();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "limit": 2,
        "graph_reasoning": {
            "paths": false,
            "communities": true,
            "risk": true
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-one"), "symbol_one", 8.0),
            candidate(Some("sym-two"), "symbol_two", 7.0),
        ],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext::default(),
        &db_path,
    )
    .await;

    assert_eq!(pack["risk_scorecard"][0]["status"], "available");
    assert_eq!(pack["risk_scorecard"][0]["churn_90d"], 9);
    assert_eq!(pack["community_context"][0]["status"], "available");
    assert_eq!(pack["community_context"][0]["component_id"], 10);
    assert_eq!(pack["temporal_context"][0]["stable_symbol_id"], "sym-one");
    assert_eq!(pack["temporal_context"][0]["churn_90d"], 9);
    assert!(pack["temporal_context"][0]["last_touched"]
        .as_str()
        .expect("last touched")
        .contains("2026-06-17"));
}

#[tokio::test]
async fn knowledge_context_pack_2_suppresses_graph_reasoning_when_analyst_hash_is_stale() {
    let (_temp_dir, db_path) = analyst_db_with_graph_reasoning_views();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "limit": 2,
        "graph_reasoning": {
            "paths": true,
            "communities": true,
            "risk": true,
            "max_path_hops": 2,
            "max_paths": 1
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-one"), "symbol_one", 8.0),
            candidate(Some("sym-two"), "symbol_two", 7.0),
        ],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("exact-graph-hash".into()),
            response_file_oids_match: Some(true),
            impacts: Vec::new(),
        },
        &db_path,
    )
    .await;

    assert_eq!(
        pack["staleness"]["analyst_graph_content_hash"],
        "fixture-hash"
    );
    assert_eq!(pack["staleness"]["exact_graph_hash"], "exact-graph-hash");
    assert_eq!(pack["staleness"]["analyst_matches_exact_graph"], false);
    assert_eq!(pack["graph_paths"], json!([]));
    assert_eq!(pack["risk_scorecard"], json!([]));
    assert_eq!(pack["community_context"], json!([]));
    assert_eq!(pack["temporal_context"], json!([]));
    assert!(pack["caveats"]
        .as_array()
        .expect("caveats")
        .iter()
        .any(|caveat| caveat["code"] == "analyst_graph_stale"));
}

#[tokio::test]
async fn knowledge_context_pack_2_staleness_uses_rebuilt_graph_hash() {
    let _lock = async_env_lock().await;
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    write_minimal_graph_fixture(
        worktree.path(),
        "pub fn stable_symbol() -> bool {\n    true\n}\n",
    );
    commit_fixture(worktree.path());

    let (facts, _file_counts) = build_facts(worktree.path(), None).expect("build facts");
    let stale_artifact =
        artifact_from_facts(&facts, worktree.path()).expect("build stale graph artifact");
    let stale_graph_hash = stale_artifact.graph_content_hash.clone();
    let stable_symbol_id = stale_artifact
        .symbols
        .iter()
        .find(|symbol| symbol.entity_name == "stable_symbol")
        .expect("stable symbol is indexed")
        .stable_symbol_id
        .clone();
    write_graph_artifact_for_test(worktree.path(), &stale_artifact);

    fs::write(
        worktree.path().join("src/lib.rs"),
        "pub fn stable_symbol() -> bool {\n    true\n}\n\npub fn live_symbol() -> bool {\n    stable_symbol()\n}\n",
    )
    .expect("dirty fixture source");

    let (_db_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "stable symbol",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "graph_reasoning": {
            "paths": false,
            "communities": false,
            "risk": false
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some(stale_graph_hash.clone()),
        candidates: vec![candidate(Some(&stable_symbol_id), "stable_symbol", 9.0)],
    };

    let exact_context =
        spur_graph::mcp::with_worktree_root_for_request(worktree.path().to_path_buf(), async {
            exact_graph_context_for_result(&request.base, &result).await
        })
        .await;
    let pack =
        pack_query_result_v2_with_graph_reasoning(&request, result, exact_context, &db_path).await;

    assert_eq!(
        pack["staleness"]["analyst_graph_content_hash"],
        stale_graph_hash
    );
    assert_ne!(
        pack["staleness"]["exact_graph_hash"], stale_graph_hash,
        "exact graph hash must come from the rebuilt live graph, not the stale loaded artifact",
    );
    assert_eq!(pack["staleness"]["exact_graph_verified"], true);
    assert_eq!(pack["staleness"]["analyst_matches_exact_graph"], false);
}

#[tokio::test]
async fn knowledge_context_pack_2_bounds_path_and_risk_output() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "semantic search",
        "response_format": "full",
        "intent": "review",
        "scope": "code",
        "limit": 3,
        "graph_reasoning": {
            "paths": true,
            "communities": false,
            "risk": true,
            "max_path_hops": 2,
            "max_paths": 1
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-one"), "symbol_one", 9.0),
            candidate(Some("sym-two"), "symbol_two", 8.0),
            candidate(Some("sym-three"), "symbol_three", 7.0),
            candidate(Some("sym-four"), "symbol_four", 6.0),
            candidate(Some("sym-five"), "symbol_five", 5.0),
        ],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext::default(),
        &db_path,
    )
    .await;

    assert!(
        pack["risk_scorecard"].as_array().expect("risk").len() <= 3,
        "risk rows should be bounded by the request limit"
    );
    let path_rows = pack["graph_paths"]
        .as_array()
        .expect("graph paths")
        .iter()
        .map(|path| path["rows"].as_array().map_or(0, Vec::len))
        .sum::<usize>();
    assert!(
        path_rows <= 1,
        "path rows should be bounded by graph_reasoning.max_paths"
    );
}

#[tokio::test]
async fn knowledge_context_pack_2_reads_fixture_db_end_to_end() {
    let _lock = async_env_lock().await;
    let _embed_guard = disable_embed_query_for_test();
    let (_temp_dir, repo) = kcp2_fixture_repo(true);

    let pack = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        run_knowledge_context_pack_2(&json!({
            "query": "dispatch approval evidence",
            "response_format": "full",
            "intent": "review",
            "scope": "all",
            "limit": 5,
            "graph_reasoning": {
                "paths": true,
                "communities": true,
                "risk": true,
                "max_path_hops": 2,
                "max_paths": 1
            }
        }))
        .await
    })
    .await
    .expect("kcp2 fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert_eq!(pack["query"], "dispatch approval evidence");
    assert_eq!(pack["graph_content_hash"], "kcp2-fixture-hash");
    assert_eq!(pack["answerable"], true);
    assert!(
        pack["primary_evidence"]
            .as_array()
            .expect("primary evidence")
            .iter()
            .any(|evidence| evidence["stable_symbol_id"] == "graph://symbol/sym-dispatch"),
        "primary_evidence should include the dispatch symbol: {pack:#}"
    );
    assert!(
        pack["supporting_docs"]
            .as_array()
            .expect("supporting docs")
            .iter()
            .any(|doc| doc["stable_symbol_id"] == "doc-dispatch"),
        "supporting_docs should include the fixture doc: {pack:#}"
    );
    assert_eq!(pack["graph_paths"][0]["source_stable_id"], "sym-dispatch");
    assert_eq!(pack["graph_paths"][0]["target_stable_id"], "sym-review");
    assert_eq!(pack["graph_paths"][0]["status"], "path_found");
    assert_eq!(pack["graph_paths"][0]["engine"], "recursive_sql");
    assert_eq!(pack["graph_paths"][0]["rows"][0]["relation"], "calls");
    assert_eq!(pack["risk_scorecard"][0]["status"], "available");
    assert_eq!(
        pack["risk_scorecard"][0]["stable_symbol_id"],
        "sym-dispatch"
    );
    assert_eq!(pack["risk_scorecard"][0]["churn_90d"], 9);
    assert_eq!(pack["community_context"][0]["status"], "available");
    assert_eq!(
        pack["community_context"][0]["stable_symbol_id"],
        "sym-dispatch"
    );
    assert_eq!(pack["community_context"][0]["component_id"], 10);
    assert_eq!(pack["community_context"][0]["community_id"], 20);
    assert_eq!(
        pack["recommended_next_tools"][0]["selector"],
        "graph://symbol/sym-dispatch"
    );
    assert!(
        pack["caveats"].as_array().expect("caveats").is_empty(),
        "complete fixture should not emit caveats: {pack:#}"
    );
}

#[tokio::test]
async fn knowledge_context_pack_2_missing_graph_views_keeps_candidates_and_returns_caveats() {
    let _lock = async_env_lock().await;
    let _embed_guard = disable_embed_query_for_test();
    let (_temp_dir, repo) = kcp2_fixture_repo(false);

    let pack = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        run_knowledge_context_pack_2(&json!({
            "query": "dispatch approval evidence",
            "response_format": "full",
            "intent": "review",
            "scope": "all",
            "limit": 5,
            "graph_reasoning": {
                "paths": true,
                "communities": true,
                "risk": true,
                "max_path_hops": 2,
                "max_paths": 1
            }
        }))
        .await
    })
    .await
    .expect("kcp2 missing-view fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert!(
        !pack["primary_evidence"]
            .as_array()
            .expect("primary evidence")
            .is_empty(),
        "missing graph views should not suppress retrieved candidates: {pack:#}"
    );
    assert!(
        !pack["recommended_next_tools"]
            .as_array()
            .expect("recommended next tools")
            .is_empty(),
        "candidate follow-up tools should still be present: {pack:#}"
    );
    assert_eq!(pack["risk_scorecard"][0]["status"], "available");
    assert_eq!(pack["community_context"][0]["status"], "unavailable");
    assert_eq!(pack["graph_paths"][0]["status"], "unavailable");
    let caveat_codes = pack["caveats"]
        .as_array()
        .expect("caveats")
        .iter()
        .filter_map(|caveat| caveat["code"].as_str())
        .collect::<Vec<_>>();
    assert!(caveat_codes.contains(&"community_unavailable"));
    assert!(caveat_codes.contains(&"graph_metrics_unavailable"));
    assert!(caveat_codes.contains(&"graph_path_unavailable"));
}

#[tokio::test]
async fn knowledge_context_pack_2_preserves_popular_sink_impact_boundary() {
    let (_temp_dir, db_path) = minimal_analyst_db_with_meta();
    let request = KnowledgeContextPackV2Request::parse(&json!({
        "query": "popular impact",
        "response_format": "full",
        "intent": "change",
        "scope": "code",
        "graph_reasoning": {
            "paths": false,
            "communities": false,
            "risk": false
        }
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![candidate(Some("sym-sink"), "sink_symbol", 9.0)],
    };

    let pack = pack_query_result_v2_with_graph_reasoning(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("fixture-hash".into()),
            response_file_oids_match: Some(true),
            impacts: vec![Some(SymbolImpactSummary {
                selector: "graph://symbol/sym-sink".into(),
                callers_count: POPULAR_SINK_CALLERS_THRESHOLD + 1,
                callees_count: 2,
                caller_neighbors: vec![json!({ "title": "caller_a" })],
                callee_neighbors: vec![json!({ "title": "callee_a" })],
            })],
        },
        &db_path,
    )
    .await;

    assert_eq!(pack["impact"]["popular_sink"], true);
    assert_eq!(
        pack["impact"]["caller_neighbors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        pack["impact"]["callee_neighbors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(pack["graph_paths"], json!([]));
    assert_eq!(pack["risk_scorecard"], json!([]));
    assert_eq!(pack["community_context"], json!([]));
}

#[test]
fn merge_graph_candidates_deduplicates_stable_symbols_by_higher_score() {
    let mut result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-dup"), "bm25 duplicate", 3.0),
            candidate(None, "bm25 no symbol", 2.0),
            candidate(Some("sym-bm25"), "bm25 unique", 5.0),
        ],
    };
    let graph_result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            candidate(Some("sym-dup"), "graph duplicate", 8.0),
            candidate(Some("sym-bm25"), "graph lower duplicate", 1.0),
            candidate(Some("sym-graph"), "graph unique", 4.0),
            candidate(None, "graph no symbol", 6.0),
        ],
    };

    merge_graph_candidates(&mut result, graph_result);

    let titles = result
        .candidates
        .iter()
        .map(|candidate| candidate.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        vec![
            "graph duplicate",
            "bm25 no symbol",
            "bm25 unique",
            "graph unique",
            "graph no symbol"
        ]
    );
}

#[tokio::test]
async fn knowledge_context_pack_explains_why_evidence_is_relevant() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search"
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![KnowledgeCandidate {
            kind: "code".into(),
            title: "query_context_candidates".into(),
            file_path: "crates/spur-analyst/src/lib.rs".into(),
            stable_symbol_id: Some("sym-1".into()),
            symbol_kind: Some("function".into()),
            score: 7.5,
            signal: Some("stable".into()),
            neighbor_kind: Some("primary".into()),
            edge_bind_method: None,
            grounding: "bm25-graph-expanded".into(),
        }],
    };

    let pack = pack_query_result(&request, result).await;
    let why_relevant = pack["primary_evidence"][0]["why_relevant"]
        .as_str()
        .expect("why relevant");

    assert!(why_relevant.starts_with("graph 7.5"));
    assert!(why_relevant.contains("stable"));
    assert!(why_relevant.contains("kind=function"));
    assert!(why_relevant.contains("grounding=bm25-graph-expanded"));
}

#[tokio::test]
async fn knowledge_context_pack_reports_high_confidence_for_strong_evidence_set() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search",
        "limit": 3
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            KnowledgeCandidate {
                kind: "code".into(),
                title: "top_symbol".into(),
                file_path: "crates/spur-mcp/src/lib.rs".into(),
                stable_symbol_id: Some("sym-top".into()),
                symbol_kind: Some("function".into()),
                score: 9.2,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "code".into(),
                title: "supporting_symbol".into(),
                file_path: "crates/spur-core/src/lib.rs".into(),
                stable_symbol_id: Some("sym-support".into()),
                symbol_kind: Some("function".into()),
                score: 4.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "doc".into(),
                title: "Knowledge Context API".into(),
                file_path: "docs/context.md".into(),
                stable_symbol_id: Some("doc-1".into()),
                symbol_kind: Some("section".into()),
                score: 3.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-doc".into(),
            },
        ],
    };

    let pack = pack_query_result(&request, result).await;

    assert_eq!(pack["confidence"], "high");
}

#[tokio::test]
async fn knowledge_context_pack_uses_lower_high_threshold_for_hybrid_evidence() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search",
        "limit": 3
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            KnowledgeCandidate {
                kind: "code".into(),
                title: "top_symbol".into(),
                file_path: "crates/spur-mcp/src/lib.rs".into(),
                stable_symbol_id: Some("sym-top".into()),
                symbol_kind: Some("function".into()),
                score: 1.1,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "hybrid-code".into(),
            },
            KnowledgeCandidate {
                kind: "code".into(),
                title: "supporting_symbol".into(),
                file_path: "crates/spur-core/src/lib.rs".into(),
                stable_symbol_id: Some("sym-support".into()),
                symbol_kind: Some("function".into()),
                score: 0.8,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "hybrid-code".into(),
            },
            KnowledgeCandidate {
                kind: "doc".into(),
                title: "Knowledge Context API".into(),
                file_path: "docs/context.md".into(),
                stable_symbol_id: Some("doc-1".into()),
                symbol_kind: Some("section".into()),
                score: 0.4,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "hybrid-doc".into(),
            },
        ],
    };

    let pack = pack_query_result(&request, result).await;

    assert_eq!(pack["confidence"], "high");
}

#[tokio::test]
async fn knowledge_context_pack_reports_low_confidence_for_weak_evidence_set() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search"
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            KnowledgeCandidate {
                kind: "code".into(),
                title: "weak_symbol".into(),
                file_path: "crates/spur-mcp/src/lib.rs".into(),
                stable_symbol_id: Some("sym-weak".into()),
                symbol_kind: Some("function".into()),
                score: 2.5,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "doc".into(),
                title: "Weak Context API".into(),
                file_path: "docs/context.md".into(),
                stable_symbol_id: Some("doc-weak".into()),
                symbol_kind: Some("section".into()),
                score: 2.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-doc".into(),
            },
        ],
    };

    let pack = pack_query_result(&request, result).await;

    assert_eq!(pack["confidence"], "low");
}

#[tokio::test]
async fn knowledge_context_pack_reports_candidate_totals() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search",
        "limit": 3
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            KnowledgeCandidate {
                kind: "code".into(),
                title: "code_symbol".into(),
                file_path: "crates/spur-mcp/src/lib.rs".into(),
                stable_symbol_id: Some("sym-code".into()),
                symbol_kind: Some("function".into()),
                score: 7.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "symbol".into(),
                title: "graph_symbol".into(),
                file_path: "crates/spur-graph/src/lib.rs".into(),
                stable_symbol_id: Some("sym-graph".into()),
                symbol_kind: Some("struct".into()),
                score: 6.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "doc".into(),
                title: "Knowledge Context API".into(),
                file_path: "docs/context.md".into(),
                stable_symbol_id: Some("doc-1".into()),
                symbol_kind: Some("section".into()),
                score: 5.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-doc".into(),
            },
        ],
    };

    let pack = pack_query_result(&request, result).await;

    assert_eq!(pack["candidates"]["total"], 3);
    assert_eq!(pack["candidates"]["returned_primary"], 2);
    assert_eq!(pack["candidates"]["returned_supporting_docs"], 1);
    assert_eq!(pack["candidates"]["total_code"], 2);
    assert_eq!(pack["candidates"]["total_docs"], 1);
}

#[tokio::test]
async fn knowledge_context_pack_returns_grounded_evidence_and_followups() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "semantic search",
        "intent": "change",
        "scope": "code",
        "limit": 4,
        "include_tests": false,
        "max_symbol_bodies": 1
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![
            KnowledgeCandidate {
                kind: "code".into(),
                title: "query_context_candidates".into(),
                file_path: "crates/spur-analyst/src/lib.rs".into(),
                stable_symbol_id: Some("sym-1".into()),
                symbol_kind: Some("function".into()),
                score: 7.5,
                signal: Some("stable".into()),
                neighbor_kind: Some("primary".into()),
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
            KnowledgeCandidate {
                kind: "doc".into(),
                title: "Knowledge Context API".into(),
                file_path: "docs/context.md".into(),
                stable_symbol_id: Some("doc-1".into()),
                symbol_kind: Some("section".into()),
                score: 3.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-doc".into(),
            },
            KnowledgeCandidate {
                kind: "code".into(),
                title: "test helper".into(),
                file_path: "crates/spur-mcp/tests/context_tests.rs".into(),
                stable_symbol_id: Some("test-sym".into()),
                symbol_kind: Some("function".into()),
                score: 1.0,
                signal: None,
                neighbor_kind: None,
                edge_bind_method: None,
                grounding: "bm25-code".into(),
            },
        ],
    };

    let pack = pack_query_result(&request, result).await;

    assert_eq!(pack["query"], "semantic search");
    assert_eq!(pack["intent"], "change");
    assert_eq!(pack["scope"], "code");
    assert_eq!(pack["answerable"], true);
    assert_eq!(pack["confidence"], "medium");
    assert_eq!(pack["graph_content_hash"], "fixture-hash");
    assert_eq!(pack["staleness"]["graph_hash_present"], true);
    assert_eq!(pack["primary_evidence"][0]["kind"], "symbol");
    assert_eq!(
        pack["primary_evidence"][0]["stable_symbol_id"],
        "graph://symbol/sym-1"
    );
    assert_eq!(pack["supporting_docs"][0]["kind"], "doc");
    assert_eq!(pack["supporting_docs"][0]["stable_symbol_id"], "doc-1");
    assert_eq!(
        pack["supporting_docs"][0]["next"][0]["tool"],
        "code_read_symbol"
    );
    assert_eq!(
        pack["supporting_docs"][0]["next"][0]["stable_symbol_id"],
        "doc-1"
    );
    assert_eq!(
        pack["supporting_docs"][0]["next"][1]["tool"],
        "doc_navigate"
    );
    assert_eq!(pack["supporting_docs"][0]["next"][1]["root"], "doc-1");
    assert_eq!(pack["recommended_next_tools"][0]["tool"], "code_callers");
    assert_eq!(
        pack["recommended_next_tools"][0]["selector"],
        "graph://symbol/sym-1"
    );
    assert_eq!(pack["impact"]["popular_sink"], Value::Null);
    assert_eq!(
        pack["primary_evidence"]
            .as_array()
            .expect("primary evidence")
            .len(),
        1,
        "include_tests=false should filter test evidence"
    );
}

#[tokio::test]
async fn knowledge_context_pack_includes_bounded_impact_for_top_code_evidence() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "change impact",
        "intent": "change",
        "scope": "code"
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![KnowledgeCandidate {
            kind: "code".into(),
            title: "top_symbol".into(),
            file_path: "crates/spur-mcp/src/lib.rs".into(),
            stable_symbol_id: Some("sym-top".into()),
            symbol_kind: Some("function".into()),
            score: 9.0,
            signal: Some("stable".into()),
            neighbor_kind: Some("primary".into()),
            edge_bind_method: None,
            grounding: "bm25-code".into(),
        }],
    };

    let pack = pack_query_result_with_exact_context(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("fixture-hash".into()),
            response_file_oids_match: Some(true),
            impacts: vec![Some(SymbolImpactSummary {
                selector: "graph://symbol/sym-top".into(),
                callers_count: 4,
                callees_count: 2,
                caller_neighbors: vec![json!({ "title": "caller_a" })],
                callee_neighbors: vec![json!({ "title": "callee_a" })],
            })],
        },
    )
    .await;

    assert_eq!(pack["impact"]["callers_count"], 4);
    assert_eq!(pack["impact"]["callees_count"], 2);
    assert_eq!(pack["impact"]["popular_sink"], false);
    assert_eq!(
        pack["staleness"]["analyst_graph_content_hash"],
        "fixture-hash"
    );
    assert_eq!(pack["staleness"]["exact_graph_hash"], "fixture-hash");
    assert_eq!(pack["staleness"]["analyst_matches_exact_graph"], true);
    assert_eq!(pack["primary_evidence"][0]["impact"]["callers_count"], 4);
    assert_eq!(pack["primary_evidence"][0]["impact"]["callees_count"], 2);
    assert_eq!(pack["primary_evidence"][0]["impact"]["popular_sink"], false);
}

#[tokio::test]
async fn knowledge_context_pack_attaches_aggregate_impact_for_top_two_code_evidence() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "change impact",
        "intent": "change",
        "scope": "code",
        "limit": 4
    }))
    .expect("request");
    let candidates = ["one", "two", "three", "four"]
        .into_iter()
        .enumerate()
        .map(|(index, suffix)| KnowledgeCandidate {
            kind: "code".into(),
            title: format!("symbol_{suffix}"),
            file_path: "crates/spur-mcp/src/lib.rs".into(),
            stable_symbol_id: Some(format!("sym-{suffix}")),
            symbol_kind: Some("function".into()),
            score: 9.0 - index as f64,
            signal: Some("stable".into()),
            neighbor_kind: Some("primary".into()),
            edge_bind_method: None,
            grounding: "bm25-code".into(),
        })
        .collect();
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates,
    };

    let pack = pack_query_result_with_exact_context(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("fixture-hash".into()),
            response_file_oids_match: Some(true),
            impacts: vec![
                Some(SymbolImpactSummary {
                    selector: "graph://symbol/sym-one".into(),
                    callers_count: 4,
                    callees_count: 2,
                    caller_neighbors: vec![json!({ "title": "caller_a" })],
                    callee_neighbors: vec![json!({ "title": "callee_a" })],
                }),
                Some(SymbolImpactSummary {
                    selector: "graph://symbol/sym-two".into(),
                    callers_count: 31,
                    callees_count: 3,
                    caller_neighbors: vec![json!({ "title": "caller_b" })],
                    callee_neighbors: vec![json!({ "title": "callee_b" })],
                }),
            ],
        },
    )
    .await;

    assert_eq!(pack["impact"]["callers_count"], 35);
    assert_eq!(pack["impact"]["callees_count"], 5);
    assert_eq!(pack["impact"]["popular_sink"], true);
    assert_eq!(
        pack["impact"]["caller_neighbors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        pack["impact"]["callee_neighbors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(pack["primary_evidence"][0]["impact"]["callers_count"], 4);
    assert_eq!(pack["primary_evidence"][1]["impact"]["callers_count"], 31);
    assert_eq!(pack["primary_evidence"][2].get("impact"), None);
    assert_eq!(pack["primary_evidence"][3].get("impact"), None);
    assert_eq!(
        pack["primary_evidence"][0]["impact"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn knowledge_context_pack_marks_popular_sink_without_expanding_neighbors() {
    let request = KnowledgeContextPackRequest::parse(&json!({
        "query": "popular impact",
        "intent": "change",
        "scope": "code"
    }))
    .expect("request");
    let result = KnowledgeQueryResult {
        db_path: "/repo/.spur/analyst.duckdb".into(),
        graph_content_hash: Some("fixture-hash".into()),
        candidates: vec![KnowledgeCandidate {
            kind: "code".into(),
            title: "sink_symbol".into(),
            file_path: "crates/spur-mcp/src/lib.rs".into(),
            stable_symbol_id: Some("sym-sink".into()),
            symbol_kind: Some("function".into()),
            score: 9.0,
            signal: Some("load-bearing wall".into()),
            neighbor_kind: Some("primary".into()),
            edge_bind_method: None,
            grounding: "bm25-code".into(),
        }],
    };

    let pack = pack_query_result_with_exact_context(
        &request,
        result,
        ExactGraphContext {
            graph_content_hash: Some("fixture-hash".into()),
            response_file_oids_match: Some(true),
            impacts: vec![Some(SymbolImpactSummary {
                selector: "graph://symbol/sym-sink".into(),
                callers_count: 31,
                callees_count: 2,
                caller_neighbors: vec![json!({ "title": "caller_a" })],
                callee_neighbors: vec![json!({ "title": "callee_a" })],
            })],
        },
    )
    .await;

    assert_eq!(pack["impact"]["callers_count"], 31);
    assert_eq!(pack["impact"]["popular_sink"], true);
    assert_eq!(
        pack["impact"]["caller_neighbors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        pack["impact"]["callee_neighbors"].as_array().unwrap().len(),
        0
    );
}

#[tokio::test]
async fn knowledge_context_pack_reports_confidence_from_real_hybrid_fusion() {
    let fixture = build_hybrid_confidence_fixture();
    let conn = crate::db::connection::open_analyst_connection_read_only(&fixture.db_path)
        .expect("open fixture query connection");
    load_pack_extensions(&conn);

    let strong_request = KnowledgeContextPackRequest::parse(&json!({
        "query": "ranking beacon",
        "scope": "all",
        "limit": 3
    }))
    .expect("request");
    let strong_result = query_context_candidates_with_conn(
        &conn,
        &fixture.db_path,
        "ranking beacon",
        KnowledgeSearchScope::All,
        KnowledgeQueryOptions {
            limit: 3,
            intent: KnowledgeQueryIntent::Explain,
            query_vec: Some(fixture.query_vec.clone()),
        },
    )
    .expect("query strong hybrid candidates");
    let strong_primary = strong_result
        .candidates
        .iter()
        .filter(|candidate| !candidate.grounding.starts_with("bm25"))
        .collect::<Vec<_>>();
    assert!(
        !strong_primary.is_empty(),
        "expected strong-query hybrid candidates, got {:?}",
        strong_result.candidates
    );
    let strong_pack = pack_query_result(&strong_request, strong_result).await;
    println!(
        "strong pack: {}",
        serde_json::to_string_pretty(&strong_pack).unwrap_or_else(|_| strong_pack.to_string())
    );
    let strong_top = strong_pack["primary_evidence"]
        .as_array()
        .and_then(|values| values.first())
        .expect("strong result should include primary evidence");
    let strong_score = strong_top["score"].as_f64().expect("strong top score");
    let strong_grounding = strong_top["grounding"].as_str().unwrap_or("<missing>");
    let strong_confidence = strong_pack["confidence"]
        .as_str()
        .expect("strong confidence");

    assert!(
        strong_grounding.starts_with("hybrid-"),
        "strong top grounding should be hybrid, got {strong_grounding}"
    );
    assert!(
        strong_score >= 0.55,
        "strong hybrid top score={strong_score:.6}, grounding={strong_grounding}"
    );
    assert!(
        strong_pack["candidates"]["returned_primary"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "expected at least one primary candidate, got {:?}",
        strong_pack["candidates"]
    );
    assert!(
        matches!(strong_confidence, "medium" | "high"),
        "cross-signal hybrid should not be reported as low confidence, got {strong_confidence}"
    );

    let weak_request = KnowledgeContextPackRequest::parse(&json!({
        "query": "private lexical-only weakness signal",
        "scope": "docs",
        "limit": 3
    }))
    .expect("request");
    let weak_result = query_context_candidates_with_conn(
        &conn,
        &fixture.db_path,
        "private lexical-only weakness signal",
        KnowledgeSearchScope::Docs,
        KnowledgeQueryOptions {
            limit: 1,
            intent: KnowledgeQueryIntent::Explain,
            query_vec: None,
        },
    )
    .expect("query weak hybrid candidates");
    let weak_primary = weak_result
        .candidates
        .iter()
        .filter(|candidate| candidate.kind == "doc")
        .collect::<Vec<_>>();
    assert!(
        !weak_primary.is_empty(),
        "expected weak-query doc candidates, got {:?}",
        weak_result.candidates
    );
    let weak_pack = pack_query_result(&weak_request, weak_result).await;
    println!(
        "weak pack: {}",
        serde_json::to_string_pretty(&weak_pack).unwrap_or_else(|_| weak_pack.to_string())
    );
    let weak_top = weak_pack["supporting_docs"]
        .as_array()
        .and_then(|values| values.first())
        .expect("weak result should include supporting docs");
    assert_eq!(
        weak_pack["candidates"]["returned_primary"]
            .as_u64()
            .unwrap_or(0),
        0
    );
    let weak_score = weak_top["score"].as_f64().expect("weak top score");
    let weak_grounding = weak_top["grounding"].as_str().unwrap_or("<missing>");

    println!(
        "measured top scores: strong={:.6}, weak={:.6}",
        strong_score, weak_score
    );
    assert_eq!(weak_grounding, "bm25-doc");
    assert_eq!(weak_pack["confidence"], "low");
}

/// Explicit acceptance path: a real owned helper supplies the pack query.
#[cfg(feature = "embed")]
#[tokio::test]
#[ignore = "requires explicitly configured real model helper and DuckDB extensions"]
async fn real_owned_helper_returns_indexed_context_pack() {
    let fixture = build_hybrid_confidence_fixture();
    let repo = fixture._temp_dir.path().join("repo");
    let spur_dir = repo.join(".spur");
    fs::create_dir_all(&spur_dir).unwrap();
    fs::copy(&fixture.db_path, spur_dir.join("analyst.duckdb")).unwrap();
    let runtime = OwnedEmbeddingRuntime::new();
    runtime.warm().await.expect("real helper warm inference");
    let pack = spur_graph::mcp::with_worktree_root_for_request(repo, async {
        let request = KnowledgeContextPackRequest::parse(&json!({
            "query": "ranking beacon", "scope": "all", "limit": 3
        }))
        .unwrap();
        knowledge_context_pack_with_runtime(request, &runtime).await
    })
    .await
    .expect("indexed pack with real owned helper");
    runtime
        .shutdown()
        .await
        .expect("checked owned helper shutdown");
    println!("S6C_REAL_PACK {}", serde_json::to_string(&pack).unwrap());
    assert_eq!(pack["answerable"], true, "{pack}");
    assert!(
        pack["primary_evidence"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()),
        "{pack}"
    );
}

/// Keep the indexed fixture alive while a real CLI host serves a pack request.
#[cfg(feature = "embed")]
#[test]
#[ignore = "requires an explicitly configured CLI, real helper and DuckDB extensions"]
fn real_cli_host_serves_indexed_context_pack() {
    let fixture = build_hybrid_confidence_fixture();
    let repo = fixture._temp_dir.path().join("repo");
    let spur_dir = repo.join(".spur");
    fs::create_dir_all(&spur_dir).unwrap();
    fs::copy(&fixture.db_path, spur_dir.join("analyst.duckdb")).unwrap();
    let driver = std::env::var_os("SPUR_S6C_HOST_DRIVER").expect("explicit host driver");
    let status = Command::new("/usr/bin/python3")
        .arg(driver)
        .env("SPUR_S6C_HOST_ROOT", repo)
        .status()
        .expect("start host driver");
    assert!(status.success(), "real CLI host driver failed: {status}");
}
