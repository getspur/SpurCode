//! Per-`BrainSession` HTTP/JSON-RPC server exposing the curated worker MCP
//! tool subset to delegated workers.
//!
//! The transport layer is RMCP Streamable HTTP (`axum` + `StreamableHttpService`)
//! and tools are exposed as first-class RMCP tool methods that delegate to the
//! existing freestanding handlers in [`crate::handlers`]. Audit emission and
//! per-delegation lifecycle guards remain in this module.

use std::borrow::Cow;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

/// Configuration for the background audit flusher and idle thresholds.
#[derive(Debug, Clone)]
pub struct WorkerMcpServerConfig {
    /// How long a read-audit buffer must be untouched before the background
    /// flusher considers it idle and emits a `ReadAggregate` sentinel.
    pub idle_threshold: Duration,
    /// How often the flusher scans the per-delegation buffer map.
    pub scan_interval: Duration,
    /// Worker MCP leaf families to advertise. Default is all leaves.
    pub mcp_families: std::collections::BTreeSet<crate::mcp::McpFamily>,
}

impl Default for WorkerMcpServerConfig {
    fn default() -> Self {
        Self {
            idle_threshold: Duration::from_secs(30),
            scan_interval: Duration::from_secs(10),
            mcp_families: crate::mcp::McpFamily::ALL.iter().copied().collect(),
        }
    }
}

use parking_lot::Mutex;
use rand::RngCore;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext as RmcpToolCallContext},
    model::{
        object as rmcp_object, CallToolRequestParams, CallToolResult, Implementation, JsonObject,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::{local::LocalSessionManager, SessionId, SessionManager},
        StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use axum::serve::Listener;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode},
    middleware::{self, Next},
    Router,
};

use crate::handlers::{McpHandlerError, WorkerCallContext};
use spur_acp::config::ContextServiceConfig;
use spur_mcp::events::McpEventSink;
use spur_mcp::token::validate_token;

/// MCP server name advertised to workers. Claude Code derives its tool names
/// from this as `mcp__spur-worker-mcp__<tool>`; the dispatch entry in
/// `orchestrator::worker_mcp` and profile-allowlist augmentation in
/// `crate::mcp::worker_mcp_claude_tool_names` must all agree on it.
pub(crate) const WORKER_MCP_SERVER_NAME: &str = "spur-worker-mcp";

