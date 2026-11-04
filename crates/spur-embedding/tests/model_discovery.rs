#![cfg(feature = "test-support")]
use spur_embedding::{ClientError, ClientPolicy, EmbeddingClient, ModelId, ProtocolLimits};
use std::time::{Duration, Instant};

fn read_descriptor_pid(path: &std::path::Path) -> Option<i32> {
    let pid = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    (pid > 0).then_some(pid)
}

#[test]
fn descriptor_pid_file_requires_a_positive_numeric_pid() {
    let path = std::env::temp_dir().join(format!("spur-pid-validation-{}", uuid::Uuid::new_v4()));
    for invalid in ["", "0", "-1", "garbage"] {
        std::fs::write(&path, invalid).unwrap();
        assert!(read_descriptor_pid(&path).is_none(), "accepted {invalid:?}");
    }
    std::fs::write(&path, "123").unwrap();
    assert_eq!(read_descriptor_pid(&path), Some(123));
    std::fs::remove_file(path).unwrap();
}

fn describe(mode: &str, timeout: Duration) -> Result<spur_embedding::ModelIdentity, ClientError> {
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
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: timeout,
        request_timeout: timeout,
        shutdown_timeout: timeout,
        shutdown_poll_interval: Duration::from_millis(2),
    };
    EmbeddingClient::describe_model(
        env!("CARGO_BIN_EXE_embedding-fault-worker"),
        vec![mode.into()],
        ModelId::NomicEmbedTextV15,
        100,
        limits,
        policy,
        Instant::now() + timeout,
    )
}

#[test]
fn correlated_descriptor_requires_complete_reply_and_clean_exit() {
    let timeout = Duration::from_secs(5);
    let identity = describe("describe_ok", timeout).unwrap();
    assert_eq!(identity.model, ModelId::NomicEmbedTextV15);
    for mode in [
        "describe_wrong_model",
        "describe_wrong_generation",
        "describe_wrong_request",
        "describe_truncated",
        "describe_trailing",
        "describe_failed_exit",
    ] {
        assert!(
            matches!(describe(mode, timeout), Err(ClientError::Transport(_))),
            "{mode}"
        );
    }
    assert!(matches!(
        describe("describe_error", timeout),
        Err(ClientError::Remote { .. })
    ));
}

#[test]
fn descriptor_timeout_rejects_hung_process() {
    let started = Instant::now();
    assert!(matches!(
        describe("describe_hang", Duration::from_millis(100)),
        Err(ClientError::Timeout)
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn cancellation_interrupts_hung_descriptor_discovery() {
    use spur_embedding::Cancellation;
    let timeout = Duration::from_secs(5);
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
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: timeout,
        request_timeout: timeout,
        shutdown_timeout: timeout,
        shutdown_poll_interval: timeout,
    };
    let cancel = Cancellation::new();
    let trigger = cancel.clone();
    let job = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        trigger.cancel();
    });
    let start = Instant::now();
    let result = EmbeddingClient::describe_model_cancellable(
        env!("CARGO_BIN_EXE_embedding-fault-worker"),
        vec!["describe_hang".into()],
        ModelId::NomicEmbedTextV15,
        100,
        limits,
        policy,
        start + timeout,
        cancel,
    );
    job.join().unwrap();
    assert!(matches!(result, Err(ClientError::Cancelled)), "{result:?}");
    assert!(
        start.elapsed() < timeout,
        "discovery did not respond to cancellation"
    );
}

#[test]
fn owner_can_join_cancelled_descriptor_process() {
    use spur_embedding::{Cancellation, CleanupOwner};
    use std::sync::Arc;
    let timeout = Duration::from_secs(5);
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
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: timeout,
        request_timeout: timeout,
        shutdown_timeout: timeout,
        shutdown_poll_interval: Duration::from_millis(2),
    };
    let cleanup = Arc::new(CleanupOwner::new(policy.shutdown_poll_interval).unwrap());
    let path = std::env::temp_dir().join(format!("spur-descriptor-pid-{}", uuid::Uuid::new_v4()));
    let watch = path.clone();
    let cancel = Cancellation::new();
    let trigger = cancel.clone();
    let job = std::thread::spawn(move || {
        let until = Instant::now() + timeout;
        let pid = loop {
            if let Some(pid) = read_descriptor_pid(&watch) {
                break pid;
            }
            assert!(Instant::now() < until, "descriptor process never started");
            std::thread::yield_now();
        };
        let probe = std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "valid child probe failed: {probe:?}"
        );
        trigger.cancel();
        pid
    });
    let result = EmbeddingClient::describe_model_cancellable_with_cleanup(
        "/bin/sh",
        vec![
            "-c".into(),
            "printf '%s' $$ > \"$1\"; exec /usr/bin/tail -f /dev/null".into(),
            "descriptor".into(),
            path.as_os_str().to_owned(),
        ],
        ModelId::NomicEmbedTextV15,
        100,
        limits,
        policy,
        Instant::now() + timeout,
        cancel,
        Arc::clone(&cleanup),
    );
    let pid = job.join().unwrap();
    assert!(matches!(result, Err(ClientError::Cancelled)), "{result:?}");
    cleanup
        .wait_for_cleanup(Instant::now() + timeout)
        .expect("join descriptor cleanup");
    let probe = std::process::Command::new("/bin/kill")
        .env("LC_ALL", "C")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap();
    assert!(
        probe.status.code() == Some(1)
            && String::from_utf8_lossy(&probe.stderr).contains("No such process"),
        "descriptor process {pid} did not yield a valid absence probe: {probe:?}"
    );
    std::fs::remove_file(path).unwrap();
}
