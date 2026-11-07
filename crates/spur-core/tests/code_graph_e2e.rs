use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use rmcp::{
    model::CallToolRequestParams,
    service::ServiceError,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use spur_acp::{BrainSessionId, SessionId, SpurEventBody};
use spur_core::handlers::PlanResolver;
use spur_core::server::{community_feature_gate, DetachedContinuationCtx};
use spur_core::worker_server::{WorkerMcpDeps, WorkerMcpServer};
use spur_core::McpCallbackServer;
use spur_graph::store::cache::write_with_dedup;
use spur_graph::{
    artifact_from_facts, build_facts, build_facts_for_paths, write_artifact_parquet,
    write_current_pointer, ChangeKind, CommitArtifact, CommitIndexArtifact, Confidence,
    EdgeEndpoint, GraphEdgeArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader,
    GraphSymbolArtifact, NodeId, RelationKind, RenamePrev, SnapshotKey, SymbolSnapshotArtifact,
    TemporalEdgeArtifact, WalkStrategy, WriteOptions, GRAPH_INDEX_VERSION_TEMPORAL,
};
use spur_mcp::events::McpEventSink;
use spur_pm::PmService;
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

mod common;

const ROOT_SYMBOL: &str = "orchestrate_order";
const OLD_SHA: &str = "1111111111111111111111111111111111111111";
const NEW_SHA: &str = "2222222222222222222222222222222222222222";
const OLD_ROOT_ID: &str = "symbol-foo";
const NEW_ROOT_ID: &str = "symbol-bar";
const OLD_CALLER_ID: &str = "caller-foo";
const OLD_CALLEE_ID: &str = "callee-foo";
const NEW_CALLER_ID: &str = "caller-bar";
const NEW_CALLEE_ID: &str = "callee-bar";
const DELETE_SHA: &str = "3333333333333333333333333333333333333333";
const AMBIGUOUS_SHA: &str = "4444444444444444444444444444444444444444";
const UNKNOWN_TARGET_SHA: &str = "5555555555555555555555555555555555555555";
const UNINDEXED_SHA: &str = "6666666666666666666666666666666666666666";
const FOUND_ORIGIN_ID: &str = "found-origin";
const FOUND_TARGET_ID: &str = "found-target";
const DELETED_ID: &str = "deleted-symbol";
const AMBIGUOUS_ORIGIN_ID: &str = "ambiguous-origin";
const AMBIGUOUS_LEFT_ID: &str = "ambiguous-left";
const AMBIGUOUS_RIGHT_ID: &str = "ambiguous-right";
const UNKNOWN_ID: &str = "unknown-symbol";

static CWD_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();

async fn cwd_lock() -> tokio::sync::MutexGuard<'static, ()> {
    CWD_LOCK.get_or_init(|| TokioMutex::new(())).lock().await
}

#[derive(Clone, Copy, Debug)]
enum CodeGraphFixtureBackend {
    Parquet,
    InMemory,
}

struct CwdGuard {
    original: std::path::PathBuf,
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

fn enter_dir(path: &Path) -> CwdGuard {
    let original = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(path).expect("set current dir");
    CwdGuard { original }
}

fn continuation_ctx() -> DetachedContinuationCtx {
    DetachedContinuationCtx {
        on_complete: Arc::new(|_, _| Box::pin(async {})),
    }
}

fn test_server() -> McpCallbackServer {
    let session_id = BrainSessionId::new(SessionId("brain-code-graph-e2e".into()));
    let (server, _channel) = McpCallbackServer::new(
        Some(&session_id),
        None,
        None,
        continuation_ctx(),
        Arc::new(spur_blob_store::MemoryOutcomeStore::new()),
        community_feature_gate(),
    );
    server
}

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/code_graph_sample")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn copy_fixture_crate(worktree: &Path) {
    let fixture = fixture_root();
    std::fs::create_dir_all(worktree.join("src")).expect("create fixture src dir");
    std::fs::copy(fixture.join("Cargo.toml"), worktree.join("Cargo.toml"))
        .expect("copy fixture manifest");
    std::fs::copy(fixture.join("src/lib.rs"), worktree.join("src/lib.rs"))
        .expect("copy fixture source");
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
}

fn write_ambiguous_symbol_fixture(worktree: &Path) {
    std::fs::create_dir_all(worktree.join("src")).expect("create fixture src dir");
    std::fs::write(
        worktree.join("Cargo.toml"),
        "[package]\nname = \"ambiguous-code-graph-sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write fixture manifest");
    std::fs::write(
        worktree.join("src/lib.rs"),
        "pub struct Alpha;\n\
         pub struct Beta;\n\
         \n\
         impl Alpha {\n\
             pub fn run(&self) -> bool {\n\
                 true\n\
             }\n\
         }\n\
         \n\
         impl Beta {\n\
             pub fn run(&self) -> bool {\n\
                 false\n\
             }\n\
         }\n",
    )
    .expect("write fixture source");
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
}

fn write_wide_fixture_crate(worktree: &Path, helper_count: usize) {
    std::fs::create_dir_all(worktree.join("src")).expect("create fixture src dir");
    std::fs::write(
        worktree.join("Cargo.toml"),
        "[package]\nname = \"wide-code-graph-sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write fixture manifest");

    let mut source = String::from("pub fn wide_root() {\n");
    for index in 0..helper_count {
        source.push_str(&format!("    wide_child_{index:03}();\n"));
    }
    source.push_str("}\n\n");
    for index in 0..helper_count {
        source.push_str(&format!("pub fn wide_child_{index:03}() {{}}\n"));
    }

    std::fs::write(worktree.join("src/lib.rs"), source).expect("write fixture source");
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
}

fn write_multi_file_fixture_crate(worktree: &Path) {
    copy_fixture_crate(worktree);
    std::fs::write(
        worktree.join("src/support.rs"),
        "pub fn unrelated_support_symbol() -> bool {\n    true\n}\n",
    )
    .expect("write support source");
    let lib_path = worktree.join("src/lib.rs");
    let mut lib = std::fs::read_to_string(&lib_path).expect("read fixture source");
    lib.push_str("\npub mod support;\n");
    std::fs::write(lib_path, lib).expect("write fixture source with support module");
}

fn git(worktree: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()
        .unwrap_or_else(|error| panic!("run git {}: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout UTF-8")
}

fn commit_fixture(worktree: &Path) {
    git(worktree, &["init", "-q"]);
    git(worktree, &["config", "user.email", "test@spur"]);
    git(worktree, &["config", "user.name", "SPUR Test"]);
    git(worktree, &["add", "."]);
    git(worktree, &["commit", "-m", "fixture"]);
}

fn add_linked_worktree(main: &Path, worker: &Path, branch: &str) {
    let worker_arg = worker.to_str().expect("worker path is UTF-8");
    git(
        main,
        &["worktree", "add", "-q", "-b", branch, worker_arg, "HEAD"],
    );
}

fn assert_no_graph_artifact(worktree: &Path) {
    assert!(!worktree.join(".spur/graph/CURRENT").exists());
    assert!(!worktree.join(".spur/graph-index.pointer.json").exists());
    assert!(!worktree.join(".spur/graph-index.json").exists());
}

fn build_graph_artifact(worktree: &Path) -> GraphIndexArtifact {
    let (facts, _file_counts) = build_facts(worktree, None).expect("build graph facts");
    let artifact = artifact_from_facts(&facts, worktree).expect("build graph artifact");
    write_graph_artifact(worktree, &artifact);
    artifact
}

fn build_graph_artifact_with_pointer(worktree: &Path) -> GraphIndexArtifact {
    let (facts, _file_counts) = build_facts(worktree, None).expect("build graph facts");
    let artifact = artifact_from_facts(&facts, worktree).expect("build graph artifact");
    let ctx = spur_graph::git::detect(worktree).expect("git worktree metadata");
    write_with_dedup(&artifact, worktree, &ctx).expect("write graph artifact with pointer");
    artifact
}

fn write_temporal_fixture_artifact(worktree: &Path) {
    write_temporal_fixture_artifact_for_backend(worktree, CodeGraphFixtureBackend::Parquet);
}

fn write_temporal_fixture_artifact_for_backend(worktree: &Path, backend: CodeGraphFixtureBackend) {
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
    let old_root = snapshot(OLD_ROOT_ID, OLD_SHA, "foo");
    let new_root = snapshot(NEW_ROOT_ID, NEW_SHA, "bar");
    let graph = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "temporal-fixture".to_string(),
        graph_content_hash: "temporal-fixture".to_string(),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: vec![
            graph_symbol(OLD_CALLER_ID, "launch_foo"),
            graph_symbol(OLD_ROOT_ID, "foo"),
            graph_symbol(OLD_CALLEE_ID, "helper_foo"),
            graph_symbol(NEW_CALLER_ID, "launch_bar"),
            graph_symbol(NEW_ROOT_ID, "bar"),
            graph_symbol(NEW_CALLEE_ID, "helper_bar"),
        ],
        symbol_node_ids: node_ids(6),
        edges: vec![
            graph_edge(OLD_CALLER_ID, OLD_ROOT_ID),
            graph_edge(OLD_ROOT_ID, OLD_CALLEE_ID),
            graph_edge(NEW_CALLER_ID, NEW_ROOT_ID),
            graph_edge(NEW_ROOT_ID, NEW_CALLEE_ID),
        ],
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: vec![
            CommitArtifact {
                sha: OLD_SHA.to_string(),
                parents: Vec::new(),
                author_time: 1,
                author_name: "Ada Lovelace".to_string(),
                author_email: "ada@example.test".to_string(),
                summary: "add foo".to_string(),
            },
            CommitArtifact {
                sha: NEW_SHA.to_string(),
                parents: vec![OLD_SHA.to_string()],
                author_time: 2,
                author_name: "Grace Hopper".to_string(),
                author_email: "grace@example.test".to_string(),
                summary: "rename foo to bar".to_string(),
            },
        ],
        symbol_snapshots: vec![old_root.clone(), new_root.clone()],
        temporal_edges: vec![
            temporal_touch(OLD_SHA, old_root.key.clone(), ChangeKind::Added),
            temporal_touch(
                NEW_SHA,
                new_root.key.clone(),
                ChangeKind::RenamedFrom(RenamePrev::Symbol(old_root.key.clone())),
            ),
            temporal_rename(old_root.key.clone(), new_root.key.clone()),
        ],
        ..GraphIndexArtifact::empty()
    };
    match backend {
        CodeGraphFixtureBackend::Parquet => write_graph_artifact(worktree, &graph),
        CodeGraphFixtureBackend::InMemory => write_graph_artifact(worktree, &graph),
    }

    let commits = CommitIndexArtifact {
        schema_version: GRAPH_INDEX_VERSION_TEMPORAL
            .parse()
            .expect("temporal graph index version is numeric"),
        commits: graph.commits.clone(),
        refs: [("HEAD".to_string(), NEW_SHA.to_string())].into(),
        indexed_at: "2026-05-20T12:00:00Z".to_string(),
        walk_strategy: WalkStrategy::Reachable,
    };
    spur_graph::store::commit_index::save_artifact(worktree, ".spur/commit-index.json", &commits)
        .expect("write commit index artifact");
    spur_graph::store::commit_index::save_pointer(
        worktree,
        &spur_graph::store::commit_index::CommitIndexPointer {
            schema_version: GRAPH_INDEX_VERSION_TEMPORAL
                .parse()
                .expect("temporal graph index version is numeric"),
            artifact_relative_path: ".spur/commit-index.json".to_string(),
            indexed_at: commits.indexed_at.clone(),
            refs: commits.refs.clone(),
        },
    )
    .expect("write commit index pointer");
}

