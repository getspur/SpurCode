use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use spur_graph::{
    CodeSelectorResolution, Confidence, GraphEdgeArtifact, GraphEdgeKind, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphQueryClient,
    GraphSymbolArtifact, InMemoryClient, NodeId, OverlayClient, OverlayGeneration,
    OverlayGenerationIdentity, OverlayGenerationQueryMeasurements, OverlayPathState,
    OwnedCalleeRecord, OwnedCallerRecord, RelationKind, SearchFilters, SearchMode, SearchOptions,
    SearchResult,
};

use spur_graph::temporal::TemporalIndex;

#[test]
fn overlay_generation_matches_fresh_oracle_across_full_path_state_lifecycle() {
    let base = Arc::new(generation_base_artifact());
    let seed = Arc::new(OverlayGeneration::seed(Arc::clone(&base)).expect("seed generation"));
    let unchanged_seed = seed
        .file_segment("src/unchanged.rs")
        .expect("unchanged seed segment");
    let restored_seed = seed
        .file_segment("src/changed.rs")
        .expect("restorable seed segment");

    let delta_v1 = Arc::new(generation_delta_v1());
    let path_state_v1 = BTreeMap::from([
        (
            "src/changed.rs".to_owned(),
            OverlayPathState::Tracked("oid-changed-v1".to_owned()),
        ),
        ("src/deleted.rs".to_owned(), OverlayPathState::Deleted),
        ("src/rename_old.rs".to_owned(), OverlayPathState::Deleted),
        (
            "src/rename_new.rs".to_owned(),
            OverlayPathState::Untracked("oid-rename-v1".to_owned()),
        ),
        (
            "src/untracked.rs".to_owned(),
            OverlayPathState::Untracked("oid-untracked-v1".to_owned()),
        ),
    ]);
    let generation_v1 = Arc::new(
        OverlayGeneration::update(
            &seed,
            generation_identity("generation-v1", 1),
            &path_state_v1,
            Arc::clone(&delta_v1),
        )
        .expect("generation v1"),
    );
    let oracle_v1 = InMemoryClient::new(Arc::new(generation_oracle_v1()));

    assert_generation_matches_oracle(&generation_v1, &oracle_v1, "v1");
    assert!(Arc::ptr_eq(
        &unchanged_seed,
        &generation_v1
            .file_segment("src/unchanged.rs")
            .expect("unchanged v1 segment")
    ));
    assert_eq!(
        generation_v1.rebuilt_paths(),
        &BTreeSet::from([
            "src/changed.rs".to_owned(),
            "src/deleted.rs".to_owned(),
            "src/rename_new.rs".to_owned(),
            "src/rename_old.rs".to_owned(),
            "src/untracked.rs".to_owned(),
        ])
    );
    assert_eq!(
        generation_v1.rewritten_query_paths(),
        generation_v1.rebuilt_paths(),
        "v1 rewrites only its changed-path query slots"
    );
    assert!(
        generation_v1.shared_query_chunk_count(&seed) > 0,
        "v1 must structurally share at least one unchanged visible-query chunk"
    );
    println!(
        "overlay_generation_storage state=v1 rebuilt_paths={} rewritten_query_paths={} shared_query_chunks={} query_chunks={} unchanged_segment_reused=true",
        generation_v1.rebuilt_paths().len(),
        generation_v1.rewritten_query_paths().len(),
        generation_v1.shared_query_chunk_count(&seed),
        generation_v1.query_chunk_count(),
    );

    let renamed_v1 = generation_v1
        .file_segment("src/rename_new.rs")
        .expect("renamed v1 segment");
    let untracked_v1 = generation_v1
        .file_segment("src/untracked.rs")
        .expect("untracked v1 segment");
    let path_state_v2 = BTreeMap::from([
        ("src/deleted.rs".to_owned(), OverlayPathState::Deleted),
        ("src/rename_old.rs".to_owned(), OverlayPathState::Deleted),
        (
            "src/rename_new.rs".to_owned(),
            OverlayPathState::Untracked("oid-rename-v1".to_owned()),
        ),
        (
            "src/untracked.rs".to_owned(),
            OverlayPathState::Untracked("oid-untracked-v1".to_owned()),
        ),
    ]);
    let generation_v2 = Arc::new(
        OverlayGeneration::update(
            &generation_v1,
            generation_identity("generation-v2", 2),
            &path_state_v2,
            Arc::new(generation_delta_v2()),
        )
        .expect("generation v2"),
    );
    let oracle_v2 = InMemoryClient::new(Arc::new(generation_oracle_v2()));

    assert_generation_matches_oracle(&generation_v2, &oracle_v2, "v2");
    assert!(Arc::ptr_eq(
        &unchanged_seed,
        &generation_v2
            .file_segment("src/unchanged.rs")
            .expect("unchanged v2 segment")
    ));
    assert!(Arc::ptr_eq(
        &restored_seed,
        &generation_v2
            .file_segment("src/changed.rs")
            .expect("restored base segment")
    ));
    assert!(Arc::ptr_eq(
        &renamed_v1,
        &generation_v2
            .file_segment("src/rename_new.rs")
            .expect("renamed v2 segment")
    ));
    assert!(Arc::ptr_eq(
        &untracked_v1,
        &generation_v2
            .file_segment("src/untracked.rs")
            .expect("untracked v2 segment")
    ));
    assert_eq!(
        generation_v2.rebuilt_paths(),
        &BTreeSet::from(["src/changed.rs".to_owned()])
    );
    assert_eq!(
        generation_v2.rewritten_query_paths(),
        generation_v2.rebuilt_paths(),
        "v2 rewrites only the restored-path query slot"
    );
    assert!(
        generation_v2.shared_query_chunk_count(&generation_v1) > 0,
        "v2 must structurally share unchanged visible-query chunks"
    );
    println!(
        "overlay_generation_storage state=v2 rebuilt_paths={} rewritten_query_paths={} shared_query_chunks={} query_chunks={} reused_segments=4",
        generation_v2.rebuilt_paths().len(),
        generation_v2.rewritten_query_paths().len(),
        generation_v2.shared_query_chunk_count(&generation_v1),
        generation_v2.query_chunk_count(),
    );
}