#[async_trait]
pub trait WorkerSignalSink: Send + Sync {
    async fn report_signal(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError>;

    async fn report_progress(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError>;
}

#[async_trait]
pub trait WorkerReadToolSink: Send + Sync {
    async fn get_plan_status(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError>;

    async fn get_task_diff(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError>;

    async fn fetch_outcome_artifact(
        &self,
        ctx: &WorkerCallContext,
        args: Value,
    ) -> Result<Value, McpHandlerError>;
}

/// Per-delegation context cached by the dispatcher so gating checks never
/// hit the PM on the hot path.
#[derive(Debug, Clone, Copy, Default)]
pub struct DelegationContext {
    pub enable_worker_progress: bool,
}

/// One read-tool call recorded for later aggregation into a single
/// `worker-mcp` audit comment by the background flusher (T21).
///
/// Read tools (`get_issue`, `list_issues`, `get_task_diff`,
/// `get_plan_status`, `fetch_outcome_artifact`) are coalesced per delegation
/// rather than producing one beads comment per call so the audit trail stays
/// readable.
#[derive(Debug, Clone)]
pub struct ReadAuditEntry {
    pub tool_name: String,
    pub target_issue_id: Option<String>,
    /// Unix seconds, captured at append time.
    pub ts: u64,
}

/// Drained payload sent across the flush channel to the background audit
/// task. Carries the full delegation_id so the receiver can route the
/// aggregated comment to the correct beads issue.
#[derive(Debug)]
pub struct FlushMessage {
    pub delegation_id: String,
    pub entries: Vec<ReadAuditEntry>,
}

/// Per-delegation aggregation buffer for read-tool audit entries.
///
/// Held inside the dispatcher's `read_audit_buffers` map keyed by
/// `delegation_id`. Each read-tool call appends a [`ReadAuditEntry`]; the
/// background flusher (T21) drains the buffer either on idle timeout or on
/// explicit delegation completion. As a safety net, the synchronous [`Drop`]
/// impl performs a non-blocking `send` on the flush channel so entries are
/// not lost even if the buffer is removed from the map without an explicit
/// flush call.
pub struct ReadAuditBuffer {
    delegation_id: String,
    entries: Mutex<Vec<ReadAuditEntry>>,
    flush_tx: mpsc::UnboundedSender<FlushMessage>,
}

impl ReadAuditBuffer {
    pub fn new(delegation_id: String, flush_tx: mpsc::UnboundedSender<FlushMessage>) -> Self {
        Self {
            delegation_id,
            entries: Mutex::new(Vec::new()),
            flush_tx,
        }
    }

    /// Append an entry. O(1) amortized; only blocks on the per-buffer lock,
    /// never on the flush channel.
    pub fn append(&self, entry: ReadAuditEntry) {
        self.entries.lock().push(entry);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn delegation_id(&self) -> &str {
        &self.delegation_id
    }

    /// Drain the buffer and return the accumulated entries. Used by
    /// [`WorkerMcpServer::flush_delegation`] to forward entries on the
    /// flush channel in-band rather than relying on the buffer's `Drop`.
    pub fn take_entries(&self) -> Vec<ReadAuditEntry> {
        std::mem::take(&mut *self.entries.lock())
    }

    /// Test-only escape hatch so unit tests can populate the buffer without
    /// going through the dispatcher.
    #[cfg(any(test, feature = "test-support"))]
    pub fn append_for_test(&self, entry: ReadAuditEntry) {
        self.append(entry);
    }
}

impl Drop for ReadAuditBuffer {
    fn drop(&mut self) {
        let entries = std::mem::take(&mut *self.entries.lock());
        if entries.is_empty() {
            return;
        }
        // `mpsc::UnboundedSender::send` never blocks. If the receiver has
        // been dropped we silently swallow the error: there's no actor left
        // to deliver the audit to, and panicking inside `Drop` would abort
        // the process. The background flusher (T21) keeps the receiver
        // alive for the server's lifetime.
        let _ = self.flush_tx.send(FlushMessage {
            delegation_id: std::mem::take(&mut self.delegation_id),
            entries,
        });
    }
}

/// Shared dependencies the dispatcher hands to handlers. Bundled into a
/// struct so [`WorkerMcpServer::start`] stays one positional arg per
/// orthogonal concept and so future tasks (T18 dual-gating, T19 audit) can
/// extend the set without churning every call site.
#[derive(Clone)]
pub struct WorkerMcpDeps {
    pub pm_service: Arc<spur_pm::PmService>,
    pub feature_gate: Arc<spur_license::FeatureGate>,
    pub funnel: Arc<dyn McpEventSink>,
    pub worker_signal_sink: Arc<dyn WorkerSignalSink>,
    pub worker_read_sink: Arc<dyn WorkerReadToolSink>,
    /// Required by `get_task_diff` when reconstructing diffs from persisted
    /// worker branches; `None` disables that recovery branch.
    pub repo_root: Option<PathBuf>,
}

/// Per-`BrainSession` worker MCP server. Owns a TCP listener on
/// `127.0.0.1:<random-port>` and a cancellable accept loop.
pub struct WorkerMcpServer {
    addr: SocketAddr,
    /// 32-byte HMAC key generated at start. Held in-process only — never
    /// logged or persisted. Used by token validation middleware (T16+).
    hmac_key: [u8; 32],
    /// `BrainSession` id stamped onto every `WorkerCallContext` issued by this
    /// server (T16+).
    brain_session_id: String,
    /// Materializer is derived from `outcome_store` once at start so the
    /// dispatcher hot path doesn't pay for the construction per request.
    deps: Arc<DispatcherDeps>,
    shutdown: CancellationToken,
    connection_force_shutdown: CancellationToken,
    connections: Arc<ConnectionTracker>,
    session_manager: Arc<LocalSessionManager>,
    session_contexts:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, AuthenticatedWorkerContext>>>,
    accept_loop_handle: Mutex<Option<JoinHandle<()>>>,
    flusher_handle: Mutex<Option<JoinHandle<()>>>,
    /// Receiver half of the read-audit flush channel. Created in `start()`
    /// and owned here until T21 spawns the background flusher task that
    /// `take()`s it. Holding the receiver here prevents the channel from
    /// closing prematurely during the T20→T21 transitional window.
    flush_rx: Mutex<Option<mpsc::UnboundedReceiver<FlushMessage>>>,
    /// In-flight dispatcher count. Incremented on `dispatch` entry by
    /// [`ActiveCallGuard`] and decremented on its `Drop`. [`shutdown`]
    /// polls this counter (via [`active_count`](Self::active_count)) so
    /// in-flight requests are drained before the flusher task is joined.
    /// `Arc` so the guard owned by each dispatch task can decrement
    /// independently of the server's lifetime.
    active_delegations: Arc<AtomicU32>,
    /// High-water mark of `active_delegations`, raised by [`ActiveCallGuard`]
    /// on each increment and never lowered. Lets [`peak_active_count`]
    /// report true max concurrency without lossy time-sampling. Same `Arc`
    /// instance as in [`DispatcherDeps`].
    peak_active_delegations: Arc<AtomicU32>,
    /// Claude allowlist names derived from the exact registry advertised by
    /// this server instance.
    worker_mcp_tool_names: Vec<String>,
}

#[derive(Default)]
struct ConnectionTracker {
    state: Mutex<ConnectionTrackerState>,
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct ConnectionTrackerState {
    closing: bool,
    active: usize,
}

struct ConnectionGuard {
    tracker: Arc<ConnectionTracker>,
}

impl ConnectionTracker {
    fn register(self: &Arc<Self>) -> (bool, ConnectionGuard) {
        let mut state = self.state.lock();
        state.active += 1;
        let closing = state.closing;
        drop(state);
        (
            closing,
            ConnectionGuard {
                tracker: Arc::clone(self),
            },
        )
    }

    fn begin_closing(&self) {
        self.state.lock().closing = true;
    }

    async fn wait(&self) {
        loop {
            let changed = self.changed.notified();
            if self.state.lock().active == 0 {
                return;
            }
            changed.await;
        }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut state = self.tracker.state.lock();
        state.active = state.active.saturating_sub(1);
        drop(state);
        self.tracker.changed.notify_waiters();
    }
}

struct TrackedTcpListener {
    inner: TcpListener,
    connections: Arc<ConnectionTracker>,
    force_shutdown: CancellationToken,
}

impl Listener for TrackedTcpListener {
    type Io = TrackedTcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => {
                    let (closing, guard) = self.connections.register();
                    let force_shutdown = if closing {
                        let rejected = CancellationToken::new();
                        rejected.cancel();
                        rejected
                    } else {
                        self.force_shutdown.clone()
                    };
                    return (TrackedTcpStream::new(stream, force_shutdown, guard), addr);
                }
                Err(error) => {
                    tracing::error!(%error, "worker MCP accept error");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

struct TrackedTcpStream {
    inner: TcpStream,
    force_shutdown: Pin<Box<tokio_util::sync::WaitForCancellationFutureOwned>>,
    _guard: ConnectionGuard,
}

impl TrackedTcpStream {
    fn new(inner: TcpStream, force_shutdown: CancellationToken, guard: ConnectionGuard) -> Self {
        Self {
            inner,
            force_shutdown: Box::pin(force_shutdown.cancelled_owned()),
            _guard: guard,
        }
    }

    fn poll_forced(&mut self, cx: &mut TaskContext<'_>) -> bool {
        self.force_shutdown.as_mut().poll(cx).is_ready()
    }

    fn forced_error() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "worker MCP connection force-closed",
        )
    }
}

impl AsyncRead for TrackedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.poll_forced(cx) {
            return Poll::Ready(Err(Self::forced_error()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TrackedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.poll_forced(cx) {
            return Poll::Ready(Err(Self::forced_error()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        if self.poll_forced(cx) {
            return Poll::Ready(Err(Self::forced_error()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.poll_forced(cx) {
            return Poll::Ready(Err(Self::forced_error()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        if self.poll_forced(cx) {
            return Poll::Ready(Err(Self::forced_error()));
        }
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
}

/// RAII guard that increments [`WorkerMcpServer::active_delegations`] on
/// construction and decrements on drop. Created at the top of [`dispatch`]
/// so the count is panic-safe — even if a handler panics between increment
/// and decrement, unwinding through `Drop` keeps the counter consistent.
/// This mirrors the [`ReadAuditBuffer`] Drop pattern used by T20.
struct ActiveCallGuard {
    counter: Arc<AtomicU32>,
}

#[derive(Default)]
struct HandlerAbortRegistry {
    next_id: AtomicU64,
    state: Mutex<HandlerAbortState>,
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct HandlerAbortState {
    closing: bool,
    handles: std::collections::HashMap<u64, futures::future::AbortHandle>,
}

struct HandlerAbortRegistration {
    id: Option<u64>,
    registry: Arc<HandlerAbortRegistry>,
}

#[cfg(test)]
pub(crate) struct TestHandlerLifecycle {
    abort_registration: Option<futures::future::AbortRegistration>,
    _handler_registration: HandlerAbortRegistration,
    _active_guard: ActiveCallGuard,
}

#[cfg(test)]
impl TestHandlerLifecycle {
    pub(crate) async fn aborted_within(&mut self, deadline: Duration) -> bool {
        let registration = self
            .abort_registration
            .take()
            .expect("test handler abort registration consumed once");
        matches!(
            tokio::time::timeout(
                deadline,
                futures::future::Abortable::new(std::future::pending::<()>(), registration),
            )
            .await,
            Ok(Err(_))
        )
    }
}

impl HandlerAbortRegistry {
    fn register(
        self: &Arc<Self>,
    ) -> (futures::future::AbortRegistration, HandlerAbortRegistration) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (handle, registration) = futures::future::AbortHandle::new_pair();
        let mut state = self.state.lock();
        let id = if state.closing {
            handle.abort();
            None
        } else {
            state.handles.insert(id, handle);
            Some(id)
        };
        (
            registration,
            HandlerAbortRegistration {
                id,
                registry: Arc::clone(self),
            },
        )
    }

    fn begin_closing(&self) {
        self.state.lock().closing = true;
    }

    fn abort_all(&self) {
        self.begin_closing();
        let state = self.state.lock();
        for handle in state.handles.values() {
            handle.abort();
        }
    }

    async fn wait(&self) {
        loop {
            let changed = self.changed.notified();
            if self.state.lock().handles.is_empty() {
                return;
            }
            changed.await;
        }
    }
}

impl Drop for HandlerAbortRegistration {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.registry.state.lock().handles.remove(&id);
        }
        self.registry.changed.notify_waiters();
    }
}

impl ActiveCallGuard {
    fn new(counter: Arc<AtomicU32>, peak: &AtomicU32) -> Self {
        // `fetch_add` returns the prior value; the new in-flight count is +1.
        let in_flight = counter.fetch_add(1, Ordering::SeqCst) + 1;
        // Raise the high-water mark to at least the current in-flight count.
        // Done atomically here, so it captures every momentary overlap with
        // no sampling gap.
        peak.fetch_max(in_flight, Ordering::SeqCst);
        Self { counter }
    }
}

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One worker-MCP call recorded on the per-delegation summary guard.
struct CallRecord {
    tool_name: String,
    latency_ms: u64,
    is_error: bool,
}

/// Per-delegation tracker that emits a `WorkerMcpDelegationSummary` event on
/// Drop. Held in [`DispatcherDeps::delegation_guards`] and removed by
/// [`WorkerMcpServer::complete_delegation`] (or on server shutdown) so the
/// summary fires exactly once per delegation.
pub struct DelegationDispatchGuard {
    delegation_id: String,
    brain_session_id: String,
    funnel: Arc<dyn McpEventSink>,
    calls: Mutex<Vec<CallRecord>>,
    /// Delegation-level error count, separate from per-call `is_error`.
    /// Bumped by [`mark_error`] when the brain reports a terminal error
    /// outcome or the flush channel closes; added to the per-call error
    /// count in the emitted `WorkerMcpDelegationSummary`. This preserves
    /// the delegation-level outcome bit even when no per-call errors
    /// occurred (e.g. clean dispatch, then flush channel closure).
    extra_errors: AtomicU64,
    /// Set to `true` by [`WorkerMcpServer::complete_delegation`] so the
    /// summary only fires on explicit delegation close, not on server
    /// shutdown or map eviction.
    completed: AtomicBool,
}

impl DelegationDispatchGuard {
    pub fn new(
        delegation_id: String,
        brain_session_id: String,
        funnel: Arc<dyn McpEventSink>,
    ) -> Self {
        Self {
            delegation_id,
            brain_session_id,
            funnel,
            calls: Mutex::new(Vec::new()),
            extra_errors: AtomicU64::new(0),
            completed: AtomicBool::new(false),
        }
    }

    /// Record one completed worker-MCP call with its tool name, observed
    /// latency, and whether it returned an error to the worker.
    pub fn record_call(&self, tool_name: &str, latency_ms: u64, is_error: bool) {
        self.calls.lock().push(CallRecord {
            tool_name: tool_name.to_string(),
            latency_ms,
            is_error,
        });
    }

    /// Bump a delegation-level error counter. Used by
    /// [`WorkerMcpServer::flush_delegation`] when the brain reports a
    /// terminal error outcome or the audit-flush channel closes — so the
    /// emitted `WorkerMcpDelegationSummary.errors` reflects the
    /// delegation-level failure even when no per-call errors occurred.
    pub fn mark_error(&self) {
        self.extra_errors.fetch_add(1, Ordering::Relaxed);
    }
}

/// Compute p99 latency from a slice of successful-call latencies in
/// milliseconds. Per spec: if fewer than 2 samples, return the max observed
/// (or 0 when empty). Otherwise use nearest-rank: `idx = ceil(0.99 * n) - 1`.
fn compute_p99_latency_ms(latencies: &[u64]) -> u64 {
    if latencies.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u64> = latencies.to_vec();
    sorted.sort_unstable();
    if sorted.len() < 2 {
        return *sorted.last().expect("len >= 1");
    }
    let n = sorted.len();
    let rank = ((n as f64) * 0.99).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

impl Drop for DelegationDispatchGuard {
    fn drop(&mut self) {
        if !self.completed.load(Ordering::Relaxed) {
            return;
        }
        let calls = self.calls.lock();
        let calls_total = calls.len() as u64;
        let mut calls_by_tool: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        let mut success_latencies: Vec<u64> = Vec::with_capacity(calls.len());
        let mut errors: u64 = 0;
        for call in calls.iter() {
            *calls_by_tool.entry(call.tool_name.clone()).or_insert(0) += 1;
            if call.is_error {
                errors += 1;
            } else {
                success_latencies.push(call.latency_ms);
            }
        }
        let p99_latency_ms = compute_p99_latency_ms(&success_latencies);
        // Add delegation-level errors (set via `mark_error` from
        // `flush_delegation` on terminal-error outcome or flush channel
        // closure) so the summary reflects failures even when no
        // per-call errors occurred.
        let errors = errors + self.extra_errors.load(Ordering::Relaxed);
        let body = spur_acp::SpurEventBody::WorkerMcpDelegationSummary {
            delegation_id: self.delegation_id.clone(),
            brain_session_id: self.brain_session_id.clone(),
            calls_total,
            calls_by_tool,
            p99_latency_ms,
            errors,
        };
        // `try_emit` so a full broadcast bus doesn't back-pressure the Drop.
        let _ = self.funnel.try_emit(body);
    }
}

/// Internal bundle of the materialized handler dependencies.
struct DispatcherDeps {
    embedding_runtime: spur_analyst::embedding::OwnedEmbeddingRuntime,
    pm_service: Arc<spur_pm::PmService>,
    feature_gate: Arc<spur_license::FeatureGate>,
    funnel: Arc<dyn McpEventSink>,
    worker_signal_sink: Arc<dyn WorkerSignalSink>,
    worker_read_sink: Arc<dyn WorkerReadToolSink>,
    repo_root: Option<PathBuf>,
    /// Cached per-delegation context — populated by the orchestrator at
    /// dispatch time so gating checks are O(1) and PM-free.
    /// TODO(T22/T24): add unregister_delegation() called by the orchestrator on
    /// terminal delegation status (Success/Failed/Cancelled) so long-running
    /// brain sessions don't leak DelegationContext entries indefinitely.
    delegations: Arc<parking_lot::Mutex<std::collections::HashMap<String, DelegationContext>>>,
    /// Worktree root for code graph requests, keyed by delegation id. The
    /// worker MCP server is per brain session and runs in the brain process,
    /// so code_* handlers cannot derive the worker worktree from process cwd.
    delegation_worktree_roots: Arc<parking_lot::Mutex<std::collections::HashMap<String, PathBuf>>>,
    /// Server-owned notebook capability, never supplied by tool arguments.
    notebook_readers: parking_lot::Mutex<
        std::collections::HashMap<String, Arc<crate::worker_notebook::NotebookReader>>,
    >,
    /// Persistent graph MCP module so refresh/rebuild singleflight state is
    /// shared across worker code graph calls for this server.
    graph_mcp_module: spur_graph::mcp::GraphMcpModule,
    /// Per-delegation read-tool aggregation buffers. Each entry is `Arc`'d
    /// so concurrent in-flight read calls and the (future T21) background
    /// flusher can hold cheap references without contending for the outer
    /// map lock. Keyed by `delegation_id`.
    /// TODO(T21): the background flusher periodically removes idle entries
    /// (last-touched > N minutes) so this map doesn't grow unbounded across
    /// long-lived brain sessions.
    read_audit_buffers:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, Arc<ReadAuditBuffer>>>>,
    /// Sender half of the read-audit flush channel. Cloned into every new
    /// `ReadAuditBuffer` so the buffer's `Drop` can deliver final entries
    /// even if the dispatcher tears down without an explicit flush.
    flush_tx: mpsc::UnboundedSender<FlushMessage>,
    /// Shared counter incremented/decremented by [`ActiveCallGuard`] inside
    /// [`dispatch`]. Same `Arc` instance held by [`WorkerMcpServer`].
    active_delegations: Arc<AtomicU32>,
    /// High-water mark of `active_delegations`: the max concurrent in-flight
    /// dispatcher count ever observed. Raised by [`ActiveCallGuard`] on each
    /// increment (never lowered). Same `Arc` instance held by
    /// [`WorkerMcpServer`].
    peak_active_delegations: Arc<AtomicU32>,
    handler_aborts: Arc<HandlerAbortRegistry>,
    /// Per-delegation summary guards. Each entry emits one
    /// `WorkerMcpDelegationSummary` on removal (or on server shutdown).
    delegation_guards:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, DelegationDispatchGuard>>>,
}

/// Token-derived request context cached in middleware and injected into each
/// request as an axum extension so the RMCP handler can build
/// [`WorkerCallContext`] without reading transport-specific headers.
#[derive(Debug, Clone)]
struct AuthenticatedWorkerContext {
    delegation_id: String,
    brain_session_id: String,
}

/// Shared auth state for the axum middleware that gates session creation.
#[derive(Clone)]
struct WorkerAuthMiddlewareState {
    hmac_key: [u8; 32],
    brain_session_id: String,
    session_manager: Arc<LocalSessionManager>,
    session_contexts:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, AuthenticatedWorkerContext>>>,
}

async fn invoke_code_graph_for_worker(
    deps: Arc<DispatcherDeps>,
    worker_ctx: WorkerCallContext,
    tool_name: &'static str,
    args: Value,
) -> Result<Value, McpHandlerError> {
    reject_worker_project_selector(&args)?;
    let worktree_root = {
        deps.delegation_worktree_roots
            .lock()
            .get(&worker_ctx.delegation_id)
            .cloned()
    };
    let graph_mcp_module = deps.graph_mcp_module.clone();
    let future = async move {
        match graph_mcp_module.dispatch(tool_name, args).await {
            Ok(body) => Ok(body),
            Err(error) => Err(code_graph_dispatch_error(error).await),
        }
    };
    if let Some(worktree_root) = worktree_root {
        crate::server::handlers::code_graph::with_worktree_root_for_request(worktree_root, future)
            .await
    } else {
        future.await
    }
}

async fn code_graph_dispatch_error(error: spur_graph::mcp::CodeGraphError) -> McpHandlerError {
    let response = error.into_error_response().await;
    match response.code {
        -32602 => McpHandlerError::InvalidParams(response.message),
        -32004 => McpHandlerError::NotFound(response.message),
        -32001 => McpHandlerError::Unauthorized(response.message),
        -32603 => McpHandlerError::Internal(response.message),
        _ => McpHandlerError::Internal(response.message),
    }
}

fn pm_mcp_error(error: spur_pm::mcp::McpHandlerError) -> McpHandlerError {
    match error {
        spur_pm::mcp::McpHandlerError::InvalidParams(message) => {
            McpHandlerError::InvalidParams(message)
        }
        spur_pm::mcp::McpHandlerError::NotFound(message) => McpHandlerError::NotFound(message),
        spur_pm::mcp::McpHandlerError::Unauthorized(message) => {
            McpHandlerError::Unauthorized(message)
        }
        spur_pm::mcp::McpHandlerError::UpstreamPm(message) => McpHandlerError::UpstreamPm(message),
        spur_pm::mcp::McpHandlerError::Internal(message) => McpHandlerError::Internal(message),
    }
}

fn analyst_mcp_error(error: spur_analyst::mcp::McpHandlerError) -> McpHandlerError {
    match error {
        spur_analyst::mcp::McpHandlerError::InvalidParams(message) => {
            McpHandlerError::InvalidParams(message)
        }
        spur_analyst::mcp::McpHandlerError::NotFound(message) => McpHandlerError::NotFound(message),
        spur_analyst::mcp::McpHandlerError::Unauthorized(message) => {
            McpHandlerError::Unauthorized(message)
        }
        spur_analyst::mcp::McpHandlerError::UpstreamPm(message) => {
            McpHandlerError::UpstreamPm(message)
        }
        spur_analyst::mcp::McpHandlerError::Internal(message) => McpHandlerError::Internal(message),
    }
}

fn has_analyst_db(root: &std::path::Path) -> bool {
    root.join(".spur").join("analyst.duckdb").exists()
}

fn select_knowledge_context_root(
    worker_root: Option<PathBuf>,
    repo_root: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(root) = worker_root.as_ref().filter(|root| has_analyst_db(root)) {
        return Some(root.clone());
    }
    if let Some(root) = repo_root.as_ref().filter(|root| has_analyst_db(root)) {
        return Some(root.clone());
    }
    worker_root.or(repo_root)
}

fn reject_worker_project_selector(args: &Value) -> Result<(), McpHandlerError> {
    if args
        .as_object()
        .is_some_and(|object| object.contains_key("project"))
    {
        return Err(McpHandlerError::InvalidParams(
            "field `project` is not available on this MCP server".to_owned(),
        ));
    }
    Ok(())
}

async fn invoke_query_for_worker(
    deps: Arc<DispatcherDeps>,
    worker_ctx: WorkerCallContext,
    args: Value,
) -> Result<Value, McpHandlerError> {
    reject_worker_project_selector(&args)?;
    let worktree_root = {
        deps.delegation_worktree_roots
            .lock()
            .get(&worker_ctx.delegation_id)
            .cloned()
    }
    .ok_or_else(|| {
        McpHandlerError::Unauthorized(
            "worker query requires an assigned delegation worktree".to_owned(),
        )
    })?;
    let future = async move {
        spur_analyst::mcp::query(&args)
            .await
            .map_err(analyst_mcp_error)
    };
    spur_graph::mcp::with_worktree_root_for_request(worktree_root, future).await
}

async fn invoke_knowledge_context_for_worker(
    deps: Arc<DispatcherDeps>,
    worker_ctx: WorkerCallContext,
    args: Value,
) -> Result<Value, McpHandlerError> {
    reject_worker_project_selector(&args)?;
    let worker_root = {
        deps.delegation_worktree_roots
            .lock()
            .get(&worker_ctx.delegation_id)
            .cloned()
    };
    let root = select_knowledge_context_root(worker_root, deps.repo_root.clone());
    let future = async move {
        // knowledge_context_pack is a deprecated alias that routes to v2 behavior
        // (main's knowledge_context_pack_2 first-class promotion), served through the
        // spur-analyst-owned handler (our P4 ownership boundary).
        spur_analyst::mcp::knowledge_context_pack_2_with_runtime(&args, &deps.embedding_runtime)
            .await
            .map_err(analyst_mcp_error)
    };
    if let Some(root) = root {
        spur_graph::mcp::with_worktree_root_for_request(root, future).await
    } else {
        future.await
    }
}

async fn invoke_doc_navigate_for_worker(
    deps: Arc<DispatcherDeps>,
    worker_ctx: WorkerCallContext,
    args: Value,
) -> Result<Value, McpHandlerError> {
    reject_worker_project_selector(&args)?;
    let worker_root = {
        deps.delegation_worktree_roots
            .lock()
            .get(&worker_ctx.delegation_id)
            .cloned()
    };
    let root = select_knowledge_context_root(worker_root, deps.repo_root.clone());
    let future = async move {
        spur_analyst::mcp::doc_navigate(&args)
            .await
            .map_err(analyst_mcp_error)
    };
    if let Some(root) = root {
        spur_graph::mcp::with_worktree_root_for_request(root, future).await
    } else {
        future.await
    }
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct GetIssueParams {
    id: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema, Serialize)]
struct ListIssuesParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority_max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issue_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text_search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct GetPlanStatusParams {
    plan_id: String,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct GetTaskDiffParams {
    plan_id: String,
    task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempt: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeArtifactSection {
    StatusOnly,
    Summary,
    DiffOnly,
    Full,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct FetchOutcomeArtifactParams {
    delegation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    section: Option<OutcomeArtifactSection>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodeAmbiguityMode {
    Candidates,
    Error,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodeSearchMode {
    Exact,
    Prefix,
    Substring,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum KnowledgeIntentParam {
    Explain,
    Change,
    Review,
    Debug,
    Plan,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum KnowledgeScopeParam {
    All,
    Docs,
    Code,
    Graph,
}

#[derive(Debug, Default, Deserialize, JsonSchema, Serialize)]
struct DocNavigateParams {
    /// Full-text query. Required when root is null or omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    /// Stable symbol id. When set, expand one Contains hop instead of FTS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    /// Result count. Clamped by the handler to 1..=100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    k: Option<u64>,
    /// Optional glob over worktree-relative file_path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_glob: Option<String>,
    /// Optional git commit SHA for point-in-time root symbol resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    as_of: Option<String>,
    /// Include the first body_text characters as lede. Defaults true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_lede: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct WorkerQueryParams {
    /// DuckDB SQL to execute read-only.
    query: String,
    /// Execute against a stale analyst index and return its warning. Defaults false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_stale: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodeResponseFormat {
    Full,
    Compact,
    Table,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodeReadSymbolResponseFormat {
    Full,
    Compact,
    Source,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeResolveParams {
    /// Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name.
    selector: String,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeFileSymbolsParams {
    /// Worktree-relative file path.
    file: String,
    /// Output shape. full is the default object-row response; compact omits healthy metadata defaults; table returns symbols as cols/rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_format: Option<CodeResponseFormat>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeSymbolInfoParams {
    /// Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    /// deprecated; use selector. Accepts graph://symbol/<id> or bare hex id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeReadSymbolParams {
    /// Stable symbol id from code_resolve/code_search/code_symbol_info. graph://symbol/<id> is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_symbol_id: Option<String>,
    /// Worktree-relative file path. Required with name and mutually exclusive with stable_symbol_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    /// Symbol entity_name or qualified_name within path. Required with path and mutually exclusive with stable_symbol_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Lines of context to include before and after the symbol. Values outside 0..50 are clamped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_lines: Option<u32>,
    /// Output shape. full is the default source-plus-metadata response; compact omits healthy metadata defaults; source returns a source-first object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_format: Option<CodeReadSymbolResponseFormat>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeSymbolParams {
    /// Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    /// deprecated; use selector. Accepts graph://symbol/<id> or bare hex id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    /// Ambiguity handling. Defaults to candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_ambiguous: Option<CodeAmbiguityMode>,
    /// Include unresolved caller/callee rows. Defaults to false; counts_by_kind and unresolved_sample still summarize filtered rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_unresolved: Option<bool>,
    /// Resolve the selector against a specific commit SHA (temporal time-travel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    as_of: Option<String>,
    /// Output shape. full is the default object-row response; compact omits healthy metadata defaults; table returns rows as cols/rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_format: Option<CodeResponseFormat>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeSearchParams {
    /// Search term. Non-empty.
    query: String,
    /// Search mode. Defaults to substring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<CodeSearchMode>,
    /// Optional exact symbol_kind filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symbol_kind: Option<String>,
    /// Optional exact worktree-relative file path. Mutually exclusive with file_glob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// Optional glob over worktree-relative file_path. Mutually exclusive with file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_glob: Option<String>,
    /// Result limit. Clamped to 1..=200 by the handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeSubgraphParams {
    /// Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    /// Continuation roots from a prior truncated_frontier response. Mutually exclusive with selector and symbol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_nodes: Option<Vec<String>>,
    /// deprecated; use selector. Accepts graph://symbol/<id> or bare hex id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    /// Ambiguity handling. Defaults to candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_ambiguous: Option<CodeAmbiguityMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    radius: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_nodes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_edges: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    /// Optional public edge_kind filter: calls, calls_dyn, references_hof, references_other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edge_kinds: Option<Vec<String>>,
    /// Include unresolved boundary edges. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_unresolved: Option<bool>,
    /// Resolve the selector against a specific commit SHA (temporal time-travel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    as_of: Option<String>,
    /// Output shape. full is the default JSON nodes/edges response; compact omits healthy metadata defaults; table returns nodes/edges as cols/rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_format: Option<CodeResponseFormat>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct CodeSymbolHistoryParams {
    symbol: String,
}

fn default_max_symbol_bodies() -> Option<u64> {
    Some(0)
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct KnowledgeContextPackV2Params {
    /// Natural-language query to orient code or docs exploration.
    query: String,
    /// Retrieval intent. Defaults to explain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intent: Option<KnowledgeIntentParam>,
    /// Retrieval scope. Defaults to all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<KnowledgeScopeParam>,
    /// Result limit. Clamped by the handler to 1..=20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    /// Include test symbols in code evidence. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    include_tests: Option<bool>,
    /// Optional maximum number of symbol bodies to include. Defaults to 0; accepted range is 0..=5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    #[schemars(default = "default_max_symbol_bodies")]
    #[schemars(range(min = 0, max = 5))]
    max_symbol_bodies: Option<u64>,
    /// Optional graph reasoning controls for v2 evidence sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph_reasoning: Option<KnowledgeGraphReasoningParams>,
}

#[derive(Debug, Default, Deserialize, JsonSchema, Serialize)]
struct KnowledgeGraphReasoningParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paths: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    communities: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    risk: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_path_hops: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_paths: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchors: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct ReportProgressParams {
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    percent: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
struct SubmitReviewVerdictParams {
    target_issue_id: String,
    decision: SubmitReviewDecisionParam,
    feedback: String,
    evidence: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
enum SubmitReviewDecisionParam {
    Approve,
    RequestChanges,
}

/// RMCP `ServerHandler` for the curated worker tool subset.
struct WorkerToolHandler {
    deps: Arc<DispatcherDeps>,
    brain_session_id: String,
    tool_registry: Result<spur_mcp::registry::ToolRegistry, spur_mcp::ToolRegistryError>,
    context_service_client: Option<crate::mcp::ContextServiceClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl WorkerToolHandler {
    fn new(
        deps: Arc<DispatcherDeps>,
        brain_session_id: String,
        context_service_config: &ContextServiceConfig,
        mcp_families: &std::collections::BTreeSet<crate::mcp::McpFamily>,
    ) -> (Self, Vec<String>) {
        let (tool_registry, context_service_client) =
            crate::mcp::worker_tool_dispatch_with_families(
                context_service_config,
                deps.repo_root.as_deref(),
                mcp_families,
            );
        let worker_mcp_tool_names = tool_registry
            .as_ref()
            .map(crate::mcp::worker_mcp_claude_tool_names_for_registry)
            .unwrap_or_default();
        (
            Self {
                deps,
                brain_session_id,
                tool_registry,
                context_service_client,
                tool_router: Self::tool_router(),
            },
            worker_mcp_tool_names,
        )
    }

    fn worker_registry(&self) -> Result<&spur_mcp::registry::ToolRegistry, McpError> {
        self.tool_registry.as_ref().map_err(|error| {
            McpError::internal_error(
                format!("core worker MCP tool registry is invalid: {error}"),
                None,
            )
        })
    }

    async fn call_worker_registry_tool(
        &self,
        tool_name: &'static str,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let context = spur_mcp::ToolCallContext::new(
            spur_mcp::ServerKind::Worker,
            spur_mcp::ToolAuthority::Worker,
            None,
            None,
        );
        let response = self
            .worker_registry()?
            .call_json_tool(context, tool_name, Value::Object(arguments))
            .await;
        spur_mcp::json_rpc_to_call_tool_result(response, tool_name)
    }

    #[tool(
        name = "solve_rule_spec",
        description = "Discover versioned solver rule families and exact rule guidance without running Z3."
    )]
    async fn solve_rule_spec_tool(
        &self,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_rule_spec", arguments)
            .await
    }

    #[tool(
        name = "solve_rules",
        description = "Verify or synthesize a bounded model with one versioned solver rule family."
    )]
    async fn solve_rules_tool(&self, arguments: JsonObject) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_rules", arguments)
            .await
    }

    #[tool(
        name = "solve_constraint_spec",
        description = "Discover the generic B-prime request, variable, expression, operator, limit, and example contracts without running Z3."
    )]
    async fn solve_constraint_spec_tool(
        &self,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_constraint_spec", arguments)
            .await
    }

    #[tool(
        name = "solve_constraint_check",
        description = "Validate generic constraint arguments and return path-aware repair diagnostics without running Z3."
    )]
    async fn solve_constraint_check_tool(
        &self,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_constraint_check", arguments)
            .await
    }

    #[tool(
        name = "solve_constraints",
        description = "Find feasible or optimized models for preflighted typed B-prime constraints."
    )]
    async fn solve_constraints_tool(
        &self,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_constraints", arguments)
            .await
    }

    #[tool(
        name = "solve_smt",
        description = "Solve a size-bounded, allowlisted SMT-LIB2 script."
    )]
    async fn solve_smt_tool(&self, arguments: JsonObject) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("solve_smt", arguments).await
    }

    #[tool(
        name = "get_solve_result",
        description = "Reload a persisted solver result by its solve_id."
    )]
    async fn get_solve_result_tool(
        &self,
        arguments: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("get_solve_result", arguments)
            .await
    }

    #[tool(
        name = "skill_search",
        description = "Search the current repository's eligible Explore skill catalog."
    )]
    async fn skill_search_tool(&self, arguments: JsonObject) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("skill_search", arguments)
            .await
    }

    #[tool(
        name = "skill_read",
        description = "Read exact verified text for an eligible Explore skill reference."
    )]
    async fn skill_read_tool(&self, arguments: JsonObject) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("skill_read", arguments)
            .await
    }

    #[tool(
        name = "skill_navigate",
        description = "Navigate the eligible Explore skill PageIndex via FTS or one-hop tree expand."
    )]
    async fn skill_navigate_tool(&self, arguments: JsonObject) -> Result<CallToolResult, McpError> {
        self.call_worker_registry_tool("skill_navigate", arguments)
            .await
    }

    fn context_from_request(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<WorkerCallContext, McpError> {
        let parts = context
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(|| {
                McpError::internal_error(
                    "missing HTTP request parts extension for worker auth context",
                    None,
                )
            })?;
        let auth_ctx = parts
            .extensions
            .get::<AuthenticatedWorkerContext>()
            .ok_or_else(|| {
                McpError::new(
                    rmcp::model::ErrorCode(-32001),
                    "unauthorized: missing worker delegation auth context",
                    None,
                )
            })?;
        Ok(WorkerCallContext {
            delegation_id: auth_ctx.delegation_id.clone(),
            brain_session_id: auth_ctx.brain_session_id.clone(),
        })
    }

    async fn invoke_with_lifecycle<F, Fut, E>(
        &self,
        tool_name: &'static str,
        context: RequestContext<RoleServer>,
        read_audit_target: Option<Option<String>>,
        invoke: F,
    ) -> Result<CallToolResult, McpError>
    where
        F: FnOnce(WorkerCallContext) -> Fut,
        Fut: std::future::Future<Output = Result<Value, E>>,
        E: Into<McpError>,
    {
        let worker_ctx = self.context_from_request(&context)?;
        if worker_ctx.brain_session_id != self.brain_session_id {
            return Err(McpError::new(
                rmcp::model::ErrorCode(-32001),
                "unauthorized: token brain_session_id mismatch",
                None,
            ));
        }

        let delegation_id = worker_ctx.delegation_id.clone();
        let _active_guard = ActiveCallGuard::new(
            Arc::clone(&self.deps.active_delegations),
            &self.deps.peak_active_delegations,
        );
        let (abort_registration, _handler_registration) = self.deps.handler_aborts.register();
        let call_start = Instant::now();

        if let Some(target_issue_id) = read_audit_target {
            append_read_audit_entry(&self.deps, &delegation_id, tool_name, target_issue_id);
        }

        let result: Result<Value, McpError> =
            match futures::future::Abortable::new(invoke(worker_ctx), abort_registration).await {
                Ok(result) => result.map_err(Into::into),
                Err(_) => Err(McpError::internal_error(
                    "worker MCP handler force-aborted during server shutdown",
                    None,
                )),
            };
        let latency_ms = call_start.elapsed().as_millis() as u64;
        let is_error = result.is_err();
        record_call(&self.deps, &delegation_id, tool_name, latency_ms, is_error);

        result.map(structured_only_result)
    }

    #[tool(
        name = "get_issue",
        description = "Retrieve an issue from the configured project management backend (beads, GitHub, etc.).",
        input_schema = crate::tool_schemas::schema_object::<GetIssueParams>()
    )]
    async fn get_issue_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let issue_id = args.get("id").and_then(|v| v.as_str()).map(String::from);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "get_issue",
            context,
            Some(issue_id),
            move |_worker_ctx| async move {
                spur_pm::mcp::get_issue(deps.pm_service.as_ref(), args)
                    .await
                    .map_err(pm_mcp_error)
            },
        )
        .await
    }

    #[tool(
        name = "list_issues",
        description = "List issues from the configured project management backend with optional filters.",
        input_schema = crate::tool_schemas::schema_object::<ListIssuesParams>()
    )]
    async fn list_issues_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "list_issues",
            context,
            Some(None),
            move |_worker_ctx| async move {
                spur_pm::mcp::list_issues(deps.pm_service.as_ref(), args)
                    .await
                    .map_err(pm_mcp_error)
            },
        )
        .await
    }

    #[tool(
        name = "get_task_diff",
        description = "Get the full unified diff for a plan task. Use after get_plan_status shows tasks in awaiting_review, approved, rejected, or failed state. Returns the complete diff, worker branch name, task description, and summary for brain code review. Pass `attempt` to inspect prior iteration attempts (see entry.history).",
        input_schema = crate::tool_schemas::schema_object::<GetTaskDiffParams>()
    )]
    async fn get_task_diff_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "get_task_diff",
            context,
            Some(task_id),
            move |worker_ctx| async move {
                deps.worker_read_sink.get_task_diff(&worker_ctx, args).await
            },
        )
        .await
    }

    #[tool(
        name = "get_plan_status",
        description = "Get the current status of a submitted execution plan. Returns per-task status: pending (waiting for deps), ready, dispatched (running), completed, or failed. Non-blocking — returns immediately.",
        input_schema = crate::tool_schemas::schema_object::<GetPlanStatusParams>()
    )]
    async fn get_plan_status_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let plan_id = args
            .get("plan_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "get_plan_status",
            context,
            Some(plan_id),
            move |worker_ctx| async move {
                deps.worker_read_sink
                    .get_plan_status(&worker_ctx, args)
                    .await
            },
        )
        .await
    }

    #[tool(
        name = "fetch_outcome_artifact",
        description = "Fetch the side-channel artifact (full or sectioned) for a completed delegation. Use when continuation.payload.artifact_id is Some(_) and you need fuller context. Sections let you pick what to fetch: pass 'status_only' for just status fields (~100B), 'summary' for the inline summary, 'diff_only' for full diff text, or 'full' for the entire DelegationResult JSON.",
        input_schema = crate::tool_schemas::schema_object::<FetchOutcomeArtifactParams>()
    )]
    async fn fetch_outcome_artifact_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let delegation_id = args
            .get("delegation_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "fetch_outcome_artifact",
            context,
            Some(delegation_id),
            move |worker_ctx| async move {
                deps.worker_read_sink
                    .fetch_outcome_artifact(&worker_ctx, args)
                    .await
            },
        )
        .await
    }

    #[tool(
        name = "code_resolve",
        description = "Resolve a code selector against the current worktree graph artifact and return only candidate rows. Use before code_subgraph when a selector may be ambiguous.",
        input_schema = crate::tool_schemas::schema_object::<CodeResolveParams>()
    )]
    async fn code_resolve_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_resolve",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_resolve", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_file_symbols",
        description = "List code symbols declared in one worktree-relative file from the current graph artifact. Rejects absolute paths and paths containing '..'. Supports response_format=full|compact|table.",
        input_schema = crate::tool_schemas::schema_object::<CodeFileSymbolsParams>()
    )]
    async fn code_file_symbols_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_file_symbols",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_file_symbols", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_symbol_info",
        description = "Resolve one code symbol and return metadata only. Ambiguous selectors return candidate rows.",
        input_schema = crate::tool_schemas::schema_object::<CodeSymbolInfoParams>()
    )]
    async fn code_symbol_info_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_symbol_info",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_symbol_info", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_read_symbol",
        description = "Read the indexed source for one code symbol from the current graph artifact. Select by stable_symbol_id, or by the exact worktree-relative path plus symbol name. Supports response_format=full|compact|source.",
        input_schema = crate::tool_schemas::schema_object::<CodeReadSymbolParams>()
    )]
    async fn code_read_symbol_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_read_symbol",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_read_symbol", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_callers",
        description = "List symbols that call the requested code symbol from the current worktree graph artifact. Unresolved rows are hidden by default; counts_by_kind and unresolved_sample summarize filtered rows. Supports response_format=full|compact|table.",
        input_schema = crate::tool_schemas::schema_object::<CodeSymbolParams>()
    )]
    async fn code_callers_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_callers",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_callers", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_callees",
        description = "List symbols called by the requested code symbol from the current worktree graph artifact. Unresolved rows are hidden by default; counts_by_kind and unresolved_sample summarize filtered rows. Supports response_format=full|compact|table.",
        input_schema = crate::tool_schemas::schema_object::<CodeSymbolParams>()
    )]
    async fn code_callees_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_callees",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_callees", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_symbol_search",
        description = "Search the worktree graph artifact for symbols by NAME (exact/prefix/substring). Lexical retrieval over symbol identifiers, not content. For concept/content retrieval over docs + code bodies, use code_semantic_search. Legacy alias: code_search.",
        input_schema = crate::tool_schemas::schema_object::<CodeSearchParams>()
    )]
    async fn code_search_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_symbol_search",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_symbol_search", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_subgraph",
        description = "Get a budgeted code-symbol subgraph from the current worktree graph artifact. Returns JSON nodes/edges by default, or Mermaid when format=mermaid. Supports response_format=full|compact|table for JSON output.",
        input_schema = crate::tool_schemas::schema_object::<CodeSubgraphParams>()
    )]
    async fn code_subgraph_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_subgraph",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_subgraph", args).await
            },
        )
        .await
    }

    #[tool(
        name = "code_symbol_history",
        description = "Return the causal trace of a code symbol across commits, including ChangeKind and snapshot key for each touch. Requires a temporal commit index in the current worktree.",
        input_schema = crate::tool_schemas::schema_object::<CodeSymbolHistoryParams>()
    )]
    async fn code_symbol_history_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "code_symbol_history",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_code_graph_for_worker(deps, worker_ctx, "code_symbol_history", args).await
            },
        )
        .await
    }

    #[tool(
        name = "knowledge_context_pack",
        description = "Deprecated alias for knowledge_context_pack_2; routes to v2 behavior. Use knowledge_context_pack_2 as the first-class canonical evidence pack.",
        input_schema = crate::tool_schemas::schema_object::<KnowledgeContextPackV2Params>()
    )]
    async fn knowledge_context_pack_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "knowledge_context_pack",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_knowledge_context_for_worker(deps, worker_ctx, args).await
            },
        )
        .await
    }

    #[tool(
        name = "knowledge_context_pack_2",
        description = "First-class canonical evidence pack that preserves knowledge_context_pack retrieval and exact grounding while adding graph_paths, risk_scorecard, community_context, temporal_context, and caveats for graph reasoning.",
        input_schema = crate::tool_schemas::schema_object::<KnowledgeContextPackV2Params>()
    )]
    async fn knowledge_context_pack_2_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "knowledge_context_pack_2",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_knowledge_context_for_worker(deps, worker_ctx, args).await
            },
        )
        .await
    }

    #[tool(
        name = "query",
        description = "Execute read-only DuckDB SQL against the assigned worktree's .spur/analyst.duckdb and return columns, rows, row_count, and truncation metadata.",
        input_schema = crate::tool_schemas::schema_object::<WorkerQueryParams>()
    )]
    async fn query_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle("query", context, Some(None), move |worker_ctx| async move {
            invoke_query_for_worker(deps, worker_ctx, args).await
        })
        .await
    }

    // `update_issue` is intentionally NOT exposed on the worker MCP. Workers
    // must not mutate issue state directly — it races with brain-side updates
    // and has hung delegations in the past. Workers should emit `report_signal`
    // or `report_progress` and let the brain reconcile issue state. Evidence
    // uses the append-only `report_audit` surface, not generic PM mutation.

    #[tool(
        name = "report_audit",
        description = "Append durable evidence to your currently dispatched issue without changing lifecycle state.",
        input_schema = crate::tool_schemas::schema_object::<crate::mcp::worker_evidence::ReportAuditArgs>()
    )]
    async fn report_audit_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "report_audit",
            context,
            None,
            move |worker_ctx| async move {
                if !deps
                    .delegations
                    .lock()
                    .contains_key(&worker_ctx.delegation_id)
                {
                    return Err(McpHandlerError::Unauthorized(
                        "worker delegation is no longer active".into(),
                    ));
                }
                if !deps
                    .feature_gate
                    .has(spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED)
                {
                    return Err(McpHandlerError::Unauthorized(
                        "report_audit requires Beads advanced".into(),
                    ));
                }
                crate::mcp::worker_evidence::report_audit(
                    deps.pm_service.as_ref(),
                    &worker_ctx,
                    args,
                )
                .await
            },
        )
        .await
    }

    #[tool(
        name = "doc_navigate",
        description = "Navigate indexed documentation sections (metadata + ~200-char lede only; never full body). Without root, BM25 FTS over section body_text in the Lance sidecar. With root, one-hop child sections via Contains order. Each hit includes next[] — follow code_read_symbol(stable_symbol_id) for the full section body.",
        input_schema = crate::tool_schemas::schema_object::<DocNavigateParams>()
    )]
    async fn doc_navigate_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "doc_navigate",
            context,
            Some(None),
            move |worker_ctx| async move {
                invoke_doc_navigate_for_worker(deps, worker_ctx, args).await
            },
        )
        .await
    }

    #[tool(
        name = "report_signal",
        description = "Worker-facing. Record a typed WorkerSignal on a task. Brain-side watcher will inspect and may mutate the plan.",
        input_schema = crate::mcp::signals::report_signal_input_schema()
    )]
    async fn report_signal_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "report_signal",
            context,
            None,
            move |worker_ctx| async move {
                if !deps
                    .delegations
                    .lock()
                    .contains_key(&worker_ctx.delegation_id)
                {
                    return Err(McpHandlerError::Unauthorized(
                        "worker delegation is no longer active".into(),
                    ));
                }
                deps.worker_signal_sink
                    .report_signal(&worker_ctx, args)
                    .await
            },
        )
        .await
    }

    #[tool(
        name = "submit_review_verdict",
        description = "Reviewer-only. Submit one authenticated durable verdict for the current maker completion bound to this reviewer delegation.",
        input_schema = crate::tool_schemas::schema_object::<SubmitReviewVerdictParams>()
    )]
    async fn submit_review_verdict_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "submit_review_verdict",
            context,
            None,
            move |worker_ctx| async move {
                crate::mcp::review_verdict::submit_review_verdict(
                    deps.pm_service.as_ref(),
                    &worker_ctx,
                    args,
                )
                .await
            },
        )
        .await
    }

    #[tool(
        name = "report_progress",
        description = "Worker-facing fire-and-forget progress emission. Sends a free-form `message` (and optional `percent`) to the brain as a `WorkerReportProgress` event. The handler returns `{ok: true}` on accept; the side effect IS the event. No PM writes, no audit sentinel — distinct from `report_signal` (which persists). Workers stream rich progress text without minting structured milestone names. Consumers (TUI / dashboards) decide how to render `percent` (no clamping).",
        input_schema = crate::tool_schemas::schema_object::<ReportProgressParams>()
    )]
    async fn report_progress_tool(
        &self,
        arguments: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = Value::Object(arguments);
        let deps = Arc::clone(&self.deps);
        self.invoke_with_lifecycle(
            "report_progress",
            context,
            None,
            move |worker_ctx| async move {
                let delegation_ctx = deps
                    .delegations
                    .lock()
                    .get(&worker_ctx.delegation_id)
                    .copied()
                    .unwrap_or_default();
                if !delegation_ctx.enable_worker_progress {
                    Ok(json!({ "ok": true }))
                } else {
                    deps.worker_signal_sink
                        .report_progress(&worker_ctx, args)
                        .await
                }
            },
        )
        .await
    }
}

