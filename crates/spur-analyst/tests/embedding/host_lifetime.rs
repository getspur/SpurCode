use std::process::Command;

#[tokio::test]
async fn compatibility_query_keeps_completed_result_when_cleanup_fails() {
    const CHILD: &str = "SPUR_S6_COMPAT_RESULT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init()
            .unwrap();
        assert!(
            spur_analyst::embedding::EmbeddingRuntime::global()
                .embed_query("completed query")
                .await
                .is_some(),
            "a completed result must survive later cleanup failure"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/embedding/fixtures/owned_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let log_path = dir.path().join("worker.log");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_lifetime::compatibility_query_keeps_completed_result_when_cleanup_fails",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", &log_path)
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
        String::from_utf8_lossy(&output.stderr).contains("embedding compatibility shutdown failed"),
        "cleanup error was not reported: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.lines().any(|line| line == "shutdown"), "{log}");
    let pid = log
        .lines()
        .find_map(|line| line.strip_prefix("service_pid "))
        .expect("helper pid");
    let probe = Command::new("kill").args(["-0", pid]).output().unwrap();
    assert!(
        !probe.status.success()
            && String::from_utf8_lossy(&probe.stderr).contains("No such process"),
        "helper still exists or probe failed: {}",
        String::from_utf8_lossy(&probe.stderr)
    );
}

/// Separate request modules and the direct worker entry point must share the
/// host's owner, including its eventual joined shutdown.
#[tokio::test]
async fn request_modules_and_direct_calls_share_host_runtime() {
    const CHILD: &str = "SPUR_S6_HOST_LIFETIME_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let root = std::path::PathBuf::from(std::env::var_os("SPUR_S6_HOST_ROOT").unwrap());
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.expect("warm host");
        for query in ["first module", "second module"] {
            let module =
                spur_analyst::mcp::AnalystMcpModule::new().with_embedding_runtime(runtime.clone());
            let result = spur_graph::mcp::with_worktree_root_for_request(
                root.clone(),
                module.dispatch(
                    "knowledge_context_pack_2",
                    serde_json::json!({"query": query}),
                ),
            )
            .await;
            // The empty fixture deliberately has no analyst tables. The call
            // must reach embedding before returning the database error.
            assert!(result.is_err());
        }
        let result = spur_graph::mcp::with_worktree_root_for_request(
            root,
            spur_analyst::mcp::knowledge_context_pack_2_with_runtime(
                &serde_json::json!({"query": "direct worker"}),
                &runtime,
            ),
        )
        .await;
        assert!(result.is_err());
        runtime.shutdown().await.expect("joined host shutdown");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".spur")).unwrap();
    drop(duckdb::Connection::open(dir.path().join(".spur/analyst.duckdb")).unwrap());
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2,
            "max_text_bytes": 8192, "max_total_text_bytes": 16384,
            "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0,
        "startup_timeout_ms": 10000, "request_timeout_ms": 10000,
        "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let log_path = dir.path().join("worker.log");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_lifetime::request_modules_and_direct_calls_share_host_runtime",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_S6_HOST_ROOT", dir.path())
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
        4,
        "{log}"
    );
    assert_eq!(log.lines().last(), Some("shutdown"), "{log}");
}
