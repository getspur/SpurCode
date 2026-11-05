#![cfg(all(feature = "test-support", feature = "embed", unix))]
use spur_embedding::{Cancellation, CleanupOwner, EmbeddingClient, LocalHelper};
use spur_graph::{store::lance_sections::load_embedding_model, EmbeddingModelSelection};
use std::{fs, path::PathBuf, process::Command, sync::Arc, time::Instant};

fn vector_delta(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut delta = 0.0f32;
    for (row, other) in a.iter().zip(b) {
        assert_eq!(row.len(), 768);
        assert_eq!(other.len(), 768);
        for (a, b) in row.iter().zip(other) {
            assert!(a.is_finite() && b.is_finite());
            delta = delta.max((a - b).abs());
        }
    }
    delta
}

#[test]
#[ignore = "explicit M4 acceptance with normal helper and three private model caches"]
fn normal_helper_batches_match_three_pinned_references() {
    let root = PathBuf::from(std::env::var_os("SPUR_M4_NORMAL_ROOT").expect("private layout"));
    if std::env::var_os("SPUR_M4_NORMAL_CHILD").is_none() {
        let host = root.join("batch host ü with spaces");
        fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
        let output = Command::new(host)
            .args([
                "--exact",
                "normal_helper_batches_match_three_pinned_references",
                "--ignored",
                "--nocapture",
            ])
            .env("SPUR_M4_NORMAL_CHILD", "1")
            .env_remove("SPUR_EMBEDDING_HELPER_CONFIG")
            .env_remove("SPUR_EMBEDDING_HELPER")
            .env_remove("SPUR_ORT_CACHE")
            .env("HOME", root.join("home"))
            .env("SPUR_MODEL_CACHE", root.join("model cache"))
            .env("HF_ENDPOINT", "http://127.0.0.1:9")
            .output()
            .unwrap();
        fs::write(root.join("batch-host.stdout"), &output.stdout).unwrap();
        fs::write(root.join("batch-host.stderr"), &output.stderr).unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    let before = std::env::var_os("SPUR_ORT_CACHE");
    assert!(before.is_none());
    let mut observations = Vec::new();
    for selection in [
        EmbeddingModelSelection::NomicEmbedTextV15,
        EmbeddingModelSelection::CodeRankEmbed,
        EmbeddingModelSelection::JinaEmbeddingsV2BaseCode,
    ] {
        let refs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/artifacts/2026-11-03-embedding-isolation/S5/worker/real")
            .join(selection.model_name());
        let inputs: Vec<String> =
            serde_json::from_slice(&fs::read(refs.join("inputs.json")).unwrap()).unwrap();
        let expected: Vec<Vec<f32>> =
            serde_json::from_slice(&fs::read(refs.join("reference.json")).unwrap()).unwrap();
        let texts = vec![inputs[0].clone(), inputs[1].clone(), inputs[0].clone()];
        let start = Instant::now();
        let mut model = load_embedding_model(selection, false).unwrap();
        let graph_rows = model.embed(&texts, Some(2)).unwrap();
        let repeated = model.embed(&texts, Some(1)).unwrap();
        model.close().expect("graph checked close");
        let reused_delta = vector_delta(&graph_rows, &repeated);
        assert!(reused_delta <= 1e-4, "reused helper delta {reused_delta}");
        let graph_ms = start.elapsed().as_millis();
        // Absolute discovery of the same normal artifact and child-scoped runtime cache.
        std::env::set_var("SPUR_EMBEDDING_HELPER", root.join("spur-embedding-worker"));
        let helper = LocalHelper::from_env().unwrap();
        std::env::remove_var("SPUR_EMBEDDING_HELPER");
        let local = helper.policy();
        let policy = local.client_policy().unwrap();
        let cleanup = Arc::new(CleanupOwner::new(policy.shutdown_poll_interval).unwrap());
        let deadline = Instant::now() + policy.startup_timeout;
        helper
            .check_compatibility(deadline, Cancellation::new(), Arc::clone(&cleanup))
            .unwrap();
        let identity = EmbeddingClient::describe_command_with_cleanup(
            helper.command().clone(),
            selection.model_id(),
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
        let client = EmbeddingClient::new_command_with_cleanup(
            helper.command().with_args(args),
            identity.clone(),
            local.limits.clone(),
            policy.clone(),
            Arc::clone(&cleanup),
        )
        .unwrap();
        let neutral = client
            .embed_batched_cancellable(
                texts.clone(),
                Some(2),
                Instant::now() + policy.request_timeout,
                Cancellation::new(),
            )
            .unwrap();
        let neutral_delta = vector_delta(&graph_rows, &neutral);
        assert!(neutral_delta <= 1e-4, "neutral delta {neutral_delta}");
        let mut delta = 0.0f32;
        for (index, row) in graph_rows.iter().enumerate() {
            assert_eq!(row.len(), 768);
            for (a, b) in row.iter().zip(&expected[index % 2]) {
                assert!(a.is_finite() && b.is_finite());
                delta = delta.max((a - b).abs());
            }
        }
        assert!(delta <= 1e-4, "{} delta {delta}", selection.model_name());
        client
            .shutdown(Instant::now() + policy.shutdown_timeout)
            .unwrap();
        cleanup
            .wait_for_cleanup(Instant::now() + policy.shutdown_timeout)
            .unwrap();
        let events = client.test_events();
        assert_eq!(events.iter().filter(|e| e.kind == "spawned").count(), 1);
        assert_eq!(events.iter().filter(|e|e.kind=="reply_validated" && e.status.as_deref()==Some("embeddings")).count(),2);
        assert!(events.iter().any(|e| e.kind == "reaped"
            && e.status.as_deref() == Some("ExitStatus(unix_wait_status(0))")));
        assert_eq!(std::env::var_os("SPUR_ORT_CACHE"), before);
        observations.push(serde_json::json!({"model":selection.model_name(),"identity":identity,"policy":local,"inputs":texts,"vectors":graph_rows,
            "reference_max_delta":delta,"reused_max_delta":reused_delta,"neutral_max_delta":neutral_delta,"reference_path":refs,"events":events,"graph_ms":graph_ms,"graph_checked_close":true,"neutral_checked_close":true,"parent_cache_unchanged":true}));
    }
    fs::write(
        root.join("batch-actual.json"),
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
}