#[test]
fn overlay_generation_resolves_stable_id_collision_once_during_update() {
    let base = Arc::new(artifact(
        "collision-base",
        vec![file("src/changed.rs"), file("src/unchanged.rs")],
        vec![
            symbol(
                "collision-id",
                "src/unchanged.rs",
                [1, 2],
                "old_collision",
                "old_collision",
            ),
            symbol(
                "changed-base-id",
                "src/changed.rs",
                [4, 5],
                "changed_base",
                "changed_base",
            ),
        ],
        vec![],
    ));
    let seed = Arc::new(OverlayGeneration::seed(base).expect("seed generation"));
    let delta = Arc::new(artifact(
        "collision-delta",
        vec![file("src/changed.rs")],
        vec![symbol(
            "collision-id",
            "src/changed.rs",
            [8, 9],
            "new_collision",
            "new_collision",
        )],
        vec![],
    ));
    let mut identity = generation_identity("collision", 3);
    identity.indexed_graph_content_hash = "collision-base".to_owned();
    let generation = OverlayGeneration::update(
        &seed,
        identity,
        &BTreeMap::from([(
            "src/changed.rs".to_owned(),
            OverlayPathState::Tracked("oid-collision".to_owned()),
        )]),
        delta,
    )
    .expect("collision generation");
    let oracle = InMemoryClient::new(Arc::new(artifact(
        "collision-oracle",
        vec![file("src/changed.rs"), file("src/unchanged.rs")],
        vec![symbol(
            "collision-id",
            "src/changed.rs",
            [8, 9],
            "new_collision",
            "new_collision",
        )],
        vec![],
    )));

    assert_generation_matches_oracle(&generation, &oracle, "collision");
    assert_eq!(
        generation
            .symbol_by_id("collision-id")
            .expect("generation symbol"),
        oracle.symbol_by_id("collision-id").expect("oracle symbol")
    );
    assert_eq!(
        generation
            .search_symbols(&options("collision", SearchMode::Substring))
            .expect("collision search")
            .total_matches,
        1
    );
    assert_eq!(
        generation.rewritten_query_paths(),
        &BTreeSet::from(["src/changed.rs".to_owned(), "src/unchanged.rs".to_owned(),]),
        "stable-ID collision resolution rewrites only the changed slot and displaced-owner closure"
    );
}

#[test]
fn overlay_generation_warm_search_performs_no_overlay_finalization() {
    let base = Arc::new(generation_base_artifact());
    let seed = Arc::new(OverlayGeneration::seed(base).expect("seed generation"));
    let generation = OverlayGeneration::update(
        &seed,
        generation_identity("measurement", 4),
        &BTreeMap::from([
            (
                "src/changed.rs".to_owned(),
                OverlayPathState::Tracked("oid-changed-v1".to_owned()),
            ),
            ("src/deleted.rs".to_owned(), OverlayPathState::Deleted),
            ("src/rename_old.rs".to_owned(), OverlayPathState::Deleted),
            (
                "src/rename_new.rs".to_owned(),
                OverlayPathState::Untracked("oid-rename-v1".to_owned()),
            ),
            (
                "src/untracked.rs".to_owned(),
                OverlayPathState::Untracked("oid-untracked-v1".to_owned()),
            ),
        ]),
        Arc::new(generation_delta_v1()),
    )
    .expect("measured generation");
    let mut measurements = OverlayGenerationQueryMeasurements::default();

    generation
        .search_symbols_with_measurements(
            &options("target", SearchMode::Substring),
            &mut measurements,
        )
        .expect("measured warm search");

    assert_eq!(
        measurements,
        OverlayGenerationQueryMeasurements::default(),
        "warm generation search must only match, filter, score, sort, and select top-k"
    );
    println!(
        "overlay_generation_query_stages path_visibility_filters={} result_layer_merges={} stable_id_dedup_checks={} total={}",
        measurements.path_visibility_filters,
        measurements.result_layer_merges,
        measurements.stable_id_dedup_checks,
        measurements.total(),
    );
}

#[test]
fn overlay_generation_adjacency_matches_fresh_oracle_and_reuses_unaffected_segments() {
    let base = Arc::new(adjacency_generation_base_artifact());
    let seed = Arc::new(OverlayGeneration::seed(base).expect("seed adjacency generation"));
    let stable_caller_seed = seed
        .adjacency_segment("stable-caller-id")
        .expect("stable caller seed adjacency");
    let stable_isolated_seed = seed
        .adjacency_segment("stable-isolated-id")
        .expect("stable isolated seed adjacency");
    let path_state = BTreeMap::from([
        (
            "src/changed.rs".to_owned(),
            OverlayPathState::Tracked("oid-adjacency-changed".to_owned()),
        ),
        ("src/deleted.rs".to_owned(), OverlayPathState::Deleted),
        ("src/rename_old.rs".to_owned(), OverlayPathState::Deleted),
        (
            "src/rename_new.rs".to_owned(),
            OverlayPathState::Untracked("oid-adjacency-rename".to_owned()),
        ),
        (
            "src/added.rs".to_owned(),
            OverlayPathState::Untracked("oid-adjacency-added".to_owned()),
        ),
    ]);
    let generation = Arc::new(
        OverlayGeneration::update(
            &seed,
            adjacency_generation_identity(),
            &path_state,
            Arc::new(adjacency_generation_delta_artifact()),
        )
        .expect("updated adjacency generation"),
    );
    let oracle = InMemoryClient::new(Arc::new(adjacency_generation_oracle_artifact()));

    assert_generation_graph_matches_oracle(&generation, &oracle, "adjacency-v1");
    assert!(Arc::ptr_eq(
        &stable_caller_seed,
        &generation
            .adjacency_segment("stable-caller-id")
            .expect("stable caller generation adjacency")
    ));
    assert!(Arc::ptr_eq(
        &stable_isolated_seed,
        &generation
            .adjacency_segment("stable-isolated-id")
            .expect("stable isolated generation adjacency")
    ));
    assert!(
        !generation
            .rebuilt_adjacency_symbols()
            .contains("stable-caller-id")
            && !generation
                .rebuilt_adjacency_symbols()
                .contains("stable-isolated-id"),
        "unaffected adjacency must remain outside the changed-endpoint dependency closure"
    );
    println!(
        "overlay_generation_adjacency_reuse rebuilt={} stable_caller_reused=true stable_isolated_reused=true",
        generation.rebuilt_adjacency_symbols().len()
    );
}

