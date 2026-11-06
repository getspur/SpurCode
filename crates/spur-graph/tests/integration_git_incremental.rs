use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

mod support;

const LEGITIMATE_GRAPH_NEIGHBORS: &[&str] = &[
    ".spur/graph-index.pointer.json.example.json",
    ".spur/graphical.json",
    ".spur/settings.json",
    "config.json",
    "nested/.spur/graph/fixture.json",
    "src/lib.rs",
];
const GENERATED_GRAPH_OUTPUTS: &[&str] = &[
    ".spur/graph-index.pointer.json",
    ".spur/commit-index.pointer.json",
    ".spur/graph/manifest.json",
    ".spur/graph/old.parquet/manifest.json",
];

fn graph_source_boundary(root: &Path, mode: &str) {
    let before = build_full(root);
    for path in GENERATED_GRAPH_OUTPUTS {
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(root.join(path), r#"{"generated":1}"#).unwrap();
    }
    let after = build_full(root);
    let identities = |a: &GraphIndexArtifact| {
        a.file_manifests
            .iter()
            .map(|f| (f.path.clone(), f.content_oid.clone()))
            .collect::<Vec<_>>()
    };
    eprintln!(
        "M4_SOURCE_{mode}={}",
        serde_json::json!({"before":identities(&before),"after":identities(&after),"before_hash":before.graph_content_hash,"after_hash":after.graph_content_hash})
    );
    assert_manifest_paths(&after, LEGITIMATE_GRAPH_NEIGHBORS);
    assert_eq!(identities(&before), identities(&after));
    assert_eq!(before.graph_content_hash, after.graph_content_hash);
}

fn write_legitimate_graph_neighbors(root: &Path) {
    for path in LEGITIMATE_GRAPH_NEIGHBORS {
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(
            root.join(path),
            if path.ends_with(".rs") {
                "pub fn sample() {}\n"
            } else {
                "{}\n"
            },
        )
        .unwrap();
    }
}

#[test]
fn filesystem_graph_output_exclusion_preserves_legitimate_sources_and_hash() {
    let dir = tempfile::tempdir().unwrap();
    write_legitimate_graph_neighbors(dir.path());
    graph_source_boundary(dir.path(), "filesystem");
}

#[test]
fn git_graph_output_exclusion_covers_dirty_and_tracked_paths() {
    let repo = GitRepo::new();
    write_legitimate_graph_neighbors(repo.path());
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "legitimate sources"]);
    graph_source_boundary(repo.path(), "git_dirty");
    repo.git(&["add", ".spur"]);
    repo.git(&["commit", "-m", "tracked outputs"]);
    let tracked = build_full(repo.path());
    assert_manifest_paths(&tracked, LEGITIMATE_GRAPH_NEIGHBORS);
}

#[test]
fn canonical_reuse_rejects_corrupt_core_macro_tables_before_pointer_publication() {
    for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
        let repo = GitRepo::new();
        repo.write("src/lib.rs", "pub fn check() { assert!(true); }\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-m", "source"]);
        let artifact = build_full(repo.path());
        let ctx = git::detect(repo.path()).unwrap();
        write_with_dedup(&artifact, repo.path(), &ctx).unwrap();
        let canonical = canonical_artifact_path(repo.path(), &artifact);
        let _ = fs::remove_file(canonical.join(table));
        fs::remove_file(repo.path().join(".spur/graph/CURRENT")).unwrap();
        fs::remove_file(repo.path().join(".spur/graph-index.pointer.json")).unwrap();
        assert!(
            write_with_dedup(&artifact, repo.path(), &ctx).is_err(),
            "reused missing {table}"
        );
        assert!(!repo.path().join(".spur/graph/CURRENT").exists());
        assert!(!repo.path().join(".spur/graph-index.pointer.json").exists());
    }
}

use serde_json::json;
use spur_graph::mcp::{
    code_symbol_info_rebuild_aware, markdown_overlay_base_delta_for_worktree,
    with_worktree_root_for_request,
};
use spur_graph::store::cache::{
    emit_base_seed_stats, load_base_artifact_for_worktree, load_base_seed_for_worktree,
    lookup_canonical, write_with_dedup,
};
use spur_graph::{
    artifact_from_facts, artifact_from_facts_incremental, build_facts, git,
    read_artifact_header_parquet, read_current_pointer, write_artifact_parquet,
    write_current_pointer, BuildMode, GraphIndexArtifact, GraphIndexPointer, SourceKind,
    WriteOptions,
};
use support::git_repo::{path_str, GitRepo};

