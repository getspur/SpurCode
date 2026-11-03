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

#[test]
fn protocol_reports_product_embeddings_unavailable_and_closes_cleanly() {
    use spur_embedding::{
        read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolLimits, Reply,
        ReplyEnvelope, Request, RequestEnvelope, PROTOCOL_VERSION,
    };
    let limits = ProtocolLimits {
        max_frame_bytes: 4096,
        max_texts: 4,
        max_text_bytes: 256,
        max_total_text_bytes: 512,
        max_dimensions: 16,
        max_identity_bytes: 256,
        max_pending: 1,
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args(["4096", "4", "256", "512", "16", "256", "1"])
        .env(
            "SPUR_ORT_CACHE",
            "/private/tmp/spur-embedding-worker-native-test-cache",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.as_mut().unwrap();
    let output = child.stdout.as_mut().unwrap();
    let identity = ModelIdentity {
        model: ModelId::CodeRankEmbed,
        dimensions: 2,
        model_fingerprint: "model".into(),
        tokenizer_fingerprint: "tokenizer".into(),
        pooling: "mean".into(),
        normalization: "unit".into(),
        prefix: "code".into(),
    };
    let generation = "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap();
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
            input,
            &RequestEnvelope {
                version: PROTOCOL_VERSION,
                generation,
                request_id: id,
                request,
            },
            &limits,
        )
        .unwrap();
        let response: ReplyEnvelope = read_frame(output, &limits).unwrap();
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
            (_, unexpected) => panic!("unexpected reply {unexpected:?}"),
        }
    }
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn unavailable_reply_fits_the_callers_small_identity_limit() {
    use spur_embedding::{
        read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolLimits, Reply,
        ReplyEnvelope, Request, RequestEnvelope, PROTOCOL_VERSION,
    };
    let limits = ProtocolLimits {
        max_frame_bytes: 4096,
        max_texts: 1,
        max_text_bytes: 1,
        max_total_text_bytes: 1,
        max_dimensions: 1,
        max_identity_bytes: 1,
        max_pending: 1,
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args(["4096", "1", "1", "1", "1", "1", "1"])
        .env(
            "SPUR_ORT_CACHE",
            "/private/tmp/spur-embedding-worker-native-test-cache",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let identity = ModelIdentity {
        model: ModelId::CodeRankEmbed,
        dimensions: 1,
        model_fingerprint: "m".into(),
        tokenizer_fingerprint: "t".into(),
        pooling: "p".into(),
        normalization: "n".into(),
        prefix: "x".into(),
    };
    let generation = "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap();
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
                identity,
                texts: vec!["x".into()],
            },
        ),
        (3, Request::Shutdown),
    ] {
        write_frame(
            child.stdin.as_mut().unwrap(),
            &RequestEnvelope {
                version: PROTOCOL_VERSION,
                generation,
                request_id: id,
                request,
            },
            &limits,
        )
        .unwrap();
        let response: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits).unwrap();
        match (id, response.reply) {
            (1, Reply::Ready { .. })
            | (
                2,
                Reply::Error {
                    code: ErrorCode::Unavailable,
                    ..
                },
            )
            | (3, Reply::Closed) => {}
            (_, unexpected) => panic!("unexpected reply {unexpected:?}"),
        }
    }
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[cfg(target_os = "macos")]
fn root_check_initialization_error_reply(identity_limit: usize) {
    use spur_embedding::{
        read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolLimits, Reply,
        ReplyEnvelope, Request, RequestEnvelope, PROTOCOL_VERSION,
    };
    use std::io::{Read, Write};
    use std::os::unix::process::ExitStatusExt;
    let limits = ProtocolLimits {
        max_frame_bytes: 4096,
        max_texts: 1,
        max_text_bytes: 1,
        max_total_text_bytes: 1,
        max_dimensions: 1,
        max_identity_bytes: identity_limit,
        max_pending: 1,
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args(["4096", "1", "1", "1", "1", &identity_limit.to_string(), "1"])
        .env("SPUR_ORT_CACHE", "/dev/null/not-a-directory")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let generation = "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap();
    let sent = write_frame(
        child.stdin.as_mut().unwrap(),
        &RequestEnvelope {
            version: PROTOCOL_VERSION,
            generation,
            request_id: 1,
            request: Request::Hello {
                identity: ModelIdentity {
                    model: ModelId::CodeRankEmbed,
                    dimensions: 1,
                    model_fingerprint: "m".into(),
                    tokenizer_fingerprint: "t".into(),
                    pooling: "p".into(),
                    normalization: "n".into(),
                    prefix: "x".into(),
                },
            },
        },
        &limits,
    );
    let _ = child.stdin.as_mut().unwrap().flush();
    let response: Result<ReplyEnvelope, _> = read_frame(child.stdout.as_mut().unwrap(), &limits);
    drop(child.stdin.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    let wire = match &response {
        Ok(reply)
            if reply.version != PROTOCOL_VERSION
                || reply.generation != generation
                || reply.request_id != 1 =>
        {
            "uncorrelated"
        }
        Ok(ReplyEnvelope {
            reply:
                Reply::Error {
                    code: ErrorCode::Internal | ErrorCode::Unavailable,
                    ..
                },
            ..
        }) => "framed_failure",
        Ok(_) => "wrong_reply",
        Err(_) => "stderr_only",
    };
    println!("ROOT_NATIVE_INIT_CASE {{\"identity_limit\":{identity_limit},\"wire_outcome\":\"{wire}\",\"sent\":{},\"raw_wait_status\":{}}}", sent.is_ok(), status.clone().into_raw());
    eprintln!("ROOT_NATIVE_INIT_DETAILS limit={identity_limit} reply={response:?} status={status:?} stderr={stderr}");
    assert_eq!(
        wire, "framed_failure",
        "native startup failure must reach the valid Hello caller as a correlated Error frame"
    );
    assert!(
        status.signal().is_none(),
        "typed startup failure must not abort: {status:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn root_initialization_failure_is_framed_with_one_byte_error_limit() {
    root_check_initialization_error_reply(1);
}

#[cfg(target_os = "macos")]
#[test]
fn root_initialization_failure_is_framed_with_regular_error_limit() {
    root_check_initialization_error_reply(256);
}
