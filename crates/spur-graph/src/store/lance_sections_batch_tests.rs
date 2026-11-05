// Included in the existing test module to exercise the actual graph/analyst model handle.
fn m4_fixture(fault: &str, policy: spur_embedding::LocalPolicy) -> GraphHelperFixture {
    let mut fixture = GraphHelperFixture::new(fault);
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/embedding_fault_worker.py");
    let mut config = serde_json::to_value(policy).unwrap();
    config["executable"] = serde_json::json!("/usr/bin/python3");
    config["args"] = serde_json::json!([script]);
    fixture._guards.push(EnvGuard::set(
        "SPUR_EMBEDDING_HELPER_CONFIG",
        &config.to_string(),
    ));
    fixture
}

fn m4_model() -> EmbeddingModelHandle {
    load_embedding_model(EmbeddingModelSelection::NomicEmbedTextV15, false).unwrap()
}

fn m4_body(texts: Vec<String>) -> usize {
    serde_json::to_vec(&spur_embedding::RequestEnvelope {
        version: spur_embedding::PROTOCOL_VERSION,
        generation: "ffffffff-ffff-ffff-ffff-ffffffffffff".parse().unwrap(),
        request_id: u64::MAX,
        request: spur_embedding::Request::Embed {
            identity: spur_embedding::ModelIdentity {
                model: spur_embedding::ModelId::NomicEmbedTextV15,
                dimensions: 768,
                model_fingerprint: "test-model".into(),
                tokenizer_fingerprint: "test-tokenizer".into(),
                pooling: "mean".into(),
                normalization: "unit".into(),
                prefix: "legacy_preformatted".into(),
            },
            texts,
        },
    })
    .unwrap()
    .len()
}

fn m4_export(
    case: &str,
    fixture: &GraphHelperFixture,
    model: &EmbeddingModelHandle,
    outcome: serde_json::Value,
) {
    let events = model.client.test_events();
    let capture = serde_json::json!({"case": case, "outcome": outcome, "events": events,
        "batches": fixture.input_batches(), "log": fixture.log(),
        "envelopes": fs::read_to_string(fixture.dir.path().join("worker.log.envelopes")).unwrap_or_default()
            .lines().map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()).collect::<Vec<_>>()});
    println!(
        "M4_CASE {}",
        serde_json::json!({"case":case,"outcome":capture["outcome"]})
    );
    if let Some(root) = std::env::var_os("SPUR_M4_EXPORT") {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            PathBuf::from(root).join(format!("{case}.json")),
            serde_json::to_vec_pretty(&capture).unwrap(),
        )
        .unwrap();
    }
}

fn m4_success(
    case: &str,
    texts: Vec<String>,
    batch: Option<usize>,
    policy: spur_embedding::LocalPolicy,
    expected: &[usize],
) {
    let fixture = m4_fixture("", policy.clone());
    let mut model = m4_model();
    let result = model.embed(&texts, batch);
    assert!(
        result.is_ok(),
        "{case}: expected bounded success, got {result:?}"
    );
    let rows = result.unwrap();
    assert_eq!(rows.len(), texts.len(), "{case}");
    for (row, text) in rows.iter().zip(&texts) {
        assert_eq!(row, &vec![text.chars().count() as f32; 768], "{case}");
    }
    let batches = fixture.input_batches();
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        expected,
        "{case}: actual caller batch sizes"
    );
    assert_eq!(
        batches.iter().flatten().collect::<Vec<_>>(),
        texts.iter().collect::<Vec<_>>()
    );
    for texts in &batches {
        assert!(texts.len() <= policy.limits.max_texts);
        assert!(texts
            .iter()
            .all(|t| !t.is_empty() && t.len() <= policy.limits.max_text_bytes));
        assert!(texts.iter().map(String::len).sum::<usize>() <= policy.limits.max_total_text_bytes);
        assert!(m4_body(texts.clone()) <= policy.limits.max_frame_bytes as usize);
    }
    model.close().unwrap();
    let events = model.client.test_events();
    assert_eq!(events.iter().filter(|e| e.kind == "spawned").count(), 1);
    let requests: Vec<_> = events.iter().filter(|e| e.kind == "request").collect();
    assert!(requests
        .iter()
        .all(|e| e.generation == requests[0].generation));
    assert!(requests
        .windows(2)
        .all(|pair| pair[0].request_id < pair[1].request_id));
    assert!(events
        .iter()
        .any(|e| e.kind == "reaped"
            && e.status.as_deref() == Some("ExitStatus(unix_wait_status(0))")));
    m4_export(
        case,
        &fixture,
        &model,
        serde_json::json!({"rows": rows.len(), "dimensions":768,"ordered":true,"checked_close":true}),
    );
}