#[test]
fn discovery_uses_git_when_available() {
    let repo = GitRepo::new();
    repo.write(".hidden.rs", "pub fn hidden() {}\n");
    repo.write("src/visible.rs", "pub fn visible() {}\n");
    repo.git(&["add", ".hidden.rs", "src/visible.rs"]);
    repo.git(&["commit", "-m", "add tracked files"]);

    let artifact = build_full(repo.path());

    assert_manifest_paths(&artifact, &[".hidden.rs", "src/visible.rs"]);
}

#[test]
fn discovery_filters_symlink_gitlink_sparse() {
    let repo = GitRepo::new();
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.write("src/sparse.rs", "pub fn sparse() {}\n");
    repo.git(&["add", "src/keep.rs", "src/sparse.rs"]);

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("src/keep.rs", repo.path().join("link.rs"))
            .expect("create symlink");
        repo.git(&["add", "link.rs"]);
    }

    let submodule_oid = gitlink_target_oid("submodule one");
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000",
        &submodule_oid,
        "vendor/submodule.rs",
    ]);
    repo.git(&["commit", "-m", "add mixed entries"]);
    repo.git(&["update-index", "--skip-worktree", "src/sparse.rs"]);

    let artifact = build_full(repo.path());

    assert!(manifest_entry(&artifact, "src/keep.rs").is_some());
    assert!(manifest_entry(&artifact, "src/sparse.rs").is_none());
    #[cfg(unix)]
    assert!(manifest_entry(&artifact, "link.rs").is_none());
    assert_eq!(
        manifest_entry(&artifact, "vendor/submodule.rs")
            .expect("gitlink manifest")
            .content_oid,
        format!("gitlink:{submodule_oid}")
    );
}

#[test]
fn content_oid_replaces_mtime_size() {
    let repo = GitRepo::new();
    let content = "pub fn stable() {}\n";
    repo.write("src/lib.rs", content);
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "add lib"]);

    let first = build_full(repo.path());
    let first_manifest = manifest_entry(&first, "src/lib.rs").expect("first manifest");
    assert_eq!(
        first_manifest.content_oid,
        git_hash_object(content.as_bytes())
    );

    repo.write("src/lib.rs", content);
    let second = build_full(repo.path());
    let second_manifest = manifest_entry(&second, "src/lib.rs").expect("second manifest");

    assert_eq!(first_manifest.content_oid, second_manifest.content_oid);
    assert_eq!(first.graph_content_hash, second.graph_content_hash);

    let json = serde_json::to_string(&second).expect("serialize artifact");
    assert!(!json.contains("mtime"));
    assert!(!json.contains("size_bytes"));
}

#[test]
fn inrust_manifest_diff_handles_add_modify_delete() {
    let repo = GitRepo::new();
    repo.write("src/a.rs", "pub fn alpha() {}\n");
    repo.write("src/b.rs", "pub fn beta() {}\n");
    repo.git(&["add", "src/a.rs", "src/b.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    let baseline = build_full(repo.path());
    let removed_id = manifest_entry(&baseline, "src/b.rs")
        .expect("baseline b")
        .stable_file_id
        .clone();
    let alpha_id = baseline
        .symbols
        .iter()
        .find(|symbol| symbol.entity_name == "alpha")
        .expect("baseline alpha")
        .stable_symbol_id
        .clone();

    repo.write("src/a.rs", "pub fn alpha_changed() {}\n");
    repo.write("src/c.rs", "pub fn gamma() {}\n");
    fs::remove_file(repo.path().join("src/b.rs")).expect("remove b.rs");
    let (next, mode, _stats) =
        artifact_from_facts_incremental(&baseline, repo.path()).expect("incremental build");

    assert_eq!(mode, BuildMode::Incremental);
    assert!(next
        .symbols
        .iter()
        .any(|symbol| symbol.entity_name == "alpha_changed"));
    assert!(manifest_entry(&next, "src/c.rs").is_some());
    assert!(manifest_entry(&next, "src/b.rs").is_none());
    // One tombstone for the whole-file removal of b.rs, one for `alpha` being
    // renamed to `alpha_changed` within the still-present a.rs.
    assert_eq!(next.tombstones.len(), 2);
    assert!(next
        .tombstones
        .iter()
        .any(|tombstone| tombstone.path == "src/b.rs" && tombstone.stable_file_id == removed_id));
    assert!(next
        .tombstones
        .iter()
        .any(|tombstone| tombstone.path == "src/a.rs" && tombstone.stable_file_id == alpha_id));
}

#[test]
fn provenance_lives_in_pointer_not_artifact() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "add lib"]);

    let artifact = build_full(repo.path());
    write_git_cache(repo.path(), &artifact);

    let current = read_current_pointer(repo.path()).expect("read CURRENT");
    let manifest = read_artifact_header_parquet(&current).expect("read parquet manifest");
    assert_eq!(manifest.indexed_commit_oid, None);

    let pointer = read_pointer(repo.path());
    assert_eq!(pointer.source_kind, SourceKind::Git);
    assert_eq!(pointer.graph_content_hash, artifact.graph_content_hash);
    assert_eq!(
        pointer.indexed_commit_oid.as_deref(),
        Some(repo.head().as_str())
    );
}

