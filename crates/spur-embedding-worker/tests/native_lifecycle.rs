#![cfg(feature = "test-support")]

use std::process::{Command, Stdio};

fn probe(case: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_embedding-native-probe"))
        .arg(case)
        .env(
            "SPUR_ORT_CACHE",
            "/private/tmp/spur-embedding-worker-native-test-cache",
        )
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap()
}

#[test]
fn native_inference_and_checked_shutdown_exit_normally() {
    let mut cases = vec![
        "inference",
        "inference",
        "inference",
        "retained-output",
        "idle",
        "repeated-close",
        "reopen",
        "active-close",
    ];
    #[cfg(target_os = "macos")]
    {
        cases.push("init-error");
        cases.push("native-loader-error");
    }
    for case in cases {
        let output = probe(case);
        assert!(
            output.status.success(),
            "{case}: status {:?}, stdout {}, stderr {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn killed_probe_is_reported_as_signal_not_success() {
    use std::os::unix::process::ExitStatusExt;
    let mut child = Command::new(env!("CARGO_BIN_EXE_embedding-native-probe"))
        .arg("wait-for-signal")
        .env(
            "SPUR_ORT_CACHE",
            "/private/tmp/spur-embedding-worker-native-test-cache",
        )
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Read;
    let mut ready = [0u8; 1];
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(ready, [b'R']);
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9));
    assert!(!status.success());
}

use spur_embedding::{
    read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolLimits, Reply,
    ReplyEnvelope, Request, RequestEnvelope, PROTOCOL_VERSION,
};

fn limits(identity_bytes: usize) -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 65536,
        max_texts: 2,
        max_text_bytes: 4096,
        max_total_text_bytes: 8192,
        max_dimensions: 768,
        max_identity_bytes: identity_bytes,
        max_pending: 1,
    }
}

fn actual_identity() -> ModelIdentity {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
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
        .output()
        .unwrap();
    assert!(output.status.success());
    let reply: ReplyEnvelope = read_frame(&mut output.stdout.as_slice(), &limits(256)).unwrap();
    match reply.reply {
        Reply::Ready { identity } => identity,
        other => panic!("{other:?}"),
    }
}

fn worker(identity_limit: &str, cache: &str) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            identity_limit,
            "1",
            "548026095",
        ])
        .env("SPUR_ORT_CACHE", cache)
        .env(
            "SPUR_MODEL_CACHE",
            "/Users/kevintruong/.spur/cache/fastembed",
        )
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn envelope(id: u64, request: Request) -> RequestEnvelope {
    RequestEnvelope {
        version: PROTOCOL_VERSION,
        generation: "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap(),
        request_id: id,
        request,
    }
}

#[test]
fn protocol_reports_product_embeddings_unavailable_and_closes_cleanly() {
    let identity = actual_identity();
    let mut child = worker(
        "256",
        "/private/tmp/spur-embedding-worker-native-test-cache",
    );
    for (id, request) in [
        (
            1,
            Request::Hello {
                identity: identity.clone(),
            },
        ),
        (
            2,
            Request::Embed {
                identity: identity.clone(),
                texts: vec!["hello".into()],
            },
        ),
        (3, Request::Shutdown),
    ] {
        write_frame(
            child.stdin.as_mut().unwrap(),
            &envelope(id, request),
            &limits(256),
        )
        .unwrap();
        let response: ReplyEnvelope =
            read_frame(child.stdout.as_mut().unwrap(), &limits(256)).unwrap();
        assert_eq!(response.request_id, id);
        match (id, response.reply) {
            (1, Reply::Ready { identity: returned }) => assert_eq!(returned, identity),
            (
                2,
                Reply::Error {
                    code: ErrorCode::Unavailable,
                    ..
                },
            )
            | (3, Reply::Closed) => {}
            (_, other) => panic!("unexpected reply {other:?}"),
        }
    }
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn unavailable_reply_fits_the_callers_small_identity_limit() {
    let mut child = worker("1", "/dev/null/not-a-directory");
    let identity = ModelIdentity {
        model: ModelId::CodeRankEmbed,
        dimensions: 1,
        model_fingerprint: "m".into(),
        tokenizer_fingerprint: "t".into(),
        pooling: "p".into(),
        normalization: "n".into(),
        prefix: "x".into(),
    };
    write_frame(
        child.stdin.as_mut().unwrap(),
        &envelope(1, Request::Hello { identity }),
        &limits(1),
    )
    .unwrap();
    let response: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits(1)).unwrap();
    assert!(
        matches!(response.reply, Reply::Error { code: ErrorCode::Unavailable, ref message } if message.len() == 1)
    );
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[cfg(target_os = "macos")]
#[test]
fn native_initialization_failure_is_correlated_for_verified_identity() {
    use std::os::unix::process::ExitStatusExt;
    let identity = actual_identity();
    let mut child = worker("256", "/dev/null/not-a-directory");
    write_frame(
        child.stdin.as_mut().unwrap(),
        &envelope(1, Request::Hello { identity }),
        &limits(256),
    )
    .unwrap();
    let response: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits(256)).unwrap();
    assert_eq!(response.request_id, 1);
    assert_eq!(
        response.generation,
        envelope(1, Request::Shutdown).generation
    );
    assert!(matches!(
        response.reply,
        Reply::Error {
            code: ErrorCode::Unavailable,
            ..
        }
    ));
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    println!("ROOT_NATIVE_INIT_CASE {{\"identity_limit\":256,\"wire_outcome\":\"framed_failure\",\"raw_wait_status\":{}}}", status.into_raw());
    assert!(status.signal().is_none());
    assert!(!status.success());
}

#[test]
fn one_byte_identity_limit_rejects_valid_length_fingerprints_before_startup() {
    let mut child = worker("1", "/dev/null/not-a-directory");
    let identity = ModelIdentity {
        model: ModelId::NomicEmbedTextV15,
        dimensions: 768,
        model_fingerprint: "0".repeat(64),
        tokenizer_fingerprint: "0".repeat(64),
        pooling: "mean".into(),
        normalization: "unit".into(),
        prefix: "legacy_preformatted".into(),
    };
    write_frame(
        child.stdin.as_mut().unwrap(),
        &envelope(1, Request::Hello { identity }),
        &limits(256),
    )
    .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("identity field"));
}
