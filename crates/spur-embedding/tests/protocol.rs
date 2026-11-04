use spur_embedding::{
    read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolError, ProtocolLimits,
    Reply, ReplyEnvelope, Request, RequestEnvelope, Session, PROTOCOL_VERSION,
};
use std::io::{self, Cursor, Read};
use uuid::Uuid;

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 4096,
        max_texts: 2,
        max_text_bytes: 4,
        max_total_text_bytes: 6,
        max_dimensions: 3,
        max_identity_bytes: 32,
        max_pending: 2,
    }
}
fn model() -> ModelIdentity {
    ModelIdentity {
        model: ModelId::CodeRankEmbed,
        dimensions: 2,
        model_fingerprint: "model".into(),
        tokenizer_fingerprint: "tokenizer".into(),
        pooling: "mean".into(),
        normalization: "l2".into(),
        prefix: "code".into(),
    }
}
fn generation() -> Uuid {
    Uuid::from_u128(1)
}
fn req(id: u64, request: Request) -> RequestEnvelope {
    RequestEnvelope {
        version: PROTOCOL_VERSION,
        generation: generation(),
        request_id: id,
        request,
    }
}
fn rep(id: u64, reply: Reply) -> ReplyEnvelope {
    ReplyEnvelope {
        version: PROTOCOL_VERSION,
        generation: generation(),
        request_id: id,
        reply,
    }
}
fn frame(body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(body);
    v
}

#[test]
fn all_envelopes_roundtrip() {
    let requests = [
        req(1, Request::Hello { identity: model() }),
        req(
            2,
            Request::Embed {
                identity: model(),
                texts: vec!["abc".into()],
            },
        ),
        req(3, Request::Shutdown),
    ];
    for request in requests {
        let mut out = vec![];
        write_frame(&mut out, &request, &limits()).unwrap();
        let decoded: RequestEnvelope = read_frame(&mut Cursor::new(out), &limits()).unwrap();
        assert_eq!(request, decoded);
    }
    let replies = [
        rep(1, Reply::Ready { identity: model() }),
        rep(
            2,
            Reply::Embeddings {
                identity: model(),
                rows: vec![vec![1.0, 2.0]],
            },
        ),
        rep(3, Reply::Closed),
        rep(
            4,
            Reply::Error {
                code: ErrorCode::InvalidRequest,
                message: "bad".into(),
            },
        ),
    ];
    for reply in replies {
        let mut out = vec![];
        write_frame(&mut out, &reply, &limits()).unwrap();
        let decoded: ReplyEnvelope = read_frame(&mut Cursor::new(out), &limits()).unwrap();
        assert_eq!(reply, decoded);
    }
    for id in [
        ModelId::NomicEmbedTextV15,
        ModelId::CodeRankEmbed,
        ModelId::JinaEmbeddingsV2BaseCode,
    ] {
        let mut m = model();
        m.model = id;
        assert!(m.validate(&limits()).is_ok());
    }
}

