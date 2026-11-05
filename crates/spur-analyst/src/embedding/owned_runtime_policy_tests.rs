use super::Timeouts;

#[test]
fn configuration_adapter_validates_full_policy_without_discovery() {
    const CHILD: &str = "SPUR_ANALYST_POLICY_ADAPTER_CHILD";
    if let Ok(case) = std::env::var(CHILD) {
        let result = Timeouts::from_env();
        if case == "explicit" {
            let actual = result.expect("valid explicit policy must not discover missing helper");
            assert_eq!(actual.queue_capacity, 1);
            assert_eq!(actual.startup_timeout_ms, 10000);
            assert_eq!(actual.request_timeout_ms, 10000);
            assert_eq!(actual.shutdown_timeout_ms, 10000);
            assert_eq!(actual.shutdown_poll_interval_ms, 10);
        } else {
            assert!(
                result.is_err(),
                "configuration adapter accepted invalid {case}"
            );
        }
        return;
    }
    let valid = serde_json::json!({
        "executable": "/missing/helper", "args": [], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let mut cases = vec![("explicit", valid.clone())];
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("limits");
    cases.push(("missing limits", missing));
    let mut unknown = valid.clone();
    unknown["typo"] = true.into();
    cases.push(("unknown field", unknown));
    let mut zero = valid.clone();
    zero["limits"]["max_pending"] = 0.into();
    cases.push(("zero protocol limit", zero));
    let mut queue_overflow = valid.clone();
    queue_overflow["queue_capacity"] = serde_json::json!(usize::MAX);
    cases.push(("queue overflow", queue_overflow));
    let mut row_overflow = valid;
    row_overflow["limits"]["max_texts"] = serde_json::json!(usize::MAX);
    cases.push(("row overflow", row_overflow));
    let mut failures = Vec::new();
    for (case, config) in cases {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "embedding::owned_runtime::policy_tests::configuration_adapter_validates_full_policy_without_discovery", "--nocapture"])
            .env(CHILD, case)
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .env("SPUR_EMBEDDING_HELPER", "/missing/unused-helper")
            .env("SPUR_MODEL_CACHE_DIR", "/dev/null/unused-cache")
            .output().unwrap();
        if !output.status.success() {
            failures.push(format!(
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
