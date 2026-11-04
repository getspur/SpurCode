use std::process::Command;

#[tokio::test]
async fn owned_runtime_reuses_one_warmed_helper_and_sends_shutdown() {
    const CHILD: &str = "SPUR_S6_OWNED_RUNTIME_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::EmbeddingRuntime::new();
        runtime.warm().await.expect("warm helper inference");
        let first = runtime.clone();
        let second = runtime.clone();
        assert!(first.embed_query("first query").await.is_some());
        assert!(second.embed_query("second query").await.is_some());
        runtime.shutdown().await.expect("joined helper shutdown");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
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
            "owned_runtime::owned_runtime_reuses_one_warmed_helper_and_sends_shutdown",
            "--nocapture",
        ])
        .env(CHILD, "1")
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
    let log = std::fs::read_to_string(log_path).unwrap_or_default();
    assert_eq!(
        log.lines().filter(|line| *line == "service").count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("hello "))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("embed "))
            .count(),
        3,
        "{log}"
    );
    assert_eq!(log.lines().last(), Some("shutdown"), "{log}");
}
