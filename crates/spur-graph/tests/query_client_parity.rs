use std::collections::BTreeMap;
use std::sync::Arc;

use spur_graph::schema::GRAPH_INDEX_VERSION_TEMPORAL;
use spur_graph::temporal::symbol_history;
use spur_graph::{
    read_artifact_parquet, write_artifact_parquet, ChangeKind, CommitArtifact, CommitIndexArtifact,
    Confidence, EdgeEndpoint, GraphEdgeArtifact, GraphEdgeKind, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphQueryClient as _,
    GraphSymbolArtifact, InMemoryClient, NodeId, OwnedCalleeRecord, OwnedCallerRecord,
    ParquetClient, RelationKind, RenamePrev, SearchFilters, SearchMode, SearchOptions, SnapshotKey,
    SymbolSnapshotArtifact, TemporalEdgeArtifact, WalkStrategy, WriteOptions,
};

fn artifact() -> GraphIndexArtifact {
    let symbols = vec![
        symbol("qualified-match", "src/b.rs", [2, 3], "helper", "target"),
        symbol(
            "entity-match",
            "src/a.rs",
            [10, 11],
            "target",
            "module::target",
        ),
        symbol(
            "submit-plan",
            "src/lib.rs",
            [30, 31],
            "submit_plan",
            "submit_plan",
        ),
        symbol(
            "submitter",
            "src/lib.rs",
            [20, 21],
            "submitter",
            "submitter",
        ),
        symbol("submit", "src/lib.rs", [10, 11], "submit", "submit"),
        symbol(
            "alpha-def",
            "src/lib.rs",
            [40, 41],
            "alpha_def",
            "alpha_def",
        ),
        symbol("z-def-long", "src/lib.rs", [30, 31], "zdeflong", "zdeflong"),
        symbol("a-def", "src/lib.rs", [20, 21], "adef", "adef"),
        symbol("def", "src/lib.rs", [10, 11], "def", "def"),
        symbol(
            "aaaaaaaaaaaaaaaa",
            "src/hex.rs",
            [1, 2],
            "hex_target",
            "hex_target",
        ),
        symbol(
            "foo-lib",
            "crates/foo/src/lib.rs",
            [1, 2],
            "run_query",
            "run_query",
        ),
        symbol(
            "foo-nested",
            "crates/foo/src/nested/mod.rs",
            [1, 2],
            "run_query",
            "run_query",
        ),
        symbol(
            "bar-lib",
            "crates/bar/src/lib.rs",
            [1, 2],
            "run_query",
            "run_query",
        ),
        symbol("caller", "src/callers.rs", [50, 51], "caller", "caller"),
        symbol(
            "unresolved-caller",
            "src/callers.rs",
            [55, 56],
            "unresolved_caller",
            "unresolved_caller",
        ),
        symbol("target", "src/callees.rs", [60, 61], "target", "target"),
        symbol("root", "src/root.rs", [70, 71], "root", "root"),
        symbol("callee", "src/root.rs", [80, 81], "callee", "callee"),
    ];
    let symbol_node_ids = (1..=symbols.len()).map(|id| NodeId(id as u64)).collect();

    GraphIndexArtifact {
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "test".to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "test".to_owned(),
        graph_content_hash: "query-client-parity".to_owned(),
        file_manifests: file_manifests(),
        files: files(),
        file_node_ids: (101..=107).map(NodeId).collect(),
        symbols,
        symbol_node_ids,
        edges: vec![
            edge_with_evidence(
                "caller",
                Some("target"),
                Some("target"),
                Some("catalog"),
                Some("Catalog"),
            ),
            edge("unresolved-caller", None, Some("target")),
            edge_with_evidence(
                "root",
                Some("callee"),
                Some("callee"),
                Some("service"),
                Some("Service"),
            ),
            edge("root", None, Some("external_call")),
        ],
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    }
}

