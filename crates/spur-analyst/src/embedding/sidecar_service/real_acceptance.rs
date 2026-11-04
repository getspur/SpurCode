use super::*;

#[tokio::test]
#[ignore = "requires an explicitly configured real helper and model cache"]
async fn warm_query_and_v1_multi_text_share_owned_runtime() {
    let query = std::env::var("SPUR_S6C_QUERY").expect("explicit query");
    let output = std::env::var("SPUR_S6C_OUTPUT").expect("explicit output");
    let mut cycles = Vec::new();
    for cycle in 0..2 {
        let runtime = OwnedEmbeddingRuntime::new();
        runtime.warm().await.expect("real warm inference");
        let first = runtime.embed_query(&query).await.expect("real query");
        let service = EmbedService::owned(runtime.clone());
        let request = json!({"v": 1, "id": "batch", "op": "embed", "texts": [query, query]});
        let batch = service.handle_line(&request.to_string()).await;
        assert!(batch.get("error").is_none(), "{batch}");
        let rows = batch["vectors"].as_array().expect("v1 batch vectors");
        assert_eq!(rows.len(), 2);
        let rows = rows.clone();
        drop(service);
        runtime.shutdown().await.expect("checked owned shutdown");
        cycles.push(json!({"cycle": cycle, "query": first.to_vec(), "batch": rows}));
        std::fs::write(&output, serde_json::to_vec_pretty(&cycles).unwrap()).unwrap();
    }
}