#[test]
#[cfg(unix)]
fn current_pointer_targets_canonical_parquet_directory() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "add lib"]);

    let artifact = build_full(repo.path());
    write_git_cache(repo.path(), &artifact);
    let canonical = canonical_artifact_path(repo.path(), &artifact);
    let current = read_current_pointer(repo.path()).expect("read CURRENT");

    assert_eq!(current, canonical);
}

#[test]
fn dirty_then_commit_collides_with_clean_key() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn baseline() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let bytes = b"pub fn same_after_commit() {}\n";
    fs::write(repo.path().join("src/lib.rs"), bytes).expect("write dirty lib");
    let dirty = build_full(repo.path());

    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "commit same bytes"]);
    let clean = build_full(repo.path());

    assert_eq!(
        manifest_entry(&dirty, "src/lib.rs")
            .expect("dirty manifest")
            .content_oid,
        git_hash_object(bytes)
    );
    assert_eq!(dirty.graph_content_hash, clean.graph_content_hash);
}

#[test]
fn head_change_during_build_only_affects_provenance() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn stable() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "add lib"]);

    let artifact = build_full(repo.path());
    write_git_cache(repo.path(), &artifact);
    let first_pointer = read_pointer(repo.path());

    repo.git(&["commit", "--allow-empty", "-m", "metadata only"]);
    write_git_cache(repo.path(), &artifact);
    let second_pointer = read_pointer(repo.path());

    assert_eq!(
        first_pointer.graph_content_hash,
        second_pointer.graph_content_hash
    );
    assert_eq!(
        first_pointer.canonical_artifact_path,
        second_pointer.canonical_artifact_path
    );
    assert_ne!(
        first_pointer.indexed_commit_oid,
        second_pointer.indexed_commit_oid
    );
    assert_eq!(
        second_pointer.indexed_commit_oid.as_deref(),
        Some(repo.head().as_str())
    );
}

#[test]
fn delete_emits_value_level_tombstone() {
    let repo = GitRepo::new();
    repo.write("src/delete_me.rs", "pub fn delete_me() {}\n");
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.git(&["add", "src/delete_me.rs", "src/keep.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    let baseline = build_full(repo.path());

    fs::remove_file(repo.path().join("src/delete_me.rs")).expect("remove delete_me.rs");
    let (next, mode, _stats) =
        artifact_from_facts_incremental(&baseline, repo.path()).expect("incremental delete");

    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(next.tombstones.len(), 1);
    let tombstones_json = serde_json::to_string(&next.tombstones).expect("serialize tombstones");
    assert!(tombstones_json.contains("delete_me.rs"));
    assert!(!tombstones_json.contains("content_oid"));
    assert!(!tombstones_json.contains("indexed_commit_oid"));
}

#[test]
fn non_git_uses_git_blob_oid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bytes = b"\xef\xbb\xbfpub fn crlf() {\r\n    let _ = \"bytes\";\r\n}\r\n";
    fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
    fs::write(dir.path().join("src/lib.rs"), bytes).expect("write lib.rs");

    let artifact = build_full(dir.path());

    assert_eq!(
        manifest_entry(&artifact, "src/lib.rs")
            .expect("manifest")
            .content_oid,
        git_hash_object(bytes)
    );
}

#[test]
fn legacy_artifact_triggers_full_rebuild() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "add lib"]);

    let mut legacy = build_full(repo.path());
    legacy.manifest_version = "legacy-manifest-version".to_owned();
    let (rebuilt, mode, _stats) =
        artifact_from_facts_incremental(&legacy, repo.path()).expect("legacy rebuild");

    assert_eq!(mode, BuildMode::Full);
    assert_ne!(rebuilt.manifest_version, "legacy-manifest-version");
    assert!(manifest_entry(&rebuilt, "src/lib.rs").is_some());
}

