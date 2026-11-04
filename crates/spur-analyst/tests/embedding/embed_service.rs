use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use spur_analyst::embedding::sidecar_service::{EmbedService, MAX_EMBED_TEXTS};
use spur_graph::EMBEDDING_VECTOR_DIMENSIONS;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

fn vector(seed: f32) -> Vec<f32> {
    let mut vector = vec![0.0; EMBEDDING_VECTOR_DIMENSIONS];
    vector[0] = seed;
    vector[EMBEDDING_VECTOR_DIMENSIONS - 1] = seed + 1.0;
    vector
}

async fn wait_for_socket(path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(path).await {
            Ok(_) => return,
            Err(error) if tokio::time::Instant::now() < deadline => {
                if error.kind() != std::io::ErrorKind::NotFound
                    && error.kind() != std::io::ErrorKind::ConnectionRefused
                {
                    panic!("socket did not become reachable: {error}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("timed out waiting for socket: {error}"),
        }
    }
}

async fn spawn_stub_service<F>(
    embedder: F,
) -> (TempDir, tokio::task::JoinHandle<anyhow::Result<()>>)
where
    F: Fn(Vec<String>) -> Result<Vec<Vec<f32>>, String> + Send + Sync + 'static,
{
    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket = tempdir.path().join("embed.sock");
    let service = EmbedService::new("stub-model", || true, embedder);
    let handle = tokio::spawn(service.serve_socket(socket.clone()));
    wait_for_socket(&socket).await;
    (tempdir, handle)
}

#[tokio::test]
async fn socket_shutdown_joins_blocking_embedder_after_request_cancelled() {
    let tempdir = tempfile::tempdir().unwrap();
    let socket = tempdir.path().join("embed.sock");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started_tx));
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Mutex::new(release_rx);
    let service = EmbedService::new(
        "stub-model",
        || true,
        move |_texts| {
            started.lock().unwrap().take().unwrap().send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
            Ok(vec![vector(1.0)])
        },
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(service.serve_socket_until(socket.clone(), async {
        let _ = stopped.await;
    }));
    wait_for_socket(&socket).await;
    let client_socket = socket.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(client_socket).await.unwrap();
        stream
            .write_all(b"{\"v\":1,\"id\":1,\"op\":\"embed\",\"texts\":[\"query\"]}\n")
            .await
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await.unwrap()
    });
    started_rx.await.expect("embedder started");
    stop.send(()).unwrap();
    let premature = tokio::time::timeout(Duration::from_millis(100), &mut server).await;
    release_tx.send(()).unwrap();
    if premature.is_err() {
        server.await.unwrap().unwrap();
    }
    assert_eq!(
        client.await.unwrap(),
        0,
        "cancelled request returned a response"
    );
    assert!(
        premature.is_err(),
        "socket shutdown returned while blocking work was still running"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_socket_host_joins_its_blocking_embedder() {
    let tempdir = tempfile::tempdir().unwrap();
    let socket = tempdir.path().join("embed.sock");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started_tx));
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Mutex::new(release_rx);
    let service = EmbedService::new(
        "stub-model",
        || true,
        move |_texts| {
            started.lock().unwrap().take().unwrap().send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
            Ok(vec![vector(1.0)])
        },
    );
    let mut server = tokio::spawn(service.serve_socket(socket.clone()));
    wait_for_socket(&socket).await;
    let mut stream = UnixStream::connect(&socket).await.unwrap();
    stream
        .write_all(b"{\"v\":1,\"id\":1,\"op\":\"embed\",\"texts\":[\"query\"]}\n")
        .await
        .unwrap();
    started_rx.await.unwrap();
    server.abort();
    let premature = tokio::time::timeout(Duration::from_millis(100), &mut server).await;
    release_tx.send(()).unwrap();
    if premature.is_err() {
        assert!(server.await.unwrap_err().is_cancelled());
    }
    assert!(
        premature.is_err(),
        "cancelled host returned while blocking work remained"
    );
}

async fn send_request(socket: &Path, request: Value) -> Value {
    let mut stream = UnixStream::connect(socket).await.expect("connect");
    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .expect("write request");
    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut line).await.expect("read response");
    serde_json::from_str(line.trim_end()).expect("json response")
}