fn write_temporal_resolution_fixture_artifact(worktree: &Path) {
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");

    let found_origin = snapshot(FOUND_ORIGIN_ID, OLD_SHA, "found_origin");
    let found_target = snapshot(FOUND_TARGET_ID, NEW_SHA, "found_target");
    let deleted_added = snapshot(DELETED_ID, OLD_SHA, "deleted_symbol");
    let deleted_last_seen = snapshot(DELETED_ID, DELETE_SHA, "deleted_symbol");
    let ambiguous_origin = snapshot(AMBIGUOUS_ORIGIN_ID, OLD_SHA, "ambiguous_origin");
    let ambiguous_left = snapshot(AMBIGUOUS_LEFT_ID, AMBIGUOUS_SHA, "ambiguous_left");
    let ambiguous_right = snapshot(AMBIGUOUS_RIGHT_ID, AMBIGUOUS_SHA, "ambiguous_right");
    let unknown = snapshot(UNKNOWN_ID, UNINDEXED_SHA, "unknown_symbol");

    let commits = vec![
        CommitArtifact {
            sha: OLD_SHA.to_string(),
            parents: Vec::new(),
            author_time: 1,
            author_name: String::new(),
            author_email: String::new(),
            summary: "add roots".to_string(),
        },
        CommitArtifact {
            sha: NEW_SHA.to_string(),
            parents: vec![OLD_SHA.to_string()],
            author_time: 2,
            author_name: String::new(),
            author_email: String::new(),
            summary: "rename found".to_string(),
        },
        CommitArtifact {
            sha: DELETE_SHA.to_string(),
            parents: vec![NEW_SHA.to_string()],
            author_time: 3,
            author_name: String::new(),
            author_email: String::new(),
            summary: "delete symbol".to_string(),
        },
        CommitArtifact {
            sha: AMBIGUOUS_SHA.to_string(),
            parents: vec![DELETE_SHA.to_string()],
            author_time: 4,
            author_name: String::new(),
            author_email: String::new(),
            summary: "ambiguous rename".to_string(),
        },
        CommitArtifact {
            sha: UNKNOWN_TARGET_SHA.to_string(),
            parents: vec![AMBIGUOUS_SHA.to_string()],
            author_time: 5,
            author_name: String::new(),
            author_email: String::new(),
            summary: "target for unknown anchor".to_string(),
        },
    ];

    let graph = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "temporal-resolution-fixture".to_string(),
        graph_content_hash: "temporal-resolution-fixture".to_string(),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: vec![
            graph_symbol(FOUND_ORIGIN_ID, "found_origin"),
            graph_symbol(FOUND_TARGET_ID, "found_target"),
            graph_symbol(DELETED_ID, "deleted_symbol"),
            graph_symbol(AMBIGUOUS_ORIGIN_ID, "ambiguous_origin"),
            graph_symbol(AMBIGUOUS_LEFT_ID, "ambiguous_left"),
            graph_symbol(AMBIGUOUS_RIGHT_ID, "ambiguous_right"),
            graph_symbol(UNKNOWN_ID, "unknown_symbol"),
        ],
        symbol_node_ids: node_ids(7),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: commits.clone(),
        symbol_snapshots: vec![
            found_origin.clone(),
            found_target.clone(),
            deleted_added.clone(),
            deleted_last_seen.clone(),
            ambiguous_origin.clone(),
            ambiguous_left.clone(),
            ambiguous_right.clone(),
            unknown.clone(),
        ],
        temporal_edges: vec![
            temporal_touch(OLD_SHA, found_origin.key.clone(), ChangeKind::Added),
            temporal_touch(
                NEW_SHA,
                found_target.key.clone(),
                ChangeKind::RenamedFrom(RenamePrev::Symbol(found_origin.key.clone())),
            ),
            temporal_rename(found_origin.key.clone(), found_target.key.clone()),
            temporal_touch(OLD_SHA, deleted_added.key.clone(), ChangeKind::Added),
            temporal_touch(
                DELETE_SHA,
                deleted_last_seen.key.clone(),
                ChangeKind::Deleted,
            ),
            temporal_touch(OLD_SHA, ambiguous_origin.key.clone(), ChangeKind::Added),
            temporal_touch(
                AMBIGUOUS_SHA,
                ambiguous_left.key.clone(),
                ChangeKind::RenamedFrom(RenamePrev::Symbol(ambiguous_origin.key.clone())),
            ),
            temporal_rename(ambiguous_origin.key.clone(), ambiguous_left.key.clone()),
            temporal_touch(
                AMBIGUOUS_SHA,
                ambiguous_right.key.clone(),
                ChangeKind::RenamedFrom(RenamePrev::Symbol(ambiguous_origin.key.clone())),
            ),
            temporal_rename(ambiguous_origin.key.clone(), ambiguous_right.key.clone()),
            temporal_touch(UNINDEXED_SHA, unknown.key.clone(), ChangeKind::Added),
        ],
        ..GraphIndexArtifact::empty()
    };
    write_graph_artifact(worktree, &graph);

    let commit_index = CommitIndexArtifact {
        schema_version: GRAPH_INDEX_VERSION_TEMPORAL
            .parse()
            .expect("temporal graph index version is numeric"),
        commits,
        refs: [("HEAD".to_string(), UNKNOWN_TARGET_SHA.to_string())].into(),
        indexed_at: "2026-05-20T12:00:00Z".to_string(),
        walk_strategy: WalkStrategy::Reachable,
    };
    spur_graph::store::commit_index::save_artifact(
        worktree,
        ".spur/commit-index.json",
        &commit_index,
    )
    .expect("write commit index artifact");
    spur_graph::store::commit_index::save_pointer(
        worktree,
        &spur_graph::store::commit_index::CommitIndexPointer {
            schema_version: GRAPH_INDEX_VERSION_TEMPORAL
                .parse()
                .expect("temporal graph index version is numeric"),
            artifact_relative_path: ".spur/commit-index.json".to_string(),
            indexed_at: commit_index.indexed_at.clone(),
            refs: commit_index.refs.clone(),
        },
    )
    .expect("write commit index pointer");
}

fn write_graph_without_commit_index(worktree: &Path) {
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
    let graph = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "missing-commit-index-fixture".to_string(),
        graph_content_hash: "missing-commit-index-fixture".to_string(),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: vec![graph_symbol(NEW_ROOT_ID, "bar")],
        symbol_node_ids: node_ids(1),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..GraphIndexArtifact::empty()
    };
    write_graph_artifact(worktree, &graph);
}

fn write_graph_with_empty_snapshots(worktree: &Path) {
    std::fs::create_dir_all(worktree.join(".git")).expect("create git marker");
    let commits = vec![CommitArtifact {
        sha: OLD_SHA.to_string(),
        parents: Vec::new(),
        author_time: 1,
        author_name: String::new(),
        author_email: String::new(),
        summary: "empty snapshots".to_string(),
    }];
    let graph = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "empty-snapshots-fixture".to_string(),
        graph_content_hash: "empty-snapshots-fixture".to_string(),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: vec![graph_symbol(NEW_ROOT_ID, "bar")],
        symbol_node_ids: node_ids(1),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: commits.clone(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..GraphIndexArtifact::empty()
    };
    write_graph_artifact(worktree, &graph);

    let commit_index = CommitIndexArtifact {
        schema_version: GRAPH_INDEX_VERSION_TEMPORAL
            .parse()
            .expect("temporal graph index version is numeric"),
        commits,
        refs: [("HEAD".to_string(), OLD_SHA.to_string())].into(),
        indexed_at: "2026-05-20T12:00:00Z".to_string(),
        walk_strategy: WalkStrategy::Reachable,
    };
    spur_graph::store::commit_index::save_artifact(
        worktree,
        ".spur/commit-index.json",
        &commit_index,
    )
    .expect("write commit index artifact");
    spur_graph::store::commit_index::save_pointer(
        worktree,
        &spur_graph::store::commit_index::CommitIndexPointer {
            schema_version: GRAPH_INDEX_VERSION_TEMPORAL
                .parse()
                .expect("temporal graph index version is numeric"),
            artifact_relative_path: ".spur/commit-index.json".to_string(),
            indexed_at: commit_index.indexed_at.clone(),
            refs: commit_index.refs.clone(),
        },
    )
    .expect("write commit index pointer");
}

fn graph_symbol(id: &str, entity_name: &str) -> GraphSymbolArtifact {
    GraphSymbolArtifact {
        stable_symbol_id: id.to_string(),
        file_path: format!("src/{entity_name}.rs"),
        byte_range: [0, 10],
        line_range: [1, 3],
        entity_name: entity_name.to_string(),
        qualified_name: entity_name.to_string(),
        symbol_kind: "function".to_string(),
        anchor_hash: format!("anchor-{id}"),
        enclosing_scope: None,
    }
}

fn node_ids(count: usize) -> Vec<NodeId> {
    (1..=count).map(|id| NodeId(id as u64)).collect()
}

fn graph_edge(source: &str, target: &str) -> GraphEdgeArtifact {
    GraphEdgeArtifact {
        source_stable_symbol_id: source.to_string(),
        target_stable_symbol_id: Some(target.to_string()),
        target_label: None,
        import_path: None,
        receiver_text: None,
        scope_text: None,
        relation: RelationKind::Calls,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: None,
        bind_method: None,
    }
}

fn snapshot(id: &str, commit: &str, entity_name: &str) -> SymbolSnapshotArtifact {
    SymbolSnapshotArtifact {
        key: SnapshotKey {
            stable_symbol_id: id.to_string(),
            commit: commit.to_string(),
        },
        file_path: format!("src/{entity_name}.rs").into(),
        entity_name: entity_name.to_string(),
        symbol_kind: "function".to_string(),
        enclosing_scope: None,
        byte_range: [0, 10],
        line_range: [1, 3],
        anchor_hash: "shared-anchor".to_string(),
        tokens: vec![entity_name.to_string(), "body".to_string()],
    }
}

fn temporal_touch(commit: &str, key: SnapshotKey, change_kind: ChangeKind) -> TemporalEdgeArtifact {
    TemporalEdgeArtifact {
        source: EdgeEndpoint::Commit {
            sha: commit.to_string(),
        },
        target: EdgeEndpoint::Snapshot { key },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(change_kind),
    }
}

fn temporal_rename(previous: SnapshotKey, next: SnapshotKey) -> TemporalEdgeArtifact {
    TemporalEdgeArtifact {
        source: EdgeEndpoint::Snapshot {
            key: previous.clone(),
        },
        target: EdgeEndpoint::Snapshot { key: next },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(previous))),
    }
}

fn build_real_tools_graph_artifact(worktree: &Path) -> GraphIndexArtifact {
    let root = workspace_root();
    let files = [
        PathBuf::from("crates/spur-core/src/mcp/catalog.rs"),
        PathBuf::from("crates/spur-core/src/mcp/plan.rs"),
        PathBuf::from("crates/spur-graph/src/mcp/mod.rs"),
        PathBuf::from("crates/spur-core/tests/rework_reuse_prior_worktree_e2e.rs"),
    ];
    let facts = build_facts_for_paths(&root, &files).expect("build graph facts for real tools");
    let artifact = artifact_from_facts(&facts, &root).expect("build graph artifact");
    write_graph_artifact(worktree, &artifact);
    artifact
}