struct HeaderOnly {
    bytes: Cursor<[u8; 4]>,
    body_read: bool,
}
impl Read for HeaderOnly {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.bytes.position() == 4 {
            self.body_read = true;
            return Err(io::Error::other("body read"));
        }
        self.bytes.read(buf)
    }
}
#[test]
fn rejects_oversize_before_body_read() {
    let mut probe = HeaderOnly {
        bytes: Cursor::new((limits().max_frame_bytes + 1).to_be_bytes()),
        body_read: false,
    };
    assert!(matches!(
        read_frame::<_, RequestEnvelope>(&mut probe, &limits()),
        Err(ProtocolError::FrameTooLarge { .. })
    ));
    assert!(!probe.body_read);
}
#[test]
fn rejects_truncated_and_invalid_frames() {
    for bytes in [
        vec![],
        vec![0],
        vec![0, 0, 0],
        vec![0, 0, 0, 2, b'{'],
        frame(b"{} trailing"),
        frame(b"\xff"),
        frame(b"{} {}"),
    ] {
        assert!(read_frame::<_, RequestEnvelope>(&mut Cursor::new(bytes), &limits()).is_err());
    }
}
#[test]
fn schema_rejects_unknown_duplicate_and_wrong_version() {
    let valid = req(1, Request::Shutdown);
    let json = serde_json::to_string(&valid).unwrap();
    for bad in [
        json.replacen('{', "{\"extra\":0,", 1),
        json.replacen('{', "{\"version\":2,", 1),
        json.replace(
            &format!("\"version\":{PROTOCOL_VERSION}"),
            "\"version\":999",
        ),
    ] {
        let decoded =
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(frame(bad.as_bytes())), &limits());
        assert!(
            decoded.and_then(|v| v.validate(&limits())).is_err(),
            "{bad}"
        );
    }
}
#[test]
fn identity_text_and_arithmetic_bounds_are_enforced() {
    let l = limits();
    assert!(l.validate().is_ok());
    let mut invalid = l.clone();
    invalid.max_frame_bytes = 0;
    assert!(invalid.validate().is_err());
    let mut invalid = l.clone();
    invalid.max_texts = usize::MAX;
    invalid.max_dimensions = usize::MAX;
    assert!(invalid.validate().is_err());
    let mut m = model();
    m.model_fingerprint.clear();
    assert!(m.validate(&l).is_err());
    let mut m = model();
    m.tokenizer_fingerprint = "x".repeat(l.max_identity_bytes + 1);
    assert!(m.validate(&l).is_err());
    let embed = |texts: Vec<String>| {
        req(
            2,
            Request::Embed {
                identity: model(),
                texts,
            },
        )
    };
    assert!(embed(vec!["abcd".into(), "ef".into()]).validate(&l).is_ok());
    for texts in [
        vec![],
        vec![String::new()],
        vec!["abcde".into()],
        vec!["abcd".into(), "efg".into()],
        vec!["a".into(), "b".into(), "c".into()],
    ] {
        assert!(embed(texts).validate(&l).is_err());
    }
}
#[test]
fn reply_validation_binds_session_request_model_and_shape() {
    let l = limits();
    let hello = req(1, Request::Hello { identity: model() });
    let mut session = Session::new(generation(), model(), l.clone()).unwrap();
    session.validate_request(&hello).unwrap();
    session
        .complete_reply(&hello, &rep(1, Reply::Ready { identity: model() }))
        .unwrap();
    let embed = req(
        2,
        Request::Embed {
            identity: model(),
            texts: vec!["a".into(), "b".into()],
        },
    );
    session.validate_request(&embed).unwrap();
    let good = rep(
        2,
        Reply::Embeddings {
            identity: model(),
            rows: vec![vec![1.0, 2.0], vec![3.0, 4.0]],
        },
    );
    assert!(session.validate_reply(&embed, &good).is_ok());
    let mut bad = good.clone();
    bad.generation = Uuid::from_u128(2);
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    bad.request_id = 1;
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    if let Reply::Embeddings { identity, .. } = &mut bad.reply {
        identity.model = ModelId::NomicEmbedTextV15;
    }
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    if let Reply::Embeddings { identity, .. } = &mut bad.reply {
        identity.tokenizer_fingerprint.push('x');
    }
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    if let Reply::Embeddings { rows, .. } = &mut bad.reply {
        rows.pop();
    }
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    if let Reply::Embeddings { rows, .. } = &mut bad.reply {
        rows[0].pop();
    }
    assert!(session.validate_reply(&embed, &bad).is_err());
    let mut bad = good.clone();
    if let Reply::Embeddings { rows, .. } = &mut bad.reply {
        rows[0][0] = f32::NAN;
    }
    assert!(session.validate_reply(&embed, &bad).is_err());
    assert!(session.validate_request(&embed).is_err());
    session.complete_reply(&embed, &good).unwrap();
    let shutdown = req(3, Request::Shutdown);
    session.validate_request(&shutdown).unwrap();
    assert!(session
        .validate_reply(&shutdown, &rep(3, Reply::Closed))
        .is_ok());
}

#[test]
fn pending_requests_are_bounded_and_consumed_only_on_valid_completion() {
    let mut limits = limits();
    limits.max_pending = 1;
    let mut session = Session::new(generation(), model(), limits).unwrap();
    let hello = req(1, Request::Hello { identity: model() });
    session.validate_request(&hello).unwrap();
    let next = req(2, Request::Shutdown);
    assert!(session.validate_request(&next).is_err());
    assert!(session
        .complete_reply(&hello, &rep(1, Reply::Closed))
        .is_err());
    assert!(session.validate_request(&next).is_err());
    session
        .complete_reply(&hello, &rep(1, Reply::Ready { identity: model() }))
        .unwrap();
    assert!(session
        .complete_reply(&hello, &rep(1, Reply::Ready { identity: model() }))
        .is_err());
    session.validate_request(&next).unwrap();
}

#[test]
fn embed_requires_completed_hello_and_shutdown_closes_session() {
    let mut session = Session::new(generation(), model(), limits()).unwrap();
    let embed = req(
        2,
        Request::Embed {
            identity: model(),
            texts: vec!["a".into()],
        },
    );
    assert!(session.validate_request(&embed).is_err());
    let hello = req(1, Request::Hello { identity: model() });
    session.validate_request(&hello).unwrap();
    assert!(session.validate_request(&embed).is_err());
    session
        .complete_reply(&hello, &rep(1, Reply::Ready { identity: model() }))
        .unwrap();
    session.validate_request(&embed).unwrap();
    session
        .complete_reply(
            &embed,
            &rep(
                2,
                Reply::Embeddings {
                    identity: model(),
                    rows: vec![vec![1.0, 2.0]],
                },
            ),
        )
        .unwrap();
    let shutdown = req(3, Request::Shutdown);
    session.validate_request(&shutdown).unwrap();
    assert!(session
        .validate_request(&req(
            4,
            Request::Embed {
                identity: model(),
                texts: vec!["a".into()]
            }
        ))
        .is_err());
    session
        .complete_reply(&shutdown, &rep(3, Reply::Closed))
        .unwrap();
    assert!(session
        .validate_request(&req(4, Request::Shutdown))
        .is_err());
}