#[test]
fn two_worktrees_with_same_content_share_cache() {
    let repo = GitRepo::new();
    repo.git(&["commit", "--allow-empty", "-m", "base"]);
    let base = repo.head();

    repo.git(&["checkout", "-b", "order-ab", &base]);
    repo.write("src/a.rs", "pub fn alpha() {}\n");
    repo.git(&["add", "src/a.rs"]);
    repo.git(&["commit", "-m", "add a"]);
    repo.write("src/b.rs", "pub fn beta() {}\n");
    repo.git(&["add", "src/b.rs"]);
    repo.git(&["commit", "-m", "add b"]);

    repo.git(&["checkout", "-b", "order-ba", &base]);
    repo.write("src/b.rs", "pub fn beta() {}\n");
    repo.git(&["add", "src/b.rs"]);
    repo.git(&["commit", "-m", "add b"]);
    repo.write("src/a.rs", "pub fn alpha() {}\n");
    repo.git(&["add", "src/a.rs"]);
    repo.git(&["commit", "-m", "add a"]);

    repo.git(&["checkout", &base]);
    let wt_ab = repo.temp.path().join("wt-ab");
    let wt_ba = repo.temp.path().join("wt-ba");
    repo.git(&["worktree", "add", path_str(&wt_ab), "order-ab"]);
    repo.git(&["worktree", "add", path_str(&wt_ba), "order-ba"]);

    let artifact_ab = build_full(&wt_ab);
    write_git_cache(&wt_ab, &artifact_ab);
    let artifact_ba = build_full(&wt_ba);
    write_git_cache(&wt_ba, &artifact_ba);

    let pointer_ab = read_pointer(&wt_ab);
    let pointer_ba = read_pointer(&wt_ba);
    assert_ne!(pointer_ab.indexed_commit_oid, pointer_ba.indexed_commit_oid);
    assert_eq!(
        artifact_ab.graph_content_hash,
        artifact_ba.graph_content_hash
    );
    assert_eq!(
        pointer_ab.canonical_artifact_path,
        pointer_ba.canonical_artifact_path
    );
}

#[test]
#[ignore = "requires a real cross-device canonical/worktree pair; store unit test injects EXDEV"]
fn cross_fs_write_falls_back_to_copy() {}

#[test]
fn submodule_pointer_change_invalidates() {
    let repo = GitRepo::new();
    let first_oid = gitlink_target_oid("submodule one");
    let second_oid = gitlink_target_oid("submodule two");

    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000",
        &first_oid,
        "vendor/submodule.rs",
    ]);
    repo.git(&["commit", "-m", "add submodule pointer"]);
    let first = build_full(repo.path());

    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000",
        &second_oid,
        "vendor/submodule.rs",
    ]);
    repo.git(&["commit", "-m", "move submodule pointer"]);
    let second = build_full(repo.path());

    assert_eq!(
        manifest_entry(&first, "vendor/submodule.rs")
            .expect("first gitlink")
            .content_oid,
        format!("gitlink:{first_oid}")
    );
    assert_eq!(
        manifest_entry(&second, "vendor/submodule.rs")
            .expect("second gitlink")
            .content_oid,
        format!("gitlink:{second_oid}")
    );
    assert_ne!(first.graph_content_hash, second.graph_content_hash);
}

#[test]
fn fresh_linked_worktree_seeds_incremental_from_main_with_no_changes() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.git(&["add", "src/lib.rs", "src/keep.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);
    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);
    assert!(!worker.join(".spur/graph-index.pointer.json").exists());

    let (base, lines) = capture_base_seed_trace(|| {
        let seed = load_base_seed_for_worktree(&worker).expect("main worktree base artifact");
        assert_eq!(seed.artifact.rust_evidence, main_artifact.rust_evidence);
        assert_rust_sources(&seed.artifact, repo.path());
        let (rebuilt, mode, stats) =
            artifact_from_facts_incremental(&seed.artifact, &worker).expect("incremental rebuild");
        assert_eq!(seed.base, "main_worktree");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats,
            spur_graph::build::BuildStats {
                reused_buckets: 2,
                ..Default::default()
            }
        );
        let expected = build_full(&worker);
        assert_eq!(rebuilt.files, expected.files);
        assert_eq!(rebuilt.symbols, expected.symbols);
        assert_eq!(rebuilt.edges, expected.edges);
        assert_eq!(rebuilt.rust_evidence, expected.rust_evidence);
        assert_eq!(rebuilt.rust_evidence, seed.artifact.rust_evidence);
        assert_rust_sources(&rebuilt, &worker);
        emit_base_seed_stats(seed.base, stats);
        rebuilt
    });
    assert_eq!(base.graph_content_hash, main_artifact.graph_content_hash);
    assert_base_seed_trace(&lines, "main_worktree", 0, 2);
}

