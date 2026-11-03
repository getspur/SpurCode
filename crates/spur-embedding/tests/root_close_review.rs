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

fn client(mode: &str) -> EmbeddingClient {
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
        ProtocolLimits {
            max_frame_bytes: 65536,
            max_texts: 16,
            max_text_bytes: 1024,
            max_total_text_bytes: 4096,
            max_dimensions: 16,
            max_identity_bytes: 128,
            max_pending: 4,
        },
        ClientPolicy {
            queue_capacity: 1,
            max_restarts: 0,
            startup_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(2),
            shutdown_poll_interval: Duration::from_millis(1),
        },
    )
    .unwrap()
}

fn until() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output()
        .unwrap();
    assert!(
        out.status.success() || out.status.code() == Some(1),
        "ps failed to inspect pid {pid}: {:?}",
        out.status
    );
    !out.stdout.is_empty()
}

#[test]
fn expired_shutdown_must_not_strand_a_live_idle_child() {
    let c = client("normal");
    c.embed(vec!["warm".into()], until()).unwrap();
    let pid = c.test_events().iter().find_map(|e| e.pid).unwrap();
    let first = c.shutdown(Instant::now());
    let second = c.shutdown(until());
    let grace = until();
    while !c.test_events().iter().any(|e| e.kind == "reaped") && Instant::now() < grace {
        thread::sleep(Duration::from_millis(1));
    }
    let reaped_before_drop = c.test_events().iter().any(|e| e.kind == "reaped");
    let alive_before_drop = alive(pid);
    println!(
        "ROOT_CLOSE_CASE {}",
        serde_json::json!({
            "case":"expired_idle", "first":format!("{first:?}"), "second":format!("{second:?}"),
            "reaped_before_safety_cleanup":reaped_before_drop,
            "alive_before_safety_cleanup":alive_before_drop,
            "events":c.test_events()
        })
    );
    // Safety cleanup belongs to the review test; it must not certify shutdown.
    drop(c);
    let grace = until();
    while alive(pid) && Instant::now() < grace {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        reaped_before_drop && !alive_before_drop,
        "expired shutdown left an idle child live; a subsequent close was {second:?}"
    );
}

#[test]
fn shutdown_deadline_must_fail_active_work_and_reclaim_its_child() {
    let c = Arc::new(client("hang"));
    let token = Cancellation::new();
    let running = Arc::clone(&c);
    let running_token = token.clone();
    let pending = thread::spawn(move || {
        running.embed_cancellable(
            vec!["hang".into()],
            Instant::now() + Duration::from_secs(5),
            running_token,
        )
    });
    let ready = until();
    while !c
        .test_events()
        .iter()
        .any(|e| e.kind == "write_completed" && e.request_id == Some(2))
    {
        assert!(
            Instant::now() < ready,
            "fixture never entered the active request"
        );
        thread::yield_now();
    }
    let pid = c.test_events().iter().find_map(|e| e.pid).unwrap();
    let close_started = Instant::now();
    let close = c.shutdown(close_started + Duration::from_millis(100));
    let close_elapsed_ns = close_started.elapsed().as_nanos();
    let grace = until();
    while !(pending.is_finished() && c.test_events().iter().any(|e| e.kind == "reaped"))
        && Instant::now() < grace
    {
        thread::sleep(Duration::from_millis(1));
    }
    let finished_before_safety_cancel = pending.is_finished();
    let reaped_before_safety_cancel = c.test_events().iter().any(|e| e.kind == "reaped");
    let alive_before_safety_cancel = alive(pid);
    println!(
        "ROOT_CLOSE_CASE {}",
        serde_json::json!({
            "case":"busy_deadline", "close":format!("{close:?}"), "close_elapsed_ns":close_elapsed_ns, "observation_elapsed_ns":close_started.elapsed().as_nanos(),
            "finished_before_safety_cleanup":finished_before_safety_cancel,
            "reaped_before_safety_cleanup":reaped_before_safety_cancel,
            "alive_before_safety_cleanup":alive_before_safety_cancel,
            "events":c.test_events()
        })
    );
    // Release the intentionally hung fixture even when the reviewed code fails.
    token.cancel();
    let outcome = pending.join().unwrap();
    println!(
        "ROOT_CLOSE_OUTCOME {}",
        serde_json::json!({"case":"busy_deadline", "outcome":format!("{outcome:?}"), "finished_before_safety_cleanup":finished_before_safety_cancel})
    );
    assert!(
        matches!(close, Err(ClientError::Timeout)) || close.is_ok(),
        "unexpected close: {close:?}"
    );
    assert!(outcome.is_err());
    assert!(
        finished_before_safety_cancel && reaped_before_safety_cancel && !alive_before_safety_cancel,
        "shutdown deadline left active work and its child running until manual cancellation"
    );
}

#[test]
fn busy_close_wakes_queued_and_duplicate_closes_and_reaps() {
    let c = Arc::new(client("hang"));
    let token = Cancellation::new();
    let running = Arc::clone(&c);
    let running_token = token.clone();
    let active = thread::spawn(move || {
        running.embed_cancellable(
            vec!["hang".into()],
            until() + Duration::from_secs(2),
            running_token,
        )
    });
    let ready = until();
    while !c
        .test_events()
        .iter()
        .any(|e| e.kind == "write_completed" && e.request_id == Some(2))
    {
        assert!(Instant::now() < ready);
        thread::yield_now();
    }
    let queued_client = Arc::clone(&c);
    let queued = thread::spawn(move || queued_client.embed(vec!["queued".into()], until()));
    while c.test_queue_depth() != 1 {
        assert!(Instant::now() < ready);
        thread::yield_now();
    }
    let closing_client = Arc::clone(&c);
    let closing =
        thread::spawn(move || closing_client.shutdown(Instant::now() + Duration::from_millis(100)));
    while !queued.is_finished() {
        assert!(Instant::now() < ready);
        thread::yield_now();
    }
    let duplicate = c.shutdown(until());
    let close = closing.join().unwrap();
    let grace = until();
    while !(active.is_finished() && c.test_events().iter().any(|e| e.kind == "reaped"))
        && Instant::now() < grace
    {
        thread::sleep(Duration::from_millis(1));
    }
    let finished = active.is_finished();
    let reaped = c.test_events().iter().any(|e| e.kind == "reaped");
    println!(
        "CLOSE_QUEUE_CASE {}",
        serde_json::json!({"duplicate":format!("{duplicate:?}"), "close":format!("{close:?}"), "finished_before_safety_cleanup":finished, "reaped_before_safety_cleanup":reaped, "events":c.test_events()})
    );
    token.cancel();
    let active_outcome = active.join().unwrap();
    let queued_outcome = queued.join().unwrap();
    println!(
        "CLOSE_QUEUE_OUTCOME {}",
        serde_json::json!({"active":format!("{active_outcome:?}"), "queued":format!("{queued_outcome:?}"), "finished_before_safety_cleanup":finished})
    );
    assert!(active_outcome.is_err());
    assert!(matches!(queued_outcome, Err(ClientError::Closed)));
    assert!(matches!(duplicate, Err(ClientError::Closed)));
    assert!(matches!(close, Err(ClientError::Timeout)));
    assert!(finished && reaped, "busy close failed to own cleanup");
}