fn files() -> Vec<GraphFileArtifact> {
    [
        "src/a.rs",
        "src/b.rs",
        "src/lib.rs",
        "crates/foo/src/lib.rs",
        "crates/foo/src/nested/mod.rs",
        "crates/bar/src/lib.rs",
        "src/hex.rs",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, path)| GraphFileArtifact {
        stable_file_id: format!("file-{index}"),
        file_path: path.to_owned(),
    })
    .collect()
}

fn file_manifests() -> Vec<GraphFileManifestEntry> {
    files()
        .into_iter()
        .enumerate()
        .map(|(index, file)| GraphFileManifestEntry {
            stable_file_id: file.stable_file_id,
            path: file.file_path,
            content_oid: format!("{:040x}", index + 1),
            node_ids: vec![NodeId((index as u64) + 1), NodeId((index as u64) + 50)],
        })
        .collect()
}

fn symbol(
    id: &str,
    file_path: &str,
    line_range: [usize; 2],
    entity_name: &str,
    qualified_name: &str,
) -> GraphSymbolArtifact {
    GraphSymbolArtifact {
        stable_symbol_id: id.to_owned(),
        file_path: file_path.to_owned(),
        byte_range: [0, 8],
        line_range,
        entity_name: entity_name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        symbol_kind: "function".to_owned(),
        anchor_hash: format!("hash-{id}"),
        enclosing_scope: None,
    }
}

fn edge(source: &str, target: Option<&str>, target_label: Option<&str>) -> GraphEdgeArtifact {
    edge_with_evidence(source, target, target_label, None, None)
}

fn edge_with_evidence(
    source: &str,
    target: Option<&str>,
    target_label: Option<&str>,
    receiver_text: Option<&str>,
    scope_text: Option<&str>,
) -> GraphEdgeArtifact {
    GraphEdgeArtifact {
        source_stable_symbol_id: source.to_owned(),
        target_stable_symbol_id: target.map(str::to_string),
        target_label: target_label.map(str::to_string),
        import_path: None,
        receiver_text: receiver_text.map(str::to_string),
        scope_text: scope_text.map(str::to_string),
        relation: RelationKind::Calls,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::Calls),
        bind_method: None,
    }
}

fn temporal_artifact() -> GraphIndexArtifact {
    let old_snapshot = snapshot("old-root", "commit-a", "foo");
    let new_snapshot = snapshot("new-root", "commit-b", "bar");
    let mut artifact = artifact();
    artifact.header.graph_index_version = GRAPH_INDEX_VERSION_TEMPORAL.to_owned();
    artifact.commits = commits();
    artifact.symbol_snapshots = vec![old_snapshot.clone(), new_snapshot.clone()];
    artifact.temporal_edges = vec![
        temporal_touch("commit-a", old_snapshot.key.clone(), ChangeKind::Added),
        temporal_touch(
            "commit-b",
            new_snapshot.key.clone(),
            ChangeKind::RenamedFrom(RenamePrev::Symbol(old_snapshot.key.clone())),
        ),
        temporal_rename(old_snapshot.key, new_snapshot.key),
    ];
    artifact
}

fn commits() -> Vec<CommitArtifact> {
    vec![
        CommitArtifact {
            sha: "commit-a".to_owned(),
            parents: Vec::new(),
            author_time: 1,
            author_name: String::new(),
            author_email: String::new(),
            summary: "add foo".to_owned(),
        },
        CommitArtifact {
            sha: "commit-b".to_owned(),
            parents: vec!["commit-a".to_owned()],
            author_time: 2,
            author_name: String::new(),
            author_email: String::new(),
            summary: "rename foo to bar".to_owned(),
        },
    ]
}

fn commit_index(commits: Vec<CommitArtifact>) -> CommitIndexArtifact {
    CommitIndexArtifact {
        schema_version: 7,
        commits,
        refs: BTreeMap::from([("HEAD".to_owned(), "commit-b".to_owned())]),
        indexed_at: "2026-05-26T00:00:00Z".to_owned(),
        walk_strategy: WalkStrategy::Reachable,
    }
}