fn structured_only_result(value: Value) -> CallToolResult {
    let mut result = CallToolResult::success(Vec::new());
    result.structured_content = Some(value);
    result.is_error = Some(false);
    result
}

fn unconfigured_context_service() -> ContextServiceConfig {
    ContextServiceConfig {
        url: String::new(),
        ..ContextServiceConfig::default()
    }
}

fn worker_rmcp_tool(definition: spur_mcp::tools::ToolDefinition) -> Tool {
    let notebook_read = crate::worker_notebook::READ_TOOLS.contains(&definition.name.as_str());
    let mut tool = Tool::new(
        definition.name,
        definition.description,
        rmcp_object(definition.input_schema),
    );
    if notebook_read {
        tool.annotations = Some(rmcp::model::ToolAnnotations::new().read_only(true));
    }
    tool
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for WorkerToolHandler {
    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if self.context_service_client.is_none()
            && crate::mcp::is_context_service_tool_name(&request.name)
        {
            return Err(McpError::internal_error(
                "context service is not configured for worker MCP".to_owned(),
                None,
            ));
        }
        let canonical_name = self
            .worker_registry()?
            .canonical_name_for_call(&request.name)?
            .to_owned();
        request.name = Cow::Owned(canonical_name.clone());
        if let Some(tool_name) = crate::worker_notebook::READ_TOOLS
            .into_iter()
            .find(|name| *name == canonical_name)
        {
            let deps = Arc::clone(&self.deps);
            let arguments = Value::Object(request.arguments.take().unwrap_or_default());
            return self
                .invoke_with_lifecycle(
                    tool_name,
                    context,
                    Some(None),
                    move |worker_ctx| async move {
                        let reader = deps
                            .notebook_readers
                            .lock()
                            .get(&worker_ctx.delegation_id)
                            .cloned()
                            .ok_or_else(crate::worker_notebook::not_authorized)?;
                        reader.call(tool_name, arguments).await
                    },
                )
                .await;
        }
        if !canonical_name.starts_with("external_")
            && request
                .arguments
                .as_ref()
                .is_some_and(|arguments| arguments.contains_key("project"))
        {
            return Err(McpError::invalid_params(
                "field `project` is not available on this MCP server".to_owned(),
                None,
            ));
        }
        if canonical_name.starts_with("external_") {
            let client = self.context_service_client.as_ref().ok_or_else(|| {
                McpError::internal_error(
                    "context service is not configured for worker MCP".to_owned(),
                    None,
                )
            })?;
            let args = Value::Object(request.arguments.take().unwrap_or_default());
            let value = client.call_value(&canonical_name, args).await?;
            return Ok(structured_only_result(value));
        }
        let tcc = RmcpToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self
                .worker_registry()?
                .list_tools()
                .into_iter()
                .map(worker_rmcp_tool)
                .collect(),
            next_cursor: None,
            meta: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.worker_registry()
            .ok()?
            .tool_definition_for_call(name)
            .map(worker_rmcp_tool)
    }

    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Use these tools to inspect assigned issues and report worker progress/signals.".into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();

        let mut implementation = Implementation::default();
        implementation.name = WORKER_MCP_SERVER_NAME.into();
        implementation.version = env!("CARGO_PKG_VERSION").into();
        info.server_info = implementation;
        info
    }
}

/// Errors returned by [`WorkerMcpServer::start`].
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("worker MCP listener bind failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors returned by [`WorkerMcpServer::flush_delegation`]. The flush is
/// best-effort: a failure here means the audit aggregate could not be
/// queued, but the per-delegation summary funnel event is still emitted.
#[derive(Debug, thiserror::Error)]
pub enum FlushDelegationError {
    /// The background audit flusher's receiver has been dropped (the server
    /// is mid-shutdown). Aggregated read-tool audits queued for this
    /// delegation cannot be persisted to the PM.
    #[error("read-audit flush channel closed (server shutting down)")]
    ChannelClosed,
}

/// Outcome of [`WorkerMcpServer::shutdown`]'s drain phase. `drained` is `true`
/// when every in-flight dispatcher exited before the caller-supplied deadline;
/// `false` when the deadline elapsed with at least one dispatcher still in
/// flight. `active_at_deadline` is the snapshot of [`WorkerMcpServer::active_count`]
/// taken at the moment the drain loop bailed (always `0` when `drained` is
/// `true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownOutcome {
    pub drained: bool,
    pub active_at_deadline: u32,
}

fn unauthorized_response() -> Response<Body> {
    const BODY: &str =
        r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid Request"},"id":null}"#;
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(BODY))
        .expect("valid unauthorized response")
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let trimmed = value.trim_start();
            if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("Bearer") {
                let rest = trimmed[6..].trim_start();
                if rest.is_empty() {
                    None
                } else {
                    Some(rest.to_string())
                }
            } else {
                None
            }
        })
}

fn extract_query_token(uri: &axum::http::Uri) -> Option<String> {
    uri.query().and_then(|query| {
        query.split('&').find_map(|part| {
            let (k, v) = part.split_once('=')?;
            if k == "token" {
                Some(v.to_string())
            } else {
                None
            }
        })
    })
}

async fn worker_auth_middleware(
    State(state): State<WorkerAuthMiddlewareState>,
    mut request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let session_id = request
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // Requests carrying `Mcp-Session-Id` are trusted ONLY when the id exists
    // in this server's LocalSessionManager and maps to a token-derived
    // delegation context captured at session-open time.
    if let Some(session_id) = session_id {
        let has_session = state
            .session_manager
            .has_session(&SessionId::from(session_id.clone()))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    ?error,
                    session_id = %session_id,
                    "WorkerMcp AuthDenied: failed to verify session id"
                );
                false
            });
        if has_session {
            let auth_ctx = { state.session_contexts.lock().get(&session_id).cloned() };
            if let Some(auth_ctx) = auth_ctx {
                request.extensions_mut().insert(auth_ctx);
                return next.run(request).await;
            }
            tracing::warn!(
                session_id = %session_id,
                "WorkerMcp AuthDenied: session id exists but has no delegation binding"
            );
            return unauthorized_response();
        }
    }

    let token =
        extract_bearer_token(request.headers()).or_else(|| extract_query_token(request.uri()));
    let token = match token {
        Some(token) => token,
        None => {
            tracing::warn!("WorkerMcp AuthDenied: missing token on session-opening request");
            return unauthorized_response();
        }
    };

    let payload = match validate_token(&state.hmac_key, &token, /*skew_tolerance_secs=*/ 30) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(?error, "WorkerMcp AuthDenied: invalid token");
            return unauthorized_response();
        }
    };

    if payload.b != state.brain_session_id {
        tracing::warn!(
            token_brain_session_id = %payload.b,
            expected_brain_session_id = %state.brain_session_id,
            "WorkerMcp AuthDenied: token brain_session_id mismatch"
        );
        return unauthorized_response();
    }

    let auth_ctx = AuthenticatedWorkerContext {
        delegation_id: payload.d,
        brain_session_id: payload.b,
    };
    request.extensions_mut().insert(auth_ctx.clone());

    let response = next.run(request).await;
    if let Some(session_id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let has_session = state
            .session_manager
            .has_session(&SessionId::from(session_id.to_string()))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    ?error,
                    session_id = %session_id,
                    "WorkerMcp AuthDenied: failed to verify minted session id"
                );
                false
            });
        if has_session {
            state
                .session_contexts
                .lock()
                .insert(session_id.to_string(), auth_ctx);
        } else {
            tracing::warn!(
                session_id = %session_id,
                "WorkerMcp AuthDenied: response carried unknown session id"
            );
        }
    }
    response
}

impl WorkerMcpServer {
    /// Bind a fresh TCP listener on `127.0.0.1:0`, generate an in-process HMAC
    /// key from the OS RNG, and spawn the accept loop. Returns once the
    /// listener is bound and ready to accept connections.
    pub async fn start(
        brain_session_id: String,
        deps: WorkerMcpDeps,
    ) -> Result<Arc<Self>, BindError> {
        Self::start_with_configs(
            brain_session_id,
            deps,
            WorkerMcpServerConfig::default(),
            unconfigured_context_service(),
        )
        .await
    }

    /// Start a worker MCP server that advertises and proxies context-service
    /// tools using the supplied configuration.
    ///
    /// The worker MCP and its context-service client stay in the orchestrator
    /// process. The supplied configuration is retained there and is not added
    /// to the delegated agent's launch environment. `SPUR_CONTEXT_SERVICE_URL`
    /// and `SPUR_CONTEXT_SERVICE_TOKEN` on the orchestrator process take
    /// precedence over `context_service_config`. Those process-wide variables
    /// follow normal OS environment inheritance, so operators requiring token
    /// isolation from delegated subprocesses should use `[context_service]`
    /// configuration instead. For context-service access, the agent receives
    /// this server's loopback URL and a delegation-scoped HMAC token. A missing
    /// URL or locally rejected client configuration is fail-soft: the server
    /// still starts, but does not advertise the `external_*` tools.
    pub async fn start_with_context_service_config(
        brain_session_id: String,
        deps: WorkerMcpDeps,
        context_service_config: ContextServiceConfig,
    ) -> Result<Arc<Self>, BindError> {
        Self::start_with_context_service_config_and_families(
            brain_session_id,
            deps,
            context_service_config,
            crate::mcp::McpFamily::ALL.iter().copied().collect(),
        )
        .await
    }

