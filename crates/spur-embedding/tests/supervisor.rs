#![cfg(feature = "test-support")]
use spur_embedding::{
    Cancellation, ClientError, ClientPolicy, EmbeddingClient, ModelId, ModelIdentity,
    ProtocolLimits,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn setup(mode: &str, restarts: usize, capacity: usize) -> EmbeddingClient {
    setup_logged(mode, restarts, capacity, None)
}
fn setup_logged(
    mode: &str,
    restarts: usize,
    capacity: usize,
    log: Option<&std::path::Path>,
) -> EmbeddingClient {
    let identity = ModelIdentity {
        model: ModelId::CodeRankEmbed,
        dimensions: 2,
        model_fingerprint: "model".into(),
        tokenizer_fingerprint: "tokenizer".into(),
        pooling: "mean".into(),
        normalization: "unit".into(),
        prefix: "none".into(),
    };
    let limits = ProtocolLimits {
        max_frame_bytes: 65536,
        max_texts: 16,
        max_text_bytes: 1024,
        max_total_text_bytes: 4096,
        max_dimensions: 16,
        max_identity_bytes: 128,
        max_pending: 4,
    };
    let policy = ClientPolicy {
        queue_capacity: capacity,
        max_restarts: restarts,
        startup_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        shutdown_poll_interval: Duration::from_millis(1),
    };
    EmbeddingClient::new(
        env!("CARGO_BIN_EXE_embedding-fault-worker"),
        std::iter::once(mode.into())
            .chain(log.map(|p| p.as_os_str().to_owned()))
            .collect(),
        identity,
        limits,
        policy,
    )
    .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn embed(client: &EmbeddingClient) -> Result<Vec<Vec<f32>>, ClientError> {
    client.embed(vec!["hello".into()], deadline())
}
fn await_reaped(client: &EmbeddingClient, count: usize) {
    let until = deadline();
    while client
        .test_events()
        .iter()
        .filter(|e| e.kind == "reaped")
        .count()
        < count
    {
        assert!(Instant::now() < until, "missing child wait status");
        thread::yield_now();
    }
}

#[test]
fn reusable_valid_response_and_clean_shutdown() {
    let c = setup("normal", 0, 1);
    assert_eq!(embed(&c).unwrap(), vec![vec![1.0, 1.0]]);
    assert_eq!(embed(&c).unwrap(), vec![vec![1.0, 1.0]]);
    c.shutdown(deadline()).unwrap();
}
#[test]
fn faults_are_never_successful_vectors() {
    for mode in [
        "abort",
        "abort_mid",
        "truncated",
        "malformed",
        "wrong_generation",
        "wrong_model",
        "wrong_dimension",
        "error",
    ] {
        let c = setup(mode, 0, 1);
        let outcome = embed(&c);
        if mode == "error" {
            assert!(
                matches!(outcome, Err(ClientError::Remote { .. })),
                "{mode}: {outcome:?}"
            );
        } else {
            assert!(
                matches!(outcome, Err(ClientError::Transport(_))),
                "{mode}: {outcome:?}"
            );
        }
        let shutdown = c.shutdown(deadline());
        let until = deadline();
        while !c.test_events().iter().any(|event| event.kind == "reaped") {
            assert!(Instant::now() < until, "missing wait status for {mode}");
            thread::yield_now();
        }
        println!(
            "S2_CASE {}",
            serde_json::json!({"mode": mode, "outcome": format!("{outcome:?}"), "shutdown": format!("{shutdown:?}"), "events": c.test_events()})
        );
    }
}
#[test]
fn committed_result_survives_later_abort_and_shutdown_abort_is_visible() {
    let c = setup("abort_after", 0, 1);
    let rows = embed(&c).unwrap();
    assert_eq!(rows, vec![vec![1.0, 1.0]]);
    let shutdown = c.shutdown(deadline());
    assert!(shutdown.is_err());
    assert_eq!(rows, vec![vec![1.0, 1.0]]);
    await_reaped(&c, 1);
    println!(
        "S2_CASE {}",
        serde_json::json!({"mode":"abort_after", "outcome":format!("{rows:?}"), "shutdown":format!("{shutdown:?}"), "events":c.test_events()})
    );
    let c = setup("abort_shutdown", 0, 1);
    assert!(embed(&c).is_ok());
    let shutdown = c.shutdown(deadline());
    assert!(shutdown.is_err());
    await_reaped(&c, 1);
    println!(
        "S2_CASE {}",
        serde_json::json!({"mode":"abort_shutdown", "shutdown":format!("{shutdown:?}"), "events":c.test_events()})
    );
}
#[test]
fn timeout_and_queue_pressure_release_waiters() {
    let started = Instant::now();
    let c = Arc::new(setup("hang", 0, 1));
    let token = Cancellation::new();
    let child_token = token.clone();
    let a = Arc::clone(&c);
    let t = thread::spawn(move || a.embed_cancellable(vec!["a".into()], deadline(), child_token));
    let until = deadline();
    while !c
        .test_events()
        .iter()
        .any(|e| e.kind == "request" && e.status.as_deref() == Some("embed"))
    {
        assert!(Instant::now() < until);
        thread::yield_now();
    }
    let b = Arc::clone(&c);
    let q = thread::spawn(move || {
        b.embed(
            vec!["b".into()],
            Instant::now() + Duration::from_millis(250),
        )
    });
    while c.test_queue_depth() != 1 {
        assert!(Instant::now() < until);
        thread::yield_now();
    }
    let observed_queued = c.test_queue_depth();
    let admitted_at_ns = started.elapsed().as_nanos();
    let overflow = c.embed(vec!["c".into()], deadline());
    let overflow_at_ns = started.elapsed().as_nanos();
    assert!(matches!(overflow, Err(ClientError::QueueFull)));
    let queued = q.join().unwrap();
    let queue_end_at_ns = started.elapsed().as_nanos();
    assert!(matches!(queued, Err(ClientError::Timeout)));
    token.cancel();
    let active = t.join().unwrap();
    assert!(matches!(active, Err(ClientError::Cancelled)));
    let _ = c.shutdown(deadline());
    await_reaped(&c, 1);
    println!(
        "S2_CASE {}",
        serde_json::json!({"mode":"queue", "observed_queued":observed_queued, "admitted_at_ns":admitted_at_ns, "overflow_at_ns":overflow_at_ns, "queue_end_at_ns":queue_end_at_ns, "overflow":format!("{overflow:?}"), "queued":format!("{queued:?}"), "active":format!("{active:?}"), "events":c.test_events()})
    );
}

#[test]
fn cancellation_and_restart_budget_reap_real_children() {
    let log = std::env::temp_dir().join(format!("embedding-fault-{}", uuid::Uuid::new_v4()));
    let c = setup_logged("abort", 1, 1, Some(&log));
    let first = embed(&c);
    let second = embed(&c);
    let third = embed(&c);
    assert!(first.is_err());
    assert!(second.is_err());
    assert!(matches!(third, Err(ClientError::RestartExhausted)));
    let _ = c.shutdown(deadline());
    await_reaped(&c, 2);
    let pids: Vec<u32> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(
        pids.len(),
        2,
        "the restart budget must be enforced without replay"
    );
    for pid in &pids {
        let status = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .output()
            .unwrap();
        assert!(status.stdout.is_empty(), "child {pid} was not reaped");
    }
    println!(
        "S2_CASE {}",
        serde_json::json!({"mode":"restarts", "first":format!("{first:?}"), "second":format!("{second:?}"), "third":format!("{third:?}"), "pids":pids, "events":c.test_events()})
    );
    std::fs::remove_file(log).unwrap();

    let c = Arc::new(setup("hang", 0, 1));
    let token = Cancellation::new();
    let worker_token = token.clone();
    let worker = Arc::clone(&c);
    let started = Instant::now();
    let pending = thread::spawn(move || {
        worker.embed_cancellable(vec!["cancel".into()], deadline(), worker_token)
    });
    thread::sleep(Duration::from_millis(100));
    token.cancel();
    let outcome = pending.join().unwrap();
    assert!(matches!(outcome, Err(ClientError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = c.shutdown(deadline());
}

#[test]
fn startup_hang_obeys_caller_deadline() {
    let c = setup("startup_hang", 0, 1);
    let start = Instant::now();
    assert!(c
        .embed(vec!["hello".into()], start + Duration::from_millis(200))
        .is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    let _ = c.shutdown(deadline());
}

#[test]
fn startup_deadline_includes_process_launch_delay() {
    let mut policy = test_policy();
    policy.startup_timeout = Duration::from_secs(1);
    policy.request_timeout = Duration::from_secs(5);
    let c = custom_client("normal", policy, large_limits()).unwrap();
    c.test_hold_spawn(Duration::from_secs(2));
    let started = Instant::now();
    assert!(matches!(
        c.embed(vec!["hello".into()], started + Duration::from_secs(5)),
        Err(ClientError::Timeout)
    ));
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "launch delay held the caller past the startup deadline"
    );
}

#[test]
fn reply_after_deadline_is_not_published() {
    let c = setup("late_reply", 0, 1);
    // Start the worker before measuring the operation deadline.
    assert!(embed(&c).is_ok());
    let result = c.embed(
        vec!["later".into()],
        Instant::now() + Duration::from_millis(100),
    );
    assert!(matches!(result, Err(ClientError::Timeout)));
}

#[test]
fn queued_reply_cannot_win_after_deadline_when_waiter_is_delayed() {
    let c = setup("normal", 0, 1);
    assert!(embed(&c).is_ok());
    c.test_hold_delivery(Duration::from_millis(200));
    assert!(matches!(
        c.embed(
            vec!["later".into()],
            Instant::now() + Duration::from_millis(100)
        ),
        Err(ClientError::Timeout)
    ));
}

#[test]
fn reply_validation_cannot_publish_after_deadline() {
    let c = setup("normal", 0, 1);
    assert!(embed(&c).is_ok());
    c.test_hold_validation(Duration::from_millis(200));
    assert!(matches!(
        c.embed(
            vec!["later".into()],
            Instant::now() + Duration::from_millis(100)
        ),
        Err(ClientError::Timeout)
    ));
}

#[test]
fn already_cancelled_call_never_starts_a_child() {
    let log = std::env::temp_dir().join(format!("embedding-cancel-{}", uuid::Uuid::new_v4()));
    let c = setup_logged("normal", 0, 1, Some(&log));
    let token = Cancellation::new();
    token.cancel();
    assert!(matches!(
        c.embed_cancellable(vec!["x".into()], deadline(), token),
        Err(ClientError::Cancelled)
    ));
    assert!(!log.exists());
}

#[test]
fn lifecycle_trace_records_validated_reply_and_reaped_abort() {
    let c = setup("abort", 0, 1);
    assert!(embed(&c).is_err());
    let until = deadline();
    loop {
        let events = c.test_events();
        if events.iter().any(|event| event.kind == "reaped") {
            assert!(events
                .iter()
                .any(|event| event.kind == "spawned" && event.pid.is_some()));
            assert!(events.iter().any(|event| event.kind == "request"
                && event.request_id.is_some()
                && event.generation.is_some()));
            assert!(events
                .iter()
                .any(|event| event.kind == "reaped" && event.status.is_some()));
            println!(
                "S2_TRACE {}",
                serde_json::json!({"case":"lifecycle_abort", "events":events})
            );
            break;
        }
        assert!(
            Instant::now() < until,
            "cleanup did not report a child wait"
        );
        thread::yield_now();
    }
    let c = setup("normal", 0, 1);
    assert!(embed(&c).is_ok());
    c.shutdown(deadline()).unwrap();
    let events = c.test_events();
    assert!(events
        .iter()
        .any(|event| event.kind == "reply_validated" && event.request_id.is_some()));
    assert!(events
        .iter()
        .any(|event| event.kind == "reaped" && event.status.is_some()));
    println!(
        "S2_TRACE {}",
        serde_json::json!({"case":"lifecycle_normal", "events":events})
    );
}

fn custom_client(
    mode: &str,
    policy: ClientPolicy,
    limits: ProtocolLimits,
) -> Result<EmbeddingClient, ClientError> {
    EmbeddingClient::new(
        env!("CARGO_BIN_EXE_embedding-fault-worker"),
        vec![mode.into()],
        ModelIdentity {
            model: ModelId::CodeRankEmbed,
            dimensions: 2,
            model_fingerprint: "model".into(),
            tokenizer_fingerprint: "tokenizer".into(),
            pooling: "mean".into(),
            normalization: "unit".into(),
            prefix: "none".into(),
        },
        limits,
        policy,
    )
}
fn test_policy() -> ClientPolicy {
    ClientPolicy {
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        shutdown_poll_interval: Duration::from_millis(1),
    }
}
fn large_limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 4 * 1024 * 1024,
        max_texts: 16,
        max_text_bytes: 2 * 1024 * 1024,
        max_total_text_bytes: 2 * 1024 * 1024,
        max_dimensions: 16,
        max_identity_bytes: 128,
        max_pending: 4,
    }
}

#[test]
fn invalid_policy_is_rejected_and_zero_restarts_is_valid() {
    let mut policy = test_policy();
    policy.queue_capacity = 0;
    assert!(matches!(
        custom_client("normal", policy, large_limits()),
        Err(ClientError::InvalidPolicy(_))
    ));
    let mut policy = test_policy();
    policy.request_timeout = Duration::ZERO;
    assert!(matches!(
        custom_client("normal", policy, large_limits()),
        Err(ClientError::InvalidPolicy(_))
    ));
    let mut policy = test_policy();
    policy.shutdown_poll_interval = Duration::ZERO;
    assert!(matches!(
        custom_client("normal", policy, large_limits()),
        Err(ClientError::InvalidPolicy(_))
    ));
    let mut policy = test_policy();
    policy.startup_timeout = Duration::MAX;
    assert!(matches!(
        custom_client("normal", policy, large_limits()),
        Err(ClientError::InvalidPolicy(_))
    ));
    assert!(custom_client("normal", test_policy(), large_limits()).is_ok());
}

#[test]
fn cancellation_unblocks_a_full_child_stdin_pipe_and_reaps_it() {
    let c = Arc::new(custom_client("blocked_write", test_policy(), large_limits()).unwrap());
    let token = Cancellation::new();
    let child_token = token.clone();
    let client = Arc::clone(&c);
    let pending = thread::spawn(move || {
        client.embed_cancellable(vec!["x".repeat(1024 * 1024)], deadline(), child_token)
    });
    let until = deadline();
    while !c
        .test_events()
        .iter()
        .any(|e| e.kind == "write_started" && e.request_id == Some(2))
    {
        assert!(Instant::now() < until);
        thread::yield_now();
    }
    thread::sleep(Duration::from_millis(100));
    assert!(!c
        .test_events()
        .iter()
        .any(|e| e.kind == "write_completed" && e.request_id == Some(2)));
    token.cancel();
    let outcome = pending.join().unwrap();
    assert!(matches!(outcome, Err(ClientError::Cancelled)));
    while !c
        .test_events()
        .iter()
        .any(|e| e.kind == "io_joined" && e.request_id.is_none())
    {
        assert!(Instant::now() < until);
        thread::yield_now();
    }
    let events = c.test_events();
    let reap = events.iter().position(|e| e.kind == "reaped").unwrap();
    let join = events
        .iter()
        .position(|e| e.kind == "io_joined" && e.request_id.is_none())
        .unwrap();
    let pipes_closed = events
        .iter()
        .position(|e| e.kind == "io_pipes_closed" && e.request_id == Some(2))
        .unwrap();
    assert!(
        reap < join,
        "trace must record wait when it occurs, before joining the I/O thread"
    );
    assert!(
        pipes_closed < join,
        "I/O-owned pipes must close before their thread joins"
    );
    println!(
        "S2_TRACE {}",
        serde_json::json!({"case":"blocked_write_cancel", "outcome":format!("{outcome:?}"), "events":events})
    );
}

#[test]
fn acknowledged_shutdown_hang_times_out_and_reaps_child() {
    let c = setup("shutdown_hang", 0, 1);
    assert!(embed(&c).is_ok());
    let end = Instant::now() + Duration::from_millis(200);
    let outcome = c.shutdown(end);
    assert!(matches!(outcome, Err(ClientError::Timeout)));
    let until = deadline();
    while !c.test_events().iter().any(|e| e.kind == "reaped") {
        assert!(Instant::now() < until);
        thread::yield_now();
    }
    println!(
        "S2_TRACE {}",
        serde_json::json!({"case":"shutdown_hang", "outcome":format!("{outcome:?}"), "events":c.test_events()})
    );
}

#[test]
fn owner_can_wait_for_abandoned_launch_cleanup() {
    let mut policy = test_policy();
    policy.startup_timeout = Duration::from_secs(1);
    policy.request_timeout = Duration::from_secs(5);
    let client = custom_client("normal", policy, large_limits()).unwrap();
    client.test_hold_spawn(Duration::from_secs(2));
    let started = Instant::now();
    assert!(matches!(
        client.embed(vec!["hello".into()], started + Duration::from_secs(5)),
        Err(ClientError::Timeout)
    ));
    assert!(
        matches!(
            client.wait_for_cleanup(Instant::now() + Duration::from_millis(100)),
            Err(ClientError::Timeout)
        ),
        "cleanup must report unfinished launch"
    );
    client
        .wait_for_cleanup(Instant::now() + Duration::from_secs(3))
        .expect("abandoned launch must be joined");
    assert!(
        !client
            .test_events()
            .iter()
            .any(|event| event.kind == "spawned"),
        "abandoned launch must not start a child"
    );
}

#[test]
fn cleanup_wait_joins_cancelled_child_and_its_io() {
    let client = Arc::new(setup("hang", 0, 1));
    let cancel = Cancellation::new();
    let worker_cancel = cancel.clone();
    let worker_client = Arc::clone(&client);
    let worker = thread::spawn(move || {
        worker_client.embed_cancellable(vec!["query".into()], deadline(), worker_cancel)
    });
    let limit = deadline();
    while !client
        .test_events()
        .iter()
        .any(|event| event.kind == "request" && event.status.as_deref() == Some("embed"))
    {
        assert!(
            Instant::now() < limit,
            "helper never received embedding request"
        );
        thread::yield_now();
    }
    cancel.cancel();
    assert!(
        matches!(worker.join().unwrap(), Err(ClientError::Cancelled)),
        "cancelled query must fail"
    );
    client
        .wait_for_cleanup(deadline())
        .expect("child and I/O cleanup");
    let events = client.test_events();
    let reap = events
        .iter()
        .position(|event| event.kind == "reaped")
        .expect("helper wait status");
    let io_join = events
        .iter()
        .position(|event| event.kind == "io_joined" && event.request_id.is_none())
        .expect("cancelled I/O joined");
    assert!(
        reap < io_join,
        "helper must be reaped before the I/O join: {}",
        serde_json::to_string(&events).unwrap()
    );
}

#[test]
fn cleanup_wait_reports_reap_failure() {
    let client = Arc::new(setup("hang", 0, 1));
    client.test_inject_reap_failure();
    let cancel = Cancellation::new();
    let worker_cancel = cancel.clone();
    let worker_client = Arc::clone(&client);
    let worker = thread::spawn(move || {
        worker_client.embed_cancellable(vec!["query".into()], deadline(), worker_cancel)
    });
    let limit = deadline();
    while !client
        .test_events()
        .iter()
        .any(|event| event.kind == "request" && event.status.as_deref() == Some("embed"))
    {
        assert!(
            Instant::now() < limit,
            "helper never received embedding request"
        );
        thread::yield_now();
    }
    cancel.cancel();
    assert!(
        matches!(worker.join().unwrap(), Err(ClientError::Cancelled)),
        "cancelled query must fail"
    );
    assert!(
        client.wait_for_cleanup(deadline()).is_err(),
        "reap failure must remain visible"
    );
}
