use std::process::Command;

#[tokio::test]
async fn owned_runtime_reuses_one_warmed_helper_and_sends_shutdown() {
    const CHILD: &str = "SPUR_S6_OWNED_RUNTIME_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
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

#[tokio::test]
async fn owned_runtime_off_never_starts_a_helper() {
    const CHILD: &str = "SPUR_S6_OWNED_OFF_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.expect("off warm succeeds");
        assert!(runtime.embed_query("unused").await.is_none());
        runtime.shutdown().await.expect("off shutdown succeeds");
        assert!(
            runtime.warm().await.is_err(),
            "shutdown must close admission even in off mode"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("worker.log");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::owned_runtime_off_never_starts_a_helper",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_ANALYST_EMBED_MODE", "off")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!log.exists(), "off started a worker");
}

#[tokio::test]
async fn cancelled_descriptor_is_joined_before_shutdown_returns() {
    const CHILD: &str = "SPUR_S6_OWNED_CANCEL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        let clone = runtime.clone();
        let warm = tokio::spawn(async move { clone.warm().await });
        let log = std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("descriptor_pid ")
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "descriptor never started"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        warm.abort();
        assert!(warm.await.unwrap_err().is_cancelled());
        runtime
            .shutdown()
            .await
            .expect("cancelled descriptor and cleanup joined");
        let events = std::fs::read_to_string(log).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("descriptor_pid "))
            .expect("descriptor pid was recorded");
        let probe = Command::new("/usr/bin/python3")
            .args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid])
            .status().expect("probe descriptor pid");
        assert!(
            probe.success(),
            "descriptor still exists after shutdown: {events}"
        );
        assert!(
            !events.lines().any(|line| line == "service"),
            "late publication: {events}"
        );
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
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::cancelled_descriptor_is_joined_before_shutdown_returns",
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
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn shutdown_cancels_and_reaps_inflight_query() {
    const CHILD: &str = "SPUR_S6_OWNED_QUERY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.unwrap();
        let clone = runtime.clone();
        let query = tokio::spawn(async move { clone.embed_query("pending").await });
        let log = std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let events = std::fs::read_to_string(&log).unwrap_or_default();
            if events
                .lines()
                .filter(|line| line.starts_with("embed "))
                .count()
                == 2
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "query never started: {events}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let queued_runtime = runtime.clone();
        let queued = tokio::spawn(async move { queued_runtime.embed_query("queued").await });
        tokio::task::yield_now().await;
        assert!(!queued.is_finished(), "queued query was not admitted");
        let overflow = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            runtime.embed_query("overflow"),
        )
        .await
        .expect("full queue must reject promptly");
        assert!(overflow.is_none());
        let close = runtime
            .shutdown()
            .await
            .expect_err("retired helper close must be reported");
        assert!(
            format!("{close:#}").contains("worker unavailable"),
            "{close:#}"
        );
        assert!(
            runtime.shutdown().await.is_err(),
            "close failure must be retained"
        );
        assert!(queued.await.unwrap().is_none());
        assert!(
            query.await.unwrap().is_none(),
            "cancelled query returned a vector"
        );
        assert!(runtime.embed_query("after shutdown").await.is_none());
        let events = std::fs::read_to_string(log).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("service_pid "))
            .unwrap();
        let probe = Command::new("/usr/bin/python3")
            .args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid])
            .status().unwrap();
        assert!(
            probe.success(),
            "service still exists after shutdown: {events}"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/embedding/fixtures/owned_fault_worker.py");
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
            "owned_runtime::shutdown_cancels_and_reaps_inflight_query",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_hang_after_warm")
        .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn shutdown_joins_inflight_sidecar_query() {
    const CHILD: &str = "SPUR_S6_OWNED_SIDECAR_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use tokio::io::AsyncReadExt;
        let path = std::env::var_os("SPUR_EMBED_SOCKET").unwrap();
        let listener = tokio::net::UnixListener::bind(path).unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            assert!(stream.read(&mut buf).await.unwrap() > 0);
            started_tx.send(()).unwrap();
            stream.read(&mut buf).await.unwrap()
        });
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        let clone = runtime.clone();
        let query = tokio::spawn(async move { clone.embed_query("pending").await });
        started_rx.await.unwrap();
        runtime.shutdown().await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), query)
            .await
            .expect("shutdown did not join the sidecar query")
            .unwrap();
        assert!(result.is_none());
        assert_eq!(server.await.unwrap(), 0, "sidecar connection still open");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::shutdown_joins_inflight_sidecar_query",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_ANALYST_EMBED_MODE", "sidecar")
        .env("SPUR_EMBED_SOCKET", dir.path().join("sidecar.sock"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn shutdown_retains_helper_abort() {
    const CHILD: &str = "SPUR_S6_OWNED_ABORT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.unwrap();
        let first = runtime
            .shutdown()
            .await
            .expect_err("helper abort must fail shutdown");
        if std::env::var("SPUR_GRAPH_TEST_WORKER_FAULT").as_deref() == Ok("shutdown_hang") {
            assert!(
                format!("{first:#}").contains("deadline exceeded"),
                "{first:#}"
            );
        }
        let second = runtime
            .shutdown()
            .await
            .expect_err("shutdown error must be retained");
        assert_eq!(format!("{first:#}"), format!("{second:#}"));
        let events =
            std::fs::read_to_string(std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap()).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("service_pid "))
            .unwrap();
        let probe = Command::new("/usr/bin/python3")
            .args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid])
            .status().unwrap();
        assert!(probe.success(), "aborted helper still exists: {events}");
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
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::shutdown_retains_helper_abort",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "shutdown_abort")
        .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn aborted_caller_cannot_leave_query_running_after_shutdown() {
    const CHILD: &str = "SPUR_S6_OWNED_ABORT_QUERY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.unwrap();
        let clone = runtime.clone();
        let query = tokio::spawn(async move { clone.embed_query("pending").await });
        let log = std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let events = std::fs::read_to_string(&log).unwrap_or_default();
            if events
                .lines()
                .filter(|line| line.starts_with("embed "))
                .count()
                == 2
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "query never started: {events}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
        assert!(
            runtime.shutdown().await.is_err(),
            "retired helper error must be retained"
        );
        assert!(runtime.embed_query("after shutdown").await.is_none());
        let events = std::fs::read_to_string(log).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("service_pid "))
            .unwrap();
        let probe = Command::new("/usr/bin/python3")
            .args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid])
            .status().unwrap();
        assert!(probe.success(), "aborted caller left a helper: {events}");
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
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::aborted_caller_cannot_leave_query_running_after_shutdown",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_hang_after_warm")
        .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shutdown_timeout_is_retained_and_helper_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/embedding/fixtures/owned_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 100, "shutdown_poll_interval_ms": 10
    });
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::shutdown_retains_helper_abort",
            "--nocapture",
        ])
        .env("SPUR_S6_OWNED_ABORT_CHILD", "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "shutdown_hang")
        .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cancelled_query_cannot_publish_delayed_reply() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("worker.log");
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/embedding/fixtures/owned_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_runtime::aborted_caller_cannot_leave_query_running_after_shutdown",
            "--nocapture",
        ])
        .env("SPUR_S6_OWNED_ABORT_QUERY_CHILD", "1")
        .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_delay_after_warm")
        .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::thread::sleep(std::time::Duration::from_secs(1));
    let events = std::fs::read_to_string(log).unwrap();
    assert!(
        !events.contains("late_reply"),
        "cancelled helper published late: {events}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn warm_routes_modes_without_spurious_local_helpers() {
    const CHILD: &str = "SPUR_S6_WARM_MODE_CHILD";
    if let Ok(case) = std::env::var(CHILD) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = if case == "auto_reachable" {
            let listener =
                tokio::net::UnixListener::bind(std::env::var_os("SPUR_EMBED_SOCKET").unwrap())
                    .unwrap();
            Some(tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).await.unwrap();
                assert!(String::from_utf8_lossy(&buf[..n]).contains("ping"));
                stream.write_all(b"{\"v\":1,\"ok\":true}\n").await.unwrap();
            }))
        } else {
            None
        };
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.expect("mode warm");
        if case == "auto_unavailable" {
            assert!(runtime.embed_query("reuse").await.is_some());
        }
        runtime.shutdown().await.unwrap();
        if let Some(server) = server {
            server.await.unwrap();
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
    let config = serde_json::json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    for case in ["sidecar", "auto_reachable", "auto_unavailable"] {
        let log = dir.path().join(format!("{case}.log"));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "owned_runtime::warm_routes_modes_without_spurious_local_helpers",
                "--nocapture",
            ])
            .env(CHILD, case)
            .env(
                "SPUR_ANALYST_EMBED_MODE",
                if case == "sidecar" { "sidecar" } else { "auto" },
            )
            .env("SPUR_EMBED_SOCKET", dir.path().join(format!("{case}.sock")))
            .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if case == "auto_unavailable" {
            let events = std::fs::read_to_string(&log).unwrap();
            assert_eq!(
                events.lines().filter(|line| *line == "service").count(),
                1,
                "{events}"
            );
            assert_eq!(
                events
                    .lines()
                    .filter(|line| line.starts_with("embed "))
                    .count(),
                2,
                "{events}"
            );
            assert_eq!(events.lines().last(), Some("shutdown"), "{events}");
        } else {
            assert!(!log.exists(), "{case} launched a local helper");
        }
    }
}