#[test]
fn fresh_linked_worktree_seeds_incremental_from_main_with_one_modified_file() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.git(&["add", "src/lib.rs", "src/keep.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);
    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);
    fs::write(worker.join("src/lib.rs"), "pub fn lib_changed() {}\n").expect("modify worker lib");

    let (rebuilt, lines) = capture_base_seed_trace(|| {
        let seed = load_base_seed_for_worktree(&worker).expect("main worktree base artifact");
        assert_eq!(seed.artifact.rust_evidence, main_artifact.rust_evidence);
        assert_rust_sources(&seed.artifact, repo.path());
        let (rebuilt, mode, stats) =
            artifact_from_facts_incremental(&seed.artifact, &worker).expect("incremental rebuild");
        assert_eq!(seed.base, "main_worktree");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats,
            spur_graph::build::BuildStats {
                reused_buckets: 1,
                changed_paths: 1,
                changed_or_added_paths: BTreeSet::from(["src/lib.rs".into()]),
                removed_paths: BTreeSet::new()
            }
        );
        let expected = build_full(&worker);
        assert_eq!(rebuilt.files, expected.files);
        assert_eq!(rebuilt.symbols, expected.symbols);
        assert_eq!(rebuilt.edges, expected.edges);
        assert_rust_sources(&rebuilt, &worker);
        let before = seed.artifact.rust_evidence.as_ref().unwrap();
        let after = rebuilt.rust_evidence.as_ref().unwrap();
        let keep = |set: &spur_graph::RustEvidenceSet| {
            set.files
                .iter()
                .find(|f| f.file_path == "src/keep.rs")
                .unwrap()
                .clone()
        };
        assert_eq!(keep(after), keep(before));
        assert!(after.origins.contains(&before.origins[0]));
        let facts = build_facts(&worker, None).unwrap().0;
        let selected = spur_graph::artifact_from_facts_for_paths(
            &facts,
            &worker,
            &[worker.join("src/lib.rs")],
        )
        .unwrap();
        let changed = after
            .files
            .iter()
            .find(|f| f.file_path == "src/lib.rs")
            .unwrap();
        assert_eq!(changed, &selected.rust_evidence.as_ref().unwrap().files[0]);
        assert_ne!(
            changed.source_oid,
            before
                .files
                .iter()
                .find(|f| f.file_path == "src/lib.rs")
                .unwrap()
                .source_oid
        );
        emit_base_seed_stats(seed.base, stats);
        rebuilt
    });
    assert!(rebuilt
        .symbols
        .iter()
        .any(|symbol| symbol.entity_name == "lib_changed"));
    assert_base_seed_trace(&lines, "main_worktree", 1, 1);
}

#[test]
fn fresh_linked_worktree_seeds_incremental_from_main_current_without_pointer() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.git(&["add", "src/lib.rs", "src/keep.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);

    let main_artifact = build_full(repo.path());
    write_current_only_cache(repo.path(), &main_artifact);
    assert!(!repo.path().join(".spur/graph-index.pointer.json").exists());
    assert!(canonical_artifact_path_opt(repo.path(), &main_artifact).is_none());

    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);
    assert!(!worker.join(".spur/graph/CURRENT").exists());
    assert!(!worker.join(".spur/graph-index.pointer.json").exists());

    let (base, lines) = capture_base_seed_trace(|| {
        let seed = load_base_seed_for_worktree(&worker).expect("main worktree base artifact");
        assert_eq!(seed.artifact.rust_evidence, main_artifact.rust_evidence);
        assert_rust_sources(&seed.artifact, repo.path());
        let (rebuilt, mode, stats) =
            artifact_from_facts_incremental(&seed.artifact, &worker).expect("incremental rebuild");
        assert_eq!(seed.base, "main_worktree");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats,
            spur_graph::build::BuildStats {
                reused_buckets: 2,
                ..Default::default()
            }
        );
        let expected = build_full(&worker);
        assert_eq!(rebuilt.files, expected.files);
        assert_eq!(rebuilt.symbols, expected.symbols);
        assert_eq!(rebuilt.edges, expected.edges);
        assert_eq!(rebuilt.rust_evidence, expected.rust_evidence);
        assert_eq!(rebuilt.rust_evidence, seed.artifact.rust_evidence);
        assert_rust_sources(&rebuilt, &worker);
        emit_base_seed_stats(seed.base, stats);
        rebuilt
    });

    assert_eq!(base.graph_content_hash, main_artifact.graph_content_hash);
    assert_base_seed_trace(&lines, "main_worktree", 0, 2);
}

#[test]
fn main_worktree_uses_self_pointer_as_incremental_seed() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.git(&["add", "src/lib.rs", "src/keep.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);

    let (base, lines) = capture_base_seed_trace(|| {
        let seed = load_base_seed_for_worktree(repo.path()).expect("self base artifact");
        assert_eq!(seed.artifact.rust_evidence, main_artifact.rust_evidence);
        assert_rust_sources(&seed.artifact, repo.path());
        let (rebuilt, mode, stats) = artifact_from_facts_incremental(&seed.artifact, repo.path())
            .expect("incremental rebuild");
        assert_eq!(seed.base, "self_pointer");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats,
            spur_graph::build::BuildStats {
                reused_buckets: 2,
                ..Default::default()
            }
        );
        let expected = build_full(repo.path());
        assert_eq!(rebuilt.files, expected.files);
        assert_eq!(rebuilt.symbols, expected.symbols);
        assert_eq!(rebuilt.edges, expected.edges);
        assert_eq!(rebuilt.rust_evidence, expected.rust_evidence);
        assert_eq!(rebuilt.rust_evidence, seed.artifact.rust_evidence);
        assert_rust_sources(&rebuilt, &repo.path());
        emit_base_seed_stats(seed.base, stats);
        rebuilt
    });
    assert_eq!(base.graph_content_hash, main_artifact.graph_content_hash);
    assert_base_seed_trace(&lines, "self_pointer", 0, 2);
}

