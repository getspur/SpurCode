use spur_graph::{
    load_artifact, write_artifact_parquet, Confidence, GraphEdgeArtifact, GraphEdgeKind,
    GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader,
    GraphSymbolArtifact, NodeId, RelationKind, WriteOptions,
};

const GRAPH_CONTENT_HASH: &str = "dispatch-test-graph-content-hash";

#[test]
fn load_artifact_dispatches_directory_to_parquet_reader() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let expected = fixture_artifact();
    let parquet_dir = write_artifact_parquet(
        &expected,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");

    let loaded = load_artifact(&parquet_dir).expect("load parquet artifact through dispatcher");

    assert_eq!(loaded.graph_content_hash, GRAPH_CONTENT_HASH);
    assert_eq!(loaded, expected);
}

#[test]
fn load_artifact_rejects_file_path_with_parquet_directory_error() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact_path = tempdir.path().join("graph-index.json");
    std::fs::write(&artifact_path, "{}").expect("write file artifact");

    let error = load_artifact(&artifact_path).expect_err("file path should be rejected");

    assert!(
        error
            .to_string()
            .contains("expected a Parquet artifact directory"),
        "unexpected error: {error:#}"
    );
}

fn fixture_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "spur-graph-phase2".to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "dispatch-test-manifest-version".to_owned(),
        graph_content_hash: GRAPH_CONTENT_HASH.to_owned(),
        file_manifests: vec![
            GraphFileManifestEntry {
                stable_file_id: "file-a".to_owned(),
                path: "src/a.rs".to_owned(),
                content_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                node_ids: vec![NodeId(11)],
            },
            GraphFileManifestEntry {
                stable_file_id: "file-b".to_owned(),
                path: "src/b.rs".to_owned(),
                content_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
                node_ids: vec![NodeId(21)],
            },
        ],
        files: vec![
            GraphFileArtifact {
                stable_file_id: "file-a".to_owned(),
                file_path: "src/a.rs".to_owned(),
            },
            GraphFileArtifact {
                stable_file_id: "file-b".to_owned(),
                file_path: "src/b.rs".to_owned(),
            },
        ],
        file_node_ids: vec![NodeId(10), NodeId(20)],
        symbols: vec![
            GraphSymbolArtifact {
                stable_symbol_id: "sym-a".to_owned(),
                file_path: "src/a.rs".to_owned(),
                byte_range: [1, 12],
                line_range: [1, 2],
                entity_name: "a_fn".to_owned(),
                qualified_name: "crate::a::a_fn".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-a".to_owned(),
                enclosing_scope: None,
            },
            GraphSymbolArtifact {
                stable_symbol_id: "sym-b".to_owned(),
                file_path: "src/b.rs".to_owned(),
                byte_range: [3, 18],
                line_range: [2, 4],
                entity_name: "b_fn".to_owned(),
                qualified_name: "crate::b::b_fn".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-b".to_owned(),
                enclosing_scope: Some("mod b".to_owned()),
            },
        ],
        symbol_node_ids: vec![NodeId(11), NodeId(21)],
        edges: vec![
            GraphEdgeArtifact {
                source_stable_symbol_id: "file-a".to_owned(),
                target_stable_symbol_id: Some("sym-a".to_owned()),
                target_label: Some("a_fn".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Contains,
                confidence: Confidence::SyntaxExact,
                confidence_score: 1.0,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::ReferencesOther),
                bind_method: None,
            },
            GraphEdgeArtifact {
                source_stable_symbol_id: "sym-a".to_owned(),
                target_stable_symbol_id: Some("sym-b".to_owned()),
                target_label: Some("b_fn".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Calls,
                confidence: Confidence::SyntaxExact,
                confidence_score: 0.875,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::Calls),
                bind_method: None,
            },
            GraphEdgeArtifact {
                source_stable_symbol_id: "sym-b".to_owned(),
                target_stable_symbol_id: None,
                target_label: Some("missing_fn".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Calls,
                confidence: Confidence::Heuristic,
                confidence_score: 0.5,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::CallsDyn),
                bind_method: None,
            },
        ],
        tombstones: Vec::new(),
        diagnostics: Vec::new(),

        commits: Vec::new(),

        symbol_snapshots: Vec::new(),

        temporal_edges: Vec::new(),
    }
}