fn assert_generation_graph_matches_oracle(
    generation: &Arc<OverlayGeneration>,
    oracle: &impl GraphQueryClient,
    label: &str,
) {
    let generation_client: &dyn GraphQueryClient = generation;
    let symbol_ids = [
        "old-changed-id",
        "new-changed-id",
        "new-caller-id",
        "unchanged-caller-id",
        "unchanged-target-id",
        "deleted-id",
        "rename-old-id",
        "rename-new-id",
        "added-id",
        "collision-id",
        "stable-caller-id",
        "stable-target-id",
        "stable-isolated-id",
    ];
    let selectors = [
        "target",
        "new_caller",
        "unchanged_target",
        "deleted_target",
        "renamed_target",
        "added_target",
        "new_collision",
        "src/changed.rs::target",
        "src/rename_new.rs::renamed_target",
        "graph://symbol/new-changed-id",
        "graph://symbol/deleted-id",
        "graph://symbol/collision-id",
    ];
    let paths = [
        "src/changed.rs",
        "src/unchanged.rs",
        "src/deleted.rs",
        "src/rename_old.rs",
        "src/rename_new.rs",
        "src/added.rs",
        "src/collision_base.rs",
        "src/stable.rs",
    ];
    let mut generation_rows = Vec::new();
    let mut oracle_rows = Vec::new();

    for sid in symbol_ids {
        let actual_symbol = generation_client
            .symbol_by_id(sid)
            .expect("generation symbol by id");
        let expected_symbol = oracle.symbol_by_id(sid).expect("oracle symbol by id");
        assert_eq!(
            actual_symbol, expected_symbol,
            "{label} symbol_by_id({sid})"
        );

        let actual_callers = caller_records(generation_client.find_caller_edges(sid));
        let expected_callers = caller_records(oracle.find_caller_edges(sid));
        assert_eq!(actual_callers, expected_callers, "{label} callers({sid})");

        let actual_callees = callee_records(generation_client.find_callee_edges(sid));
        let expected_callees = callee_records(oracle.find_callee_edges(sid));
        assert_eq!(actual_callees, expected_callees, "{label} callees({sid})");

        generation_rows.push(format!(
            "symbol:{sid}:{actual_symbol:?}:callers:{actual_callers:?}:callees:{actual_callees:?}"
        ));
        oracle_rows.push(format!(
            "symbol:{sid}:{expected_symbol:?}:callers:{expected_callers:?}:callees:{expected_callees:?}"
        ));
    }

    for selector in selectors {
        let actual = generation_client
            .resolve_selector(selector)
            .expect("generation selector");
        let expected = oracle.resolve_selector(selector).expect("oracle selector");
        assert_eq!(actual, expected, "{label} resolve_selector({selector})");
        generation_rows.push(format!("selector:{selector}:{actual:?}"));
        oracle_rows.push(format!("selector:{selector}:{expected:?}"));
    }

    for path in paths {
        let actual_symbols = generation_client
            .symbols_by_file(path)
            .expect("generation listed file symbols");
        let expected_symbols = oracle
            .symbols_by_file(path)
            .expect("oracle listed file symbols");
        let actual_manifest = generation_client
            .file_manifest_by_path(path)
            .expect("generation file manifest");
        let expected_manifest = oracle
            .file_manifest_by_path(path)
            .expect("oracle file manifest");
        let actual_exists = generation_client
            .file_exists(path)
            .expect("generation file exists");
        let expected_exists = oracle.file_exists(path).expect("oracle file exists");
        assert_eq!(
            actual_symbols, expected_symbols,
            "{label} list file symbols({path})"
        );
        assert_eq!(
            actual_manifest, expected_manifest,
            "{label} list file manifest({path})"
        );
        assert_eq!(
            actual_exists, expected_exists,
            "{label} file_exists({path})"
        );
        generation_rows.push(format!(
            "file:{path}:{actual_exists}:{actual_manifest:?}:{actual_symbols:?}"
        ));
        oracle_rows.push(format!(
            "file:{path}:{expected_exists}:{expected_manifest:?}:{expected_symbols:?}"
        ));
    }

    let actual_nested = nested_subgraph_rows(generation_client, "unchanged-caller-id", 2);
    let expected_nested = nested_subgraph_rows(oracle, "unchanged-caller-id", 2);
    assert_eq!(
        actual_nested, expected_nested,
        "{label} nested subgraph must remain on one generation"
    );
    generation_rows.push(format!("nested:{actual_nested:?}"));
    oracle_rows.push(format!("nested:{expected_nested:?}"));

    let generation_digest = blake3::hash(generation_rows.join("\n").as_bytes()).to_hex();
    let oracle_digest = blake3::hash(oracle_rows.join("\n").as_bytes()).to_hex();
    println!(
        "overlay_generation_graph_oracle_digest state={label} generation={generation_digest} oracle={oracle_digest}"
    );
    assert_eq!(
        generation_digest, oracle_digest,
        "{label} graph oracle digest"
    );
}

fn nested_subgraph_rows(
    client: &dyn GraphQueryClient,
    root: &str,
    radius: usize,
) -> Vec<(usize, String, String, Option<String>, bool)> {
    let mut frontier = BTreeSet::from([root.to_owned()]);
    let mut visited = BTreeSet::new();
    let mut rows = Vec::new();
    for depth in 0..=radius {
        let current = std::mem::take(&mut frontier);
        for sid in current {
            if !visited.insert(sid.clone()) {
                continue;
            }
            for record in client.find_callee_edges(&sid) {
                match record {
                    OwnedCalleeRecord::Resolved { symbol, edge } => {
                        rows.push((
                            depth,
                            edge.source_stable_symbol_id,
                            symbol.stable_symbol_id.clone(),
                            edge.target_stable_symbol_id,
                            true,
                        ));
                        if depth < radius {
                            frontier.insert(symbol.stable_symbol_id);
                        }
                    }
                    OwnedCalleeRecord::Unresolved { edge, target_label } => rows.push((
                        depth,
                        edge.source_stable_symbol_id,
                        target_label,
                        edge.target_stable_symbol_id,
                        false,
                    )),
                }
            }
        }
    }
    rows.sort();
    rows
}

