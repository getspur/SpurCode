use spur_graph::{
    artifact_from_facts, build_facts, read_artifact_parquet, write_artifact_parquet, NodeKind,
    WriteOptions,
};

#[test]
fn extern_container_kind_has_a_distinct_lossless_discriminator() {
    let kind =
        NodeKind::from_discriminator("extern_block").expect("non-callable extern container kind");
    assert_eq!(kind.discriminator(), "extern_block");
    assert_ne!(kind, NodeKind::External);
    assert_ne!(kind, NodeKind::Module);
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"extern_block\"");
    assert_eq!(serde_json::from_str::<NodeKind>(&json).unwrap(), kind);
}

#[test]
fn synthetic_extern_container_survives_artifact_and_parquet() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("lib.rs"), "fn synthetic() {}\n").unwrap();
    let (mut facts, _) = build_facts(root.path(), None).unwrap();
    let kind = NodeKind::from_discriminator("extern_block").expect("extern container kind");
    let node = facts
        .nodes
        .iter_mut()
        .find(|node| node.kind == NodeKind::Function)
        .unwrap();
    node.kind = kind;
    let stable_id = node.stable_key.clone();
    let artifact = artifact_from_facts(&facts, root.path()).unwrap();
    let symbol = artifact
        .symbols
        .iter()
        .find(|s| s.stable_symbol_id == stable_id)
        .unwrap();
    assert_eq!(symbol.symbol_kind, "extern_block");
    let expected = symbol.clone();
    let json = serde_json::to_string(&artifact).unwrap();
    let decoded: spur_graph::GraphIndexArtifact = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded
            .symbols
            .iter()
            .find(|s| s.stable_symbol_id == stable_id),
        Some(&expected)
    );
    let out = tempfile::tempdir().unwrap();
    let written =
        write_artifact_parquet(&artifact, out.path(), WriteOptions::default(), vec![]).unwrap();
    let restored = read_artifact_parquet(&written).unwrap();
    assert_eq!(
        restored
            .symbols
            .iter()
            .find(|s| s.stable_symbol_id == stable_id),
        Some(&expected)
    );
    let manifest = spur_graph::read_artifact_header_parquet(&written).unwrap();
    eprintln!(
        "G2A_OBSERVED={}",
        serde_json::json!({"symbol": expected, "schema_version": manifest.schema_version, "kind_serde": kind})
    );
    assert_eq!(spur_graph::store::SCHEMA_VERSION, "spur-graph-schema-v12");
}