#[test]
fn bounded_batch_actual_caller_row_limits_and_overrides() {
    let _guard = env_lock();
    let policy = spur_embedding::LocalPolicy::default();
    let n = policy.limits.max_texts;
    for count in [n - 1, n, n + 1] {
        let expected = if count > n { vec![n, 1] } else { vec![count] };
        m4_success(
            &format!("rows-{count}"),
            (0..count).map(|i| format!("row-{i}")).collect(),
            None,
            policy.clone(),
            &expected,
        );
    }
    for batch in [1, 2, n * 2, usize::MAX] {
        let cap = batch.min(n);
        let expected = (0..n + 1)
            .collect::<Vec<_>>()
            .chunks(cap)
            .map(<[usize]>::len)
            .collect::<Vec<_>>();
        m4_success(
            &format!("override-{batch}"),
            (0..n + 1).map(|i| format!("row-{i}")).collect(),
            Some(batch),
            policy.clone(),
            &expected,
        );
    }
}

#[test]
fn bounded_batch_utf8_total_and_exact_serialized_frame_boundaries() {
    let _guard = env_lock();
    let policy = spur_embedding::LocalPolicy::default();
    let m = policy.limits.max_text_bytes;
    for (case, text) in [("ascii", "x".repeat(m)), ("unicode", "é".repeat(m / 2))] {
        m4_success(case, vec![text], None, policy.clone(), &[1]);
    }
    let texts = vec!["x".repeat(m), "é".repeat(m / 2)];
    let mut total = policy.clone();
    total.limits.max_total_text_bytes = m * 2 - 1;
    m4_success("total-split", texts.clone(), None, total, &[1, 1]);
    let mut frame = policy.clone();
    frame.limits.max_frame_bytes = u32::try_from(m4_body(texts.clone())).unwrap();
    m4_success("frame-exact", texts.clone(), None, frame.clone(), &[2]);
    frame.limits.max_frame_bytes -= 1;
    m4_success("frame-one-less", texts, None, frame, &[1, 1]);
    let texts = vec!["\0".repeat(m), "\n".repeat(m)];
    let mut escaped = policy;
    escaped.limits.max_frame_bytes = u32::try_from(m4_body(vec![texts[0].clone()])).unwrap();
    m4_success("json-escape-split", texts, None, escaped, &[1, 1]);
}

#[test]
fn bounded_batch_invalid_individual_is_typed_and_admitted_before_any_embed() {
    let _guard = env_lock();
    let policy = spur_embedding::LocalPolicy::default();
    let m = policy.limits.max_text_bytes;
    let mut too_small_total = policy.clone();
    too_small_total.limits.max_total_text_bytes = 1;
    let oversized = "\0".repeat(m);
    let mut too_small_frame = policy.clone();
    too_small_frame.limits.max_frame_bytes =
        u32::try_from(m4_body(vec![oversized.clone()]) - 1).unwrap();
    for (case, text, policy) in [
        ("empty-text", String::new(), policy.clone()),
        ("ascii-over", "x".repeat(m + 1), policy.clone()),
        ("unicode-over", "é".repeat(m / 2 + 1), policy.clone()),
        ("individual-total", "aa".into(), too_small_total),
        ("individual-frame", oversized, too_small_frame),
    ] {
        let fixture = m4_fixture("", policy);
        let mut model = m4_model();
        let error = model
            .embed(&["a".to_owned(), text], Some(1))
            .expect_err(case);
        assert!(
            error
                .downcast_ref::<spur_embedding::ClientError>()
                .is_some(),
            "{case}: {error:?}"
        );
        assert!(
            fixture.input_batches().is_empty(),
            "{case}: invalid logical batch must not publish a prefix"
        );
        assert!(
            !fixture.log().contains("service"),
            "{case}: reject before helper service launch"
        );
        model.close().unwrap();
        m4_export(
            case,
            &fixture,
            &model,
            serde_json::json!({"error":format!("{error:?}"),"rows_published":0}),
        );
    }
}

