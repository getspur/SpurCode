use spur_embedding::{HelperConfig, LocalPolicy};
use std::time::{Duration, Instant};

fn observed_config(raw: &str) -> Result<HelperConfig, spur_embedding::PolicyError> {
    let result = HelperConfig::from_json(raw);
    println!(
        "policy_observation={}",
        serde_json::json!({
            "raw": raw, "accepted": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string)
        })
    );
    result
}

fn explicit() -> serde_json::Value {
    serde_json::json!({
        "executable": "/missing/helper with spaces", "args": ["--fixture"],
        "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2,
            "max_text_bytes": 8192, "max_total_text_bytes": 16384,
            "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0,
        "startup_timeout_ms": 10000, "request_timeout_ms": 10000,
        "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    })
}

#[test]
fn defaults_match_reviewed_policy_and_export_actual_values() {
    let policy = LocalPolicy::default();
    policy.validate().unwrap();
    let actual = serde_json::to_value(&policy).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "limits": {"max_frame_bytes": 33554432, "max_texts": 64,
                "max_text_bytes": 65536, "max_total_text_bytes": 4194304,
                "max_dimensions": 768, "max_identity_bytes": 64, "max_pending": 1},
            "artifact_budget": 644080771, "queue_capacity": 1, "max_restarts": 0,
            "startup_timeout_ms": 120000, "request_timeout_ms": 240000,
            "shutdown_timeout_ms": 30000, "shutdown_poll_interval_ms": 20
        })
    );
    println!("policy_export={actual}");
}

#[test]
fn reviewed_default_caller_arithmetic_covers_actual_escaped_frame() {
    use spur_embedding::{
        write_frame, ModelId, ModelIdentity, ProtocolError, Request, RequestEnvelope,
        PROTOCOL_VERSION,
    };
    let policy = LocalPolicy::default();
    let limits = &policy.limits;
    let field = "\0".repeat(limits.max_identity_bytes);
    let request = RequestEnvelope {
        version: PROTOCOL_VERSION,
        generation: uuid::Uuid::from_u128(u128::MAX),
        request_id: u64::MAX,
        request: Request::Embed {
            identity: ModelIdentity {
                model: ModelId::JinaEmbeddingsV2BaseCode,
                dimensions: limits.max_dimensions,
                model_fingerprint: field.clone(),
                tokenizer_fingerprint: field.clone(),
                pooling: field.clone(),
                normalization: field.clone(),
                prefix: field,
            },
            texts: vec!["\0".repeat(limits.max_text_bytes); limits.max_texts],
        },
    };
    let raw_bytes = limits.max_texts.checked_mul(limits.max_text_bytes).unwrap();
    assert_eq!(raw_bytes, limits.max_total_text_bytes);
    let mut wire = Vec::new();
    write_frame(&mut wire, &request, limits).unwrap();
    let body_bytes = u32::from_be_bytes(wire[..4].try_into().unwrap());
    assert_eq!(body_bytes, 25_168_231);
    assert_eq!(wire.len(), body_bytes as usize + 4);
    assert!(body_bytes <= limits.max_frame_bytes);
    let mut short = limits.clone();
    short.max_frame_bytes = body_bytes - 1;
    let mut unpublished = Vec::new();
    let short_error = write_frame(&mut unpublished, &request, &short).unwrap_err();
    let short_rejected = matches!(&short_error, ProtocolError::Json(error) if error.is_io());
    assert!(short_rejected);
    assert!(
        unpublished.is_empty(),
        "oversized frame was partially published"
    );
    println!(
        "caller_arithmetic={}",
        serde_json::json!({
            "rows": limits.max_texts, "text_bytes_each": limits.max_text_bytes,
            "raw_text_bytes": raw_bytes, "escaped_request_body_bytes": body_bytes,
            "frame_limit_bytes": limits.max_frame_bytes,
            "raw_float_bytes": limits.max_texts * limits.max_dimensions * std::mem::size_of::<f32>(),
            "startup_phase_budget_ms": policy.startup_timeout_ms,
            "first_inference_phase_budget_ms": 120000,
            "composed_request_ms": policy.request_timeout_ms,
            "one_byte_short_frame_rejected": short_rejected,
            "one_byte_short_frame_error": short_error.to_string(),
            "one_byte_short_frame_published_bytes": unpublished.len()
        })
    );
}

#[test]
fn explicit_override_preserves_values_arguments_and_client_projection() {
    let config = observed_config(&explicit().to_string()).unwrap();
    assert_eq!(
        config.executable.to_str(),
        Some("/missing/helper with spaces")
    );
    assert_eq!(config.args, ["--fixture"]);
    let policy = config.policy();
    assert_eq!(policy.artifact_budget, 10000);
    assert_eq!(policy.limits.max_texts, 2);
    assert_eq!(policy.limits.max_identity_bytes, 256);
    let client = policy.client_policy().unwrap();
    assert_eq!(client.queue_capacity, 1);
    assert_eq!(client.max_restarts, 0);
    assert_eq!(client.startup_timeout, Duration::from_millis(10000));
    assert_eq!(client.request_timeout, Duration::from_millis(10000));
    assert_eq!(client.shutdown_timeout, Duration::from_millis(10000));
    assert_eq!(client.shutdown_poll_interval, Duration::from_millis(10));
}

