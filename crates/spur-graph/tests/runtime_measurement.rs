//! Opt-in M6 measurement using the ordinary helper; no native library in this host.
#![cfg(all(feature = "embed", unix))]
use spur_graph::{store::lance_sections::load_embedding_model, EmbeddingModelSelection};
use std::{fs, path::PathBuf, time::Instant};

fn check(rows: &[Vec<f32>], expected: &[Vec<f32>]) -> f32 {
    assert_eq!(rows.len(), expected.len());
    let mut delta = 0.0f32;
    for (row, reference) in rows.iter().zip(expected) {
        assert_eq!(row.len(), 768);
        assert_eq!(row.len(), reference.len());
        assert!(row.iter().all(|v| v.is_finite()));
        let norm = row
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((norm - 1.0).abs() <= 1e-4, "norm {norm}");
        for (v, r) in row.iter().zip(reference) {
            delta = delta.max((v - r).abs());
        }
    }
    assert!(delta <= 1e-4, "reference delta {delta}");
    delta
}

#[test]
#[ignore = "explicit M6 private normal helper, pinned references and measurement output"]
fn separate_graph_owner_measures_distinct_batches_and_checked_close() {
    let output = PathBuf::from(std::env::var_os("SPUR_M6_GRAPH_OUTPUT").expect("output"));
    let mut observations = Vec::new();
    for selection in [
        EmbeddingModelSelection::NomicEmbedTextV15,
        EmbeddingModelSelection::CodeRankEmbed,
        EmbeddingModelSelection::JinaEmbeddingsV2BaseCode,
    ] {
        let reference_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/artifacts/2026-11-03-embedding-isolation/S5/worker/real")
            .join(selection.model_name());
        let texts: Vec<String> =
            serde_json::from_slice(&fs::read(reference_dir.join("inputs.json")).unwrap()).unwrap();
        let reference: Vec<Vec<f32>> =
            serde_json::from_slice(&fs::read(reference_dir.join("reference.json")).unwrap())
                .unwrap();
        assert!(
            texts.windows(2).any(|pair| pair[0] != pair[1]),
            "distinct batch required"
        );
        let started = Instant::now();
        let mut owner = load_embedding_model(selection, false).unwrap();
        let discovery_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let cold = owner.embed(&texts, None).unwrap();
        let session_cold_batch_ms = started.elapsed().as_secs_f64() * 1000.0;
        let cold_delta = check(&cold, &reference);
        let started = Instant::now();
        let warm = owner.embed(&texts, None).unwrap();
        let warm_batch_ms = started.elapsed().as_secs_f64() * 1000.0;
        let warm_delta = check(&warm, &reference);
        let started = Instant::now();
        owner.close().expect("checked graph close");
        let shutdown_ms = started.elapsed().as_secs_f64() * 1000.0;
        observations.push(serde_json::json!({
            "model": selection.model_name(), "inputs": texts, "reference_dir": reference_dir,
            "cold": cold, "warm": warm, "cold_reference_delta": cold_delta, "warm_reference_delta": warm_delta,
            "discovery_ms": discovery_ms, "session_cold_batch_ms": session_cold_batch_ms,
            "warm_batch_ms": warm_batch_ms, "warm_rows_per_second": texts.len() as f64 * 1000.0 / warm_batch_ms,
            "shutdown_ms": shutdown_ms, "checked_close": true,
            "owner": "separate graph owner; actual distinct multi-text requests; prepopulated disk cache; OS cache uncontrolled"
        }));
        fs::write(&output, serde_json::to_vec_pretty(&observations).unwrap()).unwrap();
        println!("M6_GRAPH_MEASUREMENT {}", observations.last().unwrap());
    }
}
