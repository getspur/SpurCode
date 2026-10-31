use spur_graph::store::ArtifactStagingDir;
use spur_graph::{
    load_artifact, read_current_pointer, write_artifact_parquet, write_current_pointer,
    CommitArtifact, EdgeEndpoint, GraphFileArtifact, GraphIndexArtifact, GraphIndexHeader, NodeId,
    RelationKind, SnapshotKey, SymbolSnapshotArtifact, TemporalEdgeArtifact, WriteOptions,
    GRAPH_INDEX_VERSION_TEMPORAL,
};

#[test]
fn diagnostics_round_trip_through_json() {
    let mut artifact = minimal_artifact();
    artifact.diagnostics = vec![
        "parse_failed path=src/lib.rs sha=abc123".to_owned(),
        "ambiguous_rename stable_symbol_id=sym-main".to_owned(),
    ];

    let encoded = serde_json::to_string(&artifact).expect("encode artifact");
    let decoded: GraphIndexArtifact = serde_json::from_str(&encoded).expect("decode artifact");

    assert_eq!(decoded.diagnostics, artifact.diagnostics);
}

#[test]
fn diagnostics_round_trip_through_artifact_io() {
    let mut artifact = minimal_artifact();
    artifact.diagnostics = vec!["parse_failed path=src/lib.rs sha=abc123".to_owned()];
    let dir = tempfile::tempdir().expect("tempdir");

    write_graph_artifact(dir.path(), &artifact);
    let current = read_current_pointer(dir.path()).expect("read CURRENT pointer");
    let decoded = load_artifact(&current).expect("load artifact");

    assert_eq!(decoded.diagnostics, artifact.diagnostics);
}

#[test]
fn temporal_collections_round_trip_through_artifact_io() {
    let artifact = temporal_artifact();
    let mut mutated = artifact.clone();
    mutated
        .symbol_snapshots
        .push(symbol_snapshot("sym-helper", "c2"));
    let original_dir = tempfile::tempdir().expect("original tempdir");
    let mutated_dir = tempfile::tempdir().expect("mutated tempdir");

    let original = write_and_load_graph_artifact(original_dir.path(), &artifact);
    let mutated = write_and_load_graph_artifact(mutated_dir.path(), &mutated);

    assert_eq!(original.symbol_snapshots, artifact.symbol_snapshots);
    assert_eq!(original.temporal_edges, artifact.temporal_edges);
    assert_eq!(mutated.symbol_snapshots.len(), 2);
    assert_eq!(mutated.temporal_edges, artifact.temporal_edges);
}

fn write_graph_artifact(worktree: &std::path::Path, artifact: &GraphIndexArtifact) {
    let artifact_base = worktree.join(".spur/graph");
    let staging =
        ArtifactStagingDir::new(&artifact_base, &artifact.graph_content_hash).expect("stage");
    write_artifact_parquet(
        artifact,
        staging.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let artifact_dir = staging.commit().expect("commit parquet artifact");
    write_current_pointer(worktree, &artifact_dir).expect("write CURRENT pointer");
}

fn write_and_load_graph_artifact(
    worktree: &std::path::Path,
    artifact: &GraphIndexArtifact,
) -> GraphIndexArtifact {
    write_graph_artifact(worktree, artifact);
    let current = read_current_pointer(worktree).expect("read CURRENT pointer");
    load_artifact(&current).expect("load artifact")
}

fn minimal_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "manifest-v1".to_owned(),
        graph_content_hash: "graph-hash-v1".to_owned(),
        file_manifests: Vec::new(),
        files: vec![GraphFileArtifact {
            stable_file_id: "file-src-lib".to_owned(),
            file_path: "src/lib.rs".to_owned(),
        }],
        file_node_ids: vec![NodeId(1)],
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    }
}

fn temporal_artifact() -> GraphIndexArtifact {
    let mut artifact = minimal_artifact();
    let snapshot = symbol_snapshot("sym-main", "c1");
    artifact.commits.push(CommitArtifact {
        sha: "c1".to_owned(),
        parents: Vec::new(),
        author_time: 1,
        author_name: String::new(),
        author_email: String::new(),
        summary: "initial import".to_owned(),
    });
    artifact.symbol_snapshots.push(snapshot.clone());
    artifact.temporal_edges.push(TemporalEdgeArtifact {
        source: EdgeEndpoint::Commit {
            sha: "c1".to_owned(),
        },
        target: EdgeEndpoint::Snapshot { key: snapshot.key },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: None,
    });
    artifact
}

fn symbol_snapshot(stable_symbol_id: &str, commit: &str) -> SymbolSnapshotArtifact {
    SymbolSnapshotArtifact {
        key: SnapshotKey {
            stable_symbol_id: stable_symbol_id.to_owned(),
            commit: commit.to_owned(),
        },
        file_path: "src/lib.rs".into(),
        entity_name: stable_symbol_id.to_owned(),
        symbol_kind: "function".to_owned(),
        enclosing_scope: None,
        byte_range: [0, 10],
        line_range: [1, 1],
        anchor_hash: format!("anchor-{stable_symbol_id}"),
        tokens: vec![stable_symbol_id.to_owned()],
    }
}
