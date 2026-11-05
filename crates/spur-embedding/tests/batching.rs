use serde_json::Value;
use spur_embedding::{
    plan_batches, LocalPolicy, ModelIdentity, Request, RequestEnvelope, PROTOCOL_VERSION,
};

#[test]
fn planner_matches_all_reviewed_reference_cases_and_rust_serialization() {
    let reference: Value = serde_json::from_str(include_str!(
        "../../../docs/research/artifacts/2026-11-05-embedding-sol/M4-pre/reference.json"
    ))
    .unwrap();
    let identity: ModelIdentity =
        serde_json::from_value(reference["protocol_identity"].clone()).unwrap();
    let defaults = LocalPolicy::default().limits;
    let m = defaults.max_text_bytes;
    let n = defaults.max_texts;
    for case in reference["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let texts: Vec<String> = match name {
            "empty" => vec![],
            "explicit_zero_batch" => vec!["a".into()],
            "empty_individual" => vec![String::new()],
            "ascii_exact" => vec!["x".repeat(m)],
            "ascii_over" => vec!["x".repeat(m + 1)],
            "unicode_exact" => vec!["é".repeat(m / 2)],
            "unicode_over" => vec!["é".repeat(m / 2 + 1)],
            "total_override_split" => vec!["x".repeat(m); 2],
            "exact_frame" | "one_less_frame" => vec!["a".into(), "b".into()],
            "escape_frame_split" => vec!["\0".into(), "\n".into()],
            "individual_frame_over" => vec!["\0".into()],
            "total_cap_too_small" => vec!["aa".into()],
            _ if name.starts_with("row_boundary_") => (0..case["input_rows"].as_u64().unwrap())
                .map(|i| i.to_string())
                .collect(),
            _ if name.starts_with("batch_override_") => (0..n + 1).map(|i| i.to_string()).collect(),
            _ => panic!("unknown reviewed case {name}"),
        };
        let limits = serde_json::from_value(case["limits"].clone()).unwrap();
        let requested = case["requested_batch"].as_u64().map(|n| n as usize);
        let plan = plan_batches(&texts, &identity, &limits, requested);
        if !case["error"].is_null() {
            assert!(plan.is_err(), "{name}: {plan:?}");
            continue;
        }
        let plan = plan.unwrap();
        assert_eq!(
            plan.len(),
            case["chunks"].as_array().unwrap().len(),
            "{name}"
        );
        let mut observed = Vec::new();
        for (range, expected) in plan.iter().zip(case["chunks"].as_array().unwrap()) {
            let chunk = texts[range.clone()].to_vec();
            let envelope = RequestEnvelope {
                version: PROTOCOL_VERSION,
                generation: "ffffffff-ffff-ffff-ffff-ffffffffffff".parse().unwrap(),
                request_id: u64::MAX,
                request: Request::Embed {
                    identity: identity.clone(),
                    texts: chunk.clone(),
                },
            };
            let mut wire = Vec::new();
            spur_embedding::write_frame(&mut wire, &envelope, &limits).unwrap();
            assert_eq!(
                chunk.len() as u64,
                expected["rows"].as_u64().unwrap(),
                "{name}"
            );
            assert_eq!(
                chunk.iter().map(String::len).sum::<usize>() as u64,
                expected["raw_bytes"].as_u64().unwrap(),
                "{name}"
            );
            assert_eq!(
                (wire.len() - 4) as u64,
                expected["body_bytes"].as_u64().unwrap(),
                "{name}: real Rust writer"
            );
            observed.extend(chunk);
        }
        assert_eq!(observed, texts, "{name}: input order/identity");
    }
}