fn write_graph_artifact(worktree: &Path, artifact: &GraphIndexArtifact) {
    let artifact_base = worktree.join(".spur/graph");
    let written = write_artifact_parquet(
        artifact,
        &artifact_base,
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    write_current_pointer(worktree, &written).expect("write CURRENT pointer");
}

fn symbol_id(artifact: &GraphIndexArtifact, entity_name: &str) -> String {
    symbol_by_entity(artifact, entity_name)
        .stable_symbol_id
        .clone()
}

fn symbol_by_file_entity_kind<'a>(
    artifact: &'a GraphIndexArtifact,
    file_path: &str,
    entity_name: &str,
    symbol_kind: &str,
) -> &'a GraphSymbolArtifact {
    artifact
        .symbols
        .iter()
        .find(|symbol| {
            symbol.file_path == file_path
                && symbol.entity_name == entity_name
                && symbol.symbol_kind == symbol_kind
        })
        .unwrap_or_else(|| {
            panic!("symbol `{entity_name}` kind `{symbol_kind}` exists in `{file_path}`")
        })
}

fn symbol_by_entity<'a>(
    artifact: &'a GraphIndexArtifact,
    entity_name: &str,
) -> &'a GraphSymbolArtifact {
    artifact
        .symbols
        .iter()
        .find(|symbol| symbol.entity_name == entity_name)
        .unwrap_or_else(|| panic!("symbol `{entity_name}` exists in artifact"))
}

fn file_oid(artifact: &GraphIndexArtifact, file_path: &str) -> String {
    artifact
        .file_manifests
        .iter()
        .find(|entry| entry.path == file_path)
        .unwrap_or_else(|| panic!("manifest exists for `{file_path}`"))
        .content_oid
        .clone()
}

fn source_for_range(worktree: &Path, file_path: &str, start: usize, end: usize) -> String {
    let source = std::fs::read_to_string(worktree.join(file_path)).expect("read fixture source");
    source
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, line)| {
            let line_no = index + 1;
            (start <= line_no && line_no <= end).then_some(line)
        })
        .collect()
}

fn total_lines(worktree: &Path, file_path: &str) -> usize {
    std::fs::read_to_string(worktree.join(file_path))
        .expect("read fixture source")
        .lines()
        .count()
}

async fn call_tool(server: &McpCallbackServer, tool: &str, arguments: Value) -> Value {
    server.__test_call_tool(tool, arguments).await
}

struct NullWorkerSink;

impl McpEventSink for NullWorkerSink {
    fn emit(&self, _event: SpurEventBody) {}
}

struct NullWorkerPlanResolver;

#[async_trait]
impl PlanResolver for NullWorkerPlanResolver {
    async fn load_or_project_plan(
        &self,
        plan_id: &str,
    ) -> Result<Arc<TokioMutex<spur_core::plan::PlanState>>, String> {
        Err(format!("test resolver: unknown plan_id '{plan_id}'"))
    }
}

fn worker_mcp_deps(pm: Arc<PmService>) -> WorkerMcpDeps {
    let funnel: Arc<dyn McpEventSink> = Arc::new(NullWorkerSink);
    let feature_gate = community_feature_gate();
    let outcome_store: Arc<dyn spur_blob_store::OutcomeStore> =
        Arc::new(spur_blob_store::MemoryOutcomeStore::new());
    let worker_signal_sink = Arc::new(common::TestWorkerSignalSink::new(Arc::clone(&funnel)));
    let worker_read_sink = Arc::new(spur_core::mcp::worker::WorkerReadMcpModule::new(
        spur_core::mcp::worker::WorkerReadMcpDeps {
            pm_service: Some(Arc::clone(&pm)),
            feature_gate: Arc::clone(&feature_gate),
            plan_resolver: Arc::new(NullWorkerPlanResolver),
            reconciler_outcomes: Arc::new(TokioMutex::new(
                spur_core::plan::outcomes::OutcomeStore::default(),
            )),
            outcome_store,
            repo_root: None,
        },
    ));
    WorkerMcpDeps {
        pm_service: pm,
        feature_gate,
        funnel,
        worker_signal_sink,
        worker_read_sink,
        repo_root: None,
    }
}

async fn call_worker_tool(
    server: &Arc<WorkerMcpServer>,
    token: &str,
    tool_name: &str,
    arguments: Value,
) -> Value {
    let config =
        StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token.to_string());
    let client =
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
            .expect("rmcp client initialize");

    let mut request = CallToolRequestParams::new(tool_name.to_string());
    request.arguments = arguments.as_object().cloned();
    let response = match client.call_tool(request).await {
        Ok(result) => result
            .structured_content
            .clone()
            .unwrap_or_else(|| serde_json::to_value(result).expect("serialize result")),
        Err(error) => worker_service_error_response(error),
    };
    drop(client);
    response
}

fn worker_service_error_response(error: ServiceError) -> Value {
    let (code, message) = match &error {
        ServiceError::McpError(mcp_error) => {
            (i64::from(mcp_error.code.0), mcp_error.message.to_string())
        }
        _ => (-32603, error.to_string()),
    };
    json!({
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn tool_body(response: Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("successful tool response with text content: {response}"));
    serde_json::from_str(text).expect("tool text is JSON")
}

fn error_data(response: &Value) -> &Value {
    response["error"]["data"]
        .as_object()
        .unwrap_or_else(|| panic!("JSON-RPC error has structured data: {response}"));
    &response["error"]["data"]
}

fn assert_successful_tool_response(response: &Value) {
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        !message.contains("graph artifact not found"),
        "tool should not return graph_artifact_missing: {response}"
    );
    assert!(
        response.get("error").is_none(),
        "tool should succeed: {response}"
    );
}

fn assert_code_symbol_history_rename_chain(body: &Value, backend: CodeGraphFixtureBackend) {
    let keys = body
        .as_object()
        .unwrap_or_else(|| panic!("symbol history body is object for {backend:?}: {body}"))
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        keys.contains("events") && keys.contains("symbol"),
        "symbol history body has required keys for {backend:?}: {keys:?}"
    );
    assert_eq!(
        body["symbol"],
        format!("graph://symbol/{NEW_ROOT_ID}"),
        "unexpected symbol URI for {backend:?}"
    );

    let events = body["events"]
        .as_array()
        .unwrap_or_else(|| panic!("history events are an array for {backend:?}: {body}"));
    assert_eq!(events.len(), 2, "unexpected event count for {backend:?}");
    for event in events {
        let event_keys = event
            .as_object()
            .unwrap_or_else(|| panic!("history event is object for {backend:?}: {event}"))
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            event_keys,
            BTreeSet::from([
                "author_email".to_string(),
                "author_name".to_string(),
                "author_time".to_string(),
                "change_kind".to_string(),
                "commit".to_string(),
                "summary".to_string(),
                "snapshot".to_string(),
            ]),
            "unexpected event keys for {backend:?}"
        );
    }

    assert_eq!(events[0]["commit"], OLD_SHA);
    assert_eq!(events[0]["author_time"], 1);
    assert_eq!(events[0]["author_name"], "Ada Lovelace");
    assert_eq!(events[0]["author_email"], "ada@example.test");
    assert_eq!(events[0]["summary"], "add foo");
    assert_eq!(events[0]["change_kind"], "added");
    assert_eq!(events[0]["snapshot"]["stable_symbol_id"], OLD_ROOT_ID);
    assert_eq!(events[1]["commit"], NEW_SHA);
    assert_eq!(events[1]["author_time"], 2);
    assert_eq!(events[1]["author_name"], "Grace Hopper");
    assert_eq!(events[1]["author_email"], "grace@example.test");
    assert_eq!(events[1]["summary"], "rename foo to bar");
    assert_eq!(
        events[1]["change_kind"]["renamed_from"]["symbol"]["stable_symbol_id"],
        OLD_ROOT_ID
    );
    assert_eq!(events[1]["snapshot"]["stable_symbol_id"], NEW_ROOT_ID);
}

fn entity_names(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .map(|row| {
            row["entity_name"]
                .as_str()
                .expect("row has entity_name")
                .to_string()
        })
        .collect()
}

fn node_entity_names(body: &Value) -> BTreeSet<String> {
    entity_names(body["nodes"].as_array().expect("nodes"))
}

fn candidate_entity_names(body: &Value) -> BTreeSet<String> {
    entity_names(body["candidates"].as_array().expect("candidates"))
}

fn qualified_names(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .map(|row| {
            row["qualified_name"]
                .as_str()
                .expect("row has qualified_name")
                .to_string()
        })
        .collect()
}

fn table_col_index(table: &Value, col: &str) -> usize {
    table["cols"]
        .as_array()
        .unwrap_or_else(|| panic!("table has cols array: {table}"))
        .iter()
        .position(|value| value == col)
        .unwrap_or_else(|| panic!("table has column `{col}`: {table}"))
}

fn assert_omits_full_graph_identity_metadata(body: &Value) {
    for key in [
        "graph_content_hash",
        "graph_index_version",
        "graph_built_at",
    ] {
        assert!(
            body.get(key).is_none(),
            "table response should omit full graph metadata key {key}: {body}"
        );
    }
}

fn assert_omits_full_metadata_defaults(body: &Value) {
    assert_omits_full_graph_identity_metadata(body);
    for key in [
        "indexed_head_oid",
        "worktree_head_oid",
        "worktree_dirty",
        "response_file_oids_match",
        "rebuild_status",
    ] {
        assert!(
            body.get(key).is_none(),
            "table response should omit full metadata key {key}: {body}"
        );
    }
}

#[tokio::test]
async fn linked_worktree_without_self_graph_overlays_root_for_code_search_and_callers() {
    let _lock = cwd_lock().await;
    let parent = TempDir::new().expect("temp parent");
    let main = parent.path().join("main");
    let worker = parent.path().join("worker");
    std::fs::create_dir(&main).expect("create main checkout");
    copy_fixture_crate(&main);
    commit_fixture(&main);
    build_graph_artifact_with_pointer(&main);
    add_linked_worktree(&main, &worker, "worker-overlay-identical");
    assert_no_graph_artifact(&worker);
    let _cwd = enter_dir(&worker);
    let server = test_server();

    let search_response = call_tool(
        &server,
        "code_search",
        json!({
            "query": ROOT_SYMBOL,
            "mode": "exact",
            "limit": 20
        }),
    )
    .await;
    assert_successful_tool_response(&search_response);
    let search = tool_body(search_response);
    assert!(candidate_entity_names(&search).contains(ROOT_SYMBOL));

    let callers_response =
        call_tool(&server, "code_callers", json!({ "selector": ROOT_SYMBOL })).await;
    assert_successful_tool_response(&callers_response);
    let callers = tool_body(callers_response);
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from(["launch_order".to_string()])
    );
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn linked_worktree_without_self_graph_overlays_modified_file_for_code_search_and_callers() {
    let _lock = cwd_lock().await;
    let parent = TempDir::new().expect("temp parent");
    let main = parent.path().join("main");
    let worker = parent.path().join("worker");
    std::fs::create_dir(&main).expect("create main checkout");
    copy_fixture_crate(&main);
    commit_fixture(&main);
    build_graph_artifact_with_pointer(&main);
    add_linked_worktree(&main, &worker, "worker-overlay-modified");
    assert_no_graph_artifact(&worker);
    let worker_source_path = worker.join("src/lib.rs");
    let mut worker_source =
        std::fs::read_to_string(&worker_source_path).expect("read worker fixture source");
    worker_source
        .push_str("\npub fn worker_overlay_symbol() -> bool {\n    orchestrate_order()\n}\n");
    std::fs::write(&worker_source_path, worker_source).expect("write worker overlay source");
    let _cwd = enter_dir(&worker);
    let server = test_server();

    let search_response = call_tool(
        &server,
        "code_search",
        json!({
            "query": "worker_overlay_symbol",
            "mode": "exact",
            "limit": 20
        }),
    )
    .await;
    assert_successful_tool_response(&search_response);
    let search = tool_body(search_response);
    assert!(candidate_entity_names(&search).contains("worker_overlay_symbol"));

    let callers_response = call_tool(
        &server,
        "code_callers",
        json!({ "selector": ROOT_SYMBOL, "include_unresolved": true }),
    )
    .await;
    assert_successful_tool_response(&callers_response);
    let callers = tool_body(callers_response);
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from([
            "launch_order".to_string(),
            "worker_overlay_symbol".to_string()
        ])
    );
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 1);
}

