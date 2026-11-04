use spur_embedding::{ClientPolicy, EmbeddingClient, ModelId, ProtocolLimits};
use std::{
    fs,
    time::{Duration, Instant},
};

#[test]
fn client_discovers_correlated_model_identity_without_starting_native_runtime() {
    let root = std::env::temp_dir().join(format!("spur-descriptor-client-{}", std::process::id()));
    let repo = root.join("models--nomic-ai--nomic-embed-text-v1.5");
    let snapshot = repo.join("snapshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir_all(snapshot.join("onnx")).unwrap();
    fs::create_dir_all(repo.join("refs")).unwrap();
    fs::write(
        repo.join("refs/main"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    for name in [
        "onnx/model.onnx",
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
    ] {
        fs::write(snapshot.join(name), name).unwrap();
    }
    // This test is isolated in its own process; the companion reads this cache.
    std::env::set_var("SPUR_MODEL_CACHE", &root);
    std::env::set_var("SPUR_ORT_CACHE", "/dev/null/unusable");
    let limits = ProtocolLimits {
        max_frame_bytes: 65536,
        max_texts: 2,
        max_text_bytes: 4096,
        max_total_text_bytes: 8192,
        max_dimensions: 768,
        max_identity_bytes: 256,
        max_pending: 1,
    };
    let policy = ClientPolicy {
        queue_capacity: 1,
        max_restarts: 0,
        startup_timeout: Duration::from_secs(10),
        request_timeout: Duration::from_secs(10),
        shutdown_timeout: Duration::from_secs(10),
        shutdown_poll_interval: Duration::from_millis(10),
    };
    let identity = EmbeddingClient::describe_model(
        env!("CARGO_BIN_EXE_spur-embedding-worker"),
        vec![],
        ModelId::NomicEmbedTextV15,
        10000,
        limits,
        policy,
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(identity.model, ModelId::NomicEmbedTextV15);
    assert_eq!(identity.dimensions, 768);
    assert_eq!(identity.model_fingerprint.len(), 64);
    assert_eq!(identity.tokenizer_fingerprint.len(), 64);
    fs::remove_dir_all(root).unwrap();
}
