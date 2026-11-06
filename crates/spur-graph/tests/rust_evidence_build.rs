use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use spur_graph::{
    artifact_from_facts, artifact_from_facts_for_paths, build_facts, build_facts_for_paths,
    collect_rust_evidence, git_blob_oid, GraphIndexArtifact, InMemoryClient, OverlayClient,
    RustCallEvidence, RustDeclarationSource,
};

fn build(root: &Path) -> spur_graph::GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}

fn evidence_git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", "2026-11-06T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-06T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn gitlink_is_manifest_only_and_preserves_original_rust_inventory() {
    use spur_graph::{artifact_from_facts_incremental, store::BuildMode};
    let root = tempfile::tempdir().unwrap();
    evidence_git(root.path(), &["init"]);
    evidence_git(root.path(), &["config", "user.name", "Evidence Test"]);
    evidence_git(
        root.path(),
        &["config", "user.email", "evidence@example.test"],
    );
    let inputs = vec![(
        "keep.rs".into(),
        b"fn leaf() {} fn keep() { leaf(); }\n".to_vec(),
    )];
    write(root.path(), &inputs);
    evidence_git(root.path(), &["add", "keep.rs"]);
    evidence_git(root.path(), &["commit", "-m", "first real commit"]);
    let first_oid = String::from_utf8(evidence_git(root.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    let first_commit = evidence_git(root.path(), &["cat-file", "commit", &first_oid]);
    fs::write(root.path().join("note.txt"), "second commit\n").unwrap();
    evidence_git(root.path(), &["add", "note.txt"]);
    evidence_git(root.path(), &["commit", "-m", "second real commit"]);
    let second_oid = String::from_utf8(evidence_git(root.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    let second_commit = evidence_git(root.path(), &["cat-file", "commit", &second_oid]);
    let link = "vendor/submodule.rs";
    evidence_git(
        root.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &first_oid,
            link,
        ],
    );
    evidence_git(root.path(), &["commit", "-m", "add gitlink"]);
    assert!(
        !root.path().join(link).exists(),
        "gitlink has no disk source"
    );

    let full = build(root.path());
    let original = full.rust_evidence.as_ref().unwrap();
    assert_eq!(
        original
            .files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
        ["keep.rs"]
    );
    assert_eq!(
        full.file_manifests
            .iter()
            .find(|m| m.path == link)
            .unwrap()
            .content_oid,
        format!("gitlink:{first_oid}")
    );
    assert_eq!(original.origins[0].sources.len(), full.file_manifests.len());
    assert!(original.origins[0]
        .sources
        .iter()
        .any(|s| s.path == link && s.content_oid == format!("gitlink:{first_oid}")));
    assert_eq!(
        collect_rust_evidence(&full, &sources(&inputs)).unwrap(),
        *original
    );
    full.validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    assert!(
        collect_rust_evidence(&full, &[]).is_err(),
        "physical Rust cannot be omitted"
    );
    let mut bogus_sources = sources(&inputs);
    bogus_sources.push(RustDeclarationSource {
        file_path: link,
        bytes: &first_commit,
    });
    let gitlink_source_error = full
        .validate_rust_evidence_sources(&bogus_sources)
        .unwrap_err()
        .to_string();
    assert!(gitlink_source_error.contains("outside physical Rust graph domain"));
    assert!(collect_rust_evidence(&full, &bogus_sources).is_err());

    let (noop, mode, stats) = artifact_from_facts_incremental(&full, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, full.file_manifests.len());
    assert_eq!(stats.changed_paths, 0);
    assert!(stats.removed_paths.is_empty());
    assert_eq!(noop.rust_evidence, full.rust_evidence);
    let noop_packet = json!({"native":packet(&noop,&inputs),"stats":stats_value(mode,&stats)});

    evidence_git(
        root.path(),
        &["update-index", "--cacheinfo", "160000", &second_oid, link],
    );
    evidence_git(root.path(), &["commit", "-m", "move gitlink"]);
    let (changed, mode, stats) = artifact_from_facts_incremental(&noop, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, full.file_manifests.len() - 1);
    // These statistics count paths sent to extraction. The manifest-only
    // pointer is replaced without extracting a source or reusing its bucket.
    assert_eq!(stats.changed_paths, 0);
    assert!(stats.changed_or_added_paths.is_empty());
    assert!(stats.removed_paths.is_empty());
    assert_ne!(changed.graph_content_hash, full.graph_content_hash);
    assert_eq!(
        changed
            .file_manifests
            .iter()
            .find(|m| m.path == link)
            .unwrap()
            .content_oid,
        format!("gitlink:{second_oid}")
    );
    let current = changed.rust_evidence.as_ref().unwrap();
    assert_eq!(current.files, original.files);
    assert_eq!(current.origins, original.origins);
    changed
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    assert_eq!(
        collect_rust_evidence(&changed, &sources(&inputs))
            .unwrap()
            .origins[0]
            .graph_content_hash,
        changed.graph_content_hash
    );
    let changed_packet =
        json!({"native":packet(&changed,&inputs),"stats":stats_value(mode,&stats)});

    evidence_git(root.path(), &["update-index", "--force-remove", link]);
    let (deleted, mode, stats) = artifact_from_facts_incremental(&changed, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, 1);
    assert_eq!(stats.changed_paths, 0);
    assert_eq!(stats.removed_paths, BTreeSet::from([link.into()]));
    assert!(deleted.file_manifests.iter().all(|m| m.path != link));
    assert_eq!(
        deleted.rust_evidence.as_ref().unwrap().origins,
        original.origins
    );
    assert_eq!(
        deleted.rust_evidence.as_ref().unwrap().files,
        original.files
    );
    deleted
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    let deleted_packet =
        json!({"native":packet(&deleted,&inputs),"stats":stats_value(mode,&stats)});

    evidence_git(root.path(), &["rm", "keep.rs"]);
    let (pruned, mode, stats) = artifact_from_facts_incremental(&deleted, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, 0);
    assert_eq!(stats.changed_paths, 0);
    assert_eq!(stats.removed_paths, BTreeSet::from(["keep.rs".into()]));
    let empty = pruned.rust_evidence.as_ref().unwrap();
    assert!(empty.files.is_empty() && empty.origins.is_empty());
    pruned.validate_rust_evidence_sources(&[]).unwrap();
    export_d4(
        "gitlink.json",
        &json!({"full":packet(&full,&inputs),"noop":noop_packet,"pointer_change":changed_packet,"deleted_gitlink":deleted_packet,"pruned":packet(&pruned,&[]),"commits":[{"oid":first_oid,"bytes":first_commit},{"oid":second_oid,"bytes":second_commit}],"gitlink_source_error":gitlink_source_error}),
    );
}

#[test]
fn reuse_keeps_original_snapshot_and_deletion_removes_active_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("keep.rs"),
        "fn leaf() {} fn keep() { leaf(); }\n",
    )
    .unwrap();
    fs::write(root.path().join("edit.rs"), "pub struct Old(u8);\n").unwrap();
    let before = build(root.path());
    let original = before
        .rust_evidence
        .as_ref()
        .unwrap()
        .files
        .iter()
        .find(|f| f.file_path == "keep.rs")
        .unwrap()
        .clone();
    fs::write(root.path().join("edit.rs"), "pub struct New(u16);\n").unwrap();
    let (after, _, _) = spur_graph::artifact_from_facts_incremental(&before, root.path()).unwrap();
    assert_ne!(after.graph_content_hash, before.graph_content_hash);
    let set = after.rust_evidence.as_ref().expect("incremental carrier");
    assert_eq!(
        *set.files.iter().find(|f| f.file_path == "keep.rs").unwrap(),
        original
    );
    assert_eq!(
        original.origin_graph_content_hash,
        before.graph_content_hash
    );
    fs::remove_file(root.path().join("edit.rs")).unwrap();
    let (deleted, _, _) = spur_graph::artifact_from_facts_incremental(&after, root.path()).unwrap();
    assert!(deleted
        .rust_evidence
        .as_ref()
        .unwrap()
        .files
        .iter()
        .all(|f| f.file_path != "edit.rs"));
    assert!(deleted.tombstones.iter().any(|t| t.path == "edit.rs"));
}

fn export_d4(name: &str, value: &Value) {
    if let Ok(dir) = std::env::var("D4_EXPORT_DIR") {
        let dir = PathBuf::from(dir);
        let dir = if dir.is_absolute() {
            dir
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(dir)
        };
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
}

fn stats_value(mode: spur_graph::store::BuildMode, stats: &spur_graph::build::BuildStats) -> Value {
    json!({"mode":format!("{mode:?}"),"reused_buckets":stats.reused_buckets,
        "changed_paths":stats.changed_paths,"changed_or_added_paths":stats.changed_or_added_paths,
        "removed_paths":stats.removed_paths})
}

#[test]
fn composition_preserves_whole_origins_zero_unknown_and_selected_snapshot() {
    use spur_graph::{artifact_from_facts_incremental, store::BuildMode};
    let root = tempfile::tempdir().unwrap();
    let mut inputs = fixtures();
    write(root.path(), &inputs);
    let before = build(root.path());
    let original = before.rust_evidence.clone().unwrap();
    export_d4("full.json", &packet(&before, &inputs));
    let (noop, mode, stats) = artifact_from_facts_incremental(&before, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, inputs.len());
    assert_eq!(stats.changed_paths, 0);
    assert!(stats.removed_paths.is_empty());
    assert_eq!(noop.rust_evidence, before.rust_evidence);
    export_d4(
        "noop.json",
        &json!({"native":packet(&noop, &inputs),"stats":stats_value(mode,&stats)}),
    );

    inputs.iter_mut().find(|(p, _)| p == "empty.rs").unwrap().1 =
        b"pub struct New(u16);\n".to_vec();
    write(root.path(), &inputs);
    let selected_paths = [PathBuf::from("empty.rs")];
    let selected_facts = build_facts_for_paths(root.path(), &selected_paths).unwrap();
    let selected =
        artifact_from_facts_for_paths(&selected_facts, root.path(), &selected_paths).unwrap();
    let (edited, mode, stats) = artifact_from_facts_incremental(&noop, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, inputs.len() - 1);
    assert_eq!(stats.changed_paths, 1);
    assert_eq!(
        stats.changed_or_added_paths,
        BTreeSet::from(["empty.rs".into()])
    );
    assert!(stats.removed_paths.is_empty());
    let set = edited.rust_evidence.as_ref().expect("edited carrier");
    assert_eq!(set.graph_content_hash, edited.graph_content_hash);
    assert_ne!(selected.graph_content_hash, edited.graph_content_hash);
    assert_eq!(
        set.origins
            .iter()
            .find(|o| o.graph_content_hash == before.graph_content_hash)
            .unwrap(),
        &original.origins[0]
    );
    for file in &original.files {
        let current = set
            .files
            .iter()
            .find(|f| f.file_path == file.file_path)
            .unwrap();
        if file.file_path == "empty.rs" {
            assert_eq!(current, &selected.rust_evidence.as_ref().unwrap().files[0]);
            assert_eq!(
                current.origin_graph_content_hash,
                selected.graph_content_hash
            );
            assert_ne!(current, file);
        } else {
            assert_eq!(current, file);
        }
    }
    edited
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    export_d4(
        "edit.json",
        &json!({"native":packet(&edited,&inputs),"selected":packet(&selected,&inputs.iter().filter(|(p,_)|p=="empty.rs").cloned().collect::<Vec<_>>()),"stats":stats_value(mode,&stats)}),
    );

    fs::remove_file(root.path().join("cfg.rs")).unwrap();
    inputs.retain(|(p, _)| p != "cfg.rs");
    let (deleted, mode, stats) = artifact_from_facts_incremental(&edited, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, inputs.len());
    assert_eq!(stats.changed_paths, 0);
    assert_eq!(stats.removed_paths, BTreeSet::from(["cfg.rs".into()]));
    let deleted_set = deleted.rust_evidence.as_ref().unwrap();
    assert_eq!(deleted_set.origins, set.origins);
    assert!(deleted_set.files.iter().all(|f| f.file_path != "cfg.rs"));
    assert!(deleted.file_manifests.iter().all(|f| f.path != "cfg.rs"));
    assert!(deleted.tombstones.iter().any(|t| t.path == "cfg.rs"));
    deleted
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    export_d4(
        "delete.json",
        &json!({"native":packet(&deleted,&inputs),"stats":stats_value(mode,&stats)}),
    );

    inputs.push(("cfg.rs".into(), b"pub fn returned() {}\n".to_vec()));
    write(root.path(), &inputs);
    let (readded, mode, stats) = artifact_from_facts_incremental(&deleted, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.changed_paths, 1);
    assert_eq!(stats.reused_buckets, inputs.len() - 1);
    assert!(stats.removed_paths.is_empty());
    assert_ne!(
        readded
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .find(|f| f.file_path == "cfg.rs")
            .unwrap(),
        original
            .files
            .iter()
            .find(|f| f.file_path == "cfg.rs")
            .unwrap()
    );
    readded
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    export_d4(
        "readd.json",
        &json!({"native":packet(&readded,&inputs),"stats":stats_value(mode,&stats)}),
    );

    for (path, _) in &inputs {
        if path != "cfg.rs" {
            fs::remove_file(root.path().join(path)).unwrap();
        }
    }
    let (last, mode, stats) = artifact_from_facts_incremental(&readded, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.removed_paths.len(), inputs.len() - 1);
    assert_eq!(last.rust_evidence.as_ref().unwrap().origins.len(), 1);
    assert_eq!(
        last.rust_evidence.as_ref().unwrap().origins[0],
        readded
            .rust_evidence
            .as_ref()
            .unwrap()
            .origins
            .iter()
            .find(|o| o.graph_content_hash
                == readded
                    .rust_evidence
                    .as_ref()
                    .unwrap()
                    .files
                    .iter()
                    .find(|f| f.file_path == "cfg.rs")
                    .unwrap()
                    .origin_graph_content_hash)
            .unwrap()
            .clone()
    );
    fs::remove_file(root.path().join("cfg.rs")).unwrap();
    let (empty, mode, stats) = artifact_from_facts_incremental(&last, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert_eq!(stats.removed_paths, BTreeSet::from(["cfg.rs".into()]));
    let empty_set = empty.rust_evidence.as_ref().unwrap();
    assert!(empty_set.files.is_empty() && empty_set.origins.is_empty());
    empty.validate_rust_evidence_sources(&[]).unwrap();
    export_d4("pruned-empty.json", &packet(&empty, &[]));
}

#[test]
fn absent_or_incompatible_versions_rebuild_full() {
    use spur_graph::{artifact_from_facts_incremental, build::BuildStats, store::BuildMode};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "fn a() {}\n").unwrap();
    let graph = build(root.path());
    let mut results = Vec::new();
    for field in [
        "absent",
        "manifest",
        "wrapper_schema",
        "wrapper_extractor",
        "declaration_schema",
        "declaration_extractor",
        "call_schema",
        "call_extractor",
    ] {
        let mut prior = serde_json::to_value(&graph).unwrap();
        match field {
            "absent" => prior["rust_evidence"] = Value::Null,
            "manifest" => prior["manifest_version"] = json!("old"),
            "wrapper_schema" => prior["rust_evidence"]["schema_version"] = json!("old"),
            "wrapper_extractor" => prior["rust_evidence"]["extractor_version"] = json!("old"),
            "declaration_schema" => {
                prior["rust_evidence"]["files"][0]["declarations"]["schema_version"] = json!("old")
            }
            "declaration_extractor" => {
                prior["rust_evidence"]["files"][0]["declarations"]["extractor_version"] =
                    json!("old")
            }
            "call_schema" => {
                prior["rust_evidence"]["files"][0]["calls"]["complete"]["evidence"]
                    ["schema_version"] = json!("old")
            }
            "call_extractor" => {
                prior["rust_evidence"]["files"][0]["calls"]["complete"]["evidence"]
                    ["extractor_version"] = json!("old")
            }
            _ => unreachable!(),
        }
        let prior: GraphIndexArtifact = serde_json::from_value(prior).unwrap();
        let (rebuilt, mode, stats) = artifact_from_facts_incremental(&prior, root.path()).unwrap();
        assert_eq!(mode, BuildMode::Full, "{field}");
        assert_eq!(stats, BuildStats::default());
        assert_eq!(rebuilt.rust_evidence, graph.rust_evidence);
        let inputs = vec![("a.rs".into(), fs::read(root.path().join("a.rs")).unwrap())];
        results.push(json!({"case":field,"stats":stats_value(mode,&stats),"before":prior,"native":packet(&rebuilt,&inputs)}));
    }
    export_d4("compatibility.json", &json!(results));
}

fn mutation_verdict(
    name: &str,
    mut graph: GraphIndexArtifact,
    root: &Path,
    inputs: &[(String, Vec<u8>)],
) -> Value {
    let before = graph
        .validate_rust_evidence_sources(&sources(inputs))
        .err()
        .map(|e| format!("{e:#}"));
    let stored = graph
        .rust_evidence
        .as_ref()
        .unwrap()
        .validate_stored(&graph)
        .err()
        .map(|e| format!("{e:#}"));
    let incremental = spur_graph::artifact_from_facts_incremental(&graph, root)
        .err()
        .map(|e| format!("{e:#}"));
    assert!(before.is_some(), "source validation accepted {name}");
    assert!(incremental.is_some(), "incremental accepted {name}");
    let hash = spur_graph::artifact_content_hash_blake3_hex(&graph).unwrap();
    graph.header.content_hash_blake3 = Some(hash.clone());
    let after = graph
        .validate_rust_evidence_sources(&sources(inputs))
        .err()
        .map(|e| format!("{e:#}"));
    let rehashed_incremental = spur_graph::artifact_from_facts_incremental(&graph, root)
        .err()
        .map(|e| format!("{e:#}"));
    assert!(
        after.is_some() && rehashed_incremental.is_some(),
        "canonical rehash masked {name}"
    );
    let source_inventory = packet(&graph, inputs)["sources"].clone();
    json!({"case":name,"artifact":graph,"sources":source_inventory,"stored_error":stored,"source_error_before":before,"incremental_error_before":incremental,"native_canonical_hash":hash,"source_error_after":after,"incremental_error_after":rehashed_incremental})
}

#[test]
fn compatible_reused_forgery_is_rejected_even_after_native_rehash() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![
        (
            "a.rs".into(),
            b"fn leaf() {} fn first() { leaf(); } fn second() { leaf(); }\n".to_vec(),
        ),
        ("edit.rs".into(), b"pub struct Old(u8);\n".to_vec()),
    ];
    write(root.path(), &inputs);
    let graph = build(root.path());
    let mut verdicts = Vec::new();
    let mut wrong_oid = graph.clone();
    wrong_oid.rust_evidence.as_mut().unwrap().files[0].source_oid = git_blob_oid(b"wrong");
    verdicts.push(mutation_verdict(
        "wrong_reused_oid",
        wrong_oid,
        root.path(),
        &inputs,
    ));
    let mut swapped = graph.clone();
    if let RustCallEvidence::Complete { evidence } =
        &mut swapped.rust_evidence.as_mut().unwrap().files[0].calls
    {
        let first = evidence.bodies[1].definition_stable_symbol_id.clone();
        evidence.bodies[1].definition_stable_symbol_id =
            evidence.bodies[2].definition_stable_symbol_id.clone();
        evidence.bodies[2].definition_stable_symbol_id = first;
    } else {
        panic!("healthy calls");
    }
    verdicts.push(mutation_verdict(
        "swapped_existing_owners",
        swapped,
        root.path(),
        &inputs,
    ));
    let mut omitted = graph.clone();
    if let RustCallEvidence::Complete { evidence } =
        &mut omitted.rust_evidence.as_mut().unwrap().files[0].calls
    {
        evidence.sites.pop().unwrap();
        evidence.sources[0].site_count -= 1;
    }
    omitted
        .rust_evidence
        .as_ref()
        .unwrap()
        .validate_stored(&omitted)
        .expect("FK/shape-valid omission control");
    verdicts.push(mutation_verdict(
        "omitted_site_fk_valid",
        omitted,
        root.path(),
        &inputs,
    ));
    let mut edited_inputs = inputs.clone();
    edited_inputs[1].1 = b"pub struct New(u16);\n".to_vec();
    write(root.path(), &edited_inputs);
    let (after, _, _) = spur_graph::artifact_from_facts_incremental(&graph, root.path()).unwrap();
    let mut rebound = after.clone();
    rebound.rust_evidence.as_mut().unwrap().files[0]
        .declarations
        .graph_content_hash = after.graph_content_hash.clone();
    verdicts.push(mutation_verdict(
        "old_nested_header_rebound_to_aggregate",
        rebound,
        root.path(),
        &edited_inputs,
    ));

    // Delete the old site from a changed source. Reinserting it with its old
    // source binding must fail even after hashing the actual mutated artifact.
    edited_inputs[0].1 = b"fn leaf() {} fn first() {} fn second() {}\n".to_vec();
    write(root.path(), &edited_inputs);
    let (clean, _, _) = spur_graph::artifact_from_facts_incremental(&after, root.path()).unwrap();
    let mut reinserted = clean;
    reinserted.rust_evidence.as_mut().unwrap().files[0].calls =
        graph.rust_evidence.as_ref().unwrap().files[0].calls.clone();
    verdicts.push(mutation_verdict(
        "deleted_site_reinserted",
        reinserted,
        root.path(),
        &edited_inputs,
    ));
    export_d4("mutations.json", &json!(verdicts));
}

#[test]
fn unknown_cannot_be_relabelled_complete_zero() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![
        ("empty.rs".into(), b"pub struct Empty;\n".to_vec()),
        ("invalid.rs".into(), vec![0xff, b'\n']),
    ];
    write(root.path(), &inputs);
    let mut graph = build(root.path());
    let set = graph.rust_evidence.as_mut().unwrap();
    let mut calls = set.files[0].calls.clone();
    let target = &set.files[1];
    assert!(matches!(target.calls, RustCallEvidence::Unknown { .. }));
    if let RustCallEvidence::Complete { evidence } = &mut calls {
        let source = &mut evidence.sources[0];
        source.file_path = target.file_path.clone();
        source.stable_file_id = target.stable_file_id.clone();
        source.source_oid = target.source_oid.clone();
        source.byte_length = target.byte_length;
    } else {
        panic!("complete zero control");
    }
    set.files[1].calls = calls;
    graph
        .rust_evidence
        .as_ref()
        .unwrap()
        .validate_stored(&graph)
        .unwrap();
    export_d4(
        "unknown-mutation.json",
        &mutation_verdict(
            "unknown_relabelled_complete_zero_fk_valid",
            graph,
            root.path(),
            &inputs,
        ),
    );
}