#[test]
fn explicit_caps_remain_independent_and_may_be_smaller_than_defaults() {
    let mut value = explicit();
    value["limits"]["max_frame_bytes"] = 1.into();
    value["limits"]["max_total_text_bytes"] = 1.into();
    value["artifact_budget"] = 1.into();
    value["max_restarts"] = 2.into();
    let policy = observed_config(&value.to_string()).unwrap().policy();
    assert_eq!(policy.limits.max_frame_bytes, 1);
    assert_eq!(policy.limits.max_total_text_bytes, 1);
    assert_eq!(policy.artifact_budget, 1);
    assert_eq!(policy.client_policy().unwrap().max_restarts, 2);
}

#[test]
fn supplied_json_requires_every_field_and_rejects_unknown_and_duplicate_fields() {
    let original = explicit();
    for name in original.as_object().unwrap().keys() {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(name);
        assert!(
            observed_config(&value.to_string()).is_err(),
            "missing {name}"
        );
    }
    for name in original["limits"].as_object().unwrap().keys() {
        let mut value = original.clone();
        value["limits"].as_object_mut().unwrap().remove(name);
        assert!(
            observed_config(&value.to_string()).is_err(),
            "missing limits.{name}"
        );
    }
    for pointer in ["", "/limits"] {
        let mut value = original.clone();
        value.pointer_mut(pointer).unwrap()["typo"] = 1.into();
        assert!(
            observed_config(&value.to_string()).is_err(),
            "unknown {pointer}"
        );
    }
    let duplicate = original
        .to_string()
        .replacen("{", "{\"artifact_budget\":1,", 1);
    assert!(observed_config(&duplicate).is_err());
    let nested_duplicate = original
        .to_string()
        .replace("\"limits\":{", "\"limits\":{\"max_texts\":1,");
    assert!(observed_config(&nested_duplicate).is_err());
    for invalid in ["", "null", "[]", "{}", "invalid JSON"] {
        assert!(observed_config(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn json_and_direct_policy_share_zero_and_overflow_validation() {
    let original = explicit();
    let pointers = [
        "/artifact_budget",
        "/queue_capacity",
        "/startup_timeout_ms",
        "/request_timeout_ms",
        "/shutdown_timeout_ms",
        "/shutdown_poll_interval_ms",
        "/limits/max_frame_bytes",
        "/limits/max_texts",
        "/limits/max_text_bytes",
        "/limits/max_total_text_bytes",
        "/limits/max_dimensions",
        "/limits/max_identity_bytes",
        "/limits/max_pending",
    ];
    for pointer in pointers {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = 0.into();
        assert!(
            observed_config(&value.to_string()).is_err(),
            "zero {pointer}"
        );
        value.as_object_mut().unwrap().remove("executable");
        value.as_object_mut().unwrap().remove("args");
        let policy: LocalPolicy = serde_json::from_value(value).unwrap();
        assert!(policy.validate().is_err(), "direct zero {pointer}");
        assert!(policy.client_policy().is_err(), "projection zero {pointer}");
    }
    for pointer in [
        "/queue_capacity",
        "/limits/max_texts",
        "/limits/max_dimensions",
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = serde_json::json!(usize::MAX);
        assert!(
            observed_config(&value.to_string()).is_err(),
            "overflow {pointer}"
        );
    }
    let mut value = original.clone();
    value["limits"]["max_frame_bytes"] = serde_json::json!(u64::from(u32::MAX) + 1);
    assert!(observed_config(&value.to_string()).is_err());
    for pointer in [
        "/startup_timeout_ms",
        "/request_timeout_ms",
        "/shutdown_timeout_ms",
        "/shutdown_poll_interval_ms",
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = serde_json::json!(u64::MAX);
        let representable = Instant::now()
            .checked_add(Duration::from_millis(u64::MAX))
            .is_some();
        assert_eq!(
            observed_config(&value.to_string()).is_ok(),
            representable,
            "deadline {pointer}"
        );
    }
    for invalid in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("10000"),
        serde_json::Value::Null,
    ] {
        let mut value = original.clone();
        value["artifact_budget"] = invalid;
        assert!(observed_config(&value.to_string()).is_err());
    }
    let mut empty_executable = original;
    empty_executable["executable"] = "".into();
    assert!(observed_config(&empty_executable.to_string()).is_err());
}

#[test]
fn environment_selection_is_child_scoped_and_never_resolves_a_helper() {
    const CHILD: &str = "SPUR_LOCAL_POLICY_ENV_CHILD";
    if let Ok(case) = std::env::var(CHILD) {
        let actual = LocalPolicy::from_env();
        match case.as_str() {
            "absent" => assert_eq!(actual.unwrap(), LocalPolicy::default()),
            "explicit" => assert_eq!(actual.unwrap().artifact_budget, 10000),
            "invalid" | "nonunicode" => assert!(actual.is_err()),
            _ => panic!("unexpected case"),
        }
        return;
    }
    for case in ["absent", "explicit", "invalid", "nonunicode"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "environment_selection_is_child_scoped_and_never_resolves_a_helper",
                "--nocapture",
            ])
            .env(CHILD, case)
            .env("SPUR_EMBEDDING_HELPER", "/missing/unused-helper")
            .env("SPUR_MODEL_CACHE_DIR", "/dev/null/unused-cache");
        match case {
            "absent" => {
                command.env_remove("SPUR_EMBEDDING_HELPER_CONFIG");
            }
            "explicit" => {
                command.env("SPUR_EMBEDDING_HELPER_CONFIG", explicit().to_string());
            }
            "invalid" => {
                command.env("SPUR_EMBEDDING_HELPER_CONFIG", "invalid JSON");
            }
            "nonunicode" => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    command.env(
                        "SPUR_EMBEDDING_HELPER_CONFIG",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                #[cfg(not(unix))]
                continue;
            }
            _ => unreachable!(),
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
