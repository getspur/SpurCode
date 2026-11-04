//! Explicitly configured real-helper acceptance; no native owner is linked into the analyst.
#[cfg(feature = "embed")]
#[tokio::test]
#[ignore = "requires explicitly configured real model helper and caches"]
async fn owned_real_query_reuses_and_closes() {
    let query = std::env::var("SPUR_S6C_QUERY").expect("explicit query");
    let output = std::env::var("SPUR_S6C_OUTPUT").expect("explicit output");
    let mut cycles = Vec::new();
    for cycle in 0..2 {
        let runtime = spur_analyst::embedding::OwnedEmbeddingRuntime::new();
        runtime.warm().await.expect("real inference warm");
        let first = runtime.embed_query(&query).await.expect("first query");
        let again = runtime
            .clone()
            .embed_query(&query)
            .await
            .expect("reused query");
        runtime
            .shutdown()
            .await
            .expect("checked close and joined cleanup");
        cycles.push(
            serde_json::json!({"cycle": cycle, "first": first.to_vec(), "again": again.to_vec()}),
        );
        std::fs::write(&output, serde_json::to_vec_pretty(&cycles).unwrap()).unwrap();
    }
}