#[test]
fn full_build_captures_zero_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = build(root.path());
    let value = serde_json::to_value(&graph).unwrap();
    assert!(value["rust_evidence"].is_object());
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let pre: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/root-pre/inputs.json")).unwrap();
    pre["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let hex = f["hex"].as_str().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (f["path"].as_str().unwrap().to_owned(), bytes)
        })
        .collect()
}

fn write(root: &Path, inputs: &[(String, Vec<u8>)]) {
    for (path, bytes) in inputs {
        fs::write(root.join(path), bytes).unwrap();
    }
}

fn sources(inputs: &[(String, Vec<u8>)]) -> Vec<RustDeclarationSource<'_>> {
    inputs
        .iter()
        .map(|(path, bytes)| RustDeclarationSource {
            file_path: path,
            bytes,
        })
        .collect()
}

fn export(name: &str, graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) {
    export_value(name, &packet(graph, inputs));
}

fn export_value(name: &str, value: &Value) {
    if let Ok(dir) = std::env::var("D3_EXPORT_DIR") {
        let dir = PathBuf::from(dir);
        let dir = if dir.is_absolute() {
            dir
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(dir)
        };
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            Path::new(&dir).join(name),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

fn packet(graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) -> Value {
    use sha2::{Digest, Sha256};
    json!({"artifact":graph,
        "sources": inputs.iter().map(|(path, bytes)| json!({"path":path,
            "hex":bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "sha256":format!("{:x}", Sha256::digest(bytes)), "source_oid":git_blob_oid(bytes),
            "byte_length":bytes.len()})).collect::<Vec<_>>(),
        "canonical_hash":spur_graph::artifact_content_hash_blake3_hex(graph).unwrap(),
        "capability_digest":spur_graph::rust_evidence_capability_digest(graph).unwrap(),
        "envelope_checksum":graph.rust_evidence.as_ref().map(|s| spur_graph::rust_evidence_checksum_blake3_hex(s).unwrap()),
        "source_error": graph.validate_rust_evidence_sources(&sources(inputs)).err().map(|e| format!("{e:#}"))})
}

fn paths(graph: &GraphIndexArtifact) -> BTreeSet<&str> {
    graph
        .file_manifests
        .iter()
        .map(|m| m.path.as_str())
        .collect()
}

#[test]
fn full_six_sources_match_accepted_manual_collection() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let graph = build(root.path());
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    let set = graph.rust_evidence.as_ref().unwrap();
    assert_eq!(set.files.len(), 6);
    assert_eq!(
        *set,
        collect_rust_evidence(&graph, &sources(&inputs)).unwrap()
    );
    let manual: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/ROOT-review/actual-full/populated.json")).unwrap();
    assert_eq!(
        serde_json::to_value(set).unwrap(),
        manual["present"]["artifact"]["rust_evidence"]
    );
    for file in &set.files {
        let input = inputs.iter().find(|(p, _)| p == &file.file_path).unwrap();
        assert_eq!(file.source_oid, git_blob_oid(&input.1));
        assert_eq!(file.byte_length, input.1.len());
    }
    let main = set.files.iter().find(|f| f.file_path == "main.rs").unwrap();
    match &main.calls {
        RustCallEvidence::Complete { evidence } => {
            assert_eq!(evidence.bodies.len(), 3);
            assert_eq!(evidence.sites.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    export("full.json", &graph, &inputs);
}

#[test]
fn selected_domain_contains_only_requested_current_files() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let selected = [PathBuf::from("main.rs"), PathBuf::from("empty.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs", "main.rs"]));
    let inputs: Vec<_> = inputs
        .into_iter()
        .filter(|(p, _)| p == "main.rs" || p == "empty.rs")
        .collect();
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    assert_eq!(graph.rust_evidence.as_ref().unwrap().files.len(), 2);
    export("selected.json", &graph, &inputs);
}

#[test]
fn partial_facts_never_advertise_unselected_zero_coverage() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![
        ("a.rs".into(), b"pub fn a() {}\n".to_vec()),
        ("b.rs".into(), b"pub struct B;\n".to_vec()),
    ];
    write(root.path(), &inputs);
    let selected = [PathBuf::from("a.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["a.rs"]));
    assert_eq!(
        graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
        ["a.rs"]
    );
}

#[test]
fn selected_normalizes_duplicates_and_rejects_escaping_paths() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let absolute = root.path().canonicalize().unwrap().join("empty.rs");
    let selected = [
        PathBuf::from("./empty.rs"),
        PathBuf::from("unused/../empty.rs"),
        absolute,
    ];
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs"]));
    assert_eq!(graph.rust_evidence.as_ref().unwrap().files.len(), 1);
    for path in [PathBuf::from("../outside.rs"), PathBuf::from("/outside.rs")] {
        assert!(artifact_from_facts_for_paths(&facts, root.path(), &[path]).is_err());
    }
}

#[test]
fn empty_workspace_is_captured_and_deleted_selection_has_no_active_source() {
    let root = tempfile::tempdir().unwrap();
    let graph = build(root.path());
    let set = graph.rust_evidence.as_ref().unwrap();
    assert!(set.files.is_empty() && set.origins.is_empty());
    graph.validate_rust_evidence_sources(&[]).unwrap();
    export("empty.json", &graph, &[]);
    let selected = [PathBuf::from("deleted.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert!(graph.file_manifests.is_empty());
    assert!(graph.rust_evidence.as_ref().unwrap().files.is_empty());
}

#[test]
fn mixed_nonrust_and_virtual_sources_do_not_enter_rust_domain() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![("empty.rs".into(), b"pub struct Empty;\n".to_vec()), ("helper.py".into(), b"def hello():\n    pass\n".to_vec()),
        ("book.ipynb".into(), br##"{"cells":[{"cell_type":"code","id":"r","metadata":{"language":"rust"},"source":["pub fn cell() {}"],"outputs":[],"execution_count":null}],"metadata":{"kernelspec":{"language":"rust","name":"rust","display_name":"Rust"}},"nbformat":4,"nbformat_minor":5}"##.to_vec())];
    write(root.path(), &inputs);
    let graph = build(root.path());
    assert!(paths(&graph).contains("helper.py"));
    assert_eq!(
        graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
        ["empty.rs"]
    );
    graph
        .validate_rust_evidence_sources(&sources(&inputs[..1]))
        .unwrap();
    export("mixed.json", &graph, &inputs[..1]);
}

#[test]
fn missing_tuple_and_foreign_owners_are_integrity_errors() {
    let mut errors = Vec::new();
    for path in ["ffi.rs", "recovery.rs"] {
        let root = tempfile::tempdir().unwrap();
        let inputs: Vec<_> = fixtures()
            .into_iter()
            .filter(|(p, _)| p == path || p == "empty.rs")
            .collect();
        write(root.path(), &inputs);
        let mut facts = build_facts(root.path(), None).unwrap().0;
        facts.nodes.retain(|n| n.kind == spur_graph::NodeKind::File);
        let error = artifact_from_facts(&facts, root.path()).unwrap_err();
        assert!(!error.to_string().is_empty());
        errors.push(json!({"source":path,"error":format!("{error:#}")}));
    }
    export_value("owner-errors.json", &json!(errors));
}

#[test]
fn source_and_oid_mutations_fail_current_native_validation() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let graph = build(root.path());
    let mut mutated = inputs.clone();
    mutated.iter_mut().find(|(p, _)| p == "main.rs").unwrap().1[0] = b'X';
    assert!(graph
        .validate_rust_evidence_sources(&sources(&mutated))
        .is_err());
    let mut forged = graph.clone();
    forged
        .file_manifests
        .iter_mut()
        .find(|m| m.path == "main.rs")
        .unwrap()
        .content_oid = git_blob_oid(b"wrong\n");
    assert!(forged
        .validate_rust_evidence_sources(&sources(&inputs))
        .is_err());
    let mut verdicts = vec![
        json!({"case":"unchanged_graph_changed_source", "error":graph.validate_rust_evidence_sources(&sources(&mutated)).unwrap_err().to_string()}),
        json!({"case":"manifest_oid_mutation", "error":forged.validate_rust_evidence_sources(&sources(&inputs)).unwrap_err().to_string()}),
    ];
    for path in ["ffi.rs", "recovery.rs"] {
        let mut forged = graph.clone();
        forged.symbols.retain(|s| s.file_path != path);
        verdicts.push(json!({"case":format!("missing_owner:{path}"), "error":forged.validate_rust_evidence_sources(&sources(&inputs)).unwrap_err().to_string()}));
    }
    export_value("mutation-verdicts.json", &json!(verdicts));
}

#[test]
fn selected_full_facts_restrict_macro_carriers_before_composition() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.rs"),
        "pub fn a() { println!(\"a\"); }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("b.rs"),
        "pub fn b() { println!(\"b\"); }\n",
    )
    .unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let graph =
        artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("a.rs")]).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["a.rs"]));
    assert!(graph
        .macro_invocations
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(|r| r.file_path == "a.rs"));
}

