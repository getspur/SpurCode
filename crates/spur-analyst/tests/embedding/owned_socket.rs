use std::process::Command;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn public_socket_sigterm_joins_helper() {
    const CHILD: &str = "SPUR_S6_SOCKET_SIGNAL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        spur_analyst::embedding::sidecar_service::serve(Some(
            std::env::var_os("SPUR_S6_SOCKET_PATH").unwrap().into(),
        ))
        .await
        .unwrap();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("embed.sock");
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
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "owned_socket::public_socket_sigterm_joins_helper",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_S6_SOCKET_PATH", &socket)
        .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = child.spawn().unwrap();
    let pid = child.id().unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let stream = loop {
        match tokio::net::UnixStream::connect(&socket).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "socket unavailable: {error}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    };
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"v\":1,\"id\":1,\"op\":\"embed\",\"texts\":[\"query\"]}\n")
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&line).unwrap()["vectors"].is_array());
    assert!(Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("server did not stop")
        .unwrap();
    let events = std::fs::read_to_string(log).unwrap();
    let helper = events
        .lines()
        .find_map(|line| line.strip_prefix("service_pid "))
        .unwrap();
    let reap_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let probe = Command::new("/usr/bin/python3").args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", helper]).status().unwrap();
        if probe.success() {
            break;
        }
        if tokio::time::Instant::now() >= reap_deadline {
            let _ = Command::new("kill").args(["-KILL", helper]).status();
            panic!("helper remained after server exit: {events}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        output.status.success(),
        "server did not exit cleanly: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(events.lines().last(), Some("shutdown"), "{events}");
    assert!(!socket.exists(), "socket remained after shutdown");
}

#[tokio::test]
async fn socket_shutdown_cancels_query_and_reports_helper_failure() {
    const CHILD: &str = "SPUR_S6_SOCKET_CANCEL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let socket = std::path::PathBuf::from(std::env::var_os("SPUR_S6_SOCKET_PATH").unwrap());
        let log = std::path::PathBuf::from(std::env::var_os("SPUR_GRAPH_TEST_WORKER_LOG").unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let service = tokio::spawn(spur_analyst::embedding::sidecar_service::serve_until(
            Some(socket.clone()),
            async {
                let _ = stopped.await;
            },
        ));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let stream = loop {
            match tokio::net::UnixStream::connect(&socket).await {
                Ok(stream) => break stream,
                Err(error) => {
                    assert!(tokio::time::Instant::now() < deadline, "{error}");
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        };
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        write
            .write_all(b"{\"v\":1,\"id\":1,\"op\":\"embed\",\"texts\":[\"first\"]}\n")
            .await
            .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&line).unwrap()["vectors"].is_array());
        write
            .write_all(b"{\"v\":1,\"id\":2,\"op\":\"embed\",\"texts\":[\"pending\"]}\n")
            .await
            .unwrap();
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
                "query did not start: {events}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        stop.send(()).unwrap();
        let error = service
            .await
            .unwrap()
            .expect_err("forced helper cleanup must be reported");
        assert!(
            format!("{error:#}").contains("worker unavailable"),
            "{error:#}"
        );
        line.clear();
        assert_eq!(
            reader.read_line(&mut line).await.unwrap(),
            0,
            "cancelled query returned data"
        );
        assert!(!socket.exists(), "socket remained after shutdown");
        let events = std::fs::read_to_string(log).unwrap();
        let pid = events
            .lines()
            .find_map(|line| line.strip_prefix("service_pid "))
            .unwrap();
        let probe = Command::new("/usr/bin/python3").args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid]).status().unwrap();
        assert!(probe.success(), "helper still exists: {events}");
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
            "owned_socket::socket_shutdown_cancels_query_and_reports_helper_failure",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_S6_SOCKET_PATH", dir.path().join("embed.sock"))
        .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_hang_after_warm")
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

#[tokio::test]
async fn public_v1_socket_uses_local_helper_and_joins_shutdown() {
    const CHILD: &str = "SPUR_S6_SOCKET_HOST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let socket = std::path::PathBuf::from(std::env::var_os("SPUR_S6_SOCKET_PATH").unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let service = tokio::spawn(spur_analyst::embedding::sidecar_service::serve_until(
            Some(socket.clone()),
            async {
                let _ = stopped.await;
            },
        ));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let stream = loop {
            match tokio::net::UnixStream::connect(&socket).await {
                Ok(stream) => break stream,
                Err(error) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "socket unavailable: {error}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        };
        let (read, mut write) = stream.into_split();
        write
            .write_all(
                b"{\"v\":1,\"id\":\"one\",\"op\":\"embed\",\"texts\":[\"alpha\",\"beta\"]}\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        BufReader::new(read).read_line(&mut response).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["id"], "one");
        assert_eq!(value["vectors"].as_array().unwrap().len(), 2);
        assert!(value["vectors"].as_array().unwrap().iter().all(|row| row
            .as_array()
            .unwrap()
            .len()
            == spur_graph::EMBEDDING_VECTOR_DIMENSIONS));
        stop.send(()).unwrap();
        service
            .await
            .unwrap()
            .expect("joined socket and helper shutdown");
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
    let log = dir.path().join("worker.log");
    let socket = dir.path().join("embed.sock");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_socket::public_v1_socket_uses_local_helper_and_joins_shutdown",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("SPUR_S6_SOCKET_PATH", &socket)
        .env("SPUR_EMBED_SOCKET", &socket)
        .env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
        .env("SPUR_ANALYST_EMBED_MODE", "sidecar")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
    let inputs = std::fs::read_to_string(format!("{}.inputs", log.display())).unwrap();
    let rows: Vec<Vec<String>> = inputs
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            vec!["search_query: alpha".to_string()],
            vec!["search_query: beta".to_string()]
        ]
    );
}