fn assert_generation_matches_oracle(
    generation: &OverlayGeneration,
    oracle: &impl GraphQueryClient,
    label: &str,
) {
    let search_options = [
        options("base_target", SearchMode::Exact),
        options("modified_target", SearchMode::Exact),
        options("unchanged", SearchMode::Substring),
        options("renamed_target", SearchMode::Exact),
        options("untracked_target", SearchMode::Exact),
        options("collision", SearchMode::Substring),
        SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters {
                file_glob: Some("src/*.rs".to_owned()),
                ..SearchFilters::default()
            },
            limit: 2,
        },
    ];
    let selectors = [
        "base-target-id",
        "modified-target-id",
        "unchanged-id",
        "deleted-id",
        "rename-id",
        "untracked-id",
        "collision-id",
        "base_target",
        "modified_target",
        "renamed_target",
        "src/changed.rs::base_target",
        "src/changed.rs::modified_target",
        "src/rename_new.rs::renamed_target",
        "graph://symbol/rename-id",
    ];

    let mut generation_rows = Vec::new();
    let mut oracle_rows = Vec::new();
    for search in &search_options {
        let actual = generation
            .search_symbols(search)
            .expect("generation search");
        let expected = oracle.search_symbols(search).expect("oracle search");
        assert_eq!(actual, expected, "{label} search options: {search:?}");
        generation_rows.push(format!("search:{search:?}:{actual:?}"));
        oracle_rows.push(format!("search:{search:?}:{expected:?}"));
    }
    for selector in selectors {
        let actual = generation
            .resolve_selector(selector)
            .expect("generation selector");
        let expected = oracle.resolve_selector(selector).expect("oracle selector");
        assert_eq!(actual, expected, "{label} selector: {selector}");
        generation_rows.push(format!("selector:{selector}:{actual:?}"));
        oracle_rows.push(format!("selector:{selector}:{expected:?}"));
    }
    for path in [
        "src/changed.rs",
        "src/unchanged.rs",
        "src/deleted.rs",
        "src/rename_old.rs",
        "src/rename_new.rs",
        "src/untracked.rs",
    ] {
        let actual = generation
            .symbols_by_file(path)
            .expect("generation file symbols");
        let expected = oracle.symbols_by_file(path).expect("oracle file symbols");
        assert_eq!(actual, expected, "{label} symbols_by_file({path})");
        assert_eq!(
            generation
                .file_manifest_by_path(path)
                .expect("generation manifest"),
            oracle.file_manifest_by_path(path).expect("oracle manifest"),
            "{label} file_manifest_by_path({path})"
        );
        assert_eq!(
            generation
                .file_exists(path)
                .expect("generation file exists"),
            oracle.file_exists(path).expect("oracle file exists"),
            "{label} file_exists({path})"
        );
    }

    let generation_digest = blake3::hash(generation_rows.join("\n").as_bytes()).to_hex();
    let oracle_digest = blake3::hash(oracle_rows.join("\n").as_bytes()).to_hex();
    println!(
        "overlay_generation_oracle_digest state={label} generation={generation_digest} oracle={oracle_digest}"
    );
    assert_eq!(generation_digest, oracle_digest, "{label} oracle digest");
}

fn generation_identity(label: &str, fingerprint_byte: u8) -> OverlayGenerationIdentity {
    OverlayGenerationIdentity {
        canonical_worktree: PathBuf::from("/test/worktree"),
        indexed_graph_content_hash: "generation-base".to_owned(),
        indexed_head_oid: Some("indexed-head".to_owned()),
        current_head_oid: label.to_owned(),
        index_identity: format!("index-{label}"),
        normalized_changed_set_fingerprint: [fingerprint_byte; 32],
    }
}

fn adjacency_generation_identity() -> OverlayGenerationIdentity {
    OverlayGenerationIdentity {
        canonical_worktree: PathBuf::from("/test/worktree"),
        indexed_graph_content_hash: "adjacency-generation-base".to_owned(),
        indexed_head_oid: Some("indexed-head".to_owned()),
        current_head_oid: "adjacency-generation-v1".to_owned(),
        index_identity: "adjacency-index-v1".to_owned(),
        normalized_changed_set_fingerprint: [9; 32],
    }
}

fn adjacency_generation_base_artifact() -> GraphIndexArtifact {
    artifact(
        "adjacency-generation-base",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
            file("src/rename_old.rs"),
            file("src/collision_base.rs"),
            file("src/stable.rs"),
        ],
        vec![
            symbol(
                "old-changed-id",
                "src/changed.rs",
                [1, 4],
                "target",
                "target",
            ),
            symbol(
                "unchanged-caller-id",
                "src/unchanged.rs",
                [10, 18],
                "unchanged_caller",
                "unchanged_caller",
            ),
            symbol(
                "unchanged-target-id",
                "src/unchanged.rs",
                [20, 24],
                "unchanged_target",
                "unchanged_target",
            ),
            symbol(
                "deleted-id",
                "src/deleted.rs",
                [30, 34],
                "deleted_target",
                "deleted_target",
            ),
            symbol(
                "rename-old-id",
                "src/rename_old.rs",
                [40, 44],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "collision-id",
                "src/collision_base.rs",
                [50, 54],
                "old_collision",
                "old_collision",
            ),
            symbol(
                "stable-caller-id",
                "src/stable.rs",
                [60, 64],
                "stable_caller",
                "stable_caller",
            ),
            symbol(
                "stable-target-id",
                "src/stable.rs",
                [70, 74],
                "stable_target",
                "stable_target",
            ),
            symbol(
                "stable-isolated-id",
                "src/stable.rs",
                [76, 78],
                "stable_isolated",
                "stable_isolated",
            ),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("old-changed-id"),
                Some("target"),
            ),
            edge(
                "unchanged-caller-id",
                Some("deleted-id"),
                Some("deleted_target"),
            ),
            edge(
                "unchanged-caller-id",
                Some("rename-old-id"),
                Some("renamed_target"),
            ),
            edge("unchanged-caller-id", None, Some("added_target")),
            edge(
                "collision-id",
                Some("stable-target-id"),
                Some("stable_target"),
            ),
            edge(
                "stable-caller-id",
                Some("stable-target-id"),
                Some("stable_target"),
            ),
        ],
    )
}

fn adjacency_generation_delta_artifact() -> GraphIndexArtifact {
    artifact(
        "adjacency-generation-delta",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/added.rs"),
            file("src/collision_base.rs"),
            file("src/stable.rs"),
        ],
        vec![
            symbol(
                "new-changed-id",
                "src/changed.rs",
                [1, 5],
                "target",
                "target",
            ),
            symbol(
                "new-caller-id",
                "src/changed.rs",
                [7, 11],
                "new_caller",
                "new_caller",
            ),
            symbol(
                "collision-id",
                "src/changed.rs",
                [13, 17],
                "new_collision",
                "new_collision",
            ),
            symbol(
                "rename-new-id",
                "src/rename_new.rs",
                [40, 45],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "added-id",
                "src/added.rs",
                [80, 84],
                "added_target",
                "added_target",
            ),
        ],
        vec![
            edge("new-changed-id", None, Some("added_target")),
            edge("new-caller-id", None, Some("unchanged_target")),
            edge("collision-id", None, Some("added_target")),
        ],
    )
}