#[tokio::test]
async fn linked_worktree_cold_open_modified_file_awaits_overlay_past_latency_budget() {
    let _lock = cwd_lock().await;
    let parent = TempDir::new().expect("temp parent");
    let main = parent.path().join("main");
    let worker = parent.path().join("worker");
    std::fs::create_dir(&main).expect("create main checkout");
    copy_fixture_crate(&main);
    commit_fixture(&main);
    build_graph_artifact_with_pointer(&main);
    add_linked_worktree(&main, &worker, "worker-overlay-budget");
    assert_no_graph_artifact(&worker);
    let worker_source_path = worker.join("src/lib.rs");
    let mut worker_source =
        std::fs::read_to_string(&worker_source_path).expect("read worker fixture source");
    worker_source.push_str("\npub fn budget_overlay_symbol() -> bool {\n    true\n}\n");
    std::fs::write(&worker_source_path, worker_source).expect("write worker overlay source");
    let _cwd = enter_dir(&worker);
    let server = test_server();
    let _budget = server.__test_set_code_graph_rebuild_budget(std::time::Duration::from_millis(1));
    let _delay = server.__test_set_code_graph_rebuild_delay(std::time::Duration::from_millis(25));

    let search_response = call_tool(
        &server,
        "code_search",
        json!({
            "query": "budget_overlay_symbol",
            "mode": "exact",
            "limit": 20
        }),
    )
    .await;
    assert_successful_tool_response(&search_response);
    let search = tool_body(search_response);
    assert!(candidate_entity_names(&search).contains("budget_overlay_symbol"));
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 1);
}

#[tokio::test]
async fn code_search_uses_overlay_for_unsaved_edit_without_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    let mut source =
        std::fs::read_to_string(worktree.path().join("src/lib.rs")).expect("read fixture source");
    source.push_str("\npub fn freshly_indexed_unsaved_symbol() -> bool {\n    true\n}\n");
    std::fs::write(worktree.path().join("src/lib.rs"), source).expect("write unsaved fixture edit");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "freshly_indexed_unsaved_symbol",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert!(candidate_entity_names(&body).contains("freshly_indexed_unsaved_symbol"));
    assert_eq!(body["worktree_dirty"], false);
    assert_eq!(body["response_file_oids_match"], true);
    assert_eq!(body["rebuild_status"], "fresh");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_search_uses_overlay_for_untracked_source_without_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    std::fs::write(
        worktree.path().join("src/untracked_overlay.rs"),
        "pub fn untracked_overlay_symbol() -> bool {\n    true\n}\n",
    )
    .expect("write untracked source file");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "untracked_overlay_symbol",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert!(candidate_entity_names(&body).contains("untracked_overlay_symbol"));
    assert_eq!(body["rebuild_status"], "fresh");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_search_ignores_untracked_non_source_scratch_file() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    std::fs::write(
        worktree.path().join("scratch-output.log"),
        "untracked scratch output\n",
    )
    .expect("write untracked scratch file");
    std::fs::write(
        worktree.path().join("scratch-notes.md"),
        "# Scratch notes\n\nuntracked scratch output\n",
    )
    .expect("write untracked markdown scratch file");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "orchestrate_order",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert_eq!(
        candidate_entity_names(&body),
        BTreeSet::from(["orchestrate_order".to_string()])
    );
    assert_eq!(body["rebuild_status"], "not_needed");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_search_uses_overlay_for_committed_unindexed_source_without_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact_with_pointer(worktree.path());
    std::fs::write(
        worktree.path().join("src/committed_overlay.rs"),
        "pub fn committed_overlay_symbol() -> bool {\n    true\n}\n",
    )
    .expect("write committed source file");
    git(worktree.path(), &["add", "src/committed_overlay.rs"]);
    git(
        worktree.path(),
        &["commit", "-m", "add committed overlay source"],
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "committed_overlay_symbol",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert!(candidate_entity_names(&body).contains("committed_overlay_symbol"));
    assert_eq!(body["rebuild_status"], "fresh");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn root_parquet_self_pointer_uses_overlay_for_unsaved_edit_without_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact_with_pointer(worktree.path());
    let mut source =
        std::fs::read_to_string(worktree.path().join("src/lib.rs")).expect("read fixture source");
    source
        .push_str("\npub fn root_overlay_unsaved_symbol() -> bool {\n    orchestrate_order()\n}\n");
    std::fs::write(worktree.path().join("src/lib.rs"), source).expect("write unsaved fixture edit");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let search = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "root_overlay_unsaved_symbol",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );
    assert!(candidate_entity_names(&search).contains("root_overlay_unsaved_symbol"));
    assert_eq!(search["worktree_dirty"], false);
    assert_eq!(search["response_file_oids_match"], true);
    assert_eq!(search["rebuild_status"], "fresh");

    let callers = tool_body(
        call_tool(
            &server,
            "code_callers",
            json!({ "selector": ROOT_SYMBOL, "include_unresolved": true }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from([
            "launch_order".to_string(),
            "root_overlay_unsaved_symbol".to_string()
        ])
    );
    assert_eq!(callers["worktree_dirty"], false);
    assert_eq!(callers["response_file_oids_match"], true);
    assert_eq!(callers["rebuild_status"], "fresh");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_search_unrelated_edit_does_not_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_multi_file_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    std::fs::write(
        worktree.path().join("src/support.rs"),
        "pub fn unrelated_support_symbol() -> bool {\n    false\n}\n",
    )
    .expect("write unrelated fixture edit");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "orchestrate_order",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert_eq!(
        candidate_entity_names(&body),
        BTreeSet::from(["orchestrate_order".to_string()])
    );
    assert_eq!(body["rebuild_status"], "not_needed");
    assert_eq!(body["response_file_oids_match"], true);
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn worker_code_search_freshness_uses_registered_worktree_root() {
    skip_if_no_loopback!("worker_code_search_freshness_uses_registered_worktree_root");
    let _lock = cwd_lock().await;
    let parent = TempDir::new().expect("temp parent");
    let main = parent.path().join("main");
    let worker = parent.path().join("worker");
    std::fs::create_dir(&main).expect("create main checkout");
    copy_fixture_crate(&main);
    commit_fixture(&main);
    build_graph_artifact(&main);

    let worker_arg = worker.to_str().expect("worker path is UTF-8");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "worker-code-graph",
            worker_arg,
            "HEAD",
        ],
    );
    build_graph_artifact(&worker);
    let worker_source_path = worker.join("src/lib.rs");
    let mut worker_source =
        std::fs::read_to_string(&worker_source_path).expect("read worker fixture source");
    worker_source.push_str("\n// worker-local dirty edit\n");
    std::fs::write(&worker_source_path, worker_source).expect("dirty worker fixture source");

    let (_beads, pm) = common::beads::init_beads_pm(&main).await;
    let _cwd = enter_dir(&main);
    let server = WorkerMcpServer::start("worker-code-graph-brain".into(), worker_mcp_deps(pm))
        .await
        .expect("worker MCP server starts");
    let delegation_id = "worker-code-graph-delegation";
    server.register_delegation_worktree_root(delegation_id.to_string(), worker.clone());
    let token = server.issue_token(delegation_id, Duration::from_secs(60));

    let body = call_worker_tool(
        &server,
        &token,
        "code_symbol_search",
        json!({
            "query": ROOT_SYMBOL,
            "mode": "exact",
            "limit": 20
        }),
    )
    .await;

    assert!(
        body.get("error").is_none(),
        "worker code_symbol_search should succeed: {body}"
    );
    assert!(candidate_entity_names(&body).contains(ROOT_SYMBOL));
    assert_eq!(
        body["response_file_oids_match"], true,
        "worker code search should refresh against the registered worker worktree, not the main cwd"
    );
    assert_eq!(body["rebuild_status"], "fresh");
}

#[tokio::test]
async fn worker_linked_worktree_without_self_graph_overlays_root_for_code_search_and_callers() {
    skip_if_no_loopback!(
        "worker_linked_worktree_without_self_graph_overlays_root_for_code_search_and_callers"
    );
    let _lock = cwd_lock().await;
    let parent = TempDir::new().expect("temp parent");
    let main = parent.path().join("main");
    let worker = parent.path().join("worker");
    std::fs::create_dir(&main).expect("create main checkout");
    copy_fixture_crate(&main);
    commit_fixture(&main);
    build_graph_artifact(&main);
    add_linked_worktree(&main, &worker, "worker-overlay-http-identical");
    assert_no_graph_artifact(&worker);

    let (_beads, pm) = common::beads::init_beads_pm(&main).await;
    let _cwd = enter_dir(&main);
    let server = WorkerMcpServer::start("worker-overlay-http-brain".into(), worker_mcp_deps(pm))
        .await
        .expect("worker MCP server starts");
    let delegation_id = "worker-overlay-http-delegation";
    server.register_delegation_worktree_root(delegation_id.to_string(), worker.clone());
    let token = server.issue_token(delegation_id, Duration::from_secs(60));

    let search = call_worker_tool(
        &server,
        &token,
        "code_symbol_search",
        json!({
            "query": ROOT_SYMBOL,
            "mode": "exact",
            "limit": 20
        }),
    )
    .await;
    assert!(
        search.get("error").is_none(),
        "worker code_symbol_search should overlay root graph, got: {search}"
    );
    assert!(candidate_entity_names(&search).contains(ROOT_SYMBOL));

    let callers = call_worker_tool(
        &server,
        &token,
        "code_callers",
        json!({ "selector": ROOT_SYMBOL }),
    )
    .await;
    assert!(
        callers.get("error").is_none(),
        "worker code_callers should overlay root graph, got: {callers}"
    );
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from(["launch_order".to_string()])
    );
}

