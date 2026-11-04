#![allow(clippy::await_holding_lock)]

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

struct ShutdownTaskDropProbe(Arc<AtomicBool>);

impl Drop for ShutdownTaskDropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn test_callback_server() -> Arc<McpCallbackServer> {
    let brain_session_id = BrainSessionId::new(SessionId::new());
    let (server, _channel) = McpCallbackServer::new(
        Some(&brain_session_id),
        None,
        None,
        DetachedContinuationCtx {
            on_complete: Arc::new(|_, _| Box::pin(async {})),
        },
        Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
        Arc::new(FeatureGate::new(
            spur_license::policy::PolicyResolver::embedded(),
        )),
    );
    Arc::new(server)
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn callback_shutdown_joins_pending_warm() {
    const CHILD: &str = "SPUR_S6_CALLBACK_WARM_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let server = test_callback_server();
        let (_url, listener) = Arc::clone(&server).start().await.unwrap();
        let log = std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let events = std::fs::read_to_string(&log).unwrap_or_default();
            if events.contains("descriptor_pid ") {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "warm did not start: {events}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::timeout(Duration::from_secs(10), server.shutdown())
            .await
            .expect("callback shutdown did not join warm");
        let _ = listener.await;
        let events = std::fs::read_to_string(log).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("descriptor_pid "))
            .unwrap();
        let probe = std::process::Command::new("/usr/bin/python3").args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid]).status().unwrap();
        assert!(
            probe.success(),
            "descriptor still exists after shutdown: {events}"
        );
        assert!(
            !events.lines().any(|line| line == "service"),
            "late helper: {events}"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
    let config = json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "server::tests::callback_shutdown_joins_pending_warm",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "descriptor_hang")
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
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn callback_requests_reuse_helper_until_server_shutdown() {
    const CHILD: &str = "SPUR_S6_CALLBACK_HOST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        if std::env::var_os("SPUR_S6_CALLBACK_TRACE").is_some() {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .try_init()
                .unwrap();
        }
        let mut server = test_callback_server();
        Arc::get_mut(&mut server).unwrap().repo_root =
            Some(std::env::var_os("SPUR_S6_CALLBACK_ROOT").unwrap().into());
        for query in ["callback first", "callback second"] {
            let response = server
                .handle_tool_call(
                    json!(1),
                    json!({
                        "name": "knowledge_context_pack_2", "arguments": {"query": query}
                    }),
                )
                .await;
            assert!(
                response.error.is_some(),
                "empty fixture must return database error"
            );
        }
        if std::env::var_os("SPUR_S6_CALLBACK_FORCE").is_some() {
            let force = CancellationToken::new();
            force.cancel();
            assert!(server.shutdown_until_forced(&force).await);
        } else {
            server.shutdown().await;
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
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
            "server::tests::callback_requests_reuse_helper_until_server_shutdown",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_S6_CALLBACK_ROOT", dir.path())
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
async fn forced_callback_shutdown_reports_helper_abort() {
    let dir = tempfile::tempdir().unwrap();
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
            "server::tests::callback_requests_reuse_helper_until_server_shutdown",
            "--nocapture",
        ])
        .env("SPUR_S6_CALLBACK_HOST_CHILD", "1")
        .env("SPUR_S6_CALLBACK_TRACE", "1")
        .env("SPUR_S6_CALLBACK_FORCE", "1")
        .env("SPUR_S6_CALLBACK_ROOT", dir.path())
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
        String::from_utf8_lossy(&output.stderr).contains("callback embedding shutdown failed"),
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