fn adjacency_generation_oracle_artifact() -> GraphIndexArtifact {
    artifact(
        "adjacency-generation-oracle",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/added.rs"),
            file("src/collision_base.rs"),
            file("src/stable.rs"),
        ],
        vec![
            symbol(
                "new-changed-id",
                "src/changed.rs",
                [1, 5],
                "target",
                "target",
            ),
            symbol(
                "new-caller-id",
                "src/changed.rs",
                [7, 11],
                "new_caller",
                "new_caller",
            ),
            symbol(
                "collision-id",
                "src/changed.rs",
                [13, 17],
                "new_collision",
                "new_collision",
            ),
            symbol(
                "unchanged-caller-id",
                "src/unchanged.rs",
                [10, 18],
                "unchanged_caller",
                "unchanged_caller",
            ),
            symbol(
                "unchanged-target-id",
                "src/unchanged.rs",
                [20, 24],
                "unchanged_target",
                "unchanged_target",
            ),
            symbol(
                "rename-new-id",
                "src/rename_new.rs",
                [40, 45],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "added-id",
                "src/added.rs",
                [80, 84],
                "added_target",
                "added_target",
            ),
            symbol(
                "stable-caller-id",
                "src/stable.rs",
                [60, 64],
                "stable_caller",
                "stable_caller",
            ),
            symbol(
                "stable-target-id",
                "src/stable.rs",
                [70, 74],
                "stable_target",
                "stable_target",
            ),
            symbol(
                "stable-isolated-id",
                "src/stable.rs",
                [76, 78],
                "stable_isolated",
                "stable_isolated",
            ),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("new-changed-id"),
                Some("target"),
            ),
            edge("unchanged-caller-id", None, Some("deleted_target")),
            edge(
                "unchanged-caller-id",
                Some("rename-new-id"),
                Some("renamed_target"),
            ),
            edge(
                "unchanged-caller-id",
                Some("added-id"),
                Some("added_target"),
            ),
            edge("new-changed-id", Some("added-id"), Some("added_target")),
            edge(
                "new-caller-id",
                Some("unchanged-target-id"),
                Some("unchanged_target"),
            ),
            edge("collision-id", Some("added-id"), Some("added_target")),
            edge(
                "stable-caller-id",
                Some("stable-target-id"),
                Some("stable_target"),
            ),
        ],
    )
}

fn generation_base_artifact() -> GraphIndexArtifact {
    artifact(
        "generation-base",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
            file("src/rename_old.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "base_target",
                "base_target",
            ),
            symbol(
                "unchanged-id",
                "src/unchanged.rs",
                [3, 4],
                "unchanged_target",
                "unchanged_target",
            ),
            symbol(
                "deleted-id",
                "src/deleted.rs",
                [5, 6],
                "deleted_target",
                "deleted_target",
            ),
            symbol(
                "rename-id",
                "src/rename_old.rs",
                [7, 8],
                "renamed_target",
                "renamed_target",
            ),
        ],
        vec![],
    )
}

fn generation_delta_v1() -> GraphIndexArtifact {
    artifact(
        "generation-delta-v1",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/untracked.rs"),
        ],
        vec![
            symbol(
                "modified-target-id",
                "src/changed.rs",
                [10, 12],
                "modified_target",
                "modified_target",
            ),
            symbol(
                "rename-id",
                "src/rename_new.rs",
                [14, 16],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "untracked-id",
                "src/untracked.rs",
                [18, 20],
                "untracked_target",
                "untracked_target",
            ),
        ],
        vec![],
    )
}

fn generation_delta_v2() -> GraphIndexArtifact {
    artifact(
        "generation-delta-v2",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/untracked.rs"),
        ],
        vec![
            symbol(
                "rename-id",
                "src/rename_new.rs",
                [14, 16],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "untracked-id",
                "src/untracked.rs",
                [18, 20],
                "untracked_target",
                "untracked_target",
            ),
        ],
        vec![],
    )
}

fn generation_oracle_v1() -> GraphIndexArtifact {
    artifact(
        "generation-oracle-v1",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/untracked.rs"),
        ],
        vec![
            symbol(
                "modified-target-id",
                "src/changed.rs",
                [10, 12],
                "modified_target",
                "modified_target",
            ),
            symbol(
                "unchanged-id",
                "src/unchanged.rs",
                [3, 4],
                "unchanged_target",
                "unchanged_target",
            ),
            symbol(
                "rename-id",
                "src/rename_new.rs",
                [14, 16],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "untracked-id",
                "src/untracked.rs",
                [18, 20],
                "untracked_target",
                "untracked_target",
            ),
        ],
        vec![],
    )
}

fn generation_oracle_v2() -> GraphIndexArtifact {
    artifact(
        "generation-oracle-v2",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/rename_new.rs"),
            file("src/untracked.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "base_target",
                "base_target",
            ),
            symbol(
                "unchanged-id",
                "src/unchanged.rs",
                [3, 4],
                "unchanged_target",
                "unchanged_target",
            ),
            symbol(
                "rename-id",
                "src/rename_new.rs",
                [14, 16],
                "renamed_target",
                "renamed_target",
            ),
            symbol(
                "untracked-id",
                "src/untracked.rs",
                [18, 20],
                "untracked_target",
                "untracked_target",
            ),
        ],
        vec![],
    )
}

#[test]
fn empty_delta_matches_base_across_query_surface() {
    let base_artifact = base_artifact();
    let base = InMemoryClient::new(Arc::new(base_artifact));
    let overlay = OverlayClient::from_artifacts(
        base.clone(),
        Arc::new(empty_delta_artifact()),
        HashSet::new(),
    )
    .expect("overlay");

    assert_clients_match(&overlay, &base, "base-target-id");
    assert!(Arc::ptr_eq(
        &overlay.temporal_index(),
        &base.temporal_index()
    ));
}