#[tokio::test]
async fn code_search_concurrent_dirty_requests_use_overlay_without_rebuild() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    let mut source =
        std::fs::read_to_string(worktree.path().join("src/lib.rs")).expect("read fixture source");
    source.push_str("\npub fn concurrent_unsaved_symbol() -> bool {\n    true\n}\n");
    std::fs::write(worktree.path().join("src/lib.rs"), source).expect("write unsaved fixture edit");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();
    let _delay = server.__test_set_code_graph_rebuild_delay(std::time::Duration::from_millis(50));

    let search_args = json!({
        "query": "concurrent_unsaved_symbol",
        "mode": "exact",
        "limit": 20
    });
    let (first, second, third, fourth) = tokio::join!(
        call_tool(&server, "code_search", search_args.clone()),
        call_tool(&server, "code_search", search_args.clone()),
        call_tool(&server, "code_search", search_args.clone()),
        call_tool(&server, "code_search", search_args.clone()),
    );

    for response in [first, second, third, fourth] {
        let body = tool_body(response);
        assert_ne!(body["rebuild_status"], "stale_rebuild_failed");
    }
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);

    // Overlay extract continues after the latency budget and fills the
    // per-worktree cache (solve_id sol_bdb8579701c64516: wait ≤ 750ms).
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(750);
    let cached = loop {
        let body = tool_body(call_tool(&server, "code_search", search_args.clone()).await);
        if candidate_entity_names(&body).contains("concurrent_unsaved_symbol") {
            break body;
        }
        assert_eq!(
            server.__test_code_graph_rebuild_invocation_count(),
            0,
            "overlay timeout must not fall through to rebuild"
        );
        if std::time::Instant::now() >= deadline {
            panic!("overlay cache did not populate after rebuild budget: {body}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    assert_eq!(cached["rebuild_status"], "fresh");
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_search_overlay_exceeds_rebuild_budget_serves_stale_without_rebuild() {
    // Overlay wait is the same latency budget as full rebuild
    // (`DEFAULT_GRAPH_REBUILD_LATENCY_BUDGET` = 750ms; solve_id sol_bdb8579701c64516).
    // Sequential overlay-then-rebuild (1500ms) is unsat against that limit
    // (solve_id sol_b21b0c5f71254ae0), so timeout must not fall through to rebuild.
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    let mut source =
        std::fs::read_to_string(worktree.path().join("src/lib.rs")).expect("read fixture source");
    source.push_str("\npub fn budget_exceeded_unsaved_symbol() -> bool {\n    true\n}\n");
    std::fs::write(worktree.path().join("src/lib.rs"), source).expect("write unsaved fixture edit");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();
    let _budget = server.__test_set_code_graph_rebuild_budget(std::time::Duration::from_millis(0));

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "budget_exceeded_unsaved_symbol",
                "mode": "exact",
                "limit": 20
            }),
        )
        .await,
    );

    assert_eq!(body["rebuild_status"], "stale_budget_exceeded");
    assert!(
        !candidate_entity_names(&body).contains("budget_exceeded_unsaved_symbol"),
        "timed-out overlay must serve the stale parquet, not wait for extract: {body}"
    );
    assert_eq!(server.__test_code_graph_rebuild_invocation_count(), 0);
}

#[tokio::test]
async fn code_graph_tools_traverse_artifact_built_from_real_rust_fixture() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let root_uri = format!("graph://symbol/{root_id}");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let callers = tool_body(
        call_tool(
            &server,
            "code_callers",
            json!({ "symbol": root_uri.clone() }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from(["launch_order".to_string()])
    );
    assert_eq!(callers["callers"][0]["edge_kind"], "calls");
    assert_eq!(callers["include_unresolved"], false);
    assert_eq!(callers["counts_by_kind"]["calls"], 1);
    assert_eq!(callers["counts_by_kind"]["unresolved"], 0);
    assert_eq!(callers["unresolved_sample"], json!([]));
    assert_eq!(callers["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        callers["graph_index_version"],
        artifact.header.graph_index_version
    );

    let callees = tool_body(
        call_tool(
            &server,
            "code_callees",
            json!({ "symbol": root_id.clone() }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callees["callees"].as_array().expect("callees")),
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    assert!(callees["callees"]
        .as_array()
        .expect("callees")
        .iter()
        .all(|row| row["edge_kind"] == "calls"));
    assert_eq!(callees["include_unresolved"], false);
    assert_eq!(callees["counts_by_kind"]["calls"], 2);
    assert_eq!(callees["counts_by_kind"]["unresolved"], 0);

    let selector_callees =
        tool_body(call_tool(&server, "code_callees", json!({ "selector": ROOT_SYMBOL })).await);
    assert_eq!(
        entity_names(
            selector_callees["callees"]
                .as_array()
                .expect("selector callees")
        ),
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    assert_eq!(
        selector_callees["graph_content_hash"],
        artifact.graph_content_hash
    );
    assert_eq!(
        selector_callees["graph_index_version"],
        artifact.header.graph_index_version
    );

    let radius_one = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({ "symbol": root_uri, "radius": 1, "edge_kinds": ["calls"] }),
        )
        .await,
    );
    assert_eq!(
        entity_names(radius_one["nodes"].as_array().expect("radius one nodes")),
        BTreeSet::from([
            "charge_order".to_string(),
            "launch_order".to_string(),
            "orchestrate_order".to_string(),
            "parse_order".to_string(),
        ])
    );
    assert_eq!(
        radius_one["edges"]
            .as_array()
            .expect("radius one edges")
            .len(),
        3
    );
    assert!(radius_one["edges"]
        .as_array()
        .expect("radius one edges")
        .iter()
        .all(|edge| edge["edge_kind"] == "calls"));
    assert_eq!(radius_one["include_unresolved"], false);

    let radius_zero = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({ "symbol": root_id, "radius": 0, "edge_kinds": ["calls"] }),
        )
        .await,
    );
    assert_eq!(
        entity_names(radius_zero["nodes"].as_array().expect("radius zero nodes")),
        BTreeSet::from(["orchestrate_order".to_string()])
    );
    assert!(radius_zero["edges"]
        .as_array()
        .expect("radius zero edges")
        .is_empty());

    let unknown = call_tool(
        &server,
        "code_callers",
        json!({ "symbol": "graph://symbol/not-in-artifact" }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32004);
    assert!(unknown["error"]["message"]
        .as_str()
        .expect("unknown error message")
        .contains("symbol not-in-artifact not found in graph artifact"));
    assert_eq!(
        unknown["error"]["data"]["graph_content_hash"],
        artifact.graph_content_hash
    );
    assert_eq!(
        unknown["error"]["data"]["graph_index_version"],
        artifact.header.graph_index_version
    );
}

#[tokio::test]
async fn code_graph_tools_accept_real_sixteen_hex_legacy_symbol_id() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    assert_eq!(root_id.len(), 16);
    assert!(root_id
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let callers = tool_body(
        call_tool(
            &server,
            "code_callers",
            json!({ "symbol": root_id.clone() }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from(["launch_order".to_string()])
    );

    let callees = tool_body(call_tool(&server, "code_callees", json!({ "symbol": root_id })).await);
    assert_eq!(
        entity_names(callees["callees"].as_array().expect("callees")),
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    assert_eq!(callees["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        callees["graph_index_version"],
        artifact.header.graph_index_version
    );
}

#[tokio::test]
async fn code_graph_response_format_omitted_keeps_full_metadata() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(call_tool(&server, "code_callees", json!({ "symbol": root_id })).await);

    assert_eq!(
        entity_names(body["callees"].as_array().expect("callees")),
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        body["graph_index_version"],
        artifact.header.graph_index_version
    );
    for key in [
        "graph_built_at",
        "indexed_head_oid",
        "worktree_head_oid",
        "worktree_dirty",
        "response_file_oids_match",
        "rebuild_status",
    ] {
        assert!(body.get(key).is_some(), "missing full metadata key {key}");
    }
    assert!(body.get("response_format").is_none());
}

#[tokio::test]
async fn code_graph_compact_metadata_omits_healthy_defaults() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_callees",
            json!({ "symbol": root_id, "response_format": "compact" }),
        )
        .await,
    );

    assert_eq!(
        entity_names(body["callees"].as_array().expect("callees")),
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    for key in [
        "graph_content_hash",
        "graph_index_version",
        "graph_built_at",
        "indexed_head_oid",
        "worktree_head_oid",
        "worktree_dirty",
        "response_file_oids_match",
        "rebuild_status",
    ] {
        assert!(
            body.get(key).is_none(),
            "compact response should omit healthy metadata key {key}: {body}"
        );
    }
}

#[tokio::test]
async fn code_graph_compact_metadata_preserves_stale_dirty_status() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    std::fs::write(
        worktree.path().join("src/lib.rs"),
        "pub fn edited_after_index() -> bool {\n    false\n}\n",
    )
    .expect("edit fixture after graph build");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "stable_symbol_id": root.stable_symbol_id, "response_format": "compact" }),
        )
        .await,
    );

    assert_eq!(body["stale"], true);
    assert_eq!(body["symbol"]["id"], root.stable_symbol_id);
    assert_eq!(body["source"], expected_source);
    assert_eq!(body["worktree_dirty"], true);
    assert_eq!(body["response_file_oids_match"], false);
    for key in [
        "graph_content_hash",
        "graph_index_version",
        "graph_built_at",
        "indexed_head_oid",
        "worktree_head_oid",
        "rebuild_status",
    ] {
        assert!(
            body.get(key).is_none(),
            "compact response should omit non-actionable metadata key {key}: {body}"
        );
    }
}

#[tokio::test]
async fn code_file_symbols_table_format_interns_file_paths_and_reduces_size() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let full = tool_body(
        call_tool(
            &server,
            "code_file_symbols",
            json!({ "file": "src/lib.rs" }),
        )
        .await,
    );
    let table = tool_body(
        call_tool(
            &server,
            "code_file_symbols",
            json!({ "file": "src/lib.rs", "response_format": "table" }),
        )
        .await,
    );

    assert_eq!(table["response_format"], "table");
    assert_omits_full_metadata_defaults(&table);
    assert_eq!(table["files"], json!(["src/lib.rs"]));
    let symbols = &table["symbols"];
    assert_eq!(
        symbols["cols"],
        json!([
            "id",
            "entity_name",
            "qualified_name",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
            "enclosing_scope"
        ])
    );
    let rows = symbols["rows"].as_array().expect("symbol table rows");
    assert_eq!(rows.len(), 5);
    let file_col = table_col_index(symbols, "file");
    assert!(rows.iter().all(|row| row[file_col] == 0));

    let rows_text = serde_json::to_string(rows).expect("serialize table rows");
    assert!(!rows_text.contains("src/lib.rs"));
    assert!(!rows_text.contains("graph://symbol/"));
    assert!(
        serde_json::to_string(&table)
            .expect("serialize table")
            .len()
            < serde_json::to_string(&full).expect("serialize full").len()
    );
}

#[tokio::test]
async fn code_callers_and_callees_table_format_returns_rows_and_counts() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let launch_id = symbol_id(&artifact, "launch_order");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let callers = tool_body(
        call_tool(
            &server,
            "code_callers",
            json!({ "symbol": root_id.clone(), "response_format": "table" }),
        )
        .await,
    );
    assert_eq!(callers["response_format"], "table");
    assert_omits_full_metadata_defaults(&callers);
    assert_eq!(callers["include_unresolved"], false);
    assert_eq!(callers["counts_by_kind"]["calls"], 1);
    assert_eq!(callers["unresolved_sample"], json!([]));
    assert_eq!(callers["files"], json!(["src/lib.rs"]));
    let caller_table = &callers["callers"];
    assert_eq!(
        caller_table["cols"],
        json!([
            "symbol_id",
            "entity_name",
            "enclosing_scope",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
            "resolved",
            "target_label",
            "edge_kind",
            "confidence",
            "bind_method"
        ])
    );
    let caller_rows = caller_table["rows"].as_array().expect("caller rows");
    assert_eq!(caller_rows.len(), 1);
    assert_eq!(
        caller_rows[0][table_col_index(caller_table, "symbol_id")],
        launch_id
    );
    assert_eq!(
        caller_rows[0][table_col_index(caller_table, "entity_name")],
        "launch_order"
    );
    assert_eq!(
        caller_rows[0][table_col_index(caller_table, "resolved")],
        true
    );

    let callees = tool_body(
        call_tool(
            &server,
            "code_callees",
            json!({ "symbol": root_id, "response_format": "table" }),
        )
        .await,
    );
    assert_eq!(callees["response_format"], "table");
    assert_omits_full_metadata_defaults(&callees);
    assert_eq!(callees["include_unresolved"], false);
    assert_eq!(callees["counts_by_kind"]["calls"], 2);
    let callee_table = &callees["callees"];
    let name_col = table_col_index(callee_table, "entity_name");
    let names = callee_table["rows"]
        .as_array()
        .expect("callee rows")
        .iter()
        .map(|row| row[name_col].as_str().expect("callee entity").to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from(["charge_order".to_string(), "parse_order".to_string()])
    );
    let callee_rows_text =
        serde_json::to_string(&callee_table["rows"]).expect("serialize callee rows");
    assert!(!callee_rows_text.contains("graph://symbol/"));
    assert!(!callee_rows_text.contains("src/lib.rs"));
}