    pub async fn start_with_context_service_config_and_families(
        brain_session_id: String,
        deps: WorkerMcpDeps,
        context_service_config: ContextServiceConfig,
        mcp_families: std::collections::BTreeSet<crate::mcp::McpFamily>,
    ) -> Result<Arc<Self>, BindError> {
        Self::start_with_configs(
            brain_session_id,
            deps,
            WorkerMcpServerConfig {
                mcp_families,
                ..WorkerMcpServerConfig::default()
            },
            context_service_config,
        )
        .await
    }

    /// Bind a fresh TCP listener on `127.0.0.1:0`, generate an in-process HMAC
    /// key from the OS RNG, spawn the accept loop, and spawn the background
    /// audit flusher task. Returns once the listener is bound and ready to
    /// accept connections.
    pub async fn start_with_config(
        brain_session_id: String,
        deps: WorkerMcpDeps,
        config: WorkerMcpServerConfig,
    ) -> Result<Arc<Self>, BindError> {
        Self::start_with_configs(
            brain_session_id,
            deps,
            config,
            unconfigured_context_service(),
        )
        .await
    }

    async fn start_with_configs(
        brain_session_id: String,
        deps: WorkerMcpDeps,
        config: WorkerMcpServerConfig,
        context_service_config: ContextServiceConfig,
    ) -> Result<Arc<Self>, BindError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let mut hmac_key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut hmac_key);

        let delegations = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let delegation_worktree_roots =
            Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let read_audit_buffers =
            Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let (flush_tx, flush_rx) = mpsc::unbounded_channel::<FlushMessage>();
        let active_delegations = Arc::new(AtomicU32::new(0));
        let peak_active_delegations = Arc::new(AtomicU32::new(0));
        let delegation_guards = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let graph_mcp_module =
            spur_graph::mcp::GraphMcpModule::new(spur_graph::mcp::GraphMcpDeps::default());
        let dispatcher_deps = Arc::new(DispatcherDeps {
            embedding_runtime: spur_analyst::embedding::OwnedEmbeddingRuntime::new(),
            pm_service: deps.pm_service,
            feature_gate: deps.feature_gate,
            funnel: deps.funnel,
            worker_signal_sink: deps.worker_signal_sink,
            worker_read_sink: deps.worker_read_sink,
            repo_root: deps.repo_root,
            delegations: Arc::clone(&delegations),
            delegation_worktree_roots: Arc::clone(&delegation_worktree_roots),
            notebook_readers: parking_lot::Mutex::new(std::collections::HashMap::new()),
            graph_mcp_module,
            read_audit_buffers: Arc::clone(&read_audit_buffers),
            flush_tx,
            active_delegations: Arc::clone(&active_delegations),
            peak_active_delegations: Arc::clone(&peak_active_delegations),
            handler_aborts: Arc::new(HandlerAbortRegistry::default()),
            delegation_guards: Arc::clone(&delegation_guards),
        });

        let shutdown = CancellationToken::new();
        let connection_force_shutdown = CancellationToken::new();
        let connections = Arc::new(ConnectionTracker::default());
        let session_manager = Arc::new(LocalSessionManager::default());
        let session_contexts = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let (handler, worker_mcp_tool_names) = WorkerToolHandler::new(
            Arc::clone(&dispatcher_deps),
            brain_session_id.clone(),
            &context_service_config,
            &config.mcp_families,
        );
        let handler = Arc::new(handler);
        let server = Arc::new(Self {
            addr,
            hmac_key,
            brain_session_id: brain_session_id.clone(),
            deps: Arc::clone(&dispatcher_deps),
            shutdown: shutdown.clone(),
            connection_force_shutdown: connection_force_shutdown.clone(),
            connections: Arc::clone(&connections),
            session_manager: Arc::clone(&session_manager),
            session_contexts: Arc::clone(&session_contexts),
            accept_loop_handle: Mutex::new(None),
            flusher_handle: Mutex::new(None),
            flush_rx: Mutex::new(Some(flush_rx)),
            active_delegations,
            peak_active_delegations,
            worker_mcp_tool_names,
        });
        let service = {
            let handler = Arc::clone(&handler);
            let mut streamable_config = StreamableHttpServerConfig::default();
            streamable_config.stateful_mode = true;
            streamable_config.cancellation_token = shutdown.clone();
            StreamableHttpService::new(
                move || Ok(Arc::clone(&handler)),
                Arc::clone(&session_manager),
                streamable_config,
            )
        };
        let auth_state = WorkerAuthMiddlewareState {
            hmac_key,
            brain_session_id: brain_session_id.clone(),
            session_manager,
            session_contexts,
        };
        let router =
            Router::new()
                .nest_service("/mcp", service)
                .layer(middleware::from_fn_with_state(
                    auth_state,
                    worker_auth_middleware,
                ));

        let listener = TrackedTcpListener {
            inner: listener,
            connections,
            force_shutdown: connection_force_shutdown,
        };
        let serve_shutdown = shutdown.clone();
        let accept_handle = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router)
                .with_graceful_shutdown(serve_shutdown.cancelled_owned())
                .await
            {
                tracing::debug!(%error, "worker MCP streamable HTTP server exited");
            }
        });
        *server.accept_loop_handle.lock() = Some(accept_handle);

        let flush_rx = server
            .flush_rx
            .lock()
            .take()
            .expect("flush_rx always present at start");
        let flusher_handle = tokio::spawn(audit_flusher_task(
            Arc::clone(&dispatcher_deps),
            flush_rx,
            shutdown.clone(),
            config,
        ));
        *server.flusher_handle.lock() = Some(flusher_handle);

        Ok(server)
    }

    /// Returns Claude-format names for this server's advertised tools.
    pub(crate) fn claude_tool_names(&self) -> &[String] {
        &self.worker_mcp_tool_names
    }

    /// The canonical `http://127.0.0.1:<port>/mcp` URL workers POST JSON-RPC
    /// requests to.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// Issue a time-limited bearer token embedding `delegation_id` and this
    /// server's `brain_session_id`. Workers present the token either in the
    /// `Authorization: Bearer <token>` header or as a `?token=<token>` query
    /// parameter.
    pub fn issue_token(&self, delegation_id: &str, ttl: Duration) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let payload = spur_mcp::token::WorkerTokenPayload {
            d: delegation_id.to_string(),
            b: self.brain_session_id.clone(),
            e: now + ttl.as_secs(),
        };
        spur_mcp::token::encode_token(&self.hmac_key, &payload)
            .expect("HMAC key length is valid for HmacSha256")
    }

    /// Issue a token with an explicit expiry timestamp (unix seconds).
    /// Primarily useful for tests that need expired tokens.
    #[cfg(any(test, feature = "test-support"))]
    pub fn issue_token_with_expiry(&self, delegation_id: &str, expiry_secs: u64) -> String {
        let payload = spur_mcp::token::WorkerTokenPayload {
            d: delegation_id.to_string(),
            b: self.brain_session_id.clone(),
            e: expiry_secs,
        };
        spur_mcp::token::encode_token(&self.hmac_key, &payload)
            .expect("HMAC key length is valid for HmacSha256")
    }

    /// Register (or update) cached per-delegation context. Called by the
    /// orchestrator at dispatch time so the `report_progress` dual-gate is
    /// a PM-free HashMap lookup.
    pub fn register_delegation(&self, delegation_id: String, ctx: DelegationContext) {
        let mut map = self.deps.delegations.lock();
        map.insert(delegation_id.clone(), ctx);
        // Ensure a summary guard exists for this delegation.
        let mut guards = self.deps.delegation_guards.lock();
        guards.entry(delegation_id.clone()).or_insert_with(|| {
            DelegationDispatchGuard::new(
                delegation_id,
                self.brain_session_id.clone(),
                Arc::clone(&self.deps.funnel),
            )
        });
    }

    /// Register the worktree root a delegated worker is running in. Code graph
    /// tools use this root for freshness checks because this HTTP server runs
    /// in the brain process rather than inside the worker process.
    pub fn register_delegation_worktree_root(&self, delegation_id: String, worktree_root: PathBuf) {
        self.deps
            .delegation_worktree_roots
            .lock()
            .insert(delegation_id.clone(), worktree_root);
        let mut guards = self.deps.delegation_guards.lock();
        guards.entry(delegation_id.clone()).or_insert_with(|| {
            DelegationDispatchGuard::new(
                delegation_id,
                self.brain_session_id.clone(),
                Arc::clone(&self.deps.funnel),
            )
        });
    }

    /// Bind notebook context before worker startup. Only this server owns the
    /// daemon socket; worker MCP configuration still contains just its HTTP URL.
    pub(crate) fn register_notebook_context(
        &self,
        delegation_id: &str,
        repo_root: &std::path::Path,
        context_files: &[String],
        task: &str,
    ) -> std::io::Result<String> {
        // Failed or empty rebinds must never retain an earlier capability.
        self.revoke_notebook_reader(delegation_id);
        let grants = crate::worker_notebook::context_grants(repo_root, context_files)?;
        let task = crate::worker_notebook::with_notebook_context(task, &grants);
        if !grants.is_empty() {
            let nonce = crate::notebook::stable_notebook_nonce(repo_root);
            let reader = Arc::new(crate::worker_notebook::NotebookReader::new(
                crate::notebook::control_socket_path(&nonce),
                grants,
            ));
            self.register_notebook_reader(delegation_id.to_owned(), reader);
        }
        Ok(task)
    }

    fn register_notebook_reader(
        &self,
        delegation_id: String,
        reader: Arc<crate::worker_notebook::NotebookReader>,
    ) {
        if let Some(previous) = self
            .deps
            .notebook_readers
            .lock()
            .insert(delegation_id, reader)
        {
            previous.revoke();
        }
    }

    fn revoke_notebook_reader(&self, delegation_id: &str) {
        if let Some(reader) = self.deps.notebook_readers.lock().remove(delegation_id) {
            reader.revoke();
        }
    }

    /// Signal that a delegation has reached a terminal state. Removes the
    /// cached context and drops the per-delegation summary guard, which emits
    /// one `WorkerMcpDelegationSummary` event. The `_outcome` parameter is
    /// retained for API stability; per-call error counts in the summary now
    /// come from the dispatcher's `record_call` telemetry.
    pub fn complete_delegation(&self, delegation_id: &str, _outcome: &str) {
        self.revoke_notebook_reader(delegation_id);
        self.deps.delegations.lock().remove(delegation_id);
        self.deps
            .delegation_worktree_roots
            .lock()
            .remove(delegation_id);
        self.deps.read_audit_buffers.lock().remove(delegation_id);
        if let Some(guard) = self.deps.delegation_guards.lock().remove(delegation_id) {
            guard.completed.store(true, Ordering::Relaxed);
            // Explicit drop to trigger the summary emission.
            drop(guard);
        }
    }

    /// Phase 5 / Task 27. Drain the per-delegation read-tool audit aggregator
    /// and emit the `WorkerMcpDelegationSummary` event for `delegation_id`.
    /// The orchestrator calls this immediately before emitting
    /// `DelegationCompleted` so downstream consumers observe the audit
    /// summary first.
    ///
    /// Idempotent: a second call (or a call for a delegation that was never
    /// registered, e.g. dispatched without `enable_worker_mcp`) is a no-op
    /// returning `Ok(())`.
    ///
    /// `outcome` is the audit-trail string (`"success"`, `"cancelled"`,
    /// `"rejected"`, or `"error"`). Only `"error"` flips the summary
    /// guard's outcome; the cleaner terminations (`"cancelled"`,
    /// `"rejected"`) preserve the guard's default success bit so the
    /// emitted `WorkerMcpDelegationSummary.outcome` stays `"success"`
    /// for those — but the audit-trail outcome string is propagated to
    /// callers that surface it elsewhere.
    ///
    /// Returns `Err(FlushDelegationError::ChannelClosed)` only when the
    /// background flusher's receiver has been dropped (server shutdown
    /// in progress). The summary event is still emitted in that case;
    /// callers should log the warning, emit a `WorkerMcpSubkind::FlushFailed`
    /// audit if they have an issue context, and continue.
    pub async fn flush_delegation(
        &self,
        delegation_id: &str,
        outcome: &str,
    ) -> Result<(), FlushDelegationError> {
        self.revoke_notebook_reader(delegation_id);
        self.deps.delegations.lock().remove(delegation_id);
        self.deps
            .delegation_worktree_roots
            .lock()
            .remove(delegation_id);

        // Drain the read-audit buffer in-band rather than relying on its Drop
        // so that send-failures surface to the caller as a Result.
        let entries = self
            .deps
            .read_audit_buffers
            .lock()
            .remove(delegation_id)
            .map(|buf| buf.take_entries())
            .unwrap_or_default();

        let mut flush_err: Option<FlushDelegationError> = None;
        if !entries.is_empty()
            && self
                .deps
                .flush_tx
                .send(FlushMessage {
                    delegation_id: delegation_id.to_string(),
                    entries,
                })
                .is_err()
        {
            flush_err = Some(FlushDelegationError::ChannelClosed);
        }

        // Drop the per-delegation summary guard regardless of flush outcome.
        // The summary event is part of the contract; losing it would leave
        // the funnel stream missing a sentinel.
        let removed_guard = self.deps.delegation_guards.lock().remove(delegation_id);
        if let Some(guard) = removed_guard {
            if outcome == "error" || flush_err.is_some() {
                guard.mark_error();
            }
            guard.completed.store(true, Ordering::Relaxed);
            drop(guard);
        }

        match flush_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Borrow the read-audit buffer for a delegation. Used by tests to assert
    /// the dispatcher correctly appends per read-tool call. Production callers
    /// (T21 background flusher) reach into `deps.read_audit_buffers` directly.
    #[cfg(any(test, feature = "test-support"))]
    pub fn peek_read_buffer(&self, delegation_id: &str) -> Option<Arc<ReadAuditBuffer>> {
        self.deps
            .read_audit_buffers
            .lock()
            .get(delegation_id)
            .cloned()
    }

    /// Take ownership of the flush-channel receiver. T21's background flusher
    /// calls this once at startup; subsequent calls return `None`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn take_flush_receiver(&self) -> Option<mpsc::UnboundedReceiver<FlushMessage>> {
        self.flush_rx.lock().take()
    }

    /// Inject a read-audit buffer directly into the map. Used by tests to set
    /// up buffer state without sending HTTP requests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn inject_read_buffer_for_test(&self, delegation_id: &str) -> Arc<ReadAuditBuffer> {
        let buf = Arc::new(ReadAuditBuffer::new(
            delegation_id.to_string(),
            self.deps.flush_tx.clone(),
        ));
        self.deps
            .read_audit_buffers
            .lock()
            .insert(delegation_id.to_string(), Arc::clone(&buf));
        buf
    }

    #[cfg(test)]
    pub(crate) fn replace_background_handles_for_test(
        &self,
        accept_loop: JoinHandle<()>,
        flusher: JoinHandle<()>,
    ) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        (
            self.accept_loop_handle.lock().replace(accept_loop),
            self.flusher_handle.lock().replace(flusher),
        )
    }

    #[cfg(test)]
    pub(crate) fn accept_loop_handle_taken_for_test(&self) -> bool {
        self.accept_loop_handle.lock().is_none()
    }

    #[cfg(test)]
    pub(crate) fn register_handler_lifecycle_for_test(&self) -> TestHandlerLifecycle {
        let active_guard = ActiveCallGuard::new(
            Arc::clone(&self.active_delegations),
            &self.peak_active_delegations,
        );
        let (abort_registration, handler_registration) = self.deps.handler_aborts.register();
        TestHandlerLifecycle {
            abort_registration: Some(abort_registration),
            _handler_registration: handler_registration,
            _active_guard: active_guard,
        }
    }

    /// Number of dispatchers currently in flight. Read by [`shutdown`] to
    /// drain in-flight requests; exposed publicly so callers and tests can
    /// observe pressure without reaching into `DispatcherDeps`.
    pub fn active_count(&self) -> u32 {
        self.active_delegations.load(Ordering::SeqCst)
    }

    /// High-water mark of [`active_count`](Self::active_count) over this
    /// server's lifetime — the max concurrent in-flight dispatcher count.
    /// Recorded atomically on each dispatch entry, so it captures every
    /// momentary overlap that time-sampled observation can miss. Used by
    /// concurrency tests to assert real parallelism deterministically.
    pub fn peak_active_count(&self) -> u32 {
        self.peak_active_delegations.load(Ordering::SeqCst)
    }

    /// Force-abort every in-flight worker tool handler and wait until each
    /// aborted future has unwound through its lifecycle guard.
    pub async fn force_abort_handlers_and_wait(&self) {
        self.deps.handler_aborts.abort_all();
        self.deps.handler_aborts.wait().await;
        while self.active_count() != 0 {
            tokio::task::yield_now().await;
        }
    }

    /// `true` while the accept loop is still running — i.e. [`shutdown`]
    /// has not been called and the cancellation token has not been
    /// triggered. Read by orchestrator cache code so a stale cached
    /// `Arc<WorkerMcpServer>` (e.g. one whose accept loop was aborted by
    /// a session retire) is evicted and rebooted on next ensure.
    pub fn is_running(&self) -> bool {
        !self.shutdown.is_cancelled()
    }

    async fn close_all_sessions(&self, close_window: Duration) {
        if close_window.is_zero() {
            return;
        }

        let session_ids: Vec<SessionId> = {
            let sessions = self.session_manager.sessions.read().await;
            sessions.keys().cloned().collect()
        };
        if session_ids.is_empty() {
            return;
        }

        let manager = Arc::clone(&self.session_manager);
        let session_contexts = Arc::clone(&self.session_contexts);
        let brain_session_id = self.brain_session_id.clone();
        let close_sessions = async move {
            for session_id in session_ids {
                if let Err(error) = manager.close_session(&session_id).await {
                    tracing::warn!(
                        %error,
                        session_id = %session_id,
                        brain_session_id = %brain_session_id,
                        "WorkerMcpServer close_session failed during shutdown drain"
                    );
                }
                session_contexts.lock().remove(session_id.as_ref());
            }
        };

        if tokio::time::timeout(close_window, close_sessions)
            .await
            .is_err()
        {
            tracing::warn!(
                brain_session_id = %self.brain_session_id,
                timeout_ms = close_window.as_millis() as u64,
                "WorkerMcpServer timed out while closing active sessions"
            );
        }
    }

    /// Fence new worker calls, force-abort in-flight handlers, and await all
    /// owned server tasks without entering a graceful drain window.
    pub async fn shutdown_immediately(self: Arc<Self>) {
        let force_shutdown = CancellationToken::new();
        force_shutdown.cancel();
        let _ = self
            .shutdown_until_forced(Duration::ZERO, force_shutdown)
            .await;
    }

    /// Cancel the accept loop, drain in-flight dispatchers up to `deadline`,
    /// then join the background audit flusher task. Returns a
    /// [`ShutdownOutcome`] describing whether the drain completed (`drained =
    /// true`) or the deadline elapsed with dispatchers still in flight
    /// (`drained = false`, `active_at_deadline > 0`).
    ///
    /// Drain semantics:
    /// 1. Handler registration closes monotonically, then the accept loop is
    ///    cancelled so the listener stops accepting new connections.
    /// 2. Active RMCP sessions are closed so long-lived SSE streams are
    ///    proactively terminated.
    /// 3. Existing dispatchers continue and decrement `active_delegations`
    ///    via [`ActiveCallGuard::drop`] as they return.
    /// 4. The drain loop polls [`active_count`](Self::active_count) every
    ///    `DRAIN_POLL_INTERVAL` (bounded — never busy-loops) until either the
    ///    counter reaches `0` or `deadline` elapses.
    /// 5. On deadline elapse a `WARN`-level log is emitted with the in-flight
    ///    count. The flusher task is still joined so its TaskTracker drains
    ///    and the deps `Arc` is released.
    ///
    /// The caller picks `deadline`; a typical default is `5s`. Callers that
    /// don't care about the outcome may discard the return value.
    /// Idempotent: a second call after shutdown finds the accept and flusher
    /// handles already taken and returns immediately with `drained = true`.
    pub async fn shutdown(self: Arc<Self>, deadline: Duration) -> ShutdownOutcome {
        self.shutdown_until_forced(deadline, CancellationToken::new())
            .await
    }

    /// Gracefully drain until `force_shutdown` is signalled, then immediately
    /// abort and await every handler/background task. The force transition is
    /// observed inside each wait so escalation never drops a future that owns
    /// a task handle.
    pub(crate) async fn shutdown_until_forced(
        self: Arc<Self>,
        deadline: Duration,
        force_shutdown: CancellationToken,
    ) -> ShutdownOutcome {
        let drain_deadline = Instant::now() + deadline;
        self.deps.handler_aborts.begin_closing();
        self.connections.begin_closing();
        self.shutdown.cancel();
        let mut forced = force_shutdown.is_cancelled();
        if forced {
            self.connection_force_shutdown.cancel();
        }
        if !forced && !deadline.is_zero() {
            let close_sessions = self.close_all_sessions(deadline);
            tokio::pin!(close_sessions);
            tokio::select! {
                biased;
                _ = force_shutdown.cancelled() => forced = true,
                _ = &mut close_sessions => {}
            }
        }

        // Listener cancellation + session close-all prevents new work and
        // terminates active SSE streams. Drain in-flight dispatchers, bounded
        // by `deadline`.
        let mut active_at_deadline = self.active_count();
        while !forced && active_at_deadline != 0 {
            if force_shutdown.is_cancelled() {
                forced = true;
                break;
            }
            if Instant::now() >= drain_deadline {
                tracing::warn!(
                    brain_session_id = %self.brain_session_id,
                    active = active_at_deadline,
                    deadline_ms = deadline.as_millis() as u64,
                    "WorkerMcpServer drain deadline elapsed with in-flight dispatchers"
                );
                break;
            }
            tokio::select! {
                biased;
                _ = force_shutdown.cancelled() => forced = true,
                _ = tokio::time::sleep(DRAIN_POLL_INTERVAL) => {
                    active_at_deadline = self.active_count();
                }
            }
        }

        let accept_handle = self.accept_loop_handle.lock().take();
        let flusher_handle = self.flusher_handle.lock().take();
        let connections = Arc::clone(&self.connections);
        let connection_force_shutdown = self.connection_force_shutdown.clone();
        let background = async {
            let accept = async {
                if let Some(handle) = accept_handle {
                    let wait = drain_deadline.saturating_duration_since(Instant::now());
                    await_or_force_connection_tasks_until_forced(
                        handle,
                        wait,
                        force_shutdown.clone(),
                        connection_force_shutdown,
                        connections,
                    )
                    .await;
                } else {
                    connections.wait().await;
                }
            };
            let flusher = async {
                if let Some(handle) = flusher_handle {
                    let wait = std::cmp::min(
                        Duration::from_secs(1),
                        drain_deadline.saturating_duration_since(Instant::now()),
                    );
                    await_or_abort_background_task_until_forced(
                        handle,
                        wait,
                        force_shutdown.clone(),
                    )
                    .await;
                }
            };
            tokio::join!(accept, flusher);
        };
        tokio::pin!(background);
        if forced {
            let handlers = self.force_abort_handlers_and_wait();
            tokio::join!(&mut background, handlers);
        } else {
            tokio::select! {
                biased;
                _ = force_shutdown.cancelled() => {
                    forced = true;
                    self.connection_force_shutdown.cancel();
                    let handlers = self.force_abort_handlers_and_wait();
                    tokio::join!(&mut background, handlers);
                }
                _ = &mut background => {}
            }
        }
        if force_shutdown.is_cancelled() {
            forced = true;
            self.connection_force_shutdown.cancel();
        }
        if forced {
            // The connection barrier prevents any request from registering
            // after this final handler snapshot.
            self.force_abort_handlers_and_wait().await;
        }
        if let Err(error) = self.deps.embedding_runtime.shutdown().await {
            tracing::error!(%error, brain_session_id = %self.brain_session_id, "worker embedding shutdown failed");
        }
        // Reference held only to keep the deps Arc alive until shutdown
        // completes; explicit drop documents intent.
        drop(self.deps.clone());
        if forced {
            ShutdownOutcome {
                drained: true,
                active_at_deadline: self.active_count(),
            }
        } else {
            ShutdownOutcome {
                drained: active_at_deadline == 0,
                active_at_deadline,
            }
        }
    }
}