#[test]
fn empty_overlay_callees_do_not_resolve_unresolved_labels_against_base() {
    let base = CountingClient::new(InMemoryClient::new(Arc::new(base_artifact())));
    let overlay = OverlayClient::from_artifacts(
        base.clone(),
        Arc::new(empty_delta_artifact()),
        HashSet::new(),
    )
    .expect("overlay");

    let records = overlay.find_callee_edges("unchanged-caller-id");
    assert!(
        records.iter().any(|record| matches!(
            record,
            OwnedCalleeRecord::Unresolved {
                target_label,
                ..
            } if target_label == "fresh"
        )),
        "base unresolved callee `fresh` must stay unresolved under an empty overlay"
    );
    assert_eq!(
        base.resolve_selector_calls(),
        0,
        "empty overlay must not probe the base for unresolved callee labels"
    );
}

#[test]
fn stable_changed_file_edit_matches_full_rebuild() {
    let overlay = overlay_from_parts(
        base_artifact(),
        artifact(
            "delta-edit",
            vec![
                file("src/changed.rs"),
                file("src/unchanged.rs"),
                file("src/deleted.rs"),
            ],
            vec![symbol(
                "base-target-id",
                "src/changed.rs",
                [3, 8],
                "target",
                "target",
            )],
            vec![],
        ),
        ["src/changed.rs"],
    );
    let full = InMemoryClient::new(Arc::new(artifact(
        "full-edit",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [3, 8],
                "target",
                "target",
            ),
            caller_symbol(),
            deleted_symbol(),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("base-target-id"),
                Some("target"),
            ),
            edge("unchanged-caller-id", None, Some("fresh")),
            edge("unchanged-caller-id", Some("deleted-id"), Some("gone")),
        ],
    )));

    assert_clients_match(&overlay, &full, "base-target-id");
}

#[test]
fn added_symbol_called_by_unchanged_file_matches_full_rebuild() {
    let overlay = overlay_from_parts(
        base_artifact(),
        artifact(
            "delta-add",
            vec![
                file("src/changed.rs"),
                file("src/unchanged.rs"),
                file("src/deleted.rs"),
            ],
            vec![
                symbol(
                    "base-target-id",
                    "src/changed.rs",
                    [1, 2],
                    "target",
                    "target",
                ),
                symbol("fresh-id", "src/changed.rs", [10, 12], "fresh", "fresh"),
            ],
            vec![],
        ),
        ["src/changed.rs"],
    );
    let full = InMemoryClient::new(Arc::new(artifact(
        "full-add",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "target",
                "target",
            ),
            symbol("fresh-id", "src/changed.rs", [10, 12], "fresh", "fresh"),
            caller_symbol(),
            deleted_symbol(),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("base-target-id"),
                Some("target"),
            ),
            edge("unchanged-caller-id", Some("deleted-id"), Some("gone")),
            edge("unchanged-caller-id", Some("fresh-id"), Some("fresh")),
        ],
    )));

    assert_clients_match(&overlay, &full, "fresh-id");
    assert_eq!(
        caller_records(overlay.find_caller_edges("fresh-id")),
        caller_records(full.find_caller_edges("fresh-id"))
    );
}

#[test]
fn renamed_symbol_called_by_unchanged_file_is_repointed_to_new_id() {
    let overlay = overlay_from_parts(
        base_artifact(),
        artifact(
            "delta-rename",
            vec![
                file("src/changed.rs"),
                file("src/unchanged.rs"),
                file("src/deleted.rs"),
            ],
            vec![symbol(
                "renamed-target-id",
                "src/changed.rs",
                [1, 2],
                "renamed",
                "renamed",
            )],
            vec![],
        ),
        ["src/changed.rs"],
    );
    let full = InMemoryClient::new(Arc::new(artifact(
        "full-rename",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol(
                "renamed-target-id",
                "src/changed.rs",
                [1, 2],
                "renamed",
                "renamed",
            ),
            caller_symbol(),
            deleted_symbol(),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("renamed-target-id"),
                Some("renamed"),
            ),
            edge("unchanged-caller-id", None, Some("fresh")),
            edge("unchanged-caller-id", Some("deleted-id"), Some("gone")),
        ],
    )));

    assert_clients_match(&overlay, &full, "renamed-target-id");
    assert_eq!(
        caller_records(overlay.find_caller_edges("renamed-target-id")),
        caller_records(full.find_caller_edges("renamed-target-id"))
    );
}

#[test]
fn deleted_symbol_is_shadowed_and_unchanged_callee_becomes_unresolved() {
    let overlay = overlay_from_parts(
        base_artifact(),
        artifact(
            "delta-delete",
            vec![
                file("src/changed.rs"),
                file("src/unchanged.rs"),
                file("src/deleted.rs"),
            ],
            vec![symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "target",
                "target",
            )],
            vec![],
        ),
        ["src/changed.rs", "src/deleted.rs"],
    );
    let full = InMemoryClient::new(Arc::new(artifact(
        "full-delete",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "target",
                "target",
            ),
            caller_symbol(),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("base-target-id"),
                Some("target"),
            ),
            edge("unchanged-caller-id", None, Some("fresh")),
            edge("unchanged-caller-id", None, Some("gone")),
        ],
    )));

    assert_eq!(
        overlay
            .symbol_by_id("deleted-id")
            .expect("overlay symbol query"),
        None
    );
    assert_clients_match(&overlay, &full, "deleted-id");
}

#[test]
fn changed_file_adding_call_to_unchanged_symbol_is_reflected_in_callers() {
    // base: `mover` (in the to-be-changed file) does NOT yet call `unchanged_target`.
    let base = artifact(
        "base-newcall",
        vec![file("src/changed.rs"), file("src/unchanged.rs")],
        vec![
            symbol("mover-id", "src/changed.rs", [1, 4], "mover", "mover"),
            symbol(
                "unchanged-target-id",
                "src/unchanged.rs",
                [10, 14],
                "unchanged_target",
                "unchanged_target",
            ),
        ],
        vec![],
    );
    // delta (changed.rs only): `mover` now calls `unchanged_target`. The target lives
    // outside the delta's extraction scope, so the edge is UNRESOLVED in the delta —
    // exactly what build_facts_for_paths produces for a cross-boundary call.
    let delta = artifact(
        "delta-newcall",
        vec![file("src/changed.rs")],
        vec![symbol(
            "mover-id",
            "src/changed.rs",
            [1, 6],
            "mover",
            "mover",
        )],
        vec![edge("mover-id", None, Some("unchanged_target"))],
    );
    let overlay = overlay_from_parts(base, delta, ["src/changed.rs"]);

    // Oracle: a full rebuild resolves the edge to the unchanged target.
    let full = InMemoryClient::new(Arc::new(artifact(
        "full-newcall",
        vec![file("src/changed.rs"), file("src/unchanged.rs")],
        vec![
            symbol("mover-id", "src/changed.rs", [1, 6], "mover", "mover"),
            symbol(
                "unchanged-target-id",
                "src/unchanged.rs",
                [10, 14],
                "unchanged_target",
                "unchanged_target",
            ),
        ],
        vec![edge(
            "mover-id",
            Some("unchanged-target-id"),
            Some("unchanged_target"),
        )],
    )));

    assert_eq!(
        caller_records(overlay.find_caller_edges("unchanged-target-id")),
        caller_records(full.find_caller_edges("unchanged-target-id")),
        "find_caller_edges(unchanged-target-id) must include the new caller from the changed file"
    );
}