#[tokio::test]
async fn embed_request_returns_vectors_from_normalized_text() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_embedder = Arc::clone(&captured);
    let (tempdir, handle) = spawn_stub_service(move |texts| {
        captured_for_embedder
            .lock()
            .expect("captured lock")
            .push(texts.clone());
        Ok(texts
            .into_iter()
            .map(|text| vector(text.len() as f32))
            .collect())
    })
    .await;

    let response = send_request(
        &tempdir.path().join("embed.sock"),
        json!({"v":1,"id":"embed-1","op":"embed","texts":["alpha","beta"]}),
    )
    .await;

    assert_eq!(response["v"], 1);
    assert_eq!(response["id"], "embed-1");
    let vectors = response["vectors"].as_array().expect("vectors array");
    assert_eq!(vectors.len(), 2);
    assert_eq!(
        vectors[0].as_array().expect("first vector").len(),
        EMBEDDING_VECTOR_DIMENSIONS
    );
    assert_eq!(vectors[0][0], "search_query: alpha".len() as f64);
    assert_eq!(
        captured.lock().expect("captured lock").as_slice(),
        &[vec![
            "search_query: alpha".to_owned(),
            "search_query: beta".to_owned(),
        ]]
    );

    handle.abort();
}

#[tokio::test]
async fn ping_reports_model_and_ready_state_without_embedding() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket = tempdir.path().join("embed.sock");
    let service = EmbedService::new(
        "stub-model",
        || false,
        |_texts| panic!("ping must not invoke embedder"),
    );
    let handle = tokio::spawn(service.serve_socket(socket.clone()));
    wait_for_socket(&socket).await;

    let response = send_request(
        &socket,
        json!({"v":1,"id":"ping-1","op":"ping","texts":["ignored"]}),
    )
    .await;

    assert_eq!(response["v"], 1);
    assert_eq!(response["id"], "ping-1");
    assert_eq!(response["ok"], true);
    assert_eq!(response["model"], "stub-model");
    assert_eq!(response["ready"], false);
    assert!(response.get("vectors").is_none());

    handle.abort();
}

#[tokio::test]
async fn malformed_line_returns_error_and_keeps_connection_alive() {
    let (tempdir, handle) = spawn_stub_service(|texts| {
        Ok(texts
            .into_iter()
            .map(|text| vector(text.len() as f32))
            .collect())
    })
    .await;
    let socket = tempdir.path().join("embed.sock");
    let stream = UnixStream::connect(&socket).await.expect("connect");
    let mut reader = BufReader::new(stream);

    reader
        .get_mut()
        .write_all(b"not json\n")
        .await
        .expect("write malformed request");
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read error");
    let error: Value = serde_json::from_str(line.trim_end()).expect("error json");
    assert_eq!(error["v"], 1);
    assert!(error["error"]
        .as_str()
        .expect("error string")
        .contains("malformed request"));

    line.clear();
    reader
        .get_mut()
        .write_all(br#"{"v":1,"id":"ping-after-error","op":"ping"}"#)
        .await
        .expect("write ping");
    reader
        .get_mut()
        .write_all(b"\n")
        .await
        .expect("write newline");
    reader.read_line(&mut line).await.expect("read ping");
    let pong: Value = serde_json::from_str(line.trim_end()).expect("pong json");
    assert_eq!(pong["id"], "ping-after-error");
    assert_eq!(pong["ok"], true);

    handle.abort();
}

#[tokio::test]
async fn embed_request_rejects_oversized_batches() {
    let (tempdir, handle) =
        spawn_stub_service(|_texts| panic!("oversized batches must not invoke embedder")).await;
    let texts = vec!["too many"; MAX_EMBED_TEXTS + 1];
    let response = send_request(
        &tempdir.path().join("embed.sock"),
        json!({"v":1,"id":"embed-too-many","op":"embed","texts":texts}),
    )
    .await;

    assert_eq!(response["v"], 1);
    assert_eq!(response["id"], "embed-too-many");
    assert!(response["error"]
        .as_str()
        .expect("error string")
        .contains("at most 16 texts"));
    assert!(response.get("vectors").is_none());

    handle.abort();
}