fn snapshot(id: &str, commit: &str, entity_name: &str) -> SymbolSnapshotArtifact {
    SymbolSnapshotArtifact {
        key: SnapshotKey {
            stable_symbol_id: id.to_owned(),
            commit: commit.to_owned(),
        },
        file_path: "src/temporal.rs".to_owned().into(),
        entity_name: entity_name.to_owned(),
        symbol_kind: "function".to_owned(),
        enclosing_scope: None,
        byte_range: [0, 8],
        line_range: [1, 2],
        anchor_hash: format!("hash-{id}-{commit}"),
        tokens: vec![entity_name.to_owned()],
    }
}

fn temporal_touch(commit: &str, key: SnapshotKey, change_kind: ChangeKind) -> TemporalEdgeArtifact {
    TemporalEdgeArtifact {
        source: EdgeEndpoint::Commit {
            sha: commit.to_owned(),
        },
        target: EdgeEndpoint::Snapshot { key },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(change_kind),
    }
}

fn temporal_rename(from: SnapshotKey, to: SnapshotKey) -> TemporalEdgeArtifact {
    TemporalEdgeArtifact {
        source: EdgeEndpoint::Snapshot { key: from.clone() },
        target: EdgeEndpoint::Snapshot { key: to },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(from))),
    }
}

fn options(query: &str, mode: SearchMode) -> SearchOptions {
    SearchOptions {
        query: query.to_owned(),
        mode,
        filters: SearchFilters::default(),
        limit: 200,
    }
}

#[test]
fn parquet_client_search_symbols_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    for options in [
        options("target", SearchMode::Exact),
        options("sub", SearchMode::Prefix),
        options("def", SearchMode::Substring),
        SearchOptions {
            query: "run".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters {
                file_glob: Some("crates/foo/**/*.rs".to_owned()),
                ..SearchFilters::default()
            },
            limit: 200,
        },
    ] {
        let expected = in_memory
            .search_symbols(&options)
            .expect("in-memory search succeeds");
        let actual = parquet
            .search_symbols(&options)
            .expect("parquet search succeeds");

        assert_eq!(actual, expected, "options: {options:?}");
    }
}

#[test]
fn parquet_client_symbols_by_files_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    let paths = [
        "src/lib.rs".to_owned(),
        "src/a.rs".to_owned(),
        "src/missing.rs".to_owned(),
    ];
    let normalize = |mut symbols: Vec<GraphSymbolArtifact>| {
        symbols.sort_by(|left, right| left.stable_symbol_id.cmp(&right.stable_symbol_id));
        symbols
    };

    let expected = normalize(
        in_memory
            .symbols_by_files(&paths)
            .expect("in-memory symbols by files"),
    );
    let actual = normalize(
        parquet
            .symbols_by_files(&paths)
            .expect("parquet symbols by files"),
    );

    assert!(!expected.is_empty(), "fixture should cover queried paths");
    assert_eq!(actual, expected);
    assert!(parquet
        .symbols_by_files(&[])
        .expect("empty path set")
        .is_empty());
}

#[test]
fn parquet_client_find_caller_edges_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    let expected = caller_records(in_memory.find_caller_edges("target"));
    let actual = caller_records(parquet.find_caller_edges("target"));

    assert_eq!(actual, expected);
    assert!(
        actual.iter().any(|record| record
            == &(
                "caller".to_owned(),
                "caller".to_owned(),
                true,
                Some("target".to_owned()),
                Some("catalog".to_owned()),
                Some("Catalog".to_owned()),
            )),
        "Parquet caller query should return non-empty receiver/scope evidence"
    );
}

#[test]
fn parquet_client_find_callee_edges_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    let expected = callee_records(in_memory.find_callee_edges("root"));
    let actual = callee_records(parquet.find_callee_edges("root"));

    assert_eq!(actual, expected);
    assert!(
        actual.iter().any(|record| record
            == &(
                "callee".to_owned(),
                "root".to_owned(),
                true,
                Some("callee".to_owned()),
                Some("service".to_owned()),
                Some("Service".to_owned()),
            )),
        "Parquet callee query should return non-empty receiver/scope evidence"
    );
}