#[tokio::test]
async fn code_subgraph_table_format_returns_node_edge_tables_and_unresolved_edges() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    std::fs::create_dir_all(worktree.path().join(".git")).expect("create git marker");
    let mut unresolved = graph_edge("root", "missing-target");
    unresolved.target_stable_symbol_id = None;
    unresolved.target_label = Some("external_charge".to_string());
    let artifact = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "table-unresolved-fixture".to_string(),
        graph_content_hash: "table-unresolved-fixture".to_string(),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: vec![
            graph_symbol("root", "root_node"),
            graph_symbol("caller", "caller_node"),
        ],
        symbol_node_ids: node_ids(2),
        edges: vec![graph_edge("caller", "root"), unresolved],
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..GraphIndexArtifact::empty()
    };
    write_graph_artifact(worktree.path(), &artifact);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let full = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({ "symbol": "graph://symbol/root", "radius": 1, "include_unresolved": true }),
        )
        .await,
    );
    let table = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "symbol": "graph://symbol/root",
                "radius": 1,
                "include_unresolved": true,
                "response_format": "table"
            }),
        )
        .await,
    );

    assert_eq!(table["response_format"], "table");
    assert_omits_full_graph_identity_metadata(&table);
    assert_eq!(table["include_unresolved"], true);
    assert_eq!(table["truncated_frontier"], json!([]));
    assert_eq!(table["metadata"]["radius"], 1);
    assert_eq!(
        table["files"],
        json!(["src/root_node.rs", "src/caller_node.rs"])
    );
    assert_eq!(
        table["nodes"]["cols"],
        json!([
            "id",
            "entity_name",
            "enclosing_scope",
            "file",
            "line_start",
            "line_end",
            "symbol_kind"
        ])
    );
    assert_eq!(
        table["edges"]["cols"],
        json!([
            "source_id",
            "target_id",
            "target_label",
            "resolved",
            "relation",
            "edge_kind",
            "confidence",
            "confidence_score",
            "bind_method"
        ])
    );
    let edge_table = &table["edges"];
    let target_label_col = table_col_index(edge_table, "target_label");
    let unresolved_row = edge_table["rows"]
        .as_array()
        .expect("edge rows")
        .iter()
        .find(|row| row[target_label_col] == "external_charge")
        .expect("unresolved edge row");
    assert_eq!(
        unresolved_row[table_col_index(edge_table, "target_id")],
        Value::Null
    );
    assert_eq!(
        unresolved_row[table_col_index(edge_table, "resolved")],
        false
    );

    let rows_text = serde_json::to_string(&table["edges"]["rows"]).expect("serialize edge rows");
    assert!(!rows_text.contains("graph://symbol/"));
    assert!(
        serde_json::to_string(&table)
            .expect("serialize table")
            .len()
            < serde_json::to_string(&full).expect("serialize full").len()
    );
}

#[tokio::test]
async fn code_graph_response_format_invalid_returns_invalid_params() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let invalid = call_tool(
        &server,
        "code_callees",
        json!({ "symbol": root_id, "response_format": "dense" }),
    )
    .await;

    assert_eq!(invalid["error"]["code"], -32602);
    let message = invalid["error"]["message"]
        .as_str()
        .expect("invalid params message");
    assert!(
        message.contains("invalid response_format `dense`"),
        "unexpected invalid response_format message: {message}"
    );
    assert!(
        message.contains("full") && message.contains("compact") && message.contains("table"),
        "invalid response_format message should list accepted formats: {message}"
    );
    assert!(
        !message.contains("source"),
        "non-source tools should not advertise source response_format: {message}"
    );
}

#[tokio::test]
async fn code_graph_source_format_is_invalid_for_non_read_symbol_tools() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let invalid = call_tool(
        &server,
        "code_callees",
        json!({ "symbol": root_id, "response_format": "source" }),
    )
    .await;

    assert_eq!(invalid["error"]["code"], -32602);
    let message = invalid["error"]["message"]
        .as_str()
        .expect("invalid params message");
    assert!(
        message.contains("invalid response_format `source`"),
        "unexpected invalid response_format message: {message}"
    );
    assert!(
        message.contains("full") && message.contains("compact") && message.contains("table"),
        "invalid response_format message should list accepted formats: {message}"
    );
}

#[tokio::test]
async fn code_subgraph_frontier_start_nodes_resume_budgeted_exploration() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_wide_fixture_crate(worktree.path(), 55);
    let artifact = build_graph_artifact(worktree.path());
    let root_id = symbol_id(&artifact, "wide_root");
    let root_uri = format!("graph://symbol/{root_id}");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let unbudgeted = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "symbol": root_uri,
                "radius": 1,
                "edge_kinds": ["calls"],
                "max_nodes": 400,
                "max_edges": 1200
            }),
        )
        .await,
    );

    let initial = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "symbol": root_id,
                "radius": 1,
                "edge_kinds": ["calls"],
                "max_nodes": 20,
                "max_edges": 1200
            }),
        )
        .await,
    );
    let frontier = initial["truncated_frontier"]
        .as_array()
        .expect("truncated_frontier")
        .clone();
    assert_eq!(initial["metadata"]["truncated"], true);
    assert_eq!(
        initial["nodes"].as_array().expect("initial nodes").len(),
        20
    );
    assert!(!frontier.is_empty());

    let continuation = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "start_nodes": frontier,
                "radius": 0,
                "edge_kinds": ["calls"],
                "max_nodes": 400,
                "max_edges": 1200
            }),
        )
        .await,
    );

    let mut combined = node_entity_names(&initial);
    combined.extend(node_entity_names(&continuation));

    assert_eq!(combined, node_entity_names(&unbudgeted));
    assert_eq!(continuation["metadata"]["truncated"], false);
    assert_eq!(continuation["truncated_frontier"], json!([]));
}

#[tokio::test]
async fn code_resolve_returns_candidate_rows_without_traversal_payloads() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body =
        tool_body(call_tool(&server, "code_resolve", json!({ "selector": ROOT_SYMBOL })).await);
    let candidates = body["candidates"].as_array().expect("candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0]["selector"],
        format!("{}::{}", root.file_path, root.qualified_name)
    );
    assert_eq!(
        candidates[0]["uri"],
        format!("graph://symbol/{}", root.stable_symbol_id)
    );
    assert_eq!(candidates[0]["id"], root.stable_symbol_id);
    assert_eq!(candidates[0]["qualified_name"], root.qualified_name);
    assert_eq!(candidates[0]["file_path"], root.file_path);
    assert_eq!(candidates[0]["line_range"], json!(root.line_range));
    assert_eq!(candidates[0]["symbol_kind"], root.symbol_kind);
    assert!(body.get("callers").is_none());
    assert!(body.get("callees").is_none());
    assert!(body.get("nodes").is_none());
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        body["graph_index_version"],
        artifact.header.graph_index_version
    );
}

#[tokio::test]
async fn code_resolve_prefers_real_submit_plan_mcp_tool_registration() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    let artifact = build_real_tools_graph_artifact(worktree.path());
    let mcp_tool = symbol_by_file_entity_kind(
        &artifact,
        "crates/spur-core/src/mcp/plan.rs",
        "submit_plan",
        "mcp_tool",
    );
    let helper = symbol_by_file_entity_kind(
        &artifact,
        "crates/spur-core/tests/rework_reuse_prior_worktree_e2e.rs",
        "submit_plan",
        "function",
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_resolve",
            json!({ "selector": "submit_plan" }),
        )
        .await,
    );
    let candidates = body["candidates"].as_array().expect("candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["id"], mcp_tool.stable_symbol_id);
    assert_eq!(candidates[0]["symbol_kind"], "mcp_tool");
    assert_eq!(candidates[0]["qualified_name"], "submit_plan");
    assert_eq!(
        candidates[0]["file_path"],
        "crates/spur-core/src/mcp/plan.rs"
    );
    assert_ne!(candidates[0]["id"], helper.stable_symbol_id);
}

#[tokio::test]
async fn code_file_symbols_returns_candidate_rows_for_worktree_relative_file() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_file_symbols",
            json!({ "file": "src/lib.rs" }),
        )
        .await,
    );
    let symbols = body["symbols"].as_array().expect("symbols");

    assert_eq!(
        qualified_names(symbols),
        BTreeSet::from([
            "audit_order".to_string(),
            "charge_order".to_string(),
            "launch_order".to_string(),
            "orchestrate_order".to_string(),
            "parse_order".to_string(),
        ])
    );
    assert!(symbols.iter().all(|row| row["file_path"] == "src/lib.rs"));
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        body["graph_index_version"],
        artifact.header.graph_index_version
    );

    let invalid = call_tool(
        &server,
        "code_file_symbols",
        json!({ "file": "../src/lib.rs" }),
    )
    .await;
    assert_eq!(invalid["error"]["code"], -32602);

    let current_dir = call_tool(
        &server,
        "code_file_symbols",
        json!({ "file": "./src/lib.rs" }),
    )
    .await;
    assert_eq!(current_dir["error"]["code"], -32602);

    let embedded_current_dir = call_tool(
        &server,
        "code_file_symbols",
        json!({ "file": "src/./lib.rs" }),
    )
    .await;
    assert_eq!(embedded_current_dir["error"]["code"], -32602);
}

#[tokio::test]
async fn code_file_symbols_uses_symbol_uri_selector_for_legacy_empty_qualified_name() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    let artifact = GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "v4".to_string(),
        graph_content_hash: "legacy-empty-qualified-name".to_string(),
        file_manifests: vec![GraphFileManifestEntry {
            stable_file_id: "file-src-lib".to_string(),
            path: "src/lib.rs".to_string(),
            content_oid: "0000000000000000000000000000000000000000".to_string(),
            node_ids: Vec::new(),
        }],
        files: vec![spur_graph::GraphFileArtifact {
            stable_file_id: "file-src-lib".to_string(),
            file_path: "src/lib.rs".to_string(),
        }],
        file_node_ids: vec![NodeId(1)],
        symbols: vec![GraphSymbolArtifact {
            stable_symbol_id: "legacy-empty-qualified-name-id".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [0, 8],
            line_range: [1, 3],
            entity_name: "legacy_symbol".to_string(),
            qualified_name: String::new(),
            symbol_kind: "function".to_string(),
            anchor_hash: "hash-legacy-empty-qualified-name-id".to_string(),
            enclosing_scope: None,
        }],
        symbol_node_ids: vec![NodeId(1_001)],
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..GraphIndexArtifact::empty()
    };
    write_graph_artifact(worktree.path(), &artifact);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_file_symbols",
            json!({ "file": "src/lib.rs" }),
        )
        .await,
    );
    let symbols = body["symbols"].as_array().expect("symbols");

    assert_eq!(symbols.len(), 1);
    assert_eq!(
        symbols[0]["selector"],
        "graph://symbol/legacy-empty-qualified-name-id"
    );
    assert_ne!(symbols[0]["selector"], "src/lib.rs::");
    assert_eq!(symbols[0]["qualified_name"], "");
}