impl Drop for WorkerMcpServer {
    fn drop(&mut self) {
        self.deps.handler_aborts.abort_all();
        self.connections.begin_closing();
        self.connection_force_shutdown.cancel();
        self.shutdown.cancel();
        if let Some(handle) = self.accept_loop_handle.lock().take() {
            handle.abort();
        }
        if let Some(handle) = self.flusher_handle.lock().take() {
            handle.abort();
        }
    }
}

/// How often [`WorkerMcpServer::shutdown`] re-checks `active_count` while
/// draining. Bounded so the polling loop never busy-spins, small enough that
/// a fast-completing dispatcher doesn't materially extend shutdown latency.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

async fn await_or_force_connection_tasks_until_forced(
    mut handle: JoinHandle<()>,
    wait: Duration,
    force_shutdown: CancellationToken,
    connection_force_shutdown: CancellationToken,
    connections: Arc<ConnectionTracker>,
) {
    let graceful = if wait.is_zero() || force_shutdown.is_cancelled() {
        false
    } else {
        tokio::select! {
            biased;
            _ = force_shutdown.cancelled() => false,
            _ = tokio::time::sleep(wait) => false,
            _ = &mut handle => true,
        }
    };

    if !graceful {
        connection_force_shutdown.cancel();
        handle.abort();
        let _ = handle.await;
    }
    connections.wait().await;
}

async fn await_or_abort_background_task_until_forced(
    mut handle: JoinHandle<()>,
    wait: Duration,
    force_shutdown: CancellationToken,
) {
    if !wait.is_zero() {
        tokio::select! {
            biased;
            _ = force_shutdown.cancelled() => {}
            _ = &mut handle => return,
            _ = tokio::time::sleep(wait) => {}
        }
    }
    handle.abort();
    let _ = handle.await;
}

/// Append a read-tool call to the per-delegation aggregation buffer. Lazily
/// creates the buffer on first call. Lock scope is intentionally tight — the
/// outer `read_audit_buffers` mutex is released before any `append` so a slow
/// `Drop` (channel send) never blocks an unrelated delegation.
fn append_read_audit_entry(
    deps: &DispatcherDeps,
    delegation_id: &str,
    tool_name: &str,
    target_issue_id: Option<String>,
) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let buf = {
        let mut map = deps.read_audit_buffers.lock();
        Arc::clone(map.entry(delegation_id.to_string()).or_insert_with(|| {
            Arc::new(ReadAuditBuffer::new(
                delegation_id.to_string(),
                deps.flush_tx.clone(),
            ))
        }))
    };
    buf.append(ReadAuditEntry {
        tool_name: tool_name.to_string(),
        target_issue_id,
        ts,
    });
}

/// Record one completed tool call on the per-delegation summary guard. No-op
/// if the guard has not been created yet (e.g., unregistered delegation).
fn record_call(
    deps: &DispatcherDeps,
    delegation_id: &str,
    tool_name: &str,
    latency_ms: u64,
    is_error: bool,
) {
    let map = deps.delegation_guards.lock();
    if let Some(guard) = map.get(delegation_id) {
        guard.record_call(tool_name, latency_ms, is_error);
    }
}

/// Maximum time to wait for an audit sentinel comment to be written before
/// giving up and returning success to the worker. Slow beads must not stall
/// the worker indefinitely.
#[allow(dead_code)]
const AUDIT_EMIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn a dedicated background task that owns the `flush_rx` half of the
/// read-audit flush channel. The task receives `FlushMessage`s (either from
/// buffer `Drop` or from the periodic idle scan) and writes aggregated
/// `ReadAggregate` sentinel comments to the PM backend. It also scans the
/// `read_audit_buffers` map every `scan_interval` and removes idle entries
/// (last entry `ts` older than `idle_threshold`).
async fn audit_flusher_task(
    deps: Arc<DispatcherDeps>,
    mut flush_rx: mpsc::UnboundedReceiver<FlushMessage>,
    shutdown: CancellationToken,
    config: WorkerMcpServerConfig,
) {
    let mut interval = tokio::time::interval(config.scan_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            Some(msg) = flush_rx.recv() => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = emit_read_aggregate(&deps, msg.delegation_id, msg.entries, &shutdown) => {},
                }
            }
            _ = interval.tick() => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = scan_and_flush_idle_buffers(&deps, &config, &shutdown) => {},
                }
            }
        }
    }
}

/// Scan the per-delegation read-audit buffer map and flush idle buffers.
///
/// Concurrency note: while the map lock is held, idle buffers are removed
/// and their entries drained. If a concurrent handler holds an `Arc` clone
/// and appends AFTER drain, the entry remains in the buffer until the
/// handler returns; `Drop` then sends a follow-up `FlushMessage`. This
/// produces a duplicate sentinel comment but no data loss in steady state.
///
/// TODO(T24): During shutdown, the flusher's recv loop exits before all
/// in-flight `Arc` clones have dropped. Drop-fired `FlushMessage`s after that
/// point are silently lost. T24's shutdown drain owns this window.
async fn scan_and_flush_idle_buffers(
    deps: &DispatcherDeps,
    config: &WorkerMcpServerConfig,
    shutdown: &CancellationToken,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let threshold = config.idle_threshold;

    let to_flush: Vec<(String, Vec<ReadAuditEntry>)> = {
        let mut map = deps.read_audit_buffers.lock();
        let mut result = Vec::new();

        let idle_ids: Vec<String> = map
            .iter()
            .filter(|(_, buf)| {
                let entries = buf.entries.lock();
                match entries.last() {
                    Some(e) => {
                        let entry_time = Duration::from_secs(e.ts);
                        now.saturating_sub(entry_time) > threshold
                    }
                    None => true,
                }
            })
            .map(|(id, _)| id.clone())
            .collect();

        for id in idle_ids {
            if let Some(buf) = map.remove(&id) {
                let entries = std::mem::take(&mut *buf.entries.lock());
                if !entries.is_empty() {
                    result.push((id, entries));
                }
            }
        }

        result
    };

    for (delegation_id, entries) in to_flush {
        if shutdown.is_cancelled() {
            break;
        }
        emit_read_aggregate(deps, delegation_id, entries, shutdown).await;
    }
}

/// Encode a `ReadAggregate` sentinel from the drained entries and write it
/// to the PM backend. The comment is placed on the first non-None
/// `target_issue_id` in the entry list; if none exists the flush is a no-op.
/// The flusher task remains cancellable via `shutdown` so it can exit promptly
/// even if PM I/O is slow.
async fn emit_read_aggregate(
    deps: &DispatcherDeps,
    delegation_id: String,
    entries: Vec<ReadAuditEntry>,
    shutdown: &CancellationToken,
) {
    if entries.is_empty() {
        return;
    }

    let issue_id = match entries.iter().find_map(|e| e.target_issue_id.clone()) {
        Some(id) => id,
        None => {
            tracing::warn!(
                delegation_id = %delegation_id,
                entry_count = entries.len(),
                tools = ?entries.iter().map(|e| &e.tool_name).collect::<Vec<_>>(),
                "ReadAggregate audit dropped: no entry has a target_issue_id (all read-only ops without explicit target)"
            );
            return;
        }
    };

    if let Err(error) = crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        &deps.feature_gate,
    ) {
        tracing::warn!(
            delegation_id = %delegation_id,
            issue_id = %issue_id,
            "ReadAggregate audit comment emission skipped: {error:?}"
        );
        return;
    }

    let kind = crate::plan::audit_sentinel::AuditSentinelKind::ReadAggregate {
        delegation_id,
        entries: entries
            .into_iter()
            .map(|e| crate::plan::audit_sentinel::ReadAggregateEntry {
                tool_name: e.tool_name,
                target_issue_id: e.target_issue_id,
                ts: e.ts,
            })
            .collect(),
    };
    let body = crate::plan::audit_sentinel::encode_comment(&kind);

    let Some(adv) = deps.pm_service.advanced() else {
        return;
    };

    tokio::select! {
        biased;
        _ = shutdown.cancelled() => {}
        _ = adv.add_comment(&issue_id, &body) => {}
    }
}

/// Emit a `[[spur-audit v1]] WorkerWrite` sentinel comment on the issue that
/// was just mutated by a worker write tool. Called synchronously before the
/// handler result is returned to the worker so the audit trail is durable even
/// if the worker process dies immediately after the call.
#[allow(dead_code)]
async fn emit_worker_write_audit(
    pm: &spur_pm::PmService,
    feature_gate: &spur_license::FeatureGate,
    delegation_id: &str,
    tool: &str,
    issue_id: &str,
) {
    if let Err(error) = crate::server::require_feature(
        spur_license::FeatureKey::PM_PRO_BEADS_ADVANCED,
        feature_gate,
    ) {
        tracing::warn!(
            delegation_id = %delegation_id,
            issue_id = %issue_id,
            tool = %tool,
            "WorkerWrite audit comment emission skipped: {error:?}"
        );
        return;
    }
    emit_worker_write_audit_inner(pm.advanced(), delegation_id, tool, issue_id).await;
}

