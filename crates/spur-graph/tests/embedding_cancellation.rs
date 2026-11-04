#![cfg(all(unix, feature = "embed"))]

use spur_embedding::Cancellation;
use spur_graph::{load_embedding_model_cancellable, EmbeddingModelSelection};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn graph_model_discovery_observes_owner_cancellation() {
    const CHILD: &str = "SPUR_S6_GRAPH_CANCEL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let cancel = Cancellation::new();
        let trigger = cancel.clone();
        let job = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            trigger.cancel();
        });
        let start = Instant::now();
        let result = load_embedding_model_cancellable(
            EmbeddingModelSelection::NomicEmbedTextV15,
            false,
            cancel,
            start + Duration::from_secs(10),
        );
        job.join().unwrap();
        let error = result.err().expect("cancelled discovery returned a model");
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "discovery did not cancel promptly"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/embedding_fault_worker.py");
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
            "graph_model_discovery_observes_owner_cancellation",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "descriptor_hang")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let log = std::fs::read_to_string(dir.path().join("worker.log")).unwrap_or_default();
    assert!(log.contains("describe "), "descriptor did not start: {log}");
    assert!(
        !log.contains("service"),
        "cancelled discovery published a service: {log}"
    );
}

#[test]
fn graph_query_observes_owner_cancellation_without_outer_lock() {
    const CHILD: &str = "SPUR_S6_GRAPH_QUERY_CANCEL_CHILD";
    const LOG: &str = "SPUR_GRAPH_TEST_WORKER_LOG";
    if std::env::var_os(CHILD).is_some() {
        let model =
            spur_graph::load_embedding_model(EmbeddingModelSelection::NomicEmbedTextV15, false)
                .unwrap();
        let cancel = Cancellation::new();
        let trigger = cancel.clone();
        let log = std::path::PathBuf::from(std::env::var_os(LOG).unwrap());
        let job = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("embed ")
            {
                assert!(Instant::now() < deadline, "query never reached helper");
                std::thread::sleep(Duration::from_millis(10));
            }
            trigger.cancel();
        });
        let result =
            model.embed_cancellable(&["query"], Instant::now() + Duration::from_secs(10), cancel);
        job.join().unwrap();
        let error = result.err().expect("cancelled query returned vectors");
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/embedding_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000, "request_timeout_ms": 10000,
        "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "graph_query_observes_owner_cancellation_without_outer_lock",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(LOG, dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_hang")
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
