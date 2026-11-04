//! Fault injection for the supervisor protocol, not an inference implementation.
use spur_embedding::{
    read_frame, write_frame, ErrorCode, ProtocolLimits, Reply, ReplyEnvelope, Request,
    RequestEnvelope,
};
use std::{
    env,
    io::{self, Write as _},
    thread,
    time::Duration,
};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 65536,
        max_texts: 16,
        max_text_bytes: 1024,
        max_total_text_bytes: 4096,
        max_dimensions: 16,
        max_identity_bytes: 128,
        max_pending: 4,
    }
}
fn hang() -> ! {
    loop {
        thread::park();
    }
}
fn main() {
    let mut args = env::args().skip(1);
    let mode = args.next().unwrap();
    if mode.starts_with("describe_") {
        use spur_embedding::{ModelId, ModelIdentity, PROTOCOL_VERSION};
        assert_eq!(args.next().as_deref(), Some("--describe-model"));
        let _model = args.next().unwrap();
        let _budget = args.next().unwrap();
        let generation = args.next().unwrap().parse().unwrap();
        let request_id = args.next().unwrap().parse().unwrap();
        if mode == "describe_hang" {
            hang();
        }
        if mode == "describe_truncated" {
            io::stdout().write_all(&100u32.to_be_bytes()).unwrap();
            io::stdout().write_all(b"{").unwrap();
            return;
        }
        let identity = ModelIdentity {
            model: if mode == "describe_wrong_model" {
                ModelId::CodeRankEmbed
            } else {
                ModelId::NomicEmbedTextV15
            },
            dimensions: 2,
            model_fingerprint: "model".into(),
            tokenizer_fingerprint: "tokenizer".into(),
            pooling: "mean".into(),
            normalization: "unit".into(),
            prefix: "none".into(),
        };
        let reply = ReplyEnvelope {
            version: PROTOCOL_VERSION,
            generation: if mode == "describe_wrong_generation" {
                uuid::Uuid::new_v4()
            } else {
                generation
            },
            request_id: if mode == "describe_wrong_request" {
                request_id + 1
            } else {
                request_id
            },
            reply: if mode == "describe_error" {
                Reply::Error {
                    code: ErrorCode::Unavailable,
                    message: "injected".into(),
                }
            } else {
                Reply::Ready { identity }
            },
        };
        write_frame(&mut io::stdout(), &reply, &limits()).unwrap();
        if mode == "describe_trailing" {
            io::stdout().write_all(b"extra").unwrap();
        }
        if mode == "describe_failed_exit" {
            panic!("injected failing exit");
        }
        return;
    }
    if let Some(path) = args.next() {
        use std::fs::OpenOptions;
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(log, "{}", std::process::id()).unwrap();
    }
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let limits = limits();
    loop {
        let request: RequestEnvelope = match read_frame(&mut input, &limits) {
            Ok(v) => v,
            Err(_) => return,
        };
        let reply = match request.request {
            Request::Hello { identity } => {
                if mode == "startup_hang" {
                    hang();
                }
                Reply::Ready { identity }
            }
            Request::Shutdown => {
                if mode == "abort_shutdown" {
                    std::process::abort();
                }
                Reply::Closed
            }
            Request::Embed {
                mut identity,
                texts,
            } => match mode.as_str() {
                "late_reply" => {
                    thread::sleep(Duration::from_millis(200));
                    Reply::Embeddings {
                        rows: vec![vec![1.0; identity.dimensions]; texts.len()],
                        identity,
                    }
                }
                "abort" => std::process::abort(),
                "abort_mid" => {
                    output.write_all(&100u32.to_be_bytes()).unwrap();
                    output.write_all(b"{").unwrap();
                    output.flush().unwrap();
                    std::process::abort();
                }
                "truncated" => {
                    output.write_all(&100u32.to_be_bytes()).unwrap();
                    output.write_all(b"{").unwrap();
                    return;
                }
                "malformed" => {
                    output.write_all(&1u32.to_be_bytes()).unwrap();
                    output.write_all(b"{").unwrap();
                    output.flush().unwrap();
                    continue;
                }
                "hang" => hang(),
                "error" => Reply::Error {
                    code: ErrorCode::InvalidRequest,
                    message: "injected".into(),
                },
                "wrong_model" => {
                    identity.model_fingerprint.push('x');
                    Reply::Embeddings {
                        rows: vec![vec![1.0; identity.dimensions]; texts.len()],
                        identity,
                    }
                }
                "wrong_dimension" => Reply::Embeddings {
                    rows: vec![vec![1.0; identity.dimensions + 1]; texts.len()],
                    identity,
                },
                _ => Reply::Embeddings {
                    rows: vec![vec![1.0; identity.dimensions]; texts.len()],
                    identity,
                },
            },
        };
        let shutdown = matches!(reply, Reply::Closed);
        let mut response = ReplyEnvelope {
            version: request.version,
            generation: request.generation,
            request_id: request.request_id,
            reply,
        };
        if mode == "wrong_generation" && matches!(response.reply, Reply::Embeddings { .. }) {
            response.generation = uuid::Uuid::new_v4();
        }
        // Emit malformed semantic values without the safe writer's outgoing validation.
        if mode == "wrong_dimension" && matches!(response.reply, Reply::Embeddings { .. }) {
            let bytes = serde_json::to_vec(&response).unwrap();
            output
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .unwrap();
            output.write_all(&bytes).unwrap();
        } else {
            write_frame(&mut output, &response, &limits).unwrap();
        }
        output.flush().unwrap();
        if shutdown {
            if mode == "shutdown_hang" {
                hang();
            }
            return;
        }
        if mode == "blocked_write" && matches!(response.reply, Reply::Ready { .. }) {
            hang();
        }
        if mode == "abort_after" && matches!(response.reply, Reply::Embeddings { .. }) {
            std::process::abort();
        }
        if mode == "delay" {
            thread::sleep(Duration::from_millis(200));
        }
    }
}