#[test]
fn configured_frame_boundary_and_nested_schema() {
    let request = req(1, Request::Hello { identity: model() });
    let body = serde_json::to_vec(&request).unwrap();
    let mut l = limits();
    l.max_frame_bytes = u32::try_from(body.len()).unwrap();
    assert!(read_frame::<_, RequestEnvelope>(&mut Cursor::new(frame(&body)), &l).is_ok());
    assert!(write_frame(&mut vec![], &request, &l).is_ok());
    l.max_frame_bytes -= 1;
    assert!(read_frame::<_, RequestEnvelope>(&mut Cursor::new(frame(&body)), &l).is_err());
    assert!(write_frame(&mut vec![], &request, &l).is_err());
    let json = String::from_utf8(body).unwrap();
    for bad in [
        json.replacen("\"pooling\":", "\"extra\":0,\"pooling\":", 1),
        json.replacen("\"pooling\":", "\"pooling\":\"other\",\"pooling\":", 1),
    ] {
        assert!(read_frame::<_, RequestEnvelope>(
            &mut Cursor::new(frame(bad.as_bytes())),
            &limits()
        )
        .is_err());
    }
}

#[test]
#[expect(
    clippy::vec_init_then_push,
    reason = "observations are assembled sequentially"
)]
fn export_observed_wire_cases() {
    fn outcome<T: std::fmt::Debug>(r: Result<T, ProtocolError>) -> String {
        match r {
            Ok(v) => format!("decoded: {v:?}"),
            Err(e) => format!("rejected: {e:?}"),
        }
    }
    let request = req(1, Request::Hello { identity: model() });
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &request, &limits()).unwrap();
    let mut records = Vec::new();
    records.push((
        "valid_hello",
        outcome(read_frame::<_, RequestEnvelope>(
            &mut Cursor::new(&bytes),
            &limits(),
        )),
    ));
    records.push((
        "truncated_header",
        outcome(read_frame::<_, RequestEnvelope>(
            &mut Cursor::new(&bytes[..3]),
            &limits(),
        )),
    ));
    records.push((
        "truncated_body",
        outcome(read_frame::<_, RequestEnvelope>(
            &mut Cursor::new(&bytes[..bytes.len() - 1]),
            &limits(),
        )),
    ));
    records.push((
        "oversized_header",
        outcome(read_frame::<_, RequestEnvelope>(
            &mut Cursor::new((limits().max_frame_bytes + 1).to_be_bytes()),
            &limits(),
        )),
    ));
    let json = serde_json::to_string(&request).unwrap();
    for (name, body) in [
        (
            "wrong_version",
            json.replace("\"version\":2", "\"version\":1"),
        ),
        ("unknown_field", json.replacen('{', "{\"extra\":0,", 1)),
        ("duplicate_field", json.replacen('{', "{\"version\":2,", 1)),
        ("trailing_json", format!("{json} {{}}")),
    ] {
        records.push((
            name,
            outcome(read_frame::<_, RequestEnvelope>(
                &mut Cursor::new(frame(body.as_bytes())),
                &limits(),
            )),
        ));
    }
    let mut session = Session::new(generation(), model(), limits()).unwrap();
    records.push(("hello_request", outcome(session.validate_request(&request))));
    records.push((
        "wrong_generation",
        outcome(session.validate_reply(
            &request,
            &ReplyEnvelope {
                generation: Uuid::from_u128(2),
                ..rep(1, Reply::Ready { identity: model() })
            },
        )),
    ));
    let mut mismatch = model();
    mismatch.tokenizer_fingerprint.push('x');
    records.push((
        "wrong_fingerprint",
        outcome(session.validate_reply(&request, &rep(1, Reply::Ready { identity: mismatch }))),
    ));
    session
        .complete_reply(&request, &rep(1, Reply::Ready { identity: model() }))
        .unwrap();
    let embed = req(
        2,
        Request::Embed {
            identity: model(),
            texts: vec!["a".into()],
        },
    );
    session.validate_request(&embed).unwrap();
    for (name, rows) in [
        ("valid_rows", vec![vec![1.0, 2.0]]),
        ("wrong_rows", vec![vec![1.0, 2.0], vec![3.0, 4.0]]),
        ("wrong_dimensions", vec![vec![1.0]]),
        ("nonfinite", vec![vec![f32::NAN, 2.0]]),
    ] {
        records.push((
            name,
            outcome(session.validate_reply(
                &embed,
                &rep(
                    2,
                    Reply::Embeddings {
                        identity: model(),
                        rows,
                    },
                ),
            )),
        ));
    }
    println!("OBSERVATIONS {}", serde_json::to_string(&records).unwrap());
}