#[tokio::test(flavor = "current_thread")]
async fn root_completion_guard_disarms_after_acknowledged_completion() {
    let server = test_callback_server();
    let root_force = CancellationToken::new();
    *server.root_force_shutdown.lock().unwrap() = Some(root_force.clone());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    let abort_on_drop = AbortRootOnDrop::new(Arc::clone(&server));
    let waiter = tokio::spawn(await_root_completion(abort_on_drop, done_rx));
    done_tx
        .send(())
        .expect("completion waiter must remain live");
    waiter.await.expect("completion waiter must not panic");

    assert!(
        !root_force.is_cancelled(),
        "acknowledged graceful completion must not be reclassified as forced"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn root_completion_guard_forces_children_when_waiter_is_aborted() {
    let server = test_callback_server();
    let root_force = CancellationToken::new();
    *server.root_force_shutdown.lock().unwrap() = Some(root_force.clone());
    let (_done_tx, done_rx) = tokio::sync::oneshot::channel();

    let abort_on_drop = AbortRootOnDrop::new(Arc::clone(&server));
    let waiter = tokio::spawn(await_root_completion(abort_on_drop, done_rx));
    waiter.abort();
    let _ = waiter.await;

    assert!(
        root_force.is_cancelled(),
        "aborting the outer waiter must force the callback-server tree"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn graceful_root_completion_is_not_reported_as_forced() {
    let server = test_callback_server();
    let root_force = CancellationToken::new();
    *server.root_force_shutdown.lock().unwrap() = Some(root_force.clone());
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    server.__test_set_root_shutdown_tx(shutdown_tx);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let abort_on_drop = AbortRootOnDrop::new(Arc::clone(&server));
    let completion_waiter = tokio::spawn(await_root_completion(abort_on_drop, done_rx));

    server.__test_set_root_handle(tokio::spawn({
        let root_force = root_force.clone();
        async move {
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {}
                () = root_force.cancelled() => {}
            }
            done_tx
                .send(())
                .expect("outer completion waiter must remain live");
            // Let the waiter process the acknowledgement before this root join
            // resolves, making graceful/forced classification deterministic.
            tokio::task::yield_now().await;
        }
    }));

    let external_force = CancellationToken::new();
    let forced = server.shutdown_until_forced(&external_force).await;
    completion_waiter
        .await
        .expect("completion waiter must not panic");

    assert!(!forced, "clean root retirement must remain graceful");
    assert!(
        !root_force.is_cancelled(),
        "normal completion must disarm the outer force guard"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_until_forced_joins_handles_taken_by_graceful_path() {
    let server = test_callback_server();
    let task_dropped = Arc::new(AtomicBool::new(false));
    let task_started = Arc::new(Notify::new());
    let recovery_task = AbortOnDropHandle::new(tokio::spawn({
        let task_dropped = Arc::clone(&task_dropped);
        let task_started = Arc::clone(&task_started);
        async move {
            let _probe = ShutdownTaskDropProbe(task_dropped);
            task_started.notify_one();
            std::future::pending::<()>().await;
        }
    }));
    task_started.notified().await;
    server.startup_recovery.lock().unwrap().handle = Some(StartupRecoveryTaskHandle {
        cancel_tx: None,
        handle: recovery_task,
    });

    let force = CancellationToken::new();
    let shutdown = tokio::spawn({
        let server = Arc::clone(&server);
        let force = force.clone();
        async move { server.shutdown_until_forced(&force).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.startup_recovery.lock().unwrap().handle.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("graceful shutdown never took the recovery handle");

    force.cancel();
    let forced = tokio::time::timeout(Duration::from_secs(1), shutdown)
        .await
        .expect("forced root shutdown must acknowledge promptly")
        .expect("shutdown task panicked");

    assert!(forced);
    assert!(
        task_dropped.load(Ordering::SeqCst),
        "root shutdown returned before its taken recovery task acknowledged abort"
    );
}

#[test]
fn telemetry_tool_name_maps_known_tools_and_hashes_unknown() {
    use spur_telemetry::events::IntoProp;

    assert_eq!(
        McpCallbackServer::telemetry_mcp_tool_name("submit_plan").into_prop(),
        json!("submit_plan")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_tool_name("dispatch_task").into_prop(),
        json!("dispatch_task")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_tool_name("review_task").into_prop(),
        json!("review_task")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_tool_name("get_task_diff").into_prop(),
        json!("get_task_diff")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_tool_name("list_tools").into_prop(),
        json!("list_tools")
    );

    let unknown = "customer_email@example.com";
    let hashed = McpCallbackServer::telemetry_mcp_tool_name(unknown).into_prop();
    assert_eq!(
        hashed,
        spur_telemetry::tier2_events::HashedShort::from_sha256_prefix(unknown).into_prop()
    );
    assert_ne!(hashed, json!(unknown));
}

#[test]
fn telemetry_server_name_maps_known_servers_and_hashes_unknown() {
    use spur_telemetry::events::IntoProp;

    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("github").into_prop(),
        json!("github")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("posthog").into_prop(),
        json!("posthog")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("spur-mcp").into_prop(),
        json!("spur-mcp")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("stitch").into_prop(),
        json!("stitch")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("playwright").into_prop(),
        json!("playwright")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("context7").into_prop(),
        json!("context7")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("firebase").into_prop(),
        json!("firebase")
    );
    assert_eq!(
        McpCallbackServer::telemetry_mcp_server_name("sequential-thinking").into_prop(),
        json!("sequential-thinking")
    );

    let unknown = "private-server-name";
    let hashed = McpCallbackServer::telemetry_mcp_server_name(unknown).into_prop();
    assert_eq!(
        hashed,
        spur_telemetry::tier2_events::HashedShort::from_sha256_prefix(unknown).into_prop()
    );
    assert_ne!(hashed, json!(unknown));
}

#[test]
fn telemetry_outcome_from_json_rpc_response_tracks_errors() {
    assert_eq!(
        McpCallbackServer::telemetry_outcome_from_json_rpc_response(&JsonRpcResponse::success(
            Value::Null,
            json!({})
        )),
        spur_telemetry::tier1_events::Outcome::Ok
    );
    assert_eq!(
        McpCallbackServer::telemetry_outcome_from_json_rpc_response(
            &JsonRpcResponse::internal_error(Value::Null, "failed")
        ),
        spur_telemetry::tier1_events::Outcome::Error
    );
}

static BEADS_SQLITE_SERIAL: OnceLock<Mutex<()>> = OnceLock::new();

fn beads_sqlite_serial_guard() -> MutexGuard<'static, ()> {
    BEADS_SQLITE_SERIAL
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap()
}

#[cfg(test)]
fn attach_beads_workspace(repo: &std::path::Path, w: &spur_pm::test_workspace::TestBeadsWorkspace) {
    let beads_dir = repo.join(".beads");
    std::fs::create_dir(&beads_dir).expect("create test .beads directory");
    // Copy db + WAL + SHM (beads_rust uses WAL mode and skips checkpoint on
    // Drop; bare `fs::copy(beads.db)` loses every uncheckpointed write).
    w.copy_db_to(&beads_dir);
}

#[cfg(test)]
async fn init_beads_pm(
    repo: &std::path::Path,
) -> (
    spur_pm::test_workspace::TestBeadsWorkspace,
    std::sync::Arc<spur_pm::PmService>,
) {
    let w = spur_pm::test_workspace::TestBeadsWorkspace::init();
    attach_beads_workspace(repo, &w);

    let pm = std::sync::Arc::new(
        spur_pm::PmService::try_new(None, true, false, repo, None)
            .await
            .expect("PmService::try_new failed")
            .expect("expected Some(PmService)"),
    );
    (w, pm)
}

mod build_worker_info_tests;
mod clobber_review_tests;
mod merge_plan_tests;
mod recover_orphaned_dispatch_tests;
mod sync_tests;
mod versioned_cache_tests;