#[test]
fn parquet_client_resolve_selector_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    for selector in [
        "graph://symbol/entity-match",
        "aaaaaaaaaaaaaaaa",
        "module::target",
        "src/a.rs::module::target",
        "submit_plan",
        "run_query",
    ] {
        assert_eq!(
            parquet
                .resolve_selector(selector)
                .expect("parquet resolve selector succeeds"),
            in_memory
                .resolve_selector(selector)
                .expect("in-memory resolve selector succeeds"),
            "selector: {selector}"
        );
    }
}

#[test]
fn parquet_client_file_manifest_by_path_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact();
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(artifact));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    let expected = in_memory
        .file_manifest_by_path("src/a.rs")
        .expect("in-memory manifest query succeeds");
    let actual = parquet
        .file_manifest_by_path("src/a.rs")
        .expect("parquet manifest query succeeds");

    assert_eq!(actual, expected);
    assert_eq!(
        actual.expect("manifest is present").node_ids,
        vec![NodeId(1), NodeId(50)]
    );
    assert_eq!(
        parquet
            .file_manifest_by_path("src/missing.rs")
            .expect("missing manifest query succeeds"),
        None
    );
}

#[test]
fn parquet_client_temporal_index_matches_in_memory_client() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = temporal_artifact();
    let commits = commit_index(artifact.commits.clone());
    let parquet_dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let in_memory = InMemoryClient::new(Arc::new(
        read_artifact_parquet(&parquet_dir).expect("read full parquet artifact"),
    ));
    let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

    let expected_index = in_memory.temporal_index();
    let actual_index = parquet.temporal_index();
    let cached_index = parquet.temporal_index();

    assert!(Arc::ptr_eq(&actual_index, &cached_index));
    assert_eq!(
        actual_index
            .edges_for_stable_symbol_id("old-root")
            .cloned()
            .collect::<Vec<_>>(),
        expected_index
            .edges_for_stable_symbol_id("old-root")
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        actual_index
            .edges_for_commit_sha("commit-b")
            .cloned()
            .collect::<Vec<_>>(),
        expected_index
            .edges_for_commit_sha("commit-b")
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        symbol_history(actual_index.as_ref(), &commits, "new-root"),
        symbol_history(expected_index.as_ref(), &commits, "new-root")
    );
}

fn caller_records(
    records: Vec<OwnedCallerRecord>,
) -> Vec<(
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    records
        .into_iter()
        .map(|record| match record {
            OwnedCallerRecord::Resolved { caller, edge } => (
                caller.stable_symbol_id.clone(),
                edge.source_stable_symbol_id.clone(),
                true,
                edge.target_label.clone(),
                edge.receiver_text.clone(),
                edge.scope_text.clone(),
            ),
            OwnedCallerRecord::Unresolved {
                caller,
                edge,
                target_label,
            } => (
                caller.stable_symbol_id.clone(),
                edge.source_stable_symbol_id.clone(),
                false,
                Some(target_label),
                edge.receiver_text.clone(),
                edge.scope_text.clone(),
            ),
        })
        .collect()
}

fn callee_records(
    records: Vec<OwnedCalleeRecord>,
) -> Vec<(
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    records
        .into_iter()
        .map(|record| match record {
            OwnedCalleeRecord::Resolved { symbol, edge } => (
                symbol.stable_symbol_id.clone(),
                edge.source_stable_symbol_id.clone(),
                true,
                edge.target_label.clone(),
                edge.receiver_text.clone(),
                edge.scope_text.clone(),
            ),
            OwnedCalleeRecord::Unresolved { edge, target_label } => (
                target_label.clone(),
                edge.source_stable_symbol_id.clone(),
                false,
                Some(target_label),
                edge.receiver_text.clone(),
                edge.scope_text.clone(),
            ),
        })
        .collect()
}