#[test]
fn stale_index_oid_is_rejected_after_current_bytes_are_read() {
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    git(&["add", "empty.rs"]);
    git(&[
        "-c",
        "user.name=D3 fixture",
        "-c",
        "user.email=d3@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--no-verify",
        "-qm",
        "fixture",
    ]);
    let before = build(root.path());
    git(&["update-index", "--assume-unchanged", "empty.rs"]);
    fs::write(root.path().join("empty.rs"), "pub struct Other;\n").unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let error = artifact_from_facts(&facts, root.path()).unwrap_err();
    assert!(
        error.to_string().contains("source OID mismatch"),
        "{error:#}"
    );
    let reused_error =
        spur_graph::artifact_from_facts_incremental(&before, root.path()).unwrap_err();
    assert!(
        reused_error.to_string().contains("source OID mismatch"),
        "{reused_error:#}"
    );
    export_d4(
        "reused-source-race.json",
        &json!({"error":format!("{reused_error:#}"),"method":"unchanged index OID classified reused, freshly read bytes disagree"}),
    );
    export_value(
        "oid-race-error.json",
        &json!({"error":format!("{error:#}"),"method":"tracked assume-unchanged index OID versus freshly read worktree bytes"}),
    );
}

#[test]
fn selected_io_errors_and_symlink_escapes_are_not_unknown() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("directory.rs")).unwrap();
    let facts = spur_graph::extract::GraphFacts::empty();
    assert!(
        artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("directory.rs")])
            .is_err()
    );
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.rs"), "pub struct Outside;\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("outside.rs"),
            root.path().join("escape.rs"),
        )
        .unwrap();
        assert!(
            artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("escape.rs")])
                .is_err()
        );
    }
}

