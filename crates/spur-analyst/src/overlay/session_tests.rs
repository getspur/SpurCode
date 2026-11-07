use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::anyhow;

use super::*;

fn rebuild_key(head_oid: &str, dirty_byte: u8) -> OverlayRebuildKey {
    let mut dirty = BTreeMap::new();
    dirty.insert(PathBuf::from("src/lib.rs"), [dirty_byte; 20]);
    OverlayRebuildKey::from(head_oid, "base-hash", &dirty)
}

#[test]
fn overlay_rebuild_key_changes_when_base_graph_changes() {
    let mut dirty = BTreeMap::new();
    dirty.insert(PathBuf::from("src/lib.rs"), [1; 20]);

    let first = OverlayRebuildKey::from("head-a", "base-a", &dirty);
    let second = OverlayRebuildKey::from("head-a", "base-b", &dirty);

    assert_ne!(first, second);
    assert_ne!(first.cache_dir_name(), second.cache_dir_name());
}

#[test]
fn overlay_rebuild_key_is_none_when_worktree_matches_indexed_oids() {
    let repo = tempfile::tempdir().expect("tempdir");
    let root = repo.path();
    init_git_repo(root);
    fs::create_dir_all(root.join("src")).expect("src");
    let v1 = b"pub fn alpha_v1() {}\n";
    fs::write(root.join("src/lib.rs"), v1).expect("write");
    run_git(root, &["add", "src/lib.rs"]);
    run_git(root, &["commit", "--no-gpg-sign", "-m", "v1"]);

    let artifact = artifact_with_file("src/lib.rs", &spur_graph::git_blob_oid(v1));
    let key = overlay_rebuild_key_for_dirty_worktree(root, &artifact);

    assert!(
        key.is_none(),
        "matching graph oids and a clean tree must not build an analyst overlay"
    );
}

#[test]
fn overlay_rebuild_key_detects_clean_head_lag() {
    let repo = tempfile::tempdir().expect("tempdir");
    let root = repo.path();
    init_git_repo(root);
    fs::create_dir_all(root.join("src")).expect("src");
    let v1 = b"pub fn alpha_v1() {}\n";
    fs::write(root.join("src/lib.rs"), v1).expect("write v1");
    run_git(root, &["add", "src/lib.rs"]);
    run_git(root, &["commit", "--no-gpg-sign", "-m", "v1"]);
    let artifact = artifact_with_file("src/lib.rs", &spur_graph::git_blob_oid(v1));

    let v2 = b"pub fn alpha_v2() {}\n";
    fs::write(root.join("src/lib.rs"), v2).expect("write v2");
    run_git(root, &["add", "src/lib.rs"]);
    run_git(root, &["commit", "--no-gpg-sign", "-m", "v2"]);

    let dirty = spur_graph::git::status_dirty_paths(root).expect("status");
    assert!(dirty.is_empty(), "HEAD lag fixture must be status-clean");

    let key = overlay_rebuild_key_for_dirty_worktree(root, &artifact);
    assert!(
        key.is_some(),
        "committed divergence from indexed oids must overlay even when git status is clean"
    );
}

#[tokio::test]
async fn merge_session_reuses_cached_delta_for_same_dirty_key() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let coordinator = OverlaySessionCoordinator::new();
    let worktree = tempdir.path().join("repo");
    let base_db = tempdir.path().join("analyst.duckdb");
    let delta_dir = tempdir.path().join("delta-one");
    let key = rebuild_key("head-a", 1);
    let builds = AtomicUsize::new(0);

    let first = coordinator
        .get_or_build_session(
            worktree.clone(),
            key.clone(),
            base_db.clone(),
            Some("base-hash".to_owned()),
            |_| {
                builds.fetch_add(1, Ordering::SeqCst);
                let delta_dir = delta_dir.clone();
                async move { Ok(delta_dir) }
            },
        )
        .await;
    let second = coordinator
        .get_or_build_session(worktree, key, base_db, Some("base-hash".to_owned()), |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            let delta_dir = delta_dir.clone();
            async move { Ok(delta_dir) }
        })
        .await;

    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "same dirty key should reuse the retained merge session"
    );
    assert!(Arc::ptr_eq(&first, &second));
    assert!(first.delta_applied());
    assert_eq!(first.algo_as_of(), Some("base-hash"));
}

