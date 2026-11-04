use serde::{de::DeserializeOwned, Serialize};
use spur_embedding::{Reply, Request};

fn check<T: DeserializeOwned + Serialize>(kind: &str, cases: &[(&str, &str, Option<&str>)]) {
    let mut failures = Vec::new();
    for (name, fields, duplicate) in cases {
        let valid = format!(r#"{{"type":"{name}"{fields}}}"#);
        let unknown = format!(r#"{{"type":"{name}"{fields},"extra":0}}"#);
        let discriminator = format!(r#"{{"type":"{name}","type":"{name}"{fields}}}"#);
        let mut observations = vec![
            ("valid", valid.clone(), true),
            ("unknown", unknown, false),
            ("duplicate_type", discriminator, false),
        ];
        if let Some(field) = duplicate {
            observations.push((
                "duplicate_field",
                format!(r#"{{"type":"{name}"{fields},{field}}}"#),
                false,
            ));
        }
        for (case, json, expected) in observations {
            let result = serde_json::from_str::<T>(&json);
            let outcome = if result.is_ok() { "accept" } else { "reject" };
            println!(
                "VARIANT_OBSERVATION {kind}_{name}_{case} {outcome} {result:?}",
                result = result.as_ref().err()
            );
            if result.is_ok() != expected {
                failures.push(format!("{kind}_{name}_{case}: {outcome}"));
            }
            if case == "valid" {
                if let Ok(value) = result {
                    let serialized = serde_json::to_string(&value).unwrap();
                    if serde_json::from_str::<T>(&serialized).is_err() {
                        failures.push(format!("{kind}_{name}_roundtrip"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

const IDENTITY: &str = r#"{"model":"CodeRankEmbed","dimensions":2,"model_fingerprint":"model","tokenizer_fingerprint":"tokenizer","pooling":"mean","normalization":"l2","prefix":"code"}"#;

#[test]
fn request_variants_enforce_schema() {
    let identity = format!(r#", "identity":{IDENTITY}"#);
    let embed = format!(r#", "identity":{IDENTITY},"texts":["x"]"#);
    let duplicate_identity = format!(r#""identity":{IDENTITY}"#);
    check::<Request>(
        "request",
        &[
            ("Hello", &identity, Some(&duplicate_identity)),
            ("Embed", &embed, Some(r#""texts":["x"]"#)),
            ("Shutdown", "", None),
        ],
    );
}

#[test]
fn reply_variants_enforce_schema() {
    let identity = format!(r#", "identity":{IDENTITY}"#);
    let rows = format!(r#", "identity":{IDENTITY},"rows":[[0.0,1.0]]"#);
    let duplicate_identity = format!(r#""identity":{IDENTITY}"#);
    check::<Reply>(
        "reply",
        &[
            ("Ready", &identity, Some(&duplicate_identity)),
            ("Embeddings", &rows, Some(r#""rows":[[0.0,1.0]]"#)),
            ("Closed", "", None),
            (
                "Error",
                r#", "code":"Internal","message":"error""#,
                Some(r#""message":"error""#),
            ),
        ],
    );
}
