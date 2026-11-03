use spur_embedding::{read_frame, ErrorCode, ProtocolLimits, Reply, ReplyEnvelope};
use spur_embedding::{write_frame, ModelIdentity, Request, RequestEnvelope, PROTOCOL_VERSION};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "spur-embedding-metadata-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let repo = root.join("models--nomic-ai--nomic-embed-text-v1.5");
        let snap = repo.join("snapshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        fs::create_dir_all(snap.join("onnx")).unwrap();
        fs::create_dir_all(repo.join("refs")).unwrap();
        fs::write(
            repo.join("refs/main"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        for file in [
            "onnx/model.onnx",
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ] {
            fs::write(snap.join(file), file).unwrap();
        }
        Self(root)
    }
    fn snapshot(&self) -> PathBuf {
        self.0.join("models--nomic-ai--nomic-embed-text-v1.5/snapshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture_descriptor(root: &Path) -> Reply {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "NomicEmbedTextV15",
            "10000",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env("SPUR_MODEL_CACHE", root)
        .env("SPUR_ORT_CACHE", "/dev/null/unusable")
        .output()
        .unwrap();
    let reply: ReplyEnvelope = read_frame(&mut Cursor::new(&output.stdout), &limits()).unwrap();
    reply.reply
}

#[test]
fn additional_model_descriptors_bind_their_own_artifacts_and_profile() {
    use spur_embedding::ModelId;
    let fixture = Fixture::new();
    for (name, repo_name, revision, model) in [
        (
            "CodeRankEmbed",
            "models--jamie8johnson--CodeRankEmbed-onnx",
            "151669b173750250e611e1be00c812f112ea6020",
            ModelId::CodeRankEmbed,
        ),
        (
            "JinaEmbeddingsV2BaseCode",
            "models--jinaai--jina-embeddings-v2-base-code",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ModelId::JinaEmbeddingsV2BaseCode,
        ),
    ] {
        let repo = fixture.0.join(repo_name);
        let snapshot = repo.join("snapshots").join(revision);
        fs::create_dir_all(snapshot.join("onnx")).unwrap();
        fs::create_dir_all(repo.join("refs")).unwrap();
        fs::write(repo.join("refs/main"), revision).unwrap();
        for file in [
            "onnx/model.onnx",
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ] {
            fs::write(snapshot.join(file), file).unwrap();
        }
        let describe = || {
            let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
                .args([
                    "--describe-model",
                    name,
                    "10000",
                    "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
                    "7",
                    "65536",
                    "2",
                    "4096",
                    "8192",
                    "768",
                    "256",
                    "1",
                ])
                .env("SPUR_MODEL_CACHE", &fixture.0)
                .output()
                .unwrap();
            assert!(output.status.success());
            let reply: ReplyEnvelope =
                read_frame(&mut Cursor::new(output.stdout), &limits()).unwrap();
            identity(reply.reply)
        };
        let before = describe();
        assert_eq!(before.model, model);
        assert_eq!(before.dimensions, 768);
        assert_eq!(before.pooling, "mean");
        assert_eq!(before.normalization, "unit");
        assert_eq!(before.prefix, "legacy_preformatted");
        fs::write(snapshot.join("config.json"), b"changed config").unwrap();
        assert_ne!(
            before.tokenizer_fingerprint,
            describe().tokenizer_fingerprint
        );
    }
}
fn identity(reply: Reply) -> ModelIdentity {
    match reply {
        Reply::Ready { identity } => identity,
        other => panic!("{other:?}"),
    }
}

#[test]
fn artifact_changes_and_missing_or_duplicate_sources_affect_identity() {
    let fixture = Fixture::new();
    let initial = identity(fixture_descriptor(&fixture.0));
    fs::write(fixture.snapshot().join("config.json"), b"changed config").unwrap();
    let config_changed = identity(fixture_descriptor(&fixture.0));
    assert_ne!(
        initial.tokenizer_fingerprint,
        config_changed.tokenizer_fingerprint
    );
    fs::write(fixture.snapshot().join("onnx/model.onnx"), b"changed model").unwrap();
    let model_changed = identity(fixture_descriptor(&fixture.0));
    assert_ne!(
        config_changed.model_fingerprint,
        model_changed.model_fingerprint
    );
    fs::remove_file(fixture.snapshot().join("tokenizer_config.json")).unwrap();
    assert!(matches!(
        fixture_descriptor(&fixture.0),
        Reply::Error { .. }
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            "config.json",
            fixture.snapshot().join("tokenizer_config.json"),
        )
        .unwrap();
        assert!(matches!(
            fixture_descriptor(&fixture.0),
            Reply::Error { .. }
        ));
        fs::remove_file(fixture.snapshot().join("tokenizer_config.json")).unwrap();
        fs::hard_link(
            fixture.snapshot().join("config.json"),
            fixture.snapshot().join("tokenizer_config.json"),
        )
        .unwrap();
        assert!(matches!(
            fixture_descriptor(&fixture.0),
            Reply::Error { .. }
        ));
    }
}

#[test]
fn stale_hash_and_profile_are_rejected_before_native_start() {
    let fixture = Fixture::new();
    let original = identity(fixture_descriptor(&fixture.0));
    for field in 0..3 {
        let mut forged = original.clone();
        match field {
            0 => forged.model_fingerprint = "0".repeat(64),
            1 => forged.tokenizer_fingerprint = "0".repeat(64),
            _ => forged.pooling = "cls".into(),
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
            .args(["65536", "2", "4096", "8192", "768", "256", "1", "10000"])
            .env("SPUR_MODEL_CACHE", &fixture.0)
            .env("SPUR_ORT_CACHE", "/dev/null/unusable")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        write_frame(
            child.stdin.as_mut().unwrap(),
            &RequestEnvelope {
                version: PROTOCOL_VERSION,
                generation: "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap(),
                request_id: 7,
                request: Request::Hello { identity: forged },
            },
            &limits(),
        )
        .unwrap();
        let reply: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits()).unwrap();
        assert_eq!(reply.request_id, 7);
        assert!(matches!(
            reply.reply,
            Reply::Error {
                code: ErrorCode::InvalidRequest,
                ..
            }
        ));
        assert!(child.wait().unwrap().success());
    }
}

#[test]
fn missing_cache_artifact_fails_with_correlated_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "NomicEmbedTextV15",
            "548026095",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env("SPUR_MODEL_CACHE", "/dev/null/missing")
        .env("SPUR_ORT_CACHE", "/dev/null/unusable")
        .output()
        .unwrap();
    let reply: ReplyEnvelope = read_frame(&mut Cursor::new(&output.stdout), &limits()).unwrap();
    assert_eq!(reply.request_id, 7);
    assert!(matches!(reply.reply, Reply::Error { .. }));
}

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

#[test]
fn invalid_artifact_budget_has_correlated_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "NomicEmbedTextV15",
            "not-a-budget",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env("SPUR_ORT_CACHE", "/dev/null/unusable")
        .output()
        .unwrap();
    let reply: ReplyEnvelope = read_frame(&mut Cursor::new(&output.stdout), &limits()).unwrap();
    assert_eq!(reply.request_id, 7);
    assert!(matches!(
        reply.reply,
        Reply::Error {
            code: ErrorCode::InvalidRequest,
            ..
        }
    ));
}

fn describe(budget: &str) -> (std::process::Output, ReplyEnvelope) {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "NomicEmbedTextV15",
            budget,
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
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
        .env("SPUR_ORT_CACHE", "/dev/null/unusable")
        .output()
        .unwrap();
    let mut bytes = Cursor::new(&output.stdout);
    let reply: ReplyEnvelope =
        read_frame(&mut bytes, &limits()).expect("correlated framed response");
    assert_eq!(
        bytes.position() as usize,
        output.stdout.len(),
        "stdout contains only one frame"
    );
    assert_eq!(reply.request_id, 7);
    assert_eq!(
        reply.generation.to_string(),
        "d32a3002-a3aa-4b1c-9ae7-ff0824970841"
    );
    (output, reply)
}

#[test]
fn descriptor_exact_budget_is_repeatable_without_native_runtime() {
    let (first_output, first) = describe("548026095");
    let (second_output, second) = describe("548026095");
    assert!(first_output.status.success());
    assert!(second_output.status.success());
    assert!(matches!(first.reply, Reply::Ready { .. }));
    assert_eq!(first, second);
}

#[test]
fn descriptor_rejects_one_byte_under_budget() {
    let (_, reply) = describe("548026094");
    assert!(matches!(
        reply.reply,
        Reply::Error {
            code: ErrorCode::Unavailable | ErrorCode::InvalidRequest,
            ..
        }
    ));
}

#[test]
fn real_additional_models_enforce_measured_exact_artifact_budgets() {
    for (name, total) in [
        ("CodeRankEmbed", 548_431_020u64),
        ("JinaEmbeddingsV2BaseCode", 644_080_771u64),
    ] {
        for (budget, expected_ready) in [(total, true), (total - 1, false)] {
            let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
                .args([
                    "--describe-model",
                    name,
                    &budget.to_string(),
                    "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
                    "7",
                    "65536",
                    "2",
                    "4096",
                    "8192",
                    "768",
                    "256",
                    "1",
                ])
                .env("SPUR_MODEL_CACHE", "/private/tmp/spur-s4c-model-cache")
                .env("SPUR_ORT_CACHE", "/dev/null/unusable")
                .output()
                .unwrap();
            assert!(output.status.success());
            let reply: ReplyEnvelope =
                read_frame(&mut Cursor::new(output.stdout), &limits()).unwrap();
            assert_eq!(
                matches!(reply.reply, Reply::Ready { .. }),
                expected_ready,
                "{name} budget={budget} reply={:?}",
                reply.reply
            );
        }
    }
}

#[test]
fn coderank_fetches_the_pinned_revision_into_the_selected_cache() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::Duration,
    };
    const REVISION: &str = "151669b173750250e611e1be00c812f112ea6020";
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_server = stop.clone();
    let server = thread::spawn(move || {
        let mut seen = Vec::new();
        while !stop_server.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("fixture listener: {error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            let request = String::from_utf8(request).unwrap();
            let url = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            assert!(url.contains(&format!("/resolve/{REVISION}/")), "{url}");
            let name = url.split(&format!("/resolve/{REVISION}/")).nth(1).unwrap();
            let body = name.as_bytes();
            let metadata = request.to_ascii_lowercase().contains("range: bytes=0-0");
            let payload = if metadata { &body[..1] } else { body };
            let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nX-Repo-Commit: {REVISION}\r\nETag: \"{}\"\r\nConnection: close\r\n\r\n", payload.len(), payload.len()-1, body.len(), name.replace('/', "-"));
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(payload).unwrap();
            seen.push(name.to_owned());
        }
        seen
    });
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "CodeRankEmbed",
            "10000",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env("SPUR_MODEL_CACHE", &fixture.0)
        .env("HF_ENDPOINT", format!("http://{address}"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    let seen = server.join().unwrap();
    assert!(output.status.success());
    let reply: ReplyEnvelope = read_frame(&mut Cursor::new(output.stdout), &limits()).unwrap();
    assert!(
        matches!(reply.reply, Reply::Ready { .. }),
        "{:?}",
        reply.reply
    );
    for name in [
        "onnx/model.onnx",
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
    ] {
        assert!(
            seen.iter().any(|value| value == name),
            "missing {name}: {seen:?}"
        );
    }
}

#[test]
fn real_worker_rejects_a_different_model_bound_to_jina_hashes() {
    use spur_embedding::ModelId;
    let cache = "/private/tmp/spur-s4c-model-cache";
    let descriptor = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args([
            "--describe-model",
            "JinaEmbeddingsV2BaseCode",
            "644080771",
            "d32a3002-a3aa-4b1c-9ae7-ff0824970841",
            "7",
            "65536",
            "2",
            "4096",
            "8192",
            "768",
            "256",
            "1",
        ])
        .env("SPUR_MODEL_CACHE", cache)
        .output()
        .unwrap();
    assert!(descriptor.status.success());
    let reply: ReplyEnvelope = read_frame(&mut Cursor::new(descriptor.stdout), &limits()).unwrap();
    let mut forged = identity(reply.reply);
    forged.model = ModelId::CodeRankEmbed;
    let mut child = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .args(["65536", "2", "4096", "8192", "768", "256", "1", "548431020"])
        .env("SPUR_MODEL_CACHE", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    write_frame(
        child.stdin.as_mut().unwrap(),
        &RequestEnvelope {
            version: PROTOCOL_VERSION,
            generation: "d32a3002-a3aa-4b1c-9ae7-ff0824970841".parse().unwrap(),
            request_id: 7,
            request: Request::Hello { identity: forged },
        },
        &limits(),
    )
    .unwrap();
    let reply: ReplyEnvelope = read_frame(child.stdout.as_mut().unwrap(), &limits()).unwrap();
    assert!(
        matches!(
            reply.reply,
            Reply::Error {
                code: ErrorCode::InvalidRequest,
                ..
            }
        ),
        "{:?}",
        reply.reply
    );
    assert!(child.wait().unwrap().success());
}