#[tokio::test]
async fn merge_session_rebuilds_after_dirty_key_changes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let coordinator = OverlaySessionCoordinator::new();
    let worktree = tempdir.path().join("repo");
    let base_db = tempdir.path().join("analyst.duckdb");
    let builds = AtomicUsize::new(0);

    let first = coordinator
        .get_or_build_session(
            worktree.clone(),
            rebuild_key("head-a", 1),
            base_db.clone(),
            Some("base-hash".to_owned()),
            |_| {
                let attempt = builds.fetch_add(1, Ordering::SeqCst);
                let delta_dir = tempdir.path().join(format!("delta-{attempt}"));
                async move { Ok(delta_dir) }
            },
        )
        .await;
    let second = coordinator
        .get_or_build_session(
            worktree,
            rebuild_key("head-a", 2),
            base_db,
            Some("base-hash".to_owned()),
            |_| {
                let attempt = builds.fetch_add(1, Ordering::SeqCst);
                let delta_dir = tempdir.path().join(format!("delta-{attempt}"));
                async move { Ok(delta_dir) }
            },
        )
        .await;

    assert_eq!(
        builds.load(Ordering::SeqCst),
        2,
        "changed dirty key should build a new merge session"
    );
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(second.delta_applied());
}

#[tokio::test]
async fn persistent_delta_failures_escalate_to_clean_rediff_attempt() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let coordinator = OverlaySessionCoordinator::new();
    let worktree = tempdir.path().join("repo");
    let base_db = tempdir.path().join("analyst.duckdb");
    let delta_dir = tempdir.path().join("delta");
    let key = rebuild_key("head-a", 1);
    let modes = Arc::new(Mutex::new(Vec::new()));

    for attempt in 1..=DELTA_FAILURE_ESCALATION_THRESHOLD {
        let modes = Arc::clone(&modes);
        let session = coordinator
            .get_or_build_session(
                worktree.clone(),
                key.clone(),
                base_db.clone(),
                Some("base-hash".to_owned()),
                move |mode| {
                    modes.lock().expect("modes").push(mode);
                    async move { Err(anyhow!("forced delta failure")) }
                },
            )
            .await;
        assert!(
            !session.delta_applied(),
            "attempt {attempt} should degrade to a base-only session"
        );
    }

    let modes_for_success = Arc::clone(&modes);
    let recovered = coordinator
        .get_or_build_session(
            worktree,
            key,
            base_db,
            Some("base-hash".to_owned()),
            move |mode| {
                modes_for_success.lock().expect("modes").push(mode);
                let delta_dir = delta_dir.clone();
                async move { Ok(delta_dir) }
            },
        )
        .await;

    assert!(recovered.delta_applied());
    let modes = modes.lock().expect("modes");
    assert_eq!(
        &modes[..DELTA_FAILURE_ESCALATION_THRESHOLD as usize],
        vec![OverlayBuildMode::IncrementalDelta; DELTA_FAILURE_ESCALATION_THRESHOLD as usize]
            .as_slice()
    );
    assert_eq!(
        modes[DELTA_FAILURE_ESCALATION_THRESHOLD as usize],
        OverlayBuildMode::CleanRediff,
        "the call after the threshold should force a clean re-diff attempt"
    );
}

fn overlay_candidate(root: &Path, name: &str, modified_secs: u64) -> OverlayDeltaCandidate {
    OverlayDeltaCandidate {
        path: root.join(name),
        modified: UNIX_EPOCH + Duration::from_secs(modified_secs),
    }
}

fn write_complete_overlay_dir(root: &Path, name: &str, modified_secs: u64) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).expect("overlay dir");
    fs::write(path.join("manifest.json"), "{}").expect("manifest");
    let file = fs::File::open(&path).expect("open overlay dir");
    file.set_modified(UNIX_EPOCH + Duration::from_secs(modified_secs))
        .expect("set overlay mtime");
    path
}

#[test]
fn stale_overlay_deltas_keeps_current_and_two_rollback_generations() {
    assert_eq!(RETAINED_OVERLAY_DELTAS, 3);
    let root = PathBuf::from("/overlays");
    let candidates = (1..=4)
        .map(|generation| overlay_candidate(&root, &format!("gen-{generation}"), generation))
        .collect();
    let protected = BTreeSet::from([root.join("gen-4")]);

    let stale = stale_overlay_deltas(candidates, &protected);

    assert_eq!(stale, vec![root.join("gen-1")]);
}