#[test]
fn bounded_batch_empty_and_zero_size_do_not_send_embed() {
    let _guard = env_lock();
    let fixture = m4_fixture("", spur_embedding::LocalPolicy::default());
    let mut model = m4_model();
    assert_eq!(
        model.embed::<&str>(&[], None).unwrap(),
        Vec::<Vec<f32>>::new()
    );
    let error = model
        .embed(&["a"], Some(0))
        .expect_err("explicit zero must be rejected");
    assert!(error
        .downcast_ref::<spur_embedding::ClientError>()
        .is_some());
    assert!(!fixture.log().contains("service"));
    model.close().unwrap();
    m4_export(
        "empty-zero",
        &fixture,
        &model,
        serde_json::json!({"empty_rows":0,"zero_error":format!("{error:?}")}),
    );
}

#[test]
fn bounded_batch_second_chunk_rejects_invalid_or_failed_output() {
    let _guard = env_lock();
    for fault in [
        "error",
        "death",
        "truncated",
        "wrong_identity",
        "wrong_generation",
        "wrong_request",
        "wrong_rows",
        "wrong_dimensions",
        "nonfinite",
        "nonfinite_number",
    ] {
        let fixture = m4_fixture(fault, spur_embedding::LocalPolicy::default());
        let mut model = m4_model();
        let result = model.embed(&["first", "second"], Some(1));
        assert!(
            result.is_err(),
            "{fault}: a logical batch cannot return partial or unchecked rows: {result:?}"
        );
        assert_eq!(
            fixture.input_batches(),
            vec![vec!["first".to_owned()], vec!["second".to_owned()]]
        );
        let events = model.client.test_events();
        assert_eq!(events.iter().filter(|e| e.kind == "reply_validated" && e.status.as_deref() == Some("embeddings")).count(),1);
        let closed = model.close();
        model
            .client
            .wait_for_cleanup(Instant::now() + Duration::from_secs(10))
            .unwrap();
        m4_export(
            &format!("second-{fault}"),
            &fixture,
            &model,
            serde_json::json!({"error":format!("{result:?}"),"close":format!("{closed:?}"),"rows_published":0,"prior_validated_chunks":1}),
        );
    }
}

#[test]
fn bounded_batch_uses_one_effective_request_deadline() {
    let _guard = env_lock();
    let mut policy = spur_embedding::LocalPolicy::default();
    policy.request_timeout_ms = 250; // Existing supervisor deadline fixture; helper waits 200ms/chunk.
    let fixture = m4_fixture("batch_delay", policy);
    let mut model = m4_model();
    let result = model.embed(&["first", "second"], Some(1));
    assert!(
        matches!(
            result
                .as_ref()
                .err()
                .and_then(|e| e.downcast_ref::<spur_embedding::ClientError>()),
            Some(spur_embedding::ClientError::Timeout)
        ),
        "{result:?}"
    );
    let _ = model.close();
    model
        .client
        .wait_for_cleanup(Instant::now() + Duration::from_secs(10))
        .unwrap();
    m4_export(
        "one-deadline",
        &fixture,
        &model,
        serde_json::json!({"outcome":format!("{result:?}")}),
    );
}

#[test]
fn bounded_batch_cancellation_and_close_reject_second_chunk() {
    let _guard = env_lock();
    for close in [false, true] {
        // Force the cancellable API's second chunk using an explicit protocol row cap.
        let mut policy = spur_embedding::LocalPolicy::default();
        policy.limits.max_texts = 1;
        let fixture = m4_fixture("second_hang", policy);
        let mut model = m4_model();
        let token = Cancellation::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = std::thread::scope(|scope| {
            let model_ref = &model;
            let token_ref = token.clone();
            let task = scope.spawn(move || {
                model_ref.embed_cancellable(&["first", "second"], deadline, token_ref)
            });
            while fixture.input_batches().len() < 2 && !task.is_finished() {
                assert!(Instant::now() < deadline, "second chunk never started");
                std::thread::yield_now();
            }
            if close {
                let _ = model
                    .client
                    .shutdown(Instant::now() + Duration::from_millis(250));
            } else {
                token.cancel();
            }
            task.join().unwrap()
        });
        let error = result.expect_err("no partial result");
        assert!(
            matches!(
                error.downcast_ref::<spur_embedding::ClientError>(),
                Some(spur_embedding::ClientError::Cancelled | spur_embedding::ClientError::Closed)
            ),
            "{error:?}"
        );
        assert_eq!(fixture.input_batches().len(), 2);
        let _ = model.close();
        model
            .client
            .wait_for_cleanup(Instant::now() + Duration::from_secs(10))
            .unwrap();
        m4_export(
            if close { "close" } else { "cancel" },
            &fixture,
            &model,
            serde_json::json!({"error":format!("{error:?}"),"rows_published":0}),
        );
    }
}
