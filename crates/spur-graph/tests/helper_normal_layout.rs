#![cfg(all(feature = "test-support", feature = "embed", unix))]
use spur_embedding::{Cancellation, CleanupOwner, EmbeddingClient, LocalHelper, ModelId};
use spur_graph::{store::lance_sections::load_embedding_model, EmbeddingModelSelection};
use std::{fs, path::PathBuf, process::Command, sync::Arc, time::Instant};

/// Requires a normal-feature helper built separately and private, pre-copied model assets.
#[test]
#[ignore = "explicit M3 acceptance with normal helper and private model/runtime caches"]
fn normal_helper_in_private_layout() {
    let root =
        PathBuf::from(std::env::var_os("SPUR_M3_NORMAL_ROOT").expect("private acceptance root"));
    if std::env::var_os("SPUR_M3_NORMAL_CHILD").is_none() {
        let host = root.join("graph host with spaces");
        fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
        let output = Command::new(host)
            .args([
                "--exact",
                "normal_helper_in_private_layout",
                "--ignored",
                "--nocapture",
            ])
            .env("SPUR_M3_NORMAL_CHILD", "1")
            .env_remove("SPUR_EMBEDDING_HELPER_CONFIG")
            .env_remove("SPUR_EMBEDDING_HELPER")
            .env_remove("SPUR_ORT_CACHE")
            .env("HOME", root.join("home"))
            .env("SPUR_MODEL_CACHE", root.join("model cache"))
            .env("HF_ENDPOINT", "http://127.0.0.1:9")
            .output()
            .unwrap();
        fs::write(root.join("host.stdout"), &output.stdout).unwrap();
        fs::write(root.join("host.stderr"), &output.stderr).unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    let before = std::env::var_os("SPUR_ORT_CACHE");
    assert!(before.is_none());
    let text = "search_document: the supervised helper returns finite embeddings";
    let start = Instant::now();
    let mut model =
        load_embedding_model(EmbeddingModelSelection::NomicEmbedTextV15, false).unwrap();
    let rows = model.embed(&[text], None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].len(), 768);
    assert!(rows[0].iter().all(|v| v.is_finite()));
    model.close().expect("graph checked worker close");
    let graph_elapsed = start.elapsed();

    let helper = LocalHelper::from_env().unwrap();
    let local = helper.policy();
    let policy = local.client_policy().unwrap();
    let cleanup = Arc::new(CleanupOwner::new(policy.shutdown_poll_interval).unwrap());
    let deadline = Instant::now() + policy.startup_timeout;
    helper
        .check_compatibility(deadline, Cancellation::new(), Arc::clone(&cleanup))
        .unwrap();
    let identity = EmbeddingClient::describe_command_with_cleanup(
        helper.command().clone(),
        ModelId::NomicEmbedTextV15,
        local.artifact_budget,
        local.limits.clone(),
        policy.clone(),
        deadline,
        Cancellation::new(),
        Arc::clone(&cleanup),
    )
    .unwrap();
    let mut args = helper.command().args().to_vec();
    args.extend(
        [
            local.limits.max_frame_bytes.to_string(),
            local.limits.max_texts.to_string(),
            local.limits.max_text_bytes.to_string(),
            local.limits.max_total_text_bytes.to_string(),
            local.limits.max_dimensions.to_string(),
            local.limits.max_identity_bytes.to_string(),
            local.limits.max_pending.to_string(),
            local.artifact_budget.to_string(),
        ]
        .into_iter()
        .map(Into::into),
    );
    let command = helper.command().with_args(args.clone());
    let client = EmbeddingClient::new_command_with_cleanup(
        command,
        identity.clone(),
        local.limits.clone(),
        policy.clone(),
        Arc::clone(&cleanup),
    )
    .unwrap();
    let neutral_rows = client
        .embed(vec![text.into()], Instant::now() + policy.request_timeout)
        .unwrap();
    assert_eq!(neutral_rows.len(), 1);
    assert_eq!(neutral_rows[0].len(), 768);
    assert!(neutral_rows[0].iter().all(|v| v.is_finite()));
    assert_eq!(rows, neutral_rows, "graph and neutral path differ");
    client
        .shutdown(Instant::now() + policy.shutdown_timeout)
        .expect("neutral checked close");
    cleanup
        .wait_for_cleanup(Instant::now() + policy.shutdown_timeout)
        .unwrap();
    assert_eq!(std::env::var_os("SPUR_ORT_CACHE"), before);
    let events = client.test_events();
    assert!(
        events.iter().any(|event| event.kind == "reaped"
            && event
                .status
                .as_deref()
                .is_some_and(|value| value.contains("unix_wait_status(0)"))),
        "no checked exit0 observation"
    );
    let observation = serde_json::json!({
        "program":helper.command().program(),"serve_args":args,
        "child_environment":helper.command().environment(),"policy":local,"identity":identity,
        "graph_elapsed_ms":graph_elapsed.as_millis(),"graph_rows":rows.len(),"dimensions":rows[0].len(),
        "finite":true,"vectors":rows,"events":events,"parent_cache_unchanged":true,
        "graph_checked_close":true,"neutral_checked_close":true,
    });
    fs::write(
        root.join("actual.json"),
        serde_json::to_vec_pretty(&observation).unwrap(),
    )
    .unwrap();
}