#[test]
fn overlay_extraction_uses_exact_selected_domain() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let selected = [PathBuf::from("main.rs"), PathBuf::from("empty.rs")];
    let (graph, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs", "main.rs"]));
    assert_eq!(shadowed.len(), 2);
    let inputs: Vec<_> = inputs
        .into_iter()
        .filter(|(p, _)| p == "main.rs" || p == "empty.rs")
        .collect();
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    export("overlay.json", &graph, &inputs);
}

/// Reusable native validator: D3_PACKET names an arbitrary exported/forged packet.
#[test]
fn replay_packet_through_current_api() {
    let Ok(path) = std::env::var("D3_PACKET") else {
        return;
    };
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let graph: GraphIndexArtifact = serde_json::from_value(value["artifact"].clone()).unwrap();
    let inputs: Vec<_> = value["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let hex = s["hex"].as_str().unwrap();
            (
                s["path"].as_str().unwrap().to_owned(),
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            )
        })
        .collect();
    let result = packet(&graph, &inputs);
    println!(
        "{}",
        json!({"source_error":result["source_error"],"canonical_hash":result["canonical_hash"],"capability_digest":result["capability_digest"],"envelope_checksum":result["envelope_checksum"]})
    );
    if let Ok(out) = std::env::var("D3_REPLAY_OUTPUT") {
        fs::write(out, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}