#[test]
fn stale_overlay_deltas_breaks_equal_modification_times_by_full_path() {
    let root = PathBuf::from("/overlays");
    let candidates = ["gen-d", "gen-b", "gen-a", "gen-c"]
        .map(|name| overlay_candidate(&root, name, 1))
        .into();

    let stale = stale_overlay_deltas(candidates, &BTreeSet::new());

    assert_eq!(stale, vec![root.join("gen-d")]);
}

#[test]
fn stale_overlay_deltas_keeps_an_older_protected_generation() {
    let root = PathBuf::from("/overlays");
    let candidates = (1..=5)
        .map(|generation| overlay_candidate(&root, &format!("gen-{generation}"), generation))
        .collect();
    let protected = BTreeSet::from([root.join("gen-1")]);

    let stale = stale_overlay_deltas(candidates, &protected);

    assert_eq!(stale, vec![root.join("gen-2")]);
}

#[test]
fn prune_overlay_deltas_skips_the_whole_pass_when_written_dir_is_outside_root() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let overlay_root = tempdir.path().join("analyst-overlays");
    fs::create_dir_all(&overlay_root).expect("overlay root");
    let completed: Vec<_> = (1..=4)
        .map(|generation| {
            write_complete_overlay_dir(&overlay_root, &format!("gen-{generation}"), generation)
        })
        .collect();
    let outside = tempdir.path().join("outside");
    fs::create_dir_all(&outside).expect("outside written dir");
    fs::write(outside.join("manifest.json"), "{}").expect("outside manifest");

    prune_overlay_deltas_best_effort(&overlay_root, &outside, &BTreeSet::new());

    assert!(completed.iter().all(|path| path.exists()));
}

#[test]
fn delete_stale_overlay_deltas_continues_after_an_individual_deletion_failure() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let missing = tempdir.path().join("missing");
    let stale = write_complete_overlay_dir(tempdir.path(), "stale", 1);

    delete_stale_overlay_deltas(vec![missing, stale.clone()]);

    assert!(!stale.exists());
}

fn complete_manifest_json() -> String {
    serde_json::to_string(&spur_graph::GraphArtifactManifest {
        graph_index_version: spur_graph::GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
        schema_version: "test".to_owned(),
        manifest_version: spur_graph::store::current_manifest_version(),
        graph_content_hash: "reuse-test".to_owned(),
        indexed_commit_oid: None,
        extractor_version: "test".to_owned(),
        complete: true,
        row_counts: spur_graph::store::parquet::GraphArtifactRowCounts {
            nodes: 0,
            edges: 0,
            edges_by_dst: None,
            edges_unresolved: 0,
            files: 0,
            file_manifests: 0,
            tombstones: 0,
            commits: 0,
            symbol_snapshots: 0,
            temporal_edges: 0,
            diagnostics: 0,
            ..spur_graph::store::parquet::GraphArtifactRowCounts::empty()
        },
        sidecar_complete: false,
        sidecar_row_counts: Default::default(),
        parquet_writer: spur_graph::store::parquet::GraphArtifactParquetWriter {
            compression: "zstd-3".to_owned(),
            row_group_size: 16_384,
        },
        edges_by_dst_present: false,
        temporal_shards: Vec::new(),
        ..spur_graph::GraphArtifactManifest::empty()
    })
    .expect("encode complete overlay manifest")
}

fn empty_previous_artifact() -> spur_graph::GraphIndexArtifact {
    artifact_with_file("src/lib.rs", "oid-unused")
}

