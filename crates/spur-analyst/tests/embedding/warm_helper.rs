use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn warm_starts_the_supervised_model_session() {
    const CHILD: &str = "SPUR_S6_WARM_TEST_CHILD";
    const LOG: &str = "SPUR_GRAPH_TEST_WORKER_LOG";
    if std::env::var_os(CHILD).is_some() {
        spur_analyst::embedding::warm_embed_model();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let log = std::fs::read_to_string(std::env::var_os(LOG).unwrap()).unwrap_or_default();
            if log.lines().any(|line| line.starts_with("hello ")) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "warm-up never opened a helper session; trace: {log}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
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
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "warm_helper::warm_starts_the_supervised_model_session",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(LOG, dir.path().join("worker.log"))
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