#[test]
fn load_base_artifact_reuses_process_local_cache_entry() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);

    let first = load_base_artifact_for_worktree(repo.path()).expect("first base artifact");
    let second = load_base_artifact_for_worktree(repo.path()).expect("second base artifact");

    assert!(
        Arc::ptr_eq(&first, &second),
        "repeated base loads should return the cached artifact Arc"
    );
}

#[test]
fn missing_self_and_main_pointers_returns_none_and_full_rebuild_still_works() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);
    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);

    let (base, lines) = capture_base_seed_trace(|| load_base_artifact_for_worktree(&worker));

    assert!(base.is_none());
    assert_base_seed_trace(&lines, "none", 0, 0);
    let rebuilt = build_full(&worker);
    assert!(rebuilt
        .symbols
        .iter()
        .any(|symbol| symbol.entity_name == "lib"));
}

#[test]
fn main_pointer_manifest_mismatch_uses_current_when_available() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);
    let canonical = canonical_artifact_path(repo.path(), &main_artifact);
    write_pointer_file(
        repo.path(),
        GraphIndexPointer {
            manifest_version: "0".to_owned(),
            canonical_artifact_path: canonical,
            ..read_pointer(repo.path())
        },
    );

    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);

    let base = load_base_artifact_for_worktree(&worker).expect("main CURRENT base artifact");

    assert_eq!(base.graph_content_hash, main_artifact.graph_content_hash);
    let rebuilt = build_full(&worker);
    assert_eq!(rebuilt.graph_content_hash, main_artifact.graph_content_hash);
}

#[test]
fn corrupted_self_pointer_falls_back_to_main_worktree_pointer() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);
    repo.git(&["branch", "worker"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);
    let worker = repo.temp.path().join("worker");
    repo.git(&["worktree", "add", path_str(&worker), "worker"]);
    let worker_pointer = worker.join(".spur/graph-index.pointer.json");
    fs::create_dir_all(worker_pointer.parent().expect("worker pointer parent"))
        .expect("create worker pointer parent");
    fs::write(&worker_pointer, "{not valid json").expect("write corrupt worker pointer");

    let (base, lines) = capture_base_seed_trace(|| {
        let seed = load_base_seed_for_worktree(&worker).expect("main worktree base artifact");
        assert_eq!(seed.artifact.rust_evidence, main_artifact.rust_evidence);
        assert_rust_sources(&seed.artifact, repo.path());
        let (rebuilt, mode, stats) =
            artifact_from_facts_incremental(&seed.artifact, &worker).expect("incremental rebuild");
        assert_eq!(seed.base, "main_worktree");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats,
            spur_graph::build::BuildStats {
                reused_buckets: 1,
                ..Default::default()
            }
        );
        let expected = build_full(&worker);
        assert_eq!(rebuilt.files, expected.files);
        assert_eq!(rebuilt.symbols, expected.symbols);
        assert_eq!(rebuilt.edges, expected.edges);
        assert_eq!(rebuilt.rust_evidence, expected.rust_evidence);
        assert_eq!(rebuilt.rust_evidence, seed.artifact.rust_evidence);
        assert_rust_sources(&rebuilt, &worker);
        emit_base_seed_stats(seed.base, stats);
        rebuilt
    });

    assert_eq!(base.graph_content_hash, main_artifact.graph_content_hash);
    assert_base_seed_trace(&lines, "main_worktree", 0, 1);
}

#[test]
fn code_symbol_info_rebuild_aware_uses_seed_rebuild_for_stale_current_with_valid_seed() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let seed_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &seed_artifact);

    let mut stale_current = build_full(repo.path());
    stale_current.manifest_version = "legacy-manifest-version".to_owned();
    // This fixture deliberately represents an unsupported pre-carrier snapshot.
    stale_current.rust_evidence = None;
    write_current_only_cache(repo.path(), &stale_current);

    repo.write(
        "src/lib.rs",
        "pub fn lib() {}\n\npub fn added_after_seed() {}\n",
    );
    let changed_full = build_full(repo.path());
    assert_ne!(
        changed_full.graph_content_hash, stale_current.graph_content_hash,
        "dirty fixture must distinguish overlay metadata from full rebuild metadata"
    );

    let args = json!({
        "selector": "src/lib.rs::lib",
    });
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (response, lines) = capture_base_seed_trace(|| {
        runtime
            .block_on(with_worktree_root_for_request(
                repo.path().to_path_buf(),
                code_symbol_info_rebuild_aware(&args),
            ))
            .expect("symbol info response")
    });

    assert_base_seed_source_trace(&lines, "self_pointer");
    assert_eq!(response["symbol"]["entity_name"], "lib");
    assert_eq!(response["rebuild_status"], "fresh");
    assert_eq!(response["response_file_oids_match"], true);
    assert_eq!(
        response["graph_content_hash"], changed_full.graph_content_hash,
        "stale CURRENT should be refreshed through a seed-aware materialized rebuild"
    );
}

