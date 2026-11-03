use spur_embedding::{read_frame, ModelId, ProtocolLimits, Reply, ReplyEnvelope};
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 65536,
        max_texts: 2,
        max_text_bytes: 4096,
        max_total_text_bytes: 8192,
        max_dimensions: 768,
        max_identity_bytes: 256,
        max_pending: 1,
    }
}

fn describe_nomic() -> spur_embedding::ModelIdentity {
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "NomicEmbedTextV15",
            "548026095",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "1",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env(
            "SPUR_MODEL_CACHE",
            "/Users/kevintruong/.spur/cache/fastembed",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("descriptor exceeded deadline");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "descriptor failed: {:?}",
        child.wait_with_output().unwrap()
    );
    let response: ReplyEnvelope = read_frame(&mut child.stdout.take().unwrap(), &limits()).unwrap();
    assert_eq!(response.request_id, 1);
    match response.reply {
        Reply::Ready { identity } => {
            assert_eq!(identity.model, ModelId::NomicEmbedTextV15);
            assert_eq!(identity.dimensions, 768);
            assert_eq!(identity.model_fingerprint.len(), 64);
            assert_eq!(identity.tokenizer_fingerprint.len(), 64);
            identity
        }
        other => panic!("expected actual identity, got {other:?}"),
    }
}

#[test]
fn descriptor_returns_actual_nomic_identity_with_bounded_execution() {
    describe_nomic();
}

// The reference uses the existing registered FastEmbed loader directly, with the
// same preformatted inputs as graph. It lives in the test process, separate from
// the worker process, and its native owner is released before returning.
#[cfg(target_os = "macos")]
#[test]
fn product_nomic_matches_registered_reference_and_closes() {
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use spur_embedding::{ClientPolicy, EmbeddingClient};

    let inputs = vec![
        "search_query: find a function that sorts numbers".to_owned(),
        "search_document: title: sorting | text: Sort values in ascending order.".to_owned(),
    ];
    assert!(ort::init_from(env!("SPUR_BUNDLED_ORT_PATH"))
        .commit()
        .unwrap());
    let mut reference = TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::NomicEmbedTextV15)
            .with_max_length(8192)
            .with_show_download_progress(false)
            .with_cache_dir("/Users/kevintruong/.spur/cache/fastembed".into()),
    )
    .unwrap();
    let expected = reference.embed(inputs.clone(), None).unwrap();
    drop(reference);
    ort::environment::try_shutdown().unwrap();
    assert_eq!(expected.len(), 2);
    assert!(expected
        .iter()
        .all(|row| row.len() == 768 && row.iter().all(|x| x.is_finite())));

    let identity = describe_nomic();
    std::env::set_var(
        "SPUR_ORT_CACHE",
        "/private/tmp/spur-embedding-worker-native-test-cache",
    );
    std::env::set_var(
        "SPUR_MODEL_CACHE",
        "/Users/kevintruong/.spur/cache/fastembed",
    );
    let policy = ClientPolicy {
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: Duration::from_secs(120),
        request_timeout: Duration::from_secs(120),
        shutdown_timeout: Duration::from_secs(30),
        shutdown_poll_interval: Duration::from_millis(20),
    };
    for _ in 0..2 {
        let client = EmbeddingClient::new(
            env!("CARGO_BIN_EXE_spur-embedding-worker"),
            ["65536", "2", "4096", "8192", "768", "256", "1", "548026095"]
                .into_iter()
                .map(Into::into)
                .collect(),
            identity.clone(),
            limits(),
            policy.clone(),
        )
        .unwrap();
        let first = client
            .embed(inputs.clone(), Instant::now() + Duration::from_secs(120))
            .unwrap();
        let second = client
            .embed(inputs.clone(), Instant::now() + Duration::from_secs(120))
            .unwrap();
        client
            .shutdown(Instant::now() + Duration::from_secs(30))
            .unwrap();

        let events = client.test_events();
        let spawned: Vec<_> = events
            .iter()
            .filter(|event| event.kind == "spawned")
            .collect();
        assert_eq!(spawned.len(), 1);
        let replies: Vec<_> = events
            .iter()
            .filter(|event| {
                event.kind == "reply_validated" && event.status.as_deref() == Some("embeddings")
            })
            .collect();
        assert_eq!(replies.len(), 2);
        assert!(replies
            .iter()
            .all(|event| event.generation == spawned[0].generation && event.pid == spawned[0].pid));
        assert!(events
            .iter()
            .any(|event| event.kind == "reaped" && event.pid == spawned[0].pid));

        assert_eq!(first.len(), expected.len());
        assert_eq!(first.len(), 2);
        for ((actual, again), reference) in first.iter().zip(&second).zip(&expected) {
            assert_eq!(actual.len(), 768);
            assert_eq!(again.len(), 768);
            assert!(actual.iter().all(|x| x.is_finite()));
            let norm = actual.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4, "norm {norm}");
            for ((a, b), expected) in actual.iter().zip(again).zip(reference) {
                assert!((a - expected).abs() < 1e-4, "worker/reference mismatch");
                assert!((a - b).abs() < 1e-4, "repeated batch mismatch");
            }
        }
    }
}

#[test]
fn hello_rejects_unverified_artifact_identity() {
    use spur_embedding::{write_frame, ModelIdentity, Request, RequestEnvelope, PROTOCOL_VERSION};
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args(["65536", "2", "4096", "8192", "768", "256", "1"])
        .env(
            "SPUR_ORT_CACHE",
            "/private/tmp/spur-embedding-worker-native-test-cache",
        )
        .env(
            "SPUR_MODEL_CACHE",
            "/Users/kevintruong/.spur/cache/fastembed",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let identity = ModelIdentity {
        model: ModelId::NomicEmbedTextV15,
        dimensions: 768,
        model_fingerprint: "0".repeat(64),
        tokenizer_fingerprint: "0".repeat(64),
        pooling: "mean".into(),
        normalization: "unit".into(),
        prefix: "nomic".into(),
    };
    let generation = "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap();
    write_frame(
        child.stdin.as_mut().unwrap(),
        &RequestEnvelope {
            version: PROTOCOL_VERSION,
            generation,
            request_id: 1,
            request: Request::Hello { identity },
        },
        &limits(),
    )
    .unwrap();
    let reply: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits()).unwrap();
    let _ = child.kill();
    child.wait().unwrap();
    assert!(
        matches!(reply.reply, Reply::Error { .. }),
        "unverified identity accepted: {:?}",
        reply.reply
    );
}