fn artifact_with_file(path: &str, content_oid: &str) -> spur_graph::GraphIndexArtifact {
    spur_graph::GraphIndexArtifact {
        header: spur_graph::GraphIndexHeader {
            graph_index_version: spur_graph::GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: spur_graph::store::current_manifest_version(),
        graph_content_hash: "reuse-test".to_owned(),
        file_manifests: vec![spur_graph::GraphFileManifestEntry {
            stable_file_id: format!("file:{path}"),
            path: path.to_owned(),
            content_oid: content_oid.to_owned(),
            node_ids: Vec::new(),
        }],
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..spur_graph::GraphIndexArtifact::empty()
    }
}

fn init_git_repo(root: &Path) {
    run_git(root, &["init"]);
    run_git(root, &["config", "user.name", "SPUR Test"]);
    run_git(root, &["config", "user.email", "spur-test@example.invalid"]);
}

fn run_git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn write_delta_for_session_reuses_complete_dir_for_the_same_key() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let worktree = tempdir.path();
    let key = rebuild_key("abc123def4567890", 1);
    let delta_dir = worktree
        .join(".spur")
        .join("analyst-overlays")
        .join(key.cache_dir_name());
    fs::create_dir_all(&delta_dir).expect("delta dir");
    fs::write(delta_dir.join("manifest.json"), complete_manifest_json()).expect("manifest");
    fs::write(delta_dir.join("sentinel"), "keep-me").expect("sentinel");

    let written = write_delta_for_session(
        worktree,
        &key,
        &empty_previous_artifact(),
        OverlayBuildMode::IncrementalDelta,
    )
    .expect("reuse complete overlay dir");

    assert_eq!(written, delta_dir);
    assert_eq!(
        fs::read_to_string(delta_dir.join("sentinel")).expect("read sentinel"),
        "keep-me",
        "a complete overlay dir for the same dirty key must not be rewritten"
    );
}

#[test]
fn write_delta_for_session_rewrites_incomplete_dir_for_the_same_key() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let worktree = tempdir.path();
    let key = rebuild_key("abc123def4567890", 1);
    let delta_dir = worktree
        .join(".spur")
        .join("analyst-overlays")
        .join(key.cache_dir_name());
    fs::create_dir_all(&delta_dir).expect("delta dir");
    fs::write(delta_dir.join("sentinel"), "stale").expect("sentinel");

    let result = write_delta_for_session(
        worktree,
        &key,
        &empty_previous_artifact(),
        OverlayBuildMode::IncrementalDelta,
    );

    assert!(
        result.is_err() || !delta_dir.join("sentinel").exists(),
        "an incomplete overlay dir should be cleared before a rebuild"
    );
}

#[test]
fn write_delta_for_session_rewrites_complete_dir_when_manifest_is_incompatible() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let worktree = tempdir.path();
    let key = rebuild_key("abc123def4567890", 1);
    let delta_dir = worktree
        .join(".spur")
        .join("analyst-overlays")
        .join(key.cache_dir_name());
    fs::create_dir_all(&delta_dir).expect("delta dir");
    let mut manifest: spur_graph::GraphArtifactManifest =
        serde_json::from_str(&complete_manifest_json()).expect("manifest");
    manifest.manifest_version = "legacy-manifest".to_owned();
    fs::write(
        delta_dir.join("manifest.json"),
        serde_json::to_vec(&manifest).expect("encode manifest"),
    )
    .expect("manifest");
    fs::write(delta_dir.join("sentinel"), "stale").expect("sentinel");

    let result = write_delta_for_session(
        worktree,
        &key,
        &empty_previous_artifact(),
        OverlayBuildMode::IncrementalDelta,
    );

    assert!(result.is_ok(), "compatible delta rewrite should succeed");
    assert!(
        !delta_dir.join("sentinel").exists(),
        "a complete but incompatible overlay dir must be rebuilt"
    );
}

#[test]
fn prune_overlay_deltas_ignores_incomplete_dirs_and_keeps_newest_complete() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let overlay_root = tempdir.path().join("analyst-overlays");
    fs::create_dir_all(&overlay_root).expect("overlay root");
    let incomplete = overlay_root.join("incomplete");
    fs::create_dir_all(&incomplete).expect("incomplete dir");
    let completed: Vec<_> = (1..=4)
        .map(|generation| {
            write_complete_overlay_dir(&overlay_root, &format!("gen-{generation}"), generation)
        })
        .collect();
    let written = completed.last().expect("written").clone();

    prune_overlay_deltas_best_effort(&overlay_root, &written, &BTreeSet::from([written.clone()]));

    assert!(
        incomplete.exists(),
        "dirs without manifest.json are not prune candidates"
    );
    assert!(!completed[0].exists());
    assert!(completed[1].exists());
    assert!(completed[2].exists());
    assert!(completed[3].exists());
}
