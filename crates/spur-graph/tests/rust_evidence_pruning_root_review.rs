use std::{fs, sync::Arc};

use spur_graph::query_client::{GraphQueryClient, InMemoryClient, ParquetClient};
use spur_graph::{
    artifact_from_facts, build_facts, read_artifact_parquet, write_artifact_parquet,
    RustEvidenceAvailability, RustEvidenceFilter, WriteOptions,
};

#[test]
fn footer_only_path_statistics_cannot_silently_drop_a_validated_source() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let filter = RustEvidenceFilter {
        file_path: Some("empty.rs".into()),
        source_oid: None,
    };
    let memory = InMemoryClient::new(Arc::new(graph.clone()));
    let expected = memory.rust_evidence(&filter).unwrap();
    assert_eq!(expected.availability, RustEvidenceAvailability::Complete);
    assert_eq!(expected.files.len(), 1);
    let control = ParquetClient::open(out.path()).unwrap();
    assert_eq!(control.rust_evidence(&filter).unwrap(), expected);

    let table = out.path().join("rust_evidence_sources.parquet");
    let original = fs::read(&table).unwrap();
    let end = original.len();
    assert_eq!(&original[end - 4..], b"PAR1");
    let footer_len = u32::from_le_bytes(original[end - 8..end - 4].try_into().unwrap()) as usize;
    let footer_start = end - 8 - footer_len;
    let wanted = b"empty.rs";
    let replacement = b"zzzzz.rs";
    assert_eq!(wanted.len(), replacement.len());
    let positions = (footer_start..=end - 8 - wanted.len())
        .filter(|&i| &original[i..i + wanted.len()] == wanted)
        .collect::<Vec<_>>();
    assert!(!positions.is_empty(), "fixture has no path statistics");
    let mut changed = original.clone();
    for &i in &positions {
        changed[i..i + wanted.len()].copy_from_slice(replacement);
    }
    assert_eq!(&changed[..footer_start], &original[..footer_start]);
    assert_eq!(&changed[end - 8..], &original[end - 8..]);
    let manifest_before = fs::read(out.path().join("manifest.json")).unwrap();
    fs::write(&table, &changed).unwrap();
    assert_eq!(
        fs::read(out.path().join("manifest.json")).unwrap(),
        manifest_before
    );

    // A full reader may reject invalid metadata. If it accepts the snapshot,
    // its decoded evidence must still match the original data pages.
    let loaded = read_artifact_parquet(out.path());
    if let Ok(loaded) = &loaded {
        assert_eq!(loaded.rust_evidence, graph.rust_evidence);
    }
    let cold = ParquetClient::open(out.path()).unwrap();
    let actual = cold.rust_evidence(&filter);
    println!(
        "D7B_ROOT_STATS {}",
        serde_json::json!({
            "footer_replacements": positions.len(),
            "data_pages_unchanged": true,
            "manifest_unchanged": true,
            "decoded_envelope_unchanged": loaded.as_ref().is_ok_and(|value| value.rust_evidence == graph.rust_evidence),
            "full_read_rejected": loaded.is_err(),
            "filter": filter,
            "expected": expected,
            "actual": actual.as_ref().map_err(|e| format!("{e:#}")),
        })
    );
    assert!(
        actual.as_ref().map_or(true, |value| value == &expected),
        "footer-only path statistics silently changed the validated projection: {actual:?}"
    );
}