#[tokio::test]
async fn code_symbol_info_returns_single_symbol_metadata() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_symbol_info",
            json!({ "selector": ROOT_SYMBOL }),
        )
        .await,
    );
    let symbol = &body["symbol"];

    assert_eq!(symbol["qualified_name"], root.qualified_name);
    assert_eq!(symbol["file_path"], root.file_path);
    assert_eq!(symbol["line_range"], json!(root.line_range));
    assert_eq!(symbol["symbol_kind"], root.symbol_kind);
    assert_eq!(symbol["enclosing_scope"], Value::Null);
    assert_eq!(
        symbol["uri"],
        format!("graph://symbol/{}", root.stable_symbol_id)
    );
    assert_eq!(symbol["id"], root.stable_symbol_id);
    assert!(body.get("callers").is_none());
    assert!(body.get("callees").is_none());
    assert!(body.get("nodes").is_none());
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
    assert_eq!(
        body["graph_index_version"],
        artifact.header.graph_index_version
    );
}

#[tokio::test]
async fn code_read_symbol_reads_source_by_stable_symbol_id() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "stable_symbol_id": root.stable_symbol_id }),
        )
        .await,
    );

    assert_eq!(body["symbol"]["id"], root.stable_symbol_id);
    assert_eq!(body["symbol"]["qualified_name"], root.qualified_name);
    assert_eq!(body["symbol"]["file_path"], root.file_path);
    assert_eq!(
        body["line_range"],
        json!({ "start": root.line_range[0], "end": root.line_range[1] })
    );
    assert_eq!(body["source"], expected_source);
    assert_eq!(body["file_oid"], file_oid(&artifact, &root.file_path));
    assert!(body.get("stale").is_none());
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
}

#[tokio::test]
async fn code_read_symbol_reads_source_by_path_name_tuple() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "path": root.file_path, "name": ROOT_SYMBOL }),
        )
        .await,
    );

    assert_eq!(body["symbol"]["id"], root.stable_symbol_id);
    assert_eq!(body["source"], expected_source);
    assert_eq!(
        body["line_range"],
        json!({ "start": root.line_range[0], "end": root.line_range[1] })
    );
    assert_eq!(body["file_oid"], file_oid(&artifact, &root.file_path));
}

#[tokio::test]
async fn code_read_symbol_source_format_by_stable_symbol_id_is_source_first() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "stable_symbol_id": root.stable_symbol_id,
                "response_format": "source"
            }),
        )
        .await,
    );

    assert_eq!(
        body,
        json!({
            "id": root.stable_symbol_id,
            "name": root.qualified_name,
            "file": root.file_path,
            "range": { "start": root.line_range[0], "end": root.line_range[1] },
            "source": expected_source,
            "file_oid": file_oid(&artifact, &root.file_path),
        })
    );
}

#[tokio::test]
async fn code_read_symbol_source_format_by_path_name_tuple_preserves_context_indicator() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let total_lines = total_lines(worktree.path(), &root.file_path);
    let start = root.line_range[0].saturating_sub(1).max(1);
    let end = (root.line_range[1] + 1).min(total_lines);
    let expected_source = source_for_range(worktree.path(), &root.file_path, start, end);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "path": root.file_path,
                "name": ROOT_SYMBOL,
                "context_lines": 1,
                "response_format": "source"
            }),
        )
        .await,
    );

    assert_eq!(
        body,
        json!({
            "id": root.stable_symbol_id,
            "name": root.qualified_name,
            "file": root.file_path,
            "range": { "start": start, "end": end },
            "source": expected_source,
            "file_oid": file_oid(&artifact, &root.file_path),
            "context_lines": 1,
        })
    );
}

#[tokio::test]
async fn code_read_symbol_source_format_preserves_stale_indexed_source_signal() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    std::fs::write(
        worktree.path().join("src/lib.rs"),
        "pub fn edited_after_index() -> bool {\n    false\n}\n",
    )
    .expect("edit fixture after graph build");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "stable_symbol_id": root.stable_symbol_id,
                "response_format": "source"
            }),
        )
        .await,
    );

    assert_eq!(
        body,
        json!({
            "id": root.stable_symbol_id,
            "name": root.qualified_name,
            "file": root.file_path,
            "range": { "start": root.line_range[0], "end": root.line_range[1] },
            "source": expected_source,
            "file_oid": file_oid(&artifact, &root.file_path),
            "stale": true,
        })
    );
}

#[tokio::test]
async fn code_read_symbol_source_format_ambiguous_request_returns_compact_candidates() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_ambiguous_symbol_fixture(worktree.path());
    commit_fixture(worktree.path());
    build_graph_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "path": "src/lib.rs",
                "name": "run",
                "response_format": "source"
            }),
        )
        .await,
    );
    let candidates = body["candidates"].as_array().expect("candidates");

    assert_eq!(body["ambiguous"], true);
    assert_eq!(candidates.len(), 2);
    assert!(body.get("graph_content_hash").is_none());
    assert!(body.get("graph_index_version").is_none());
    for candidate in candidates {
        for key in ["id", "name", "file", "range", "kind", "scope"] {
            assert!(
                candidate.get(key).is_some(),
                "source candidate should include {key}: {candidate}"
            );
        }
        for key in [
            "entity_name",
            "qualified_name",
            "file_path",
            "line_range",
            "symbol_kind",
            "enclosing_scope",
            "uri",
        ] {
            assert!(
                candidate.get(key).is_none(),
                "source candidate should omit full candidate key {key}: {candidate}"
            );
        }
        assert_eq!(candidate["file"], "src/lib.rs");
        assert_eq!(candidate["kind"], "method");
        assert!(candidate["name"]
            .as_str()
            .expect("candidate name")
            .ends_with("::run"));
    }
}

#[tokio::test]
async fn code_read_symbol_returns_candidates_for_ambiguous_path_name_tuple() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_ambiguous_symbol_fixture(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "path": "src/lib.rs", "name": "run" }),
        )
        .await,
    );
    let candidates = body["candidates"].as_array().expect("candidates");

    assert_eq!(body["ambiguous"], true);
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|candidate| {
        candidate["entity_name"] == "run" && candidate["file_path"] == "src/lib.rs"
    }));
    assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
}

#[tokio::test]
async fn code_read_symbol_marks_stale_but_returns_indexed_source() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let expected_source = source_for_range(
        worktree.path(),
        &root.file_path,
        root.line_range[0],
        root.line_range[1],
    );
    std::fs::write(
        worktree.path().join("src/lib.rs"),
        "pub fn edited_after_index() -> bool {\n    false\n}\n",
    )
    .expect("edit fixture after graph build");
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "stable_symbol_id": root.stable_symbol_id }),
        )
        .await,
    );

    assert_eq!(body["stale"], true);
    assert_eq!(body["source"], expected_source);
    assert!(!body["source"]
        .as_str()
        .expect("source text")
        .contains("edited_after_index"));
    assert_eq!(body["file_oid"], file_oid(&artifact, &root.file_path));
}

const EDITED_FIXTURE_KEEPING_ROOT: &str = "pub fn added_before() -> bool {
    true
}

pub fn launch_order() -> bool {
    orchestrate_order()
}

pub fn orchestrate_order() -> bool {
    let parsed = parse_order();
    let charged = charge_order();
    parsed && charged && added_before()
}

fn parse_order() -> bool {
    true
}

fn charge_order() -> bool {
    audit_order()
}

fn audit_order() -> bool {
    true
}
";

#[tokio::test]
async fn code_read_symbol_serves_current_worktree_source_when_stale_symbol_survives() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    std::fs::write(
        worktree.path().join("src/lib.rs"),
        EDITED_FIXTURE_KEEPING_ROOT,
    )
    .expect("edit fixture after graph build");
    let expected_source = source_for_range(worktree.path(), &root.file_path, 9, 13);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({ "stable_symbol_id": root.stable_symbol_id }),
        )
        .await,
    );

    assert_eq!(body["stale"], true);
    assert_eq!(body["source_origin"], "worktree");
    assert_eq!(body["line_range"], json!({ "start": 9, "end": 13 }));
    assert_eq!(body["source"], expected_source);
    assert!(body["source"]
        .as_str()
        .expect("source text")
        .contains("added_before()"));
    assert_eq!(
        body["file_oid"],
        spur_graph::git_blob_oid(EDITED_FIXTURE_KEEPING_ROOT.as_bytes())
    );
    assert_eq!(body["symbol"]["qualified_name"], ROOT_SYMBOL);
}

#[tokio::test]
async fn code_read_symbol_source_format_serves_current_worktree_source_when_stale() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    std::fs::write(
        worktree.path().join("src/lib.rs"),
        EDITED_FIXTURE_KEEPING_ROOT,
    )
    .expect("edit fixture after graph build");
    let expected_source = source_for_range(worktree.path(), &root.file_path, 9, 13);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "stable_symbol_id": root.stable_symbol_id,
                "response_format": "source"
            }),
        )
        .await,
    );

    assert_eq!(body["stale"], true);
    assert_eq!(body["source_origin"], "worktree");
    assert_eq!(body["range"], json!({ "start": 9, "end": 13 }));
    assert_eq!(body["source"], expected_source);
    assert_eq!(body["name"], ROOT_SYMBOL);
    assert_eq!(
        body["file_oid"],
        spur_graph::git_blob_oid(EDITED_FIXTURE_KEEPING_ROOT.as_bytes())
    );
}

#[tokio::test]
async fn code_read_symbol_clamps_and_echoes_context_lines() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    commit_fixture(worktree.path());
    let artifact = build_graph_artifact(worktree.path());
    let root = symbol_by_entity(&artifact, ROOT_SYMBOL);
    let total_lines = total_lines(worktree.path(), &root.file_path);
    let expected_source = source_for_range(worktree.path(), &root.file_path, 1, total_lines);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_read_symbol",
            json!({
                "stable_symbol_id": root.stable_symbol_id,
                "context_lines": 999
            }),
        )
        .await,
    );

    assert_eq!(body["context_lines"], 50);
    assert_eq!(body["requested_context_lines"], 999);
    assert_eq!(
        body["line_range"],
        json!({ "start": 1, "end": total_lines })
    );
    assert_eq!(body["source"], expected_source);
}