#[test]
fn missing_canonical_artifact_returns_none_and_full_rebuild_still_works() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let main_artifact = build_full(repo.path());
    write_git_cache(repo.path(), &main_artifact);
    let canonical = canonical_artifact_path(repo.path(), &main_artifact);
    fs::remove_dir_all(&canonical).expect("remove canonical artifact");

    let base = load_base_artifact_for_worktree(repo.path());

    assert!(base.is_none());
    let rebuilt = build_full(repo.path());
    assert_eq!(rebuilt.graph_content_hash, main_artifact.graph_content_hash);
}

#[tokio::test]
async fn markdown_overlay_base_delta_reports_markdown_changes_relative_to_base() {
    let repo = GitRepo::new();
    repo.write("docs/unchanged.md", "# Unchanged\n\nStable body.\n");
    repo.write("docs/changed.md", "# Changed\n\nOld body.\n");
    repo.write("docs/deleted.md", "# Deleted\n\nRemoved body.\n");
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.git(&[
        "add",
        "docs/unchanged.md",
        "docs/changed.md",
        "docs/deleted.md",
        "src/lib.rs",
    ]);
    repo.git(&["commit", "-m", "baseline"]);

    let base_artifact = build_full(repo.path());
    write_current_only_cache(repo.path(), &base_artifact);

    let clean_delta = markdown_overlay_base_delta_for_worktree(repo.path())
        .await
        .expect("base delta");
    assert_eq!(
        clean_delta.base_artifact_dir,
        read_current_pointer(repo.path()).expect("base current pointer")
    );
    assert!(clean_delta.changed_markdown_paths.is_empty());
    assert!(clean_delta.deleted_markdown_paths.is_empty());

    repo.write("docs/changed.md", "# Changed\n\nNew body.\n");
    repo.write("docs/added.markdown", "# Added\n\nNew markdown file.\n");
    repo.remove("docs/deleted.md");

    let dirty_delta = markdown_overlay_base_delta_for_worktree(repo.path())
        .await
        .expect("dirty base delta");
    assert_eq!(
        dirty_delta.changed_markdown_paths,
        string_set(&["docs/added.markdown", "docs/changed.md"])
    );
    assert_eq!(
        dirty_delta.deleted_markdown_paths,
        string_set(&["docs/deleted.md"])
    );
}

#[test]
fn crlf_bom_dirty_hash_is_bytewise() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn baseline() {}\n");
    repo.git(&["add", "src/lib.rs"]);
    repo.git(&["commit", "-m", "baseline"]);

    let bytes = b"\xef\xbb\xbfpub fn dirty() {\r\n    let bytes = [0, 1, 2];\r\n}\r\n";
    fs::write(repo.path().join("src/lib.rs"), bytes).expect("write dirty bytes");
    let artifact = build_full(repo.path());

    assert_eq!(
        manifest_entry(&artifact, "src/lib.rs")
            .expect("dirty manifest")
            .content_oid,
        git_hash_object(bytes)
    );
}

fn string_set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

fn assert_rust_sources(graph: &GraphIndexArtifact, root: &Path) {
    let inputs: Vec<_> = graph
        .file_manifests
        .iter()
        .filter(|m| m.path.ends_with(".rs"))
        .map(|m| (m.path.clone(), fs::read(root.join(&m.path)).unwrap()))
        .collect();
    let sources: Vec<_> = inputs
        .iter()
        .map(|(path, bytes)| spur_graph::RustDeclarationSource {
            file_path: path,
            bytes,
        })
        .collect();
    graph.validate_rust_evidence_sources(&sources).unwrap();
}

fn build_full(root: &Path) -> GraphIndexArtifact {
    let (facts, _counts) = build_facts(root, None).expect("build facts");
    artifact_from_facts(&facts, root).expect("build artifact")
}

fn write_git_cache(root: &Path, artifact: &GraphIndexArtifact) {
    let ctx = git::detect(root).expect("git context");
    write_with_dedup(artifact, root, &ctx).expect("write git cache");
}