#[allow(dead_code)]
async fn emit_worker_write_audit_inner(
    advanced: Option<&dyn spur_pm::BeadsAdvanced>,
    delegation_id: &str,
    tool: &str,
    issue_id: &str,
) {
    let Some(adv) = advanced else {
        return;
    };
    let kind = crate::plan::audit_sentinel::AuditSentinelKind::WorkerWrite {
        delegation_id: delegation_id.to_string(),
        tool: tool.to_string(),
        issue_id: issue_id.to_string(),
    };
    let body = crate::plan::audit_sentinel::encode_comment(&kind);
    let emit_fut = adv.add_comment(issue_id, &body);
    match tokio::time::timeout(AUDIT_EMIT_TIMEOUT, emit_fut).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            tracing::warn!(
                delegation_id = %delegation_id,
                issue_id = %issue_id,
                tool = %tool,
                "WorkerWrite audit comment emission failed: {e}"
            );
        }
        Err(_) => {
            tracing::warn!(
                delegation_id = %delegation_id,
                issue_id = %issue_id,
                tool = %tool,
                "WorkerWrite audit comment emission timed out after {}s",
                AUDIT_EMIT_TIMEOUT.as_secs()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use rmcp::{
        model::CallToolRequestParams,
        service::ServiceError,
        transport::{
            streamable_http_client::StreamableHttpClientTransportConfig,
            StreamableHttpClientTransport,
        },
        ServiceExt,
    };
    use spur_acp::SpurEventBody;
    use spur_license::policy::PolicyResolver;
    use spur_pm::{BeadsAdvanced, Comment, CommentId, DependencyCycle, IssueSummary, ReadyFilter};
    use tempfile::TempDir;
    use tracing::field::{Field, Visit};

    use super::*;

    // ─── Mock BeadsAdvanced implementations ───────────────────────────────

    struct RecordingAdvanced {
        comments: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl BeadsAdvanced for RecordingAdvanced {
        async fn list_ready(&self, _filter: ReadyFilter) -> anyhow::Result<Vec<IssueSummary>> {
            Ok(Vec::new())
        }
        async fn list_comments(&self, _issue_id: &str) -> anyhow::Result<Vec<Comment>> {
            Ok(Vec::new())
        }
        async fn add_comment(&self, issue_id: &str, body: &str) -> anyhow::Result<CommentId> {
            self.comments
                .lock()
                .unwrap()
                .push((issue_id.to_string(), body.to_string()));
            Ok("c-1".into())
        }
        async fn remove_dependency(
            &self,
            _issue_id: &str,
            _depends_on_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn dep_cycles(&self) -> anyhow::Result<Vec<DependencyCycle>> {
            Ok(Vec::new())
        }
    }

    struct FailingAdvanced;

    #[async_trait]
    impl BeadsAdvanced for FailingAdvanced {
        async fn list_ready(&self, _filter: ReadyFilter) -> anyhow::Result<Vec<IssueSummary>> {
            Ok(Vec::new())
        }
        async fn list_comments(&self, _issue_id: &str) -> anyhow::Result<Vec<Comment>> {
            Ok(Vec::new())
        }
        async fn add_comment(&self, _issue_id: &str, _body: &str) -> anyhow::Result<CommentId> {
            Err(anyhow::anyhow!("simulated add_comment failure"))
        }
        async fn remove_dependency(
            &self,
            _issue_id: &str,
            _depends_on_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn dep_cycles(&self) -> anyhow::Result<Vec<DependencyCycle>> {
            Ok(Vec::new())
        }
    }

    struct HangingAdvanced;

    #[async_trait]
    impl BeadsAdvanced for HangingAdvanced {
        async fn list_ready(&self, _filter: ReadyFilter) -> anyhow::Result<Vec<IssueSummary>> {
            Ok(Vec::new())
        }
        async fn list_comments(&self, _issue_id: &str) -> anyhow::Result<Vec<Comment>> {
            Ok(Vec::new())
        }
        async fn add_comment(&self, _issue_id: &str, _body: &str) -> anyhow::Result<CommentId> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok("c-1".into())
        }
        async fn remove_dependency(
            &self,
            _issue_id: &str,
            _depends_on_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn dep_cycles(&self) -> anyhow::Result<Vec<DependencyCycle>> {
            Ok(Vec::new())
        }
    }

    // ─── Dispatcher fixtures ──────────────────────────────────────────────

    struct NullMcpEventSink;

    impl McpEventSink for NullMcpEventSink {
        fn emit(&self, _event: SpurEventBody) {}
    }

    #[derive(Default)]
    struct RecordingWorkerSignalSink {
        progress: Mutex<Vec<(WorkerCallContext, Value)>>,
    }

    #[derive(Default)]
    struct HangingWorkerSignalSink {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl WorkerSignalSink for HangingWorkerSignalSink {
        async fn report_signal(
            &self,
            _ctx: &WorkerCallContext,
            _args: Value,
        ) -> Result<Value, McpHandlerError> {
            std::future::pending().await
        }

        async fn report_progress(
            &self,
            _ctx: &WorkerCallContext,
            _args: Value,
        ) -> Result<Value, McpHandlerError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(json!({ "ok": true }))
        }
    }

    impl RecordingWorkerSignalSink {
        fn progress_count(&self) -> usize {
            self.progress.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl WorkerSignalSink for RecordingWorkerSignalSink {
        async fn report_signal(
            &self,
            ctx: &WorkerCallContext,
            args: Value,
        ) -> Result<Value, McpHandlerError> {
            Ok(json!({
                "ok": true,
                "delegation_id": ctx.delegation_id.clone(),
                "args": args
            }))
        }

        async fn report_progress(
            &self,
            ctx: &WorkerCallContext,
            args: Value,
        ) -> Result<Value, McpHandlerError> {
            self.progress
                .lock()
                .unwrap()
                .push((ctx.clone(), args.clone()));
            Ok(json!({
                "ok": true,
                "delegation_id": ctx.delegation_id.clone(),
                "args": args
            }))
        }
    }

    struct NullWorkerReadToolSink;

    #[async_trait]
    impl WorkerReadToolSink for NullWorkerReadToolSink {
        async fn get_plan_status(
            &self,
            _ctx: &WorkerCallContext,
            _args: Value,
        ) -> Result<Value, McpHandlerError> {
            Ok(json!({ "ok": true }))
        }

        async fn get_task_diff(
            &self,
            _ctx: &WorkerCallContext,
            _args: Value,
        ) -> Result<Value, McpHandlerError> {
            Ok(json!({ "ok": true }))
        }

        async fn fetch_outcome_artifact(
            &self,
            _ctx: &WorkerCallContext,
            _args: Value,
        ) -> Result<Value, McpHandlerError> {
            Ok(json!({ "ok": true }))
        }
    }

    async fn pm_service_fixture(repo: &Path) -> Arc<spur_pm::PmService> {
        let workspace = spur_pm::test_workspace::TestBeadsWorkspace::init();
        let beads_dir = repo.join(".beads");
        std::fs::create_dir_all(&beads_dir).expect("create test .beads directory");
        workspace.copy_db_to(&beads_dir);

        Arc::new(
            spur_pm::PmService::try_new(None, true, false, repo, None)
                .await
                .expect("PmService::try_new failed")
                .expect("expected beads-backed PmService"),
        )
    }

    fn beads_advanced_for_test(pm: &spur_pm::PmService) -> &dyn BeadsAdvanced {
        spur_pm::PmService::advanced(pm).expect("advanced beads backend")
    }

    fn pro_feature_gate() -> Arc<spur_license::FeatureGate> {
        use std::collections::BTreeSet;

        let gate = spur_license::FeatureGate::new(PolicyResolver::embedded());
        let state =
            spur_license::LicenseState::active_validated(spur_license::Plan::Pro, BTreeSet::new());
        gate.update_state(&state);
        Arc::new(gate)
    }

    #[test]
    fn report_signal_schema_required_fields_match_typed_variants() {
        use crate::plan::signals::WorkerSignal;
        use std::collections::BTreeSet;

        let schema = crate::mcp::worker_tool_registry()
            .unwrap()
            .list_tools()
            .into_iter()
            .find(|tool| tool.name == "report_signal")
            .unwrap()
            .input_schema;
        let signal = &schema["properties"]["signal"];
        let common = signal["required"].as_array().unwrap();
        assert_eq!(common, &vec![json!("kind"), json!("signal_id")]);
        let branches = signal["anyOf"]
            .as_array()
            .expect("signal schema must require each kind's typed fields");
        let cases = [
            ("scope_drift", json!({"severity": 1.0, "reason": "scope"})),
            ("blocked", json!({"severity": 1.0, "reason": "blocked"})),
            ("risk", json!({"severity": 1.0, "reason": "risk"})),
            ("escalate", json!({"reason": "review"})),
            ("mark_noop", json!({"reason": "intentional no-op"})),
            (
                "retry_exhausted",
                json!({"task_id": "bd-task", "attempt": 1, "last_error": "failure"}),
            ),
        ];
        let mut advertised = BTreeSet::new();
        for branch in branches {
            for kind in branch["properties"]["kind"]["enum"].as_array().unwrap() {
                assert!(
                    advertised.insert(kind.as_str().unwrap()),
                    "ambiguous kind branch"
                );
            }
        }
        assert_eq!(advertised, cases.iter().map(|(kind, _)| *kind).collect());

        for (kind, fields) in cases {
            let branch = branches
                .iter()
                .find(|branch| {
                    branch["properties"]["kind"]["enum"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(kind))
                })
                .unwrap();
            let required: BTreeSet<_> = branch["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|field| field.as_str().unwrap())
                .collect();
            let expected = fields
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                required, expected,
                "wrong advertised requirements for {kind}"
            );

            // Only the advertised required fields: optional fields must remain optional.
            let mut payload = fields;
            payload["kind"] = json!(kind);
            payload["signal_id"] = json!(uuid::Uuid::nil());
            assert!(
                serde_json::from_value::<WorkerSignal>(payload.clone()).is_ok(),
                "{kind}"
            );
            for field in required.into_iter().chain(["kind", "signal_id"]) {
                let mut missing = payload.clone();
                missing.as_object_mut().unwrap().remove(field);
                assert!(
                    serde_json::from_value::<WorkerSignal>(missing).is_err(),
                    "{kind} field {field} must be required by both schema and handler"
                );
            }
        }
    }

    #[test]
    fn report_signal_schema_catalog_matches_rmcp_router() {
        let catalog = crate::mcp::worker_tool_registry()
            .unwrap()
            .list_tools()
            .into_iter()
            .find(|tool| tool.name == "report_signal")
            .unwrap();
        let routed = WorkerToolHandler::tool_router()
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "report_signal")
            .unwrap();
        assert_eq!(
            catalog.input_schema,
            Value::Object((*routed.input_schema).clone())
        );
    }

    #[test]
    fn worker_evidence_catalog_and_signal_schema_are_usable() {
        let tools = crate::mcp::worker_tool_registry().unwrap().list_tools();
        assert!(
            tools.iter().any(|tool| tool.name == "report_audit"),
            "workers need a durable evidence channel"
        );
        assert!(!tools.iter().any(|tool| tool.name == "update_issue"));
        let signal = tools
            .iter()
            .find(|tool| tool.name == "report_signal")
            .unwrap();
        let kinds = signal.input_schema["properties"]["signal"]["properties"]["kind"]["enum"]
            .as_array()
            .unwrap();
        for kind in ["blocked", "risk", "escalate", "mark_noop"] {
            assert!(kinds.contains(&json!(kind)), "catalog omits {kind}");
            let parsed = serde_json::from_value::<crate::plan::signals::WorkerSignal>(json!({
                "kind": kind, "signal_id": uuid::Uuid::new_v4(), "reason": "evidence transport unavailable", "severity": 1.0
            }));
            assert!(
                parsed.is_ok(),
                "typed dispatcher rejects {kind}: {parsed:?}"
            );
        }
    }

    #[tokio::test]
    async fn worker_evidence_transport_persists_idempotently_and_refuses_unowned_targets() {
        use crate::plan::audit_sentinel::{encode_comment, AuditSentinelKind};
        let dir = TempDir::new().unwrap();
        let pm = pm_service_fixture(dir.path()).await;
        let task = pm
            .create_issue(spur_pm::IssueCreate {
                title: "evidence owner".into(),
                issue_type: Some("task".into()),
                labels: vec![crate::plan::labels::delegation_id("evidence-maker")],
                ..Default::default()
            })
            .await
            .unwrap();
        let other = pm
            .create_issue(spur_pm::IssueCreate {
                title: "unrelated".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let adv = beads_advanced_for_test(&pm);
        adv.add_comment(
            &task,
            &encode_comment(&AuditSentinelKind::Dispatch {
                delegation_id: "evidence-maker".into(),
                worker: "codex".into(),
                attempt: 1,
            }),
        )
        .await
        .unwrap();
        let server = WorkerMcpServer::start(
            "evidence-brain".into(),
            worker_mcp_deps_fixture(
                Arc::clone(&pm),
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .unwrap();
        server.register_delegation("evidence-maker".into(), DelegationContext::default());
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(server.url())
                .auth_header(server.issue_token("evidence-maker", Duration::from_secs(60))),
        );
        let client = ().serve(transport).await.unwrap();
        let args = json!({"task_id": task, "audit_id": uuid::Uuid::new_v4(), "message": "PRE positive control",
            "evidence": {"solve_id": "sol_fixture", "status": "sat", "complete_receipt": {"model": {"x": true}}}});
        let request = |value: Value| {
            CallToolRequestParams::new("report_audit")
                .with_arguments(value.as_object().unwrap().clone())
        };
        let before = adv.list_comments(&task).await.unwrap().len();
        let first = client
            .call_tool(request(args.clone()))
            .await
            .expect("durable audit tool must exist");
        assert_eq!(first.structured_content.as_ref().unwrap()["recorded"], true);
        assert_eq!(
            first.structured_content.as_ref().unwrap()["audit"]["evidence"],
            args["evidence"],
            "worker must be able to verify the full reloaded receipt"
        );
        let replay = client.call_tool(request(args.clone())).await.unwrap();
        assert_eq!(
            replay.structured_content.as_ref().unwrap()["idempotent"],
            true
        );
        assert_eq!(
            first.structured_content.as_ref().unwrap()["audit"],
            replay.structured_content.as_ref().unwrap()["audit"]
        );
        let comments = adv.list_comments(&task).await.unwrap();
        assert_eq!(comments.len(), before + 1);
        let body = &comments.last().unwrap().body;
        let durable: Value = serde_json::from_str(
            body.strip_prefix(crate::plan::audit_sentinel::SENTINEL_PREFIX)
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(durable["kind"], "worker-evidence");
        assert_eq!(durable["evidence"], args["evidence"]);
        assert_eq!(durable["delegation_id"], "evidence-maker");
        assert_eq!(durable["brain_session_id"], "evidence-brain");
        let status_before = pm.get_issue(&task).await.unwrap().status;
        let mut conflicting = args.clone();
        conflicting["message"] = json!("rewritten receipt");
        assert!(client.call_tool(request(conflicting)).await.is_err());
        let mut cross_task = args.clone();
        cross_task["task_id"] = json!(other);
        assert!(client.call_tool(request(cross_task)).await.is_err());
        assert!(adv.list_comments(&other).await.unwrap().is_empty());
        assert_eq!(pm.get_issue(&task).await.unwrap().status, status_before);
        for update in [
            spur_pm::IssueUpdate {
                status: Some("closed".into()),
                ..Default::default()
            },
            spur_pm::IssueUpdate {
                status: Some("open".into()),
                add_labels: crate::plan::labels::superseded_by_labels(&["replacement".into()]),
                ..Default::default()
            },
        ] {
            pm.update_issue(&task, update).await.unwrap();
            let mut denied = args.clone();
            denied["audit_id"] = json!(uuid::Uuid::new_v4());
            assert!(client.call_tool(request(denied)).await.is_err());
            assert_eq!(adv.list_comments(&task).await.unwrap().len(), before + 1);
        }
        pm.update_issue(
            &task,
            spur_pm::IssueUpdate {
                remove_labels: crate::plan::labels::superseded_by_labels(&["replacement".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // Replaced dispatch invalidates even an otherwise still-valid bearer token.
        adv.add_comment(
            &task,
            &encode_comment(&AuditSentinelKind::Dispatch {
                delegation_id: "new-maker".into(),
                worker: "codex".into(),
                attempt: 2,
            }),
        )
        .await
        .unwrap();
        let mut stale = args.clone();
        stale["audit_id"] = json!(uuid::Uuid::new_v4());
        assert!(client.call_tool(request(stale)).await.is_err());
        server.complete_delegation("evidence-maker", "success");
        assert!(client.call_tool(request(args)).await.is_err());
        client.cancel().await.unwrap();
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn worker_evidence_signals_persist_blocked_and_risk_without_status_mutation() {
        use crate::plan::audit_sentinel::{encode_comment, AuditSentinelKind};
        let dir = TempDir::new().unwrap();
        let pm = pm_service_fixture(dir.path()).await;
        let task = pm
            .create_issue(spur_pm::IssueCreate {
                title: "blocked signal owner".into(),
                issue_type: Some("task".into()),
                labels: vec![crate::plan::labels::delegation_id("signal-maker")],
                ..Default::default()
            })
            .await
            .unwrap();
        let adv = beads_advanced_for_test(&pm);
        adv.add_comment(
            &task,
            &encode_comment(&AuditSentinelKind::Dispatch {
                delegation_id: "signal-maker".into(),
                worker: "codex".into(),
                attempt: 1,
            }),
        )
        .await
        .unwrap();
        let gate = pro_feature_gate();
        let sink = Arc::new(crate::mcp::signals::WorkerSignalMcpToolModule::new(
            crate::mcp::signals::SignalMcpDeps {
                pm_service: Some(Arc::clone(&pm)),
                event_sink: None,
                feature_gate: Arc::clone(&gate),
            },
        ));
        let server = WorkerMcpServer::start(
            "signal-brain".into(),
            worker_mcp_deps_fixture(Arc::clone(&pm), gate, sink),
        )
        .await
        .unwrap();
        server.register_delegation("signal-maker".into(), DelegationContext::default());
        let client = ()
            .serve(StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(server.url())
                    .auth_header(server.issue_token("signal-maker", Duration::from_secs(60))),
            ))
            .await
            .unwrap();
        let status_before = pm.get_issue(&task).await.unwrap().status;
        for kind in ["blocked", "risk"] {
            let args = json!({"task_id": task, "signal": {"kind": kind, "signal_id": uuid::Uuid::new_v4(),
                "severity": 0.9, "reason": "required evidence unavailable", "estimated_subtasks": 0}});
            let request = |value: Value| {
                CallToolRequestParams::new("report_signal")
                    .with_arguments(value.as_object().unwrap().clone())
            };
            client
                .call_tool(request(args.clone()))
                .await
                .expect("supported signal must persist");
            let count = adv.list_comments(&task).await.unwrap().len();
            client.call_tool(request(args.clone())).await.unwrap();
            assert_eq!(
                adv.list_comments(&task).await.unwrap().len(),
                count,
                "transport retry must not duplicate a signal"
            );
            assert!(pm
                .get_issue(&task)
                .await
                .unwrap()
                .labels
                .contains(&crate::plan::labels::signal_kind(kind)));
            let signals: Vec<_> = adv
                .list_comments(&task)
                .await
                .unwrap()
                .iter()
                .filter_map(|c| crate::plan::signals::parse_comment(&c.body))
                .collect();
            assert!(signals
                .iter()
                .any(|s| s.as_ref().is_ok_and(|s| s.kind_label() == kind)));
            let mut bad = args.clone();
            bad["signal"]["reason"] = json!("rewritten reason");
            assert!(client.call_tool(request(bad)).await.is_err());
            let mut invalid = args.clone();
            invalid["signal"]["signal_id"] = json!(uuid::Uuid::new_v4());
            invalid["signal"]["severity"] = json!(1.1);
            assert!(client.call_tool(request(invalid)).await.is_err());
            assert_eq!(adv.list_comments(&task).await.unwrap().len(), count);
        }
        assert_eq!(pm.get_issue(&task).await.unwrap().status, status_before);
        client.cancel().await.unwrap();
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn worker_notebook_catalog_is_read_only_and_unbound_calls_are_denied() {
        let dir = TempDir::new().unwrap();
        let pm = pm_service_fixture(dir.path()).await;
        let deps = worker_mcp_deps_fixture(
            pm,
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        let server = WorkerMcpServer::start("brain-notebook".into(), deps)
            .await
            .unwrap();
        let token = server.issue_token("no-notebook-grant", Duration::from_secs(60));
        let config = StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token);
        let client = ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap();
        let notebook_tools: Vec<_> = client
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .filter(|tool| tool.name.starts_with("notebook_"))
            .inspect(|tool| {
                assert_eq!(
                    tool.annotations.as_ref().and_then(|a| a.read_only_hint),
                    Some(true)
                )
            })
            .map(|t| t.name.into_owned())
            .collect();
        assert_eq!(
            notebook_tools,
            ["notebook_list_cells", "notebook_read_cell"]
        );
        for name in ["notebook_list_cells", "notebook_read_cell"] {
            let mut request = CallToolRequestParams::new(name);
            request.arguments = Some(
                json!({"notebook_path": "/ungranted.ipynb", "id": "cell"})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            let error = client.call_tool(request).await.unwrap_err();
            assert!(error.to_string().contains("not authorized"), "{error}");
        }
        for name in [
            "notebook_write_cell",
            "notebook_run_cell",
            "notebook_open",
            "notebook.unknown",
        ] {
            assert!(client
                .call_tool(CallToolRequestParams::new(name))
                .await
                .is_err());
        }
        client.cancel().await.unwrap();
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn worker_notebook_delegations_are_isolated_and_terminal_calls_revoke() {
        use crate::worker_notebook::{
            tests::{cell_response, daemon_fixture, grant_fixture},
            NotebookReader,
        };
        let dir = TempDir::new().unwrap();
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-notebook".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let grant_a = grant_fixture(&a);
        let grant_b = grant_fixture(&b);
        let path_a = grant_a.path.clone();
        let path_b = grant_b.path.clone();
        let socket = dir.path().join("daemon.sock");
        let daemon = daemon_fixture(
            &socket,
            vec![
                cell_response(&path_a, "allowed"),
                cell_response(&path_b, "allowed"),
            ],
        )
        .await;
        let reader_a = Arc::new(NotebookReader::new(socket.clone(), vec![grant_a]));
        let reader_b = Arc::new(NotebookReader::new(socket, vec![grant_b]));
        server.register_notebook_reader("a".into(), Arc::clone(&reader_a));
        server.register_notebook_reader("b".into(), Arc::clone(&reader_b));
        let connect = |id: &str| {
            StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(server.url())
                    .auth_header(server.issue_token(id, Duration::from_secs(60))),
            )
        };
        let client_a = ().serve(connect("a")).await.unwrap();
        let client_b = ().serve(connect("b")).await.unwrap();
        let request = |path: &Path| {
            CallToolRequestParams::new("notebook_read_cell").with_arguments(
                json!({"notebook_path":path,"id":"allowed"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
        };
        for (client, wrong_path) in [(&client_a, &path_b), (&client_b, &path_a)] {
            assert!(client
                .call_tool(request(wrong_path))
                .await
                .unwrap_err()
                .to_string()
                .contains("not authorized"));
        }
        assert_eq!(
            client_a
                .call_tool(request(&path_a))
                .await
                .unwrap()
                .structured_content
                .unwrap()["source"],
            "unsaved source"
        );
        server.flush_delegation("a", "success").await.unwrap();
        assert!(
            !server.deps.notebook_readers.lock().contains_key("a"),
            "flush must remove the grant"
        );
        assert!(reader_a
            .call(
                "notebook_read_cell",
                json!({"notebook_path":path_a,"id":"allowed"})
            )
            .await
            .unwrap_err()
            .message
            .contains("not authorized"));
        assert!(client_a
            .call_tool(request(&path_a))
            .await
            .unwrap_err()
            .to_string()
            .contains("not authorized"));
        assert_eq!(
            client_b
                .call_tool(request(&path_b))
                .await
                .unwrap()
                .structured_content
                .unwrap()["path"],
            json!(path_b)
        );
        server.complete_delegation("b", "success");
        assert!(
            !server.deps.notebook_readers.lock().contains_key("b"),
            "legacy completion must remove the grant"
        );
        assert!(reader_b
            .call(
                "notebook_read_cell",
                json!({"notebook_path":path_b,"id":"allowed"})
            )
            .await
            .unwrap_err()
            .message
            .contains("not authorized"));
        assert_eq!(
            daemon.await.unwrap().len(),
            2,
            "denied calls never reach daemon"
        );
        client_a.cancel().await.unwrap();
        client_b.cancel().await.unwrap();
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn worker_notebook_dispatch_context_binds_paths_and_fails_closed() {
        use crate::worker_notebook::tests::grant_fixture;
        let dir = TempDir::new().unwrap();
        let path = grant_fixture(dir.path()).path;
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-context".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .unwrap();
        let task = "Inspect this notebook.";
        let prompt = server
            .register_notebook_context("d", dir.path(), &["book.ipynb".into()], task)
            .unwrap();
        assert!(prompt.contains(&path.display().to_string()));
        assert!(prompt.contains("notebook_list_cells"));
        assert!(prompt.contains("notebook_read_cell"));
        assert!(prompt.contains("Do not edit, execute, open, or save"));
        assert!(!prompt.contains(".sock"));
        assert!(prompt.ends_with(task));
        let old = server
            .deps
            .notebook_readers
            .lock()
            .get("d")
            .cloned()
            .unwrap();
        assert!(server
            .register_notebook_context("d", dir.path(), &["missing.ipynb".into()], task)
            .is_err());
        assert!(!server.deps.notebook_readers.lock().contains_key("d"));
        assert!(old
            .call("notebook_list_cells", json!({"notebook_path":path}))
            .await
            .unwrap_err()
            .message
            .contains("not authorized"));
        assert_eq!(
            server
                .register_notebook_context("empty", dir.path(), &["source.rs".into()], task)
                .unwrap(),
            task
        );
        assert!(!server.deps.notebook_readers.lock().contains_key("empty"));
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn worker_notebook_flush_cancels_in_flight_daemon_read() {
        use crate::worker_notebook::{tests::grant_fixture, NotebookReader};
        let dir = TempDir::new().unwrap();
        let grant = grant_fixture(dir.path());
        let path = grant.path.clone();
        let socket = dir.path().join("stall.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let reader = Arc::new(NotebookReader::new(socket, vec![grant]));
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-stall".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .unwrap();
        server.register_notebook_reader("d".into(), Arc::clone(&reader));
        let pending = tokio::spawn(async move {
            reader
                .call(
                    "notebook_read_cell",
                    json!({"notebook_path":path,"id":"allowed"}),
                )
                .await
        });
        let (mut connection, _) = listener.accept().await.unwrap();
        crate::orchestrator::read_notebook_daemon_frame(&mut connection)
            .await
            .unwrap();
        // Keep the real socket open without answering initialize. Completion
        // must cancel the pending read, not wait for the production deadline.
        server.flush_delegation("d", "cancelled").await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .expect("revocation wakes the read")
            .unwrap();
        assert!(result.unwrap_err().message.contains("not authorized"));
        server.shutdown(Duration::from_secs(5)).await;
    }

    fn dispatcher_deps_fixture(
        pm_service: Arc<spur_pm::PmService>,
        feature_gate: Arc<spur_license::FeatureGate>,
        worker_signal_sink: Arc<dyn WorkerSignalSink>,
    ) -> Arc<DispatcherDeps> {
        let (flush_tx, _flush_rx) = mpsc::unbounded_channel::<FlushMessage>();
        let funnel: Arc<dyn McpEventSink> = Arc::new(NullMcpEventSink);
        let worker_read_sink: Arc<dyn WorkerReadToolSink> = Arc::new(NullWorkerReadToolSink);

        Arc::new(DispatcherDeps {
            embedding_runtime: spur_analyst::embedding::OwnedEmbeddingRuntime::new(),
            pm_service,
            feature_gate,
            funnel,
            worker_signal_sink,
            worker_read_sink,
            repo_root: None,
            delegations: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            delegation_worktree_roots: Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
            notebook_readers: parking_lot::Mutex::new(std::collections::HashMap::new()),
            graph_mcp_module: spur_graph::mcp::GraphMcpModule::new(
                spur_graph::mcp::GraphMcpDeps::default(),
            ),
            read_audit_buffers: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            flush_tx,
            active_delegations: Arc::new(AtomicU32::new(0)),
            peak_active_delegations: Arc::new(AtomicU32::new(0)),
            handler_aborts: Arc::new(HandlerAbortRegistry::default()),
            delegation_guards: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        })
    }

    fn worker_mcp_deps_fixture(
        pm_service: Arc<spur_pm::PmService>,
        feature_gate: Arc<spur_license::FeatureGate>,
        worker_signal_sink: Arc<dyn WorkerSignalSink>,
    ) -> WorkerMcpDeps {
        WorkerMcpDeps {
            pm_service,
            feature_gate,
            funnel: Arc::new(NullMcpEventSink),
            worker_signal_sink,
            worker_read_sink: Arc::new(NullWorkerReadToolSink),
            repo_root: None,
        }
    }

    fn encode_test_worker_token(
        hmac_key: &[u8; 32],
        delegation_id: &str,
        brain_session_id: &str,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let payload = spur_mcp::token::WorkerTokenPayload {
            d: delegation_id.to_string(),
            b: brain_session_id.to_string(),
            e: now + 60,
        };
        spur_mcp::token::encode_token(hmac_key, &payload).expect("valid test token")
    }

    struct AuthProbe {
        url: String,
        hmac_key: [u8; 32],
        session_manager: Arc<LocalSessionManager>,
        session_contexts:
            Arc<parking_lot::Mutex<std::collections::HashMap<String, AuthenticatedWorkerContext>>>,
        response_session_id: Arc<Mutex<Option<String>>>,
        hit_count: Arc<AtomicU32>,
        shutdown: CancellationToken,
        handle: JoinHandle<()>,
    }

    impl AuthProbe {
        async fn shutdown(self) {
            self.shutdown.cancel();
            self.handle.await.expect("auth probe server joins");
        }
    }

    async fn spawn_auth_probe(brain_session_id: &str) -> AuthProbe {
        let hmac_key = [7u8; 32];
        let session_manager = Arc::new(LocalSessionManager::default());
        let session_contexts = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let response_session_id = Arc::new(Mutex::new(None::<String>));
        let hit_count = Arc::new(AtomicU32::new(0));
        let shutdown = CancellationToken::new();

        let state = WorkerAuthMiddlewareState {
            hmac_key,
            brain_session_id: brain_session_id.to_string(),
            session_manager: Arc::clone(&session_manager),
            session_contexts: Arc::clone(&session_contexts),
        };
        let response_session_id_for_handler = Arc::clone(&response_session_id);
        let hit_count_for_handler = Arc::clone(&hit_count);
        let app = Router::new()
            .route(
                "/",
                axum::routing::post(move |request: Request<Body>| {
                    let response_session_id = Arc::clone(&response_session_id_for_handler);
                    let hit_count = Arc::clone(&hit_count_for_handler);
                    async move {
                        hit_count.fetch_add(1, Ordering::SeqCst);
                        let auth_ctx = request
                            .extensions()
                            .get::<AuthenticatedWorkerContext>()
                            .expect("middleware inserts authenticated context");
                        let mut response = Response::new(Body::from(format!(
                            "{}:{}",
                            auth_ctx.delegation_id, auth_ctx.brain_session_id
                        )));
                        if let Some(session_id) = response_session_id.lock().unwrap().clone() {
                            response.headers_mut().insert(
                                "mcp-session-id",
                                axum::http::HeaderValue::from_str(&session_id)
                                    .expect("valid session id header"),
                            );
                        }
                        response
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                state,
                worker_auth_middleware,
            ));

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind auth probe");
        let addr = listener.local_addr().expect("local addr");
        let serve_shutdown = shutdown.clone();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(serve_shutdown.cancelled_owned())
                .await
                .expect("auth probe serves");
        });

        AuthProbe {
            url: format!("http://{addr}/"),
            hmac_key,
            session_manager,
            session_contexts,
            response_session_id,
            hit_count,
            shutdown,
            handle,
        }
    }

    async fn call_report_progress(
        server: &WorkerMcpServer,
        token: &str,
        message: &str,
    ) -> CallToolResult {
        let config = StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token);
        let client =
            ().serve(StreamableHttpClientTransport::from_config(config))
                .await
                .expect("rmcp client initialize");
        let mut request = CallToolRequestParams::new("report_progress");
        request.arguments = json!({ "message": message }).as_object().cloned();
        let result = client
            .call_tool(request)
            .await
            .expect("report_progress succeeds");
        drop(client);
        result
    }

    // ─── Warning capture helper ───────────────────────────────────────────

    #[derive(Clone, Default)]
    struct CapturedWarnings {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl CapturedWarnings {
        fn contains(&self, needle: &str) -> bool {
            self.events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.contains(needle))
        }
    }

    impl tracing::Subscriber for CapturedWarnings {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() <= tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() != tracing::Level::WARN {
                return;
            }
            let mut visitor = StringVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(visitor.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[derive(Default)]
    struct StringVisitor(String);

    impl Visit for StringVisitor {
        fn record_debug(&mut self, _field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("{value:?}"));
        }
    }

    // ─── Tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn worker_auth_middleware_binds_token_context_to_minted_session() {
        let probe = spawn_auth_probe("brain-A").await;
        let (session_id, _transport) = probe
            .session_manager
            .create_session()
            .await
            .expect("create local session");
        let session_id = session_id.to_string();
        *probe.response_session_id.lock().unwrap() = Some(session_id.clone());

        let token = encode_test_worker_token(&probe.hmac_key, "del-A", "brain-A");
        let client = reqwest::Client::new();
        let response = client
            .post(&probe.url)
            .bearer_auth(&token)
            .send()
            .await
            .expect("token request");
        assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
        assert_eq!(response.text().await.expect("body"), "del-A:brain-A");
        assert_eq!(probe.hit_count.load(Ordering::SeqCst), 1);

        let cached = probe
            .session_contexts
            .lock()
            .get(&session_id)
            .cloned()
            .expect("session context cached");
        assert_eq!(cached.delegation_id, "del-A");
        assert_eq!(cached.brain_session_id, "brain-A");

        *probe.response_session_id.lock().unwrap() = None;
        let response = client
            .post(&probe.url)
            .header("mcp-session-id", &session_id)
            .send()
            .await
            .expect("session request");
        assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
        assert_eq!(response.text().await.expect("body"), "del-A:brain-A");
        assert_eq!(probe.hit_count.load(Ordering::SeqCst), 2);

        drop(client);
        probe.shutdown().await;
    }

    #[tokio::test]
    async fn worker_auth_middleware_rejects_missing_or_unbound_auth() {
        let probe = spawn_auth_probe("brain-A").await;
        let client = reqwest::Client::new();

        let missing = client
            .post(&probe.url)
            .send()
            .await
            .expect("missing token request");
        assert_eq!(missing.status().as_u16(), StatusCode::UNAUTHORIZED.as_u16());

        let wrong_brain = encode_test_worker_token(&probe.hmac_key, "del-A", "brain-B");
        let mismatch = client
            .post(&probe.url)
            .bearer_auth(&wrong_brain)
            .send()
            .await
            .expect("brain mismatch request");
        assert_eq!(
            mismatch.status().as_u16(),
            StatusCode::UNAUTHORIZED.as_u16()
        );

        let (session_id, _transport) = probe
            .session_manager
            .create_session()
            .await
            .expect("create local session");
        let unbound = client
            .post(&probe.url)
            .header("mcp-session-id", session_id.to_string())
            .send()
            .await
            .expect("unbound session request");
        assert_eq!(unbound.status().as_u16(), StatusCode::UNAUTHORIZED.as_u16());
        assert_eq!(probe.hit_count.load(Ordering::SeqCst), 0);

        drop(client);
        probe.shutdown().await;
    }

    #[tokio::test]
    async fn append_read_audit_entry_creates_and_reuses_delegation_buffer() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let deps = dispatcher_deps_fixture(
            pm,
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );

        append_read_audit_entry(&deps, "del-A", "get_issue", Some("bd-1".into()));
        append_read_audit_entry(&deps, "del-A", "list_issues", None);
        append_read_audit_entry(&deps, "del-B", "get_plan_status", Some("bd-2".into()));

        let map = deps.read_audit_buffers.lock();
        assert_eq!(map.len(), 2);
        let del_a = map.get("del-A").expect("del-A buffer");
        let entries = del_a.entries.lock();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tool_name, "get_issue");
        assert_eq!(entries[0].target_issue_id.as_deref(), Some("bd-1"));
        assert_eq!(entries[1].tool_name, "list_issues");
        assert_eq!(entries[1].target_issue_id, None);
        assert_eq!(map.get("del-B").expect("del-B buffer").entry_count(), 1);
    }

    #[tokio::test]
    async fn emit_read_aggregate_writes_first_targeted_sentinel() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let issue_id = pm
            .create_issue(spur_pm::IssueCreate {
                title: "read aggregate target".into(),
                issue_type: Some("task".into()),
                ..Default::default()
            })
            .await
            .expect("create issue");
        let deps = dispatcher_deps_fixture(
            Arc::clone(&pm),
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        let shutdown = CancellationToken::new();

        emit_read_aggregate(
            &deps,
            "del-A".into(),
            vec![
                ReadAuditEntry {
                    tool_name: "list_issues".into(),
                    target_issue_id: None,
                    ts: 1,
                },
                ReadAuditEntry {
                    tool_name: "get_issue".into(),
                    target_issue_id: Some(issue_id.clone()),
                    ts: 2,
                },
            ],
            &shutdown,
        )
        .await;

        let comments = beads_advanced_for_test(pm.as_ref())
            .list_comments(&issue_id)
            .await
            .expect("list comments");
        assert_eq!(comments.len(), 1);
        let decoded = crate::plan::audit_sentinel::parse_comment(&comments[0].body)
            .expect("parse comment")
            .expect("audit sentinel");
        match decoded {
            crate::plan::audit_sentinel::AuditSentinelKind::ReadAggregate {
                delegation_id,
                entries,
            } => {
                assert_eq!(delegation_id, "del-A");
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].tool_name, "list_issues");
                assert_eq!(entries[0].target_issue_id, None);
                assert_eq!(entries[1].tool_name, "get_issue");
                assert_eq!(
                    entries[1].target_issue_id.as_deref(),
                    Some(issue_id.as_str())
                );
            }
            other => panic!("expected ReadAggregate, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_read_aggregate_skips_entries_without_target_issue() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let issue_id = pm
            .create_issue(spur_pm::IssueCreate {
                title: "untouched issue".into(),
                issue_type: Some("task".into()),
                ..Default::default()
            })
            .await
            .expect("create issue");
        let deps = dispatcher_deps_fixture(
            Arc::clone(&pm),
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );

        emit_read_aggregate(
            &deps,
            "del-A".into(),
            vec![ReadAuditEntry {
                tool_name: "list_issues".into(),
                target_issue_id: None,
                ts: 1,
            }],
            &CancellationToken::new(),
        )
        .await;

        let comments = beads_advanced_for_test(pm.as_ref())
            .list_comments(&issue_id)
            .await
            .expect("list comments");
        assert!(comments.is_empty());
    }

    #[tokio::test]
    async fn scan_and_flush_idle_buffers_flushes_idle_and_retains_fresh() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let issue_id = pm
            .create_issue(spur_pm::IssueCreate {
                title: "idle aggregate target".into(),
                issue_type: Some("task".into()),
                ..Default::default()
            })
            .await
            .expect("create issue");
        let deps = dispatcher_deps_fixture(
            Arc::clone(&pm),
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let stale = Arc::new(ReadAuditBuffer::new(
            "del-stale".into(),
            deps.flush_tx.clone(),
        ));
        stale.append_for_test(ReadAuditEntry {
            tool_name: "get_issue".into(),
            target_issue_id: Some(issue_id.clone()),
            ts: now.saturating_sub(120),
        });
        let fresh = Arc::new(ReadAuditBuffer::new(
            "del-fresh".into(),
            deps.flush_tx.clone(),
        ));
        fresh.append_for_test(ReadAuditEntry {
            tool_name: "get_issue".into(),
            target_issue_id: Some(issue_id.clone()),
            ts: now,
        });
        let empty = Arc::new(ReadAuditBuffer::new(
            "del-empty".into(),
            deps.flush_tx.clone(),
        ));
        {
            let mut map = deps.read_audit_buffers.lock();
            map.insert("del-stale".into(), stale);
            map.insert("del-fresh".into(), fresh);
            map.insert("del-empty".into(), empty);
        }

        scan_and_flush_idle_buffers(
            &deps,
            &WorkerMcpServerConfig {
                idle_threshold: Duration::from_secs(30),
                scan_interval: Duration::from_secs(1),
                ..Default::default()
            },
            &CancellationToken::new(),
        )
        .await;

        {
            let map = deps.read_audit_buffers.lock();
            assert!(!map.contains_key("del-stale"));
            assert!(!map.contains_key("del-empty"));
            assert_eq!(
                map.get("del-fresh").expect("fresh retained").entry_count(),
                1
            );
        }
        let comments = beads_advanced_for_test(pm.as_ref())
            .list_comments(&issue_id)
            .await
            .expect("list comments");
        assert_eq!(comments.len(), 1);
        assert!(comments[0].body.contains("\"delegation_id\":\"del-stale\""));
    }

    #[cfg(feature = "embed")]
    #[tokio::test]
    async fn direct_worker_calls_reuse_helper_until_server_shutdown() {
        const CHILD: &str = "SPUR_S6_WORKER_HOST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            if std::env::var_os("SPUR_S6_WORKER_TRACE").is_some() {
                tracing_subscriber::fmt()
                    .with_ansi(false)
                    .with_writer(std::io::stderr)
                    .try_init()
                    .unwrap();
            }
            let pm_dir = TempDir::new().unwrap();
            let pm = pm_service_fixture(pm_dir.path()).await;
            let server = WorkerMcpServer::start(
                "brain-A".into(),
                worker_mcp_deps_fixture(
                    pm,
                    pro_feature_gate(),
                    Arc::new(RecordingWorkerSignalSink::default()),
                ),
            )
            .await
            .unwrap();
            server.deps.delegation_worktree_roots.lock().insert(
                "del-A".into(),
                std::env::var_os("SPUR_S6_WORKER_ROOT").unwrap().into(),
            );
            for query in ["worker first", "worker second"] {
                let result = invoke_knowledge_context_for_worker(
                    Arc::clone(&server.deps),
                    WorkerCallContext {
                        delegation_id: "del-A".into(),
                        brain_session_id: "brain-A".into(),
                    },
                    json!({"query": query}),
                )
                .await;
                assert!(
                    result.is_err(),
                    "empty fixture must return a database error"
                );
            }
            if std::env::var_os("SPUR_S6_WORKER_FORCE").is_some() {
                server.shutdown_immediately().await;
            } else {
                server.shutdown(Duration::from_secs(5)).await;
            }
            return;
        }
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".spur")).unwrap();
        drop(duckdb::Connection::open(dir.path().join(".spur/analyst.duckdb")).unwrap());
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
        let config = json!({
            "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let log_path = dir.path().join("worker.log");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker_server::tests::direct_worker_calls_reuse_helper_until_server_shutdown",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("SPUR_S6_WORKER_ROOT", dir.path())
            .env("SPUR_GRAPH_TEST_WORKER_LOG", &log_path)
            .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let log = std::fs::read_to_string(log_path).unwrap();
        assert_eq!(
            log.lines().filter(|line| *line == "service").count(),
            1,
            "{log}"
        );
        assert_eq!(
            log.lines()
                .filter(|line| line.starts_with("embed "))
                .count(),
            2,
            "{log}"
        );
        assert_eq!(log.lines().last(), Some("shutdown"), "{log}");
    }

    #[cfg(feature = "embed")]
    #[tokio::test]
    async fn forced_worker_shutdown_reports_helper_abort() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".spur")).unwrap();
        drop(duckdb::Connection::open(dir.path().join(".spur/analyst.duckdb")).unwrap());
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spur-analyst/tests/embedding/fixtures/owned_fault_worker.py");
        let config = json!({
            "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let log = dir.path().join("worker.log");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker_server::tests::direct_worker_calls_reuse_helper_until_server_shutdown",
                "--nocapture",
            ])
            .env("SPUR_S6_WORKER_HOST_CHILD", "1")
            .env("SPUR_S6_WORKER_TRACE", "1")
            .env("SPUR_S6_WORKER_FORCE", "1")
            .env("SPUR_S6_WORKER_ROOT", dir.path())
            .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
            .env("SPUR_GRAPH_TEST_WORKER_FAULT", "shutdown_abort")
            .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("worker embedding shutdown failed"),
            "cleanup error was not reported: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = std::fs::read_to_string(log).unwrap();
        assert!(events.lines().any(|line| line == "shutdown"), "{events}");
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("service_pid "))
            .unwrap();
        let probe = std::process::Command::new("/usr/bin/python3").args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid]).status().unwrap();
        assert!(probe.success(), "helper still exists: {events}");
    }

    #[tokio::test]
    async fn invoke_code_graph_for_worker_maps_dispatch_errors() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let deps = dispatcher_deps_fixture(
            pm,
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        deps.delegation_worktree_roots
            .lock()
            .insert("del-A".into(), dir.path().to_path_buf());

        let error = invoke_code_graph_for_worker(
            deps,
            WorkerCallContext {
                delegation_id: "del-A".into(),
                brain_session_id: "brain-A".into(),
            },
            "not_a_code_graph_tool",
            json!({}),
        )
        .await
        .expect_err("unknown code graph tool must error");
        match error {
            McpHandlerError::InvalidParams(message) => {
                assert!(message.contains("unknown code graph MCP tool"));
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn report_progress_tool_honors_delegation_gate_and_records_lifecycle() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let signal_sink = Arc::new(RecordingWorkerSignalSink::default());
        let signal_sink_trait: Arc<dyn WorkerSignalSink> = signal_sink.clone();
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(pm, pro_feature_gate(), signal_sink_trait),
        )
        .await
        .expect("start worker server");

        server.register_delegation(
            "del-disabled".into(),
            DelegationContext {
                enable_worker_progress: false,
            },
        );
        let disabled_token = server.issue_token("del-disabled", Duration::from_secs(60));
        let disabled = call_report_progress(&server, &disabled_token, "disabled").await;
        assert_eq!(disabled.structured_content, Some(json!({ "ok": true })));
        assert_eq!(signal_sink.progress_count(), 0);

        server.register_delegation(
            "del-enabled".into(),
            DelegationContext {
                enable_worker_progress: true,
            },
        );
        let enabled_token = server.issue_token("del-enabled", Duration::from_secs(60));
        let enabled = call_report_progress(&server, &enabled_token, "enabled").await;
        assert_eq!(signal_sink.progress_count(), 1);
        assert_eq!(
            enabled
                .structured_content
                .as_ref()
                .and_then(|value| value.get("delegation_id")),
            Some(&json!("del-enabled"))
        );

        let guards = server.deps.delegation_guards.lock();
        let disabled_guard = guards.get("del-disabled").expect("disabled guard");
        let enabled_guard = guards.get("del-enabled").expect("enabled guard");
        assert_eq!(disabled_guard.calls.lock().len(), 1);
        assert_eq!(enabled_guard.calls.lock().len(), 1);
        assert_eq!(enabled_guard.calls.lock()[0].tool_name, "report_progress");
        assert_eq!(server.active_count(), 0);
        drop(guards);

        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn solver_tools_reach_live_registry_over_worker_http() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .expect("start worker server");
        server.register_delegation("del-solver".into(), DelegationContext::default());
        let token = server.issue_token("del-solver", Duration::from_secs(60));
        let config =
            StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(&token);
        let client =
            ().serve(StreamableHttpClientTransport::from_config(config))
                .await
                .expect("rmcp client initialize");

        for tool_name in ["solve_rule_spec", "solve_constraint_spec"] {
            client
                .call_tool(CallToolRequestParams::new(tool_name))
                .await
                .unwrap_or_else(|error| panic!("{tool_name} must work without Z3: {error}"));
        }

        for tool_name in [
            "solve_constraint_check",
            "solve_constraints",
            "solve_smt",
            "get_solve_result",
        ] {
            let error = client
                .call_tool(CallToolRequestParams::new(tool_name))
                .await
                .expect_err("empty solver arguments must be rejected");
            match error {
                ServiceError::McpError(error) => {
                    assert_eq!(
                        error.code,
                        rmcp::model::ErrorCode(-32602),
                        "{tool_name} must reject empty arguments"
                    );
                    assert!(
                        error
                            .message
                            .contains(&format!("invalid `{tool_name}` request")),
                        "{tool_name} must reach the live solver module: {}",
                        error.message
                    );
                }
                other => panic!("{tool_name} returned a non-MCP error: {other}"),
            }
        }

        drop(client);
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn skill_catalog_tools_reach_rooted_worker_registry_over_http() {
        let dir = TempDir::new().expect("tempdir");
        let skill_dir = dir.path().join("assets/skills/http-catalog-needle");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        let document = concat!(
            "---\n",
            "name: http-catalog-needle\n",
            "description: Find a unique HTTP catalog needle\n",
            "role: worker\n",
            "---\n\n",
            "# HTTP Catalog Needle\n\n",
            "EXACT_HTTP_SKILL_TEXT\n",
        );
        std::fs::write(skill_dir.join("SKILL.md"), document).expect("write skill document");

        let pm = pm_service_fixture(dir.path()).await;
        let mut deps = worker_mcp_deps_fixture(
            pm,
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        deps.repo_root = Some(dir.path().to_path_buf());
        let server = WorkerMcpServer::start("brain-A".into(), deps)
            .await
            .expect("start worker server");
        server.register_delegation("del-skills".into(), DelegationContext::default());
        let token = server.issue_token("del-skills", Duration::from_secs(60));
        let config =
            StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(&token);
        let client =
            ().serve(StreamableHttpClientTransport::from_config(config))
                .await
                .expect("rmcp client initialize");

        let mut search_request = CallToolRequestParams::new("skill_search");
        search_request.arguments = json!({ "query": "unique HTTP catalog needle" })
            .as_object()
            .cloned();
        let search = client
            .call_tool(search_request)
            .await
            .expect("skill_search reaches rooted worker registry");
        let search = serde_json::to_value(search).expect("serialize search response");
        let search_text = search["content"][0]["text"]
            .as_str()
            .expect("search response contains JSON text");
        let search_payload: Value =
            serde_json::from_str(search_text).expect("search response text is JSON");
        assert!(search_payload.get("content").is_none());
        let skill_id = search_payload["results"][0]["skill_id"]
            .as_str()
            .expect("opaque skill id");

        let mut read_request = CallToolRequestParams::new("skill_read");
        read_request.arguments = json!({ "skill_id": skill_id }).as_object().cloned();
        let read = client
            .call_tool(read_request)
            .await
            .expect("skill_read reaches rooted worker registry");
        let read = serde_json::to_value(read).expect("serialize read response");
        let read_text = read["content"][0]["text"]
            .as_str()
            .expect("read response contains JSON text");
        let read_payload: Value =
            serde_json::from_str(read_text).expect("read response text is JSON");
        assert_eq!(read_payload["content"], document);

        drop(client);
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn worker_solver_registry_reads_artifacts_from_dispatcher_repo_root() {
        let dir = TempDir::new().expect("tempdir");
        let writer = spur_solver::service::SolverService::new().with_repo_root(dir.path());
        let request = spur_solver::types::SolveConstraintsRequest {
            vars: Vec::new(),
            constraints: Vec::new(),
            objectives: vec![],
            objective_priority: Default::default(),
            max_solutions: spur_solver::types::DEFAULT_MAX_SOLUTIONS,
            timeout_ms: spur_solver::types::DEFAULT_TIMEOUT_MS,
            persist: false,
            include_smt: false,
            use_cache: true,
            session_id: None,
            session_op: Default::default(),
        };
        let result = spur_solver::types::SolveConstraintsResponse {
            status: spur_solver::types::SolveStatus::Unsat,
            model: None,
            duration_ms: 12,
            solve_id: None,
            reason: None,
            smt: None,
            unsat_core: None,
            cached: false,
            session_id: None,
            optimization: None,
            solver_version: None,
        };
        let artifact = writer
            .persist(&request, &result)
            .expect("seed repository-local solver artifact");
        let pm = pm_service_fixture(dir.path()).await;
        let mut deps = worker_mcp_deps_fixture(
            pm,
            pro_feature_gate(),
            Arc::new(RecordingWorkerSignalSink::default()),
        );
        deps.repo_root = Some(dir.path().to_path_buf());
        let server = WorkerMcpServer::start("brain-A".into(), deps)
            .await
            .expect("start worker server");
        server.register_delegation("del-solver-root".into(), DelegationContext::default());
        let token = server.issue_token("del-solver-root", Duration::from_secs(60));
        let config =
            StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(&token);
        let client =
            ().serve(StreamableHttpClientTransport::from_config(config))
                .await
                .expect("rmcp client initialize");
        let mut request = CallToolRequestParams::new("get_solve_result");
        request.arguments = json!({ "solve_id": artifact.solve_id })
            .as_object()
            .cloned();

        let response = client
            .call_tool(request)
            .await
            .expect("rooted worker registry must load the artifact");
        let response = serde_json::to_value(response).expect("serialize solver response");
        let body = response["content"][0]["text"]
            .as_str()
            .expect("solver response must contain JSON text");
        let payload: Value = serde_json::from_str(body).expect("solver response text must be JSON");
        assert_eq!(payload["solve_id"], artifact.solve_id);
        assert_eq!(payload["status"], "unsat");

        drop(client);
        server.shutdown(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn shutdown_reports_active_call_until_durable_handler_acknowledges() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let signal_sink = Arc::new(HangingWorkerSignalSink::default());
        let signal_sink_trait: Arc<dyn WorkerSignalSink> = signal_sink.clone();
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(pm, pro_feature_gate(), signal_sink_trait),
        )
        .await
        .expect("start worker server");
        server.register_delegation(
            "del-hanging".into(),
            DelegationContext {
                enable_worker_progress: true,
            },
        );
        let token = server.issue_token("del-hanging", Duration::from_secs(60));
        let call_server = Arc::clone(&server);
        let mut call = tokio::spawn(async move {
            call_report_progress(call_server.as_ref(), &token, "hang").await
        });
        signal_sink.entered.notified().await;
        assert_eq!(server.active_count(), 1);

        let outcome = Arc::clone(&server)
            .shutdown(Duration::from_millis(20))
            .await;
        let active_after_shutdown = server.active_count();
        signal_sink.release.notify_waiters();
        if tokio::time::timeout(Duration::from_secs(1), &mut call)
            .await
            .is_err()
        {
            call.abort();
            let _ = call.await;
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while server.active_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("active handler did not acknowledge completion");

        assert_eq!(
            outcome,
            ShutdownOutcome {
                drained: false,
                active_at_deadline: 1,
            },
            "shutdown must report the still-running durable handler so leadership remains fenced"
        );
        assert_eq!(active_after_shutdown, 1);
        assert_eq!(server.active_count(), 0);
    }

    #[tokio::test]
    async fn force_abort_handlers_awaits_permanently_hung_call_acknowledgement() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let signal_sink = Arc::new(HangingWorkerSignalSink::default());
        let signal_sink_trait: Arc<dyn WorkerSignalSink> = signal_sink.clone();
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(pm, pro_feature_gate(), signal_sink_trait),
        )
        .await
        .expect("start worker server");
        server.register_delegation(
            "del-hanging".into(),
            DelegationContext {
                enable_worker_progress: true,
            },
        );
        let token = server.issue_token("del-hanging", Duration::from_secs(60));
        let call_server = Arc::clone(&server);
        let call = tokio::spawn(async move {
            call_report_progress(call_server.as_ref(), &token, "hang forever").await
        });
        signal_sink.entered.notified().await;
        let outcome = Arc::clone(&server)
            .shutdown(Duration::from_millis(20))
            .await;
        assert!(!outcome.drained);
        assert_eq!(server.active_count(), 1);

        server.force_abort_handlers_and_wait().await;

        assert_eq!(server.active_count(), 0);
        call.abort();
        let _ = call.await;
    }

    #[tokio::test]
    async fn shutdown_immediately_aborts_permanently_hung_call() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let signal_sink = Arc::new(HangingWorkerSignalSink::default());
        let signal_sink_trait: Arc<dyn WorkerSignalSink> = signal_sink.clone();
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(pm, pro_feature_gate(), signal_sink_trait),
        )
        .await
        .expect("start worker server");
        server.register_delegation(
            "del-immediate".into(),
            DelegationContext {
                enable_worker_progress: true,
            },
        );
        let token = server.issue_token("del-immediate", Duration::from_secs(60));
        let call_server = Arc::clone(&server);
        let call = tokio::spawn(async move {
            call_report_progress(call_server.as_ref(), &token, "hang until aborted").await
        });
        signal_sink.entered.notified().await;
        assert_eq!(server.active_count(), 1);

        tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&server).shutdown_immediately(),
        )
        .await
        .expect("immediate shutdown entered a graceful drain window");

        assert_eq!(server.active_count(), 0);
        // `active_count == 0` is the durable server-side handler
        // acknowledgement. This direct-call harness sits outside the HTTP
        // task tree, so stop that caller explicitly after checking the server.
        call.abort();
        let _ = call.await;
    }

    #[tokio::test]
    async fn shutdown_immediately_closes_partially_read_accepted_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .expect("start worker server");
        let mut connection = tokio::net::TcpStream::connect(server.addr)
            .await
            .expect("connect raw client");
        connection
            .write_all(b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\nx")
            .await
            .expect("write partial request");
        tokio::time::timeout(Duration::from_secs(1), async {
            while server.connections.state.lock().active == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("server did not accept raw connection");

        tokio::time::timeout(
            Duration::from_secs(1),
            Arc::clone(&server).shutdown_immediately(),
        )
        .await
        .expect("immediate shutdown exceeded bound");

        let mut byte = [0_u8; 1];
        let peer_closed = matches!(
            tokio::time::timeout(Duration::from_millis(100), connection.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        );
        assert!(
            peer_closed,
            "shutdown acknowledgement must include Axum's accepted connection task"
        );
    }

    #[tokio::test]
    async fn shutdown_until_forced_preempts_graceful_handler_drain() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let signal_sink = Arc::new(HangingWorkerSignalSink::default());
        let signal_sink_trait: Arc<dyn WorkerSignalSink> = signal_sink.clone();
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(pm, pro_feature_gate(), signal_sink_trait),
        )
        .await
        .expect("start worker server");
        server.register_delegation(
            "del-late-force".into(),
            DelegationContext {
                enable_worker_progress: true,
            },
        );
        let token = server.issue_token("del-late-force", Duration::from_secs(60));
        let call_server = Arc::clone(&server);
        let call = tokio::spawn(async move {
            call_report_progress(call_server.as_ref(), &token, "hang until late force").await
        });
        signal_sink.entered.notified().await;
        assert_eq!(server.active_count(), 1);

        let force_shutdown = CancellationToken::new();
        let shutdown_server = Arc::clone(&server);
        let shutdown_force = force_shutdown.clone();
        let shutdown = tokio::spawn(async move {
            shutdown_server
                .shutdown_until_forced(Duration::from_secs(5), shutdown_force)
                .await
        });
        while !server.shutdown.is_cancelled() {
            tokio::task::yield_now().await;
        }
        force_shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(1), shutdown)
            .await
            .expect("late force request waited for the graceful handler deadline")
            .expect("worker shutdown task panicked");
        assert_eq!(server.active_count(), 0);
        call.abort();
        let _ = call.await;
    }

    #[tokio::test]
    async fn retirement_cancels_handler_registration_after_abort_all() {
        let registry = Arc::new(HandlerAbortRegistry::default());
        registry.abort_all();

        let (abort_registration, handler_registration) = registry.register();
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            futures::future::Abortable::new(std::future::pending::<()>(), abort_registration),
        )
        .await
        .expect("late handler registration escaped shutdown cancellation");
        assert!(result.is_err(), "late handler must be force-aborted");
        drop(handler_registration);
        tokio::time::timeout(Duration::from_millis(50), registry.wait())
            .await
            .expect("late registration wedged handler drain");
    }

    #[tokio::test]
    async fn retirement_awaits_timed_out_accept_loop_and_flusher_handles() {
        let dir = TempDir::new().expect("tempdir");
        let pm = pm_service_fixture(dir.path()).await;
        let server = WorkerMcpServer::start(
            "brain-A".into(),
            worker_mcp_deps_fixture(
                pm,
                pro_feature_gate(),
                Arc::new(RecordingWorkerSignalSink::default()),
            ),
        )
        .await
        .expect("start worker server");

        let (accept_started_tx, accept_started_rx) = std::sync::mpsc::sync_channel(1);
        let (accept_release_tx, accept_release_rx) = std::sync::mpsc::sync_channel(1);
        let accept_loop = tokio::task::spawn_blocking(move || {
            accept_started_tx.send(()).expect("signal accept start");
            let _ = accept_release_rx.recv();
        });
        let (flusher_started_tx, flusher_started_rx) = std::sync::mpsc::sync_channel(1);
        let (flusher_release_tx, flusher_release_rx) = std::sync::mpsc::sync_channel(1);
        let flusher = tokio::task::spawn_blocking(move || {
            flusher_started_tx.send(()).expect("signal flusher start");
            let _ = flusher_release_rx.recv();
        });
        accept_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("accept blocker did not start");
        flusher_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("flusher blocker did not start");
        let (old_accept, old_flusher) =
            server.replace_background_handles_for_test(accept_loop, flusher);
        for handle in [old_accept, old_flusher].into_iter().flatten() {
            handle.abort();
            let _ = handle.await;
        }

        let shutdown_server = Arc::clone(&server);
        let shutdown = tokio::spawn(async move {
            shutdown_server.shutdown(Duration::ZERO).await;
        });
        let returned_before_accept_ack = tokio::time::timeout(Duration::from_millis(50), async {
            while !shutdown.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        accept_release_tx.send(()).expect("release accept blocker");
        let returned_before_flusher_ack = tokio::time::timeout(Duration::from_millis(50), async {
            while !shutdown.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        flusher_release_tx
            .send(())
            .expect("release flusher blocker");
        tokio::time::timeout(Duration::from_secs(1), shutdown)
            .await
            .expect("shutdown did not finish after background acknowledgements")
            .expect("shutdown task panicked");

        assert!(
            !returned_before_accept_ack,
            "shutdown returned before the timed-out accept loop acknowledged abort"
        );
        assert!(
            !returned_before_flusher_ack,
            "shutdown returned before the timed-out flusher acknowledged abort"
        );
    }

    #[tokio::test]
    async fn emit_worker_write_audit_inner_writes_sentinel_comment() {
        let recorder = RecordingAdvanced {
            comments: Mutex::new(Vec::new()),
        };
        emit_worker_write_audit_inner(Some(&recorder), "del-A", "update_issue", "bd-123").await;

        let comments = recorder.comments.lock().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].0, "bd-123");
        assert!(comments[0]
            .1
            .starts_with(crate::plan::audit_sentinel::SENTINEL_PREFIX));
        assert!(comments[0].1.contains("\"kind\":\"worker-write\""));
        assert!(comments[0].1.contains("\"delegation_id\":\"del-A\""));
        assert!(comments[0].1.contains("\"tool\":\"update_issue\""));
        assert!(comments[0].1.contains("\"issue_id\":\"bd-123\""));
    }

    #[tokio::test]
    async fn emit_worker_write_audit_inner_logs_warning_on_failure() {
        let warnings = CapturedWarnings::default();
        let _serialize = crate::tracing_test_lock::guard();
        let _guard = tracing::subscriber::set_default(warnings.clone());

        emit_worker_write_audit_inner(Some(&FailingAdvanced), "del-A", "update_issue", "bd-123")
            .await;

        assert!(
            warnings.contains("WorkerWrite audit comment emission failed"),
            "expected warning about audit emission failure, got: {:?}",
            warnings.events.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn emit_worker_write_audit_inner_noop_when_advanced_none() {
        // Should not panic and should complete immediately.
        emit_worker_write_audit_inner(None, "del-A", "update_issue", "bd-123").await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn emit_worker_write_audit_inner_times_out_and_warns() {
        let _serialize = crate::tracing_test_lock::guard();
        let warnings = CapturedWarnings::default();

        let handle = tokio::spawn({
            let warnings = warnings.clone();
            async move {
                let _guard = tracing::subscriber::set_default(warnings);
                let hanging = HangingAdvanced;
                emit_worker_write_audit_inner(Some(&hanging), "del-A", "update_issue", "bd-123")
                    .await;
            }
        });

        tokio::time::advance(Duration::from_secs(6)).await;
        handle.await.expect("emit completes");

        assert!(
            warnings.contains("timed out after 5s"),
            "expected timeout warning, got: {:?}",
            warnings.events.lock().unwrap()
        );
    }

    #[test]
    fn delegation_dispatch_guard_drop_emits_summary_via_try_emit() {
        use std::sync::atomic::AtomicBool;

        struct TryEmitRecordingSink {
            events: Mutex<Vec<spur_acp::SpurEventBody>>,
            full: AtomicBool,
        }

        impl spur_mcp::events::McpEventSink for TryEmitRecordingSink {
            fn emit(&self, _event: spur_acp::SpurEventBody) {
                panic!("emit must not be called — try_emit should be used");
            }

            fn try_emit(
                &self,
                event: spur_acp::SpurEventBody,
            ) -> Result<(), spur_acp::SpurEventBody> {
                if self.full.load(Ordering::SeqCst) {
                    return Err(event);
                }
                self.events.lock().unwrap().push(event);
                Ok(())
            }
        }

        let sink = Arc::new(TryEmitRecordingSink {
            events: Mutex::new(Vec::new()),
            full: AtomicBool::new(false),
        });

        let guard = DelegationDispatchGuard::new(
            "d-test".into(),
            "session-test".into(),
            Arc::clone(&sink) as Arc<dyn spur_mcp::events::McpEventSink>,
        );
        guard.record_call("get_issue", 10, false);
        guard.record_call("get_issue", 20, false);
        guard.record_call("update_issue", 30, true);
        guard.completed.store(true, Ordering::Relaxed);
        drop(guard);

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        if let spur_acp::SpurEventBody::WorkerMcpDelegationSummary {
            delegation_id,
            brain_session_id,
            calls_total,
            calls_by_tool,
            errors,
            ..
        } = &events[0]
        {
            assert_eq!(delegation_id, "d-test");
            assert_eq!(brain_session_id, "session-test");
            assert_eq!(*calls_total, 3);
            assert_eq!(calls_by_tool.get("get_issue"), Some(&2));
            assert_eq!(calls_by_tool.get("update_issue"), Some(&1));
            assert_eq!(*errors, 1);
        } else {
            panic!("expected WorkerMcpDelegationSummary, got: {:?}", events[0]);
        }

        // When the sink is full, the guard should silently drop the event.
        let sink2 = Arc::new(TryEmitRecordingSink {
            events: Mutex::new(Vec::new()),
            full: AtomicBool::new(true),
        });
        let guard2 = DelegationDispatchGuard::new(
            "d-full".into(),
            "session-full".into(),
            Arc::clone(&sink2) as Arc<dyn spur_mcp::events::McpEventSink>,
        );
        guard2.record_call("get_issue", 5, false);
        guard2.completed.store(true, Ordering::Relaxed);
        drop(guard2);
        assert!(
            sink2.events.lock().unwrap().is_empty(),
            "event must be dropped when sink is full"
        );
    }

    #[test]
    fn compute_p99_latency_ms_matches_spec() {
        // Empty: 0
        assert_eq!(compute_p99_latency_ms(&[]), 0);
        // Single sample: max observed.
        assert_eq!(compute_p99_latency_ms(&[42]), 42);
        // <2 was already covered; for n>=2 use nearest-rank.
        // Two samples: ceil(0.99*2) = 2 → idx 1 → max.
        assert_eq!(compute_p99_latency_ms(&[10, 100]), 100);
        // 100 evenly-spaced samples 1..=100: ceil(0.99*100) = 99 → idx 98 → 99.
        let samples: Vec<u64> = (1..=100).collect();
        assert_eq!(compute_p99_latency_ms(&samples), 99);
        // Unsorted input: still uses sorted nearest-rank.
        let unsorted: Vec<u64> = vec![50, 5, 200, 100, 25];
        // Sorted: [5,25,50,100,200]; ceil(0.99*5) = 5 → idx 4 → 200.
        assert_eq!(compute_p99_latency_ms(&unsorted), 200);
    }

    #[test]
    fn knowledge_context_pack_schema_publishes_max_symbol_bodies_contract() {
        let schema = crate::tool_schemas::schema_value::<KnowledgeContextPackV2Params>();
        let max_symbol_bodies = &schema["properties"]["max_symbol_bodies"];

        assert_eq!(
            max_symbol_bodies["default"].as_u64(),
            Some(0),
            "unexpected schema: {max_symbol_bodies:#}"
        );
        assert_eq!(max_symbol_bodies["minimum"].as_f64(), Some(0.0));
        assert_eq!(max_symbol_bodies["maximum"].as_f64(), Some(5.0));
        assert_eq!(max_symbol_bodies["type"].as_str(), Some("integer"));
        assert!(!schema["required"]
            .as_array()
            .expect("required fields")
            .iter()
            .any(|field| field.as_str() == Some("max_symbol_bodies")));
    }

    #[test]
    fn knowledge_context_root_falls_back_to_repo_db_when_worker_db_missing() {
        let worker_dir = tempfile::tempdir().expect("worker tempdir");
        let repo_dir = tempfile::tempdir().expect("repo tempdir");
        std::fs::create_dir_all(repo_dir.path().join(".spur")).expect("create repo .spur");
        std::fs::write(repo_dir.path().join(".spur").join("analyst.duckdb"), b"db")
            .expect("write repo analyst db marker");

        let selected = select_knowledge_context_root(
            Some(worker_dir.path().to_path_buf()),
            Some(repo_dir.path().to_path_buf()),
        )
        .expect("selected root");

        assert_eq!(selected, repo_dir.path());
    }

    #[test]
    fn knowledge_context_root_prefers_worker_db_when_present() {
        let worker_dir = tempfile::tempdir().expect("worker tempdir");
        let repo_dir = tempfile::tempdir().expect("repo tempdir");
        std::fs::create_dir_all(worker_dir.path().join(".spur")).expect("create worker .spur");
        std::fs::create_dir_all(repo_dir.path().join(".spur")).expect("create repo .spur");
        std::fs::write(
            worker_dir.path().join(".spur").join("analyst.duckdb"),
            b"worker db",
        )
        .expect("write worker analyst db marker");
        std::fs::write(
            repo_dir.path().join(".spur").join("analyst.duckdb"),
            b"repo db",
        )
        .expect("write repo analyst db marker");

        let selected = select_knowledge_context_root(
            Some(worker_dir.path().to_path_buf()),
            Some(repo_dir.path().to_path_buf()),
        )
        .expect("selected root");

        assert_eq!(selected, worker_dir.path());
    }
}