#[tokio::test]
async fn code_search_recovers_macro_bodied_callees_for_tool_definitions() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    let artifact = build_real_tools_graph_artifact(worktree.path());
    let graph_tools = symbol_by_file_entity_kind(
        &artifact,
        "crates/spur-graph/src/mcp/mod.rs",
        "tool_definitions",
        "function",
    );
    let graph_tools_uri = format!("graph://symbol/{}", graph_tools.stable_symbol_id);
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let callees = tool_body(
        call_tool(
            &server,
            "code_callees",
            json!({ "selector": graph_tools_uri }),
        )
        .await,
    );
    let callee_names = entity_names(callees["callees"].as_array().expect("callees"));
    assert!(callee_names.contains("code_resolve_def"));
    assert!(callee_names.contains("code_symbol_search_def"));

    let search = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "_def",
                "mode": "substring",
                "file": "crates/spur-core/src/mcp/plan.rs",
                "symbol_kind": "function",
                "limit": 100
            }),
        )
        .await,
    );
    let candidates = search["candidates"].as_array().expect("candidates");
    let names = entity_names(candidates);

    assert!(names.contains("recover_orphaned_dispatch_def"));
    assert!(names.contains("merge_plan_def"));
    assert!(names.contains("submit_plan_def"));
    assert!(
        search["total_matches"].as_u64().expect("total_matches") >= 13,
        "expected at least 13 plan *_def functions, got {}",
        search["total_matches"]
    );
    let allowed_plan_helpers = [
        "plan_management_tool_definitions",
        "plan_remainder_tool_definitions",
        "plan_tool_definition",
        "plan_tool_definitions",
        "worker_tool_definitions",
    ];
    assert!(candidates.iter().all(|candidate| {
        let name = candidate["entity_name"].as_str().expect("entity_name");
        name.ends_with("_def") || allowed_plan_helpers.contains(&name)
    }));

    let submit_tools = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "submit",
                "symbol_kind": "mcp_tool",
                "limit": 20
            }),
        )
        .await,
    );
    let submit_tool_candidates = submit_tools["candidates"].as_array().expect("candidates");
    assert!(submit_tool_candidates.iter().any(|candidate| {
        candidate["entity_name"] == "submit_plan"
            && candidate["symbol_kind"] == "mcp_tool"
            && candidate["file_path"] == "crates/spur-core/src/mcp/plan.rs"
    }));
}

#[tokio::test]
async fn code_search_echoes_requested_limit_when_clamped() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    copy_fixture_crate(worktree.path());
    build_graph_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "order",
                "mode": "substring",
                "limit": 500
            }),
        )
        .await,
    );

    assert_eq!(body["limit"], 200);
    assert_eq!(body["requested_limit"], 500);
}

#[tokio::test]
async fn code_search_candidate_rows_include_enclosing_scope() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    let artifact = build_real_tools_graph_artifact(worktree.path());
    let mcp_tool = symbol_by_file_entity_kind(
        &artifact,
        "crates/spur-core/src/mcp/plan.rs",
        "submit_plan",
        "mcp_tool",
    );
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_search",
            json!({
                "query": "submit_plan",
                "symbol_kind": "mcp_tool",
                "limit": 20
            }),
        )
        .await,
    );
    let candidates = body["candidates"].as_array().expect("candidates");
    let candidate = candidates
        .iter()
        .find(|candidate| candidate["id"] == mcp_tool.stable_symbol_id)
        .expect("submit_plan mcp_tool candidate");

    assert_eq!(candidate["enclosing_scope"], "submit_plan_def");
}

#[tokio::test]
async fn code_graph_tools_resolve_requested_symbol_as_of_historical_commit() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_temporal_fixture_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let current_root_uri = format!("graph://symbol/{NEW_ROOT_ID}");
    let historical_subgraph = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "symbol": current_root_uri.clone(),
                "as_of": OLD_SHA,
                "radius": 1,
                "edge_kinds": ["calls"]
            }),
        )
        .await,
    );
    let historical_names = entity_names(
        historical_subgraph["nodes"]
            .as_array()
            .expect("historical nodes"),
    );
    assert_eq!(
        historical_names,
        BTreeSet::from([
            "foo".to_string(),
            "helper_foo".to_string(),
            "launch_foo".to_string(),
        ])
    );
    assert!(!historical_names.contains("bar"));

    let historical_resolve = tool_body(
        call_tool(
            &server,
            "code_resolve",
            json!({ "selector": current_root_uri.clone(), "as_of": OLD_SHA }),
        )
        .await,
    );
    assert_eq!(historical_resolve["resolution"]["kind"], "renamed");
    let historical_candidates = historical_resolve["candidates"]
        .as_array()
        .expect("historical resolve candidates");
    assert_eq!(historical_candidates.len(), 1);
    assert_eq!(historical_candidates[0]["id"], OLD_ROOT_ID);

    let callers = tool_body(
        call_tool(
            &server,
            "code_callers",
            json!({ "symbol": current_root_uri.clone(), "as_of": OLD_SHA }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callers["callers"].as_array().expect("callers")),
        BTreeSet::from(["launch_foo".to_string()])
    );

    let callees = tool_body(
        call_tool(
            &server,
            "code_callees",
            json!({ "symbol": current_root_uri.clone(), "as_of": OLD_SHA }),
        )
        .await,
    );
    assert_eq!(
        entity_names(callees["callees"].as_array().expect("callees")),
        BTreeSet::from(["helper_foo".to_string()])
    );

    let historical_history = tool_body(
        call_tool(
            &server,
            "code_symbol_history",
            json!({ "symbol": current_root_uri.clone(), "as_of": OLD_SHA }),
        )
        .await,
    );
    let historical_events = historical_history["events"]
        .as_array()
        .expect("historical history events");
    assert_eq!(historical_events.len(), 1);
    assert_eq!(historical_events[0]["commit"], OLD_SHA);
    assert_eq!(
        historical_events[0]["snapshot"]["stable_symbol_id"],
        OLD_ROOT_ID
    );

    let current_subgraph = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({ "symbol": current_root_uri, "radius": 1, "edge_kinds": ["calls"] }),
        )
        .await,
    );
    let current_names = entity_names(current_subgraph["nodes"].as_array().expect("current nodes"));
    assert_eq!(
        current_names,
        BTreeSet::from([
            "bar".to_string(),
            "helper_bar".to_string(),
            "launch_bar".to_string(),
        ])
    );
}

#[tokio::test]
async fn code_subgraph_returns_deleted_with_last_seen() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_temporal_resolution_fixture_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let found = tool_body(
        call_tool(
            &server,
            "code_subgraph",
            json!({
                "symbol": format!("graph://symbol/{FOUND_ORIGIN_ID}"),
                "as_of": NEW_SHA,
                "radius": 0
            }),
        )
        .await,
    );
    assert_eq!(
        entity_names(found["nodes"].as_array().expect("found nodes")),
        BTreeSet::from(["found_target".to_string()])
    );

    let deleted = call_tool(
        &server,
        "code_subgraph",
        json!({
            "symbol": format!("graph://symbol/{DELETED_ID}"),
            "as_of": DELETE_SHA,
            "radius": 0
        }),
    )
    .await;
    assert_eq!(deleted["error"]["code"], -32005);
    let deleted_data = error_data(&deleted);
    assert_eq!(deleted_data["kind"], "deleted");
    assert_eq!(deleted_data["last_seen"]["stable_symbol_id"], DELETED_ID);
    assert_eq!(deleted_data["last_seen"]["commit"], DELETE_SHA);

    let deleted_resolve = tool_body(
        call_tool(
            &server,
            "code_resolve",
            json!({
                "selector": format!("graph://symbol/{DELETED_ID}"),
                "as_of": DELETE_SHA
            }),
        )
        .await,
    );
    assert_eq!(deleted_resolve["resolution"]["kind"], "deleted");
    assert_eq!(
        deleted_resolve["resolution"]["last_seen"]["stable_symbol_id"],
        DELETED_ID
    );
    assert_eq!(
        deleted_resolve["resolution"]["last_seen"]["commit"],
        DELETE_SHA
    );
    assert_eq!(
        deleted_resolve["candidates"]
            .as_array()
            .expect("deleted candidates")
            .len(),
        0
    );

    let not_found = call_tool(
        &server,
        "code_subgraph",
        json!({ "symbol": "graph://symbol/not-in-artifact", "radius": 0 }),
    )
    .await;
    assert_eq!(not_found["error"]["code"], -32004);
    assert_eq!(error_data(&not_found)["kind"], "not_found");

    let ambiguous = call_tool(
        &server,
        "code_subgraph",
        json!({
            "symbol": format!("graph://symbol/{AMBIGUOUS_ORIGIN_ID}"),
            "as_of": AMBIGUOUS_SHA,
            "radius": 0
        }),
    )
    .await;
    assert_eq!(ambiguous["error"]["code"], -32006);
    let ambiguous_data = error_data(&ambiguous);
    assert_eq!(ambiguous_data["kind"], "ambiguous");
    assert_eq!(
        ambiguous_data["candidates"].as_array().expect("candidates"),
        &vec![json!(AMBIGUOUS_LEFT_ID), json!(AMBIGUOUS_RIGHT_ID)]
    );

    let unknown = call_tool(
        &server,
        "code_subgraph",
        json!({
            "symbol": format!("graph://symbol/{UNKNOWN_ID}"),
            "as_of": UNKNOWN_TARGET_SHA,
            "radius": 0
        }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32007);
    let unknown_data = error_data(&unknown);
    assert_eq!(unknown_data["kind"], "unknown");
    assert_eq!(unknown_data["reason"]["kind"], "anchor_commit_not_indexed");
    assert_eq!(unknown_data["reason"]["commit"], UNINDEXED_SHA);
}

#[tokio::test]
async fn code_symbol_history_returns_rename_chain_for_parquet_and_in_memory_backends() {
    let _lock = cwd_lock().await;
    for backend in [
        CodeGraphFixtureBackend::Parquet,
        CodeGraphFixtureBackend::InMemory,
    ] {
        let worktree = TempDir::new().expect("temp worktree");
        write_temporal_fixture_artifact_for_backend(worktree.path(), backend);
        let _cwd = enter_dir(worktree.path());
        let server = test_server();

        let body = tool_body(
            call_tool(
                &server,
                "code_symbol_history",
                json!({ "symbol": format!("graph://symbol/{NEW_ROOT_ID}") }),
            )
            .await,
        );

        assert_code_symbol_history_rename_chain(&body, backend);
    }
}

#[tokio::test]
async fn code_symbol_history_uses_parquet_temporal_index_cache() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    std::fs::write(worktree.path().join("README.md"), "fixture\n").expect("write fixture file");
    commit_fixture(worktree.path());
    write_temporal_fixture_artifact(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();
    let args = json!({ "symbol": format!("graph://symbol/{NEW_ROOT_ID}") });

    let first = tool_body(call_tool(&server, "code_symbol_history", args.clone()).await);
    let second = tool_body(call_tool(&server, "code_symbol_history", args).await);

    assert_eq!(first["events"].as_array().expect("first events").len(), 2);
    assert_eq!(second["events"].as_array().expect("second events").len(), 2);
    assert_eq!(
        server.__test_code_graph_temporal_index_build_invocation_count(),
        0
    );
}

#[tokio::test]
async fn code_symbol_history_reports_missing_commit_index_cleanly() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_graph_without_commit_index(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let response = call_tool(
        &server,
        "code_symbol_history",
        json!({ "symbol": NEW_ROOT_ID }),
    )
    .await;

    assert_eq!(response["error"]["code"], -32603);
    assert!(response["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("commit index not found"));
}

#[tokio::test]
async fn code_symbol_history_returns_empty_when_no_snapshots() {
    let _lock = cwd_lock().await;
    let worktree = TempDir::new().expect("temp worktree");
    write_graph_with_empty_snapshots(worktree.path());
    let _cwd = enter_dir(worktree.path());
    let server = test_server();

    let body = tool_body(
        call_tool(
            &server,
            "code_symbol_history",
            json!({ "symbol": NEW_ROOT_ID, "as_of": OLD_SHA }),
        )
        .await,
    );

    assert_eq!(body["symbol"], format!("graph://symbol/{NEW_ROOT_ID}"));
    assert_eq!(body["events"].as_array().expect("history events").len(), 0);
}