fn write_current_only_cache(root: &Path, artifact: &GraphIndexArtifact) {
    let artifact_base = root.join(".spur/graph");
    let written = write_artifact_parquet(
        artifact,
        &artifact_base,
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write worktree graph artifact");
    write_current_pointer(root, &written).expect("write CURRENT pointer");
}

fn canonical_artifact_path(root: &Path, artifact: &GraphIndexArtifact) -> PathBuf {
    canonical_artifact_path_opt(root, artifact).expect("canonical artifact")
}

fn canonical_artifact_path_opt(root: &Path, artifact: &GraphIndexArtifact) -> Option<PathBuf> {
    let ctx = git::detect(root).expect("git context");
    lookup_canonical(
        &ctx.git_common_dir,
        &artifact.manifest_version,
        &artifact.graph_content_hash,
    )
}

fn read_pointer(root: &Path) -> GraphIndexPointer {
    let bytes = fs::read(root.join(".spur/graph-index.pointer.json")).expect("read pointer");
    serde_json::from_slice(&bytes).expect("parse pointer")
}

fn write_pointer_file(root: &Path, pointer: GraphIndexPointer) {
    let path = root.join(".spur/graph-index.pointer.json");
    let bytes = serde_json::to_vec_pretty(&pointer).expect("serialize pointer");
    fs::write(path, bytes).expect("write pointer");
}

fn manifest_entry<'a>(
    artifact: &'a GraphIndexArtifact,
    path: &str,
) -> Option<&'a spur_graph::GraphFileManifestEntry> {
    artifact
        .file_manifests
        .iter()
        .find(|entry| entry.path == path)
}

fn assert_manifest_paths(artifact: &GraphIndexArtifact, expected: &[&str]) {
    let mut actual = artifact
        .file_manifests
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}

fn gitlink_target_oid(content: &str) -> String {
    let repo = GitRepo::new();
    repo.write("README.md", content);
    repo.git(&["add", "README.md"]);
    repo.git(&["commit", "-m", "submodule target"]);
    repo.head()
}

fn git_hash_object(bytes: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn git hash-object");
    child
        .stdin
        .as_mut()
        .expect("hash-object stdin")
        .write_all(bytes)
        .expect("write hash-object stdin");
    let output = child.wait_with_output().expect("wait for git hash-object");
    assert!(
        output.status.success(),
        "git hash-object failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("hash-object stdout UTF-8")
        .trim_end()
        .to_owned()
}

type TraceLines = Arc<Mutex<Vec<String>>>;

#[derive(Clone)]
struct TraceMakeWriter {
    lines: TraceLines,
}

struct TraceWriter {
    lines: TraceLines,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceMakeWriter {
    type Writer = TraceWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TraceWriter {
            lines: Arc::clone(&self.lines),
        }
    }
}

impl io::Write for TraceWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf).to_string();
        if line.contains("worker base seed selected")
            || line.contains("worker base seed source selected")
        {
            self.lines.lock().expect("trace lines lock").push(line);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_base_seed_trace<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_max_level(tracing::Level::INFO)
        .with_writer(TraceMakeWriter {
            lines: Arc::clone(&lines),
        })
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let captured = lines.lock().expect("trace lines lock").clone();
    (result, captured)
}

fn assert_base_seed_trace(
    lines: &[String],
    expected_base: &str,
    expected_changed_paths: usize,
    minimum_reused_buckets: usize,
) {
    let line = lines
        .iter()
        .find(|line| {
            line.contains("spur_graph::base_seed") && line.contains("worker base seed selected")
        })
        .unwrap_or_else(|| panic!("missing base seed trace event in {lines:?}"));
    assert!(
        line.contains(&format!("base=\"{expected_base}\""))
            || line.contains(&format!("base={expected_base}")),
        "unexpected base seed trace line: {line}"
    );
    assert!(
        line.contains(&format!("changed_paths={expected_changed_paths}")),
        "unexpected changed_paths in trace line: {line}"
    );
    let reused = parse_trace_usize(line, "reused_buckets")
        .unwrap_or_else(|| panic!("missing reused_buckets in trace line: {line}"));
    assert!(
        reused >= minimum_reused_buckets,
        "expected reused_buckets >= {minimum_reused_buckets}, got {reused} in {line}"
    );
}

fn assert_base_seed_source_trace(lines: &[String], expected_base: &str) {
    let line = lines
        .iter()
        .find(|line| {
            line.contains("spur_graph::base_seed")
                && line.contains("worker base seed source selected")
        })
        .unwrap_or_else(|| panic!("missing base seed source trace event in {lines:?}"));
    assert!(
        line.contains(&format!("base=\"{expected_base}\""))
            || line.contains(&format!("base={expected_base}")),
        "unexpected base seed source trace line: {line}"
    );
}

fn parse_trace_usize(line: &str, key: &str) -> Option<usize> {
    let start = line.find(&format!("{key}="))? + key.len() + 1;
    let value = line[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    value.parse().ok()
}