fn assert_clients_match(
    actual: &impl spur_graph::GraphQueryClient,
    expected: &impl spur_graph::GraphQueryClient,
    focus_id: &str,
) {
    for options in [
        options("target", SearchMode::Exact),
        options("fresh", SearchMode::Exact),
        options("renamed", SearchMode::Exact),
        options("ta", SearchMode::Substring),
        SearchOptions {
            query: "t".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters {
                file: Some("src/changed.rs".to_owned()),
                ..SearchFilters::default()
            },
            limit: 1,
        },
    ] {
        assert_eq!(
            actual.search_symbols(&options).expect("actual search"),
            expected.search_symbols(&options).expect("expected search"),
            "search options: {options:?}"
        );
    }

    for sid in [
        "base-target-id",
        "fresh-id",
        "renamed-target-id",
        "deleted-id",
        "unchanged-caller-id",
    ] {
        assert_eq!(
            actual.symbol_by_id(sid).expect("actual symbol by id"),
            expected.symbol_by_id(sid).expect("expected symbol by id"),
            "symbol_by_id({sid})"
        );
    }

    for path in ["src/changed.rs", "src/unchanged.rs", "src/deleted.rs"] {
        assert_eq!(
            actual
                .symbols_by_file(path)
                .expect("actual symbols by file"),
            expected
                .symbols_by_file(path)
                .expect("expected symbols by file"),
            "symbols_by_file({path})"
        );
        assert_eq!(
            actual
                .file_manifest_by_path(path)
                .expect("actual manifest by path"),
            expected
                .file_manifest_by_path(path)
                .expect("expected manifest by path"),
            "file_manifest_by_path({path})"
        );
        assert_eq!(
            actual.file_exists(path).expect("actual file exists"),
            expected.file_exists(path).expect("expected file exists"),
            "file_exists({path})"
        );
    }

    for (path, name) in [
        ("src/changed.rs", "target"),
        ("src/changed.rs", "fresh"),
        ("src/changed.rs", "renamed"),
        ("src/deleted.rs", "gone"),
        ("src/unchanged.rs", "unchanged_caller"),
    ] {
        assert_eq!(
            actual
                .symbols_by_path_name(path, name)
                .expect("actual symbols by path name"),
            expected
                .symbols_by_path_name(path, name)
                .expect("expected symbols by path name"),
            "symbols_by_path_name({path}, {name})"
        );
    }

    for selector in [
        "target",
        "fresh",
        "renamed",
        "gone",
        "src/changed.rs::target",
        "src/changed.rs::renamed",
        "src/deleted.rs::gone",
    ] {
        assert_eq!(
            normalize_resolution(actual.resolve_selector(selector).expect("actual selector")),
            normalize_resolution(
                expected
                    .resolve_selector(selector)
                    .expect("expected selector")
            ),
            "resolve_selector({selector})"
        );
    }

    assert_eq!(
        callee_records(actual.find_callee_edges("unchanged-caller-id")),
        callee_records(expected.find_callee_edges("unchanged-caller-id")),
        "find_callee_edges(unchanged-caller-id)"
    );
    assert_eq!(
        caller_records(actual.find_caller_edges(focus_id)),
        caller_records(expected.find_caller_edges(focus_id)),
        "find_caller_edges({focus_id})"
    );
}

#[test]
fn overlay_remap_queries_base_per_changed_file_not_per_symbol() {
    let base = CountingClient::new(InMemoryClient::new(Arc::new(base_artifact())));
    let delta = artifact(
        "delta-many-symbols",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol("changed-a", "src/changed.rs", [1, 2], "target", "target"),
            symbol(
                "changed-b",
                "src/changed.rs",
                [4, 6],
                "helper_b",
                "helper_b",
            ),
            symbol(
                "changed-c",
                "src/changed.rs",
                [8, 9],
                "helper_c",
                "helper_c",
            ),
            symbol("deleted-a", "src/deleted.rs", [30, 31], "gone", "gone"),
            symbol("deleted-b", "src/deleted.rs", [33, 35], "gone_b", "gone_b"),
            symbol("deleted-c", "src/deleted.rs", [37, 39], "gone_c", "gone_c"),
        ],
        vec![],
    );

    let overlay = OverlayClient::from_artifacts(
        base.clone(),
        Arc::new(delta),
        HashSet::from(["src/changed.rs".to_owned(), "src/deleted.rs".to_owned()]),
    )
    .expect("overlay");

    assert_eq!(
        base.symbols_by_path_name_calls(),
        0,
        "overlay construction must not query the base once per delta symbol"
    );
    assert!(
        base.symbols_by_file_calls() <= 2,
        "overlay construction should fetch base symbols at most once per changed file, got {}",
        base.symbols_by_file_calls()
    );

    // The remap must still repoint stale base ids to their delta successors.
    let repointed = overlay
        .find_caller_edges("changed-a")
        .into_iter()
        .filter_map(|record| match record {
            OwnedCallerRecord::Resolved { caller, .. } => Some(caller.stable_symbol_id),
            OwnedCallerRecord::Unresolved { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(repointed, vec!["unchanged-caller-id".to_owned()]);
}

#[derive(Clone)]
struct CountingClient {
    inner: InMemoryClient,
    symbols_by_file_calls: Arc<AtomicUsize>,
    symbols_by_path_name_calls: Arc<AtomicUsize>,
    resolve_selector_calls: Arc<AtomicUsize>,
}

impl CountingClient {
    fn new(inner: InMemoryClient) -> Self {
        Self {
            inner,
            symbols_by_file_calls: Arc::new(AtomicUsize::new(0)),
            symbols_by_path_name_calls: Arc::new(AtomicUsize::new(0)),
            resolve_selector_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn symbols_by_file_calls(&self) -> usize {
        self.symbols_by_file_calls.load(Ordering::SeqCst)
    }

    fn symbols_by_path_name_calls(&self) -> usize {
        self.symbols_by_path_name_calls.load(Ordering::SeqCst)
    }

    fn resolve_selector_calls(&self) -> usize {
        self.resolve_selector_calls.load(Ordering::SeqCst)
    }
}

impl GraphQueryClient for CountingClient {
    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        self.inner.search_symbols(opts)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        self.inner.find_caller_edges(sid)
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        self.inner
            .find_unresolved_caller_edges_by_labels(target_labels)
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        self.inner.find_callee_edges(sid)
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution> {
        self.resolve_selector_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_selector(selector)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        self.inner.symbol_by_id(sid)
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_by_file_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.symbols_by_file(path)
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_by_path_name_calls
            .fetch_add(1, Ordering::SeqCst);
        self.inner.symbols_by_path_name(path, name)
    }

    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        self.inner.file_manifest_by_path(path)
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        self.inner.file_exists(path)
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        self.inner.temporal_index()
    }
}

fn normalize_resolution(resolution: CodeSelectorResolution) -> CodeSelectorResolution {
    resolution
}

fn overlay_from_parts(
    base: GraphIndexArtifact,
    delta: GraphIndexArtifact,
    shadowed: impl IntoIterator<Item = &'static str>,
) -> OverlayClient<InMemoryClient> {
    OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base)),
        Arc::new(delta),
        shadowed.into_iter().map(str::to_owned).collect(),
    )
    .expect("overlay")
}

fn base_artifact() -> GraphIndexArtifact {
    artifact(
        "overlay-base",
        vec![
            file("src/changed.rs"),
            file("src/unchanged.rs"),
            file("src/deleted.rs"),
        ],
        vec![
            symbol(
                "base-target-id",
                "src/changed.rs",
                [1, 2],
                "target",
                "target",
            ),
            caller_symbol(),
            deleted_symbol(),
        ],
        vec![
            edge(
                "unchanged-caller-id",
                Some("base-target-id"),
                Some("target"),
            ),
            edge("unchanged-caller-id", None, Some("fresh")),
            edge("unchanged-caller-id", Some("deleted-id"), Some("gone")),
        ],
    )
}

fn empty_delta_artifact() -> GraphIndexArtifact {
    artifact("empty-delta", Vec::new(), Vec::new(), Vec::new())
}

fn caller_symbol() -> GraphSymbolArtifact {
    symbol(
        "unchanged-caller-id",
        "src/unchanged.rs",
        [20, 24],
        "unchanged_caller",
        "unchanged_caller",
    )
}

fn deleted_symbol() -> GraphSymbolArtifact {
    symbol("deleted-id", "src/deleted.rs", [30, 31], "gone", "gone")
}

fn artifact(
    graph_content_hash: &str,
    files: Vec<GraphFileArtifact>,
    symbols: Vec<GraphSymbolArtifact>,
    edges: Vec<GraphEdgeArtifact>,
) -> GraphIndexArtifact {
    GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "test".to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "test".to_owned(),
        graph_content_hash: graph_content_hash.to_owned(),
        file_manifests: files
            .iter()
            .enumerate()
            .map(|(index, file)| GraphFileManifestEntry {
                stable_file_id: file.stable_file_id.clone(),
                path: file.file_path.clone(),
                content_oid: format!("{:040x}", index + 1),
                node_ids: vec![NodeId((index + 1) as u64)],
            })
            .collect(),
        file_node_ids: (1..=files.len()).map(|id| NodeId(id as u64)).collect(),
        files,
        symbol_node_ids: (101..101 + symbols.len())
            .map(|id| NodeId(id as u64))
            .collect(),
        symbols,
        edges,
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    }
}

fn file(path: &str) -> GraphFileArtifact {
    GraphFileArtifact {
        stable_file_id: format!("file:{path}"),
        file_path: path.to_owned(),
    }
}

fn symbol(
    stable_symbol_id: &str,
    file_path: &str,
    line_range: [usize; 2],
    entity_name: &str,
    qualified_name: &str,
) -> GraphSymbolArtifact {
    GraphSymbolArtifact {
        stable_symbol_id: stable_symbol_id.to_owned(),
        file_path: file_path.to_owned(),
        byte_range: [line_range[0], line_range[1]],
        line_range,
        entity_name: entity_name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        symbol_kind: "function".to_owned(),
        anchor_hash: format!("hash:{stable_symbol_id}"),
        enclosing_scope: None,
    }
}

fn edge(
    source_stable_symbol_id: &str,
    target_stable_symbol_id: Option<&str>,
    target_label: Option<&str>,
) -> GraphEdgeArtifact {
    GraphEdgeArtifact {
        source_stable_symbol_id: source_stable_symbol_id.to_owned(),
        target_stable_symbol_id: target_stable_symbol_id.map(str::to_owned),
        target_label: target_label.map(str::to_owned),
        import_path: None,
        receiver_text: None,
        scope_text: None,
        relation: RelationKind::Calls,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::Calls),
        bind_method: None,
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

fn caller_records(records: Vec<OwnedCallerRecord>) -> Vec<(String, String, Option<String>, bool)> {
    let mut rows = records
        .into_iter()
        .map(|record| match record {
            OwnedCallerRecord::Resolved { caller, edge } => (
                caller.stable_symbol_id,
                edge.source_stable_symbol_id,
                edge.target_stable_symbol_id,
                true,
            ),
            OwnedCallerRecord::Unresolved {
                caller,
                edge,
                target_label,
            } => (
                caller.stable_symbol_id,
                edge.source_stable_symbol_id,
                Some(target_label),
                false,
            ),
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn callee_records(records: Vec<OwnedCalleeRecord>) -> Vec<(String, String, Option<String>, bool)> {
    let mut rows = records
        .into_iter()
        .map(|record| match record {
            OwnedCalleeRecord::Resolved { symbol, edge } => (
                symbol.stable_symbol_id,
                edge.source_stable_symbol_id,
                edge.target_stable_symbol_id,
                true,
            ),
            OwnedCalleeRecord::Unresolved { edge, target_label } => (
                target_label.clone(),
                edge.source_stable_symbol_id,
                Some(target_label),
                false,
            ),
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}
