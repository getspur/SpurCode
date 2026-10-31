use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use spur_graph::store::artifact_content_hash_blake3_hex;
use spur_graph::{
    artifact_from_facts, artifact_from_facts_incremental, build_facts, git_blob_oid,
    macro_invocation_site_id, BuildMode, GraphIndexArtifact, MacroEvidenceSource,
    MacroInvocationEvidenceSet, MacroScopeEvidence,
};

mod support;
use support::git_repo::GitRepo;

const SOURCE: &str = "#![allow(dead_code)]\nregister!();\ntype Alias = type_macro!();\nconst C: () = panic!(\"const\");\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn demo() { panic!(\"repeat\"); panic!(\"repeat\"); }\n}\n";

fn full(root: &Path) -> GraphIndexArtifact {
    let (facts, _) = build_facts(root, None).unwrap();
    artifact_from_facts(&facts, root).unwrap()
}

fn evidence(artifact: &GraphIndexArtifact) -> Value {
    let value = serde_json::to_value(artifact).unwrap();
    assert!(
        value["macro_invocations"].is_object(),
        "new artifacts must carry captured macro evidence"
    );
    value["macro_invocations"].clone()
}

fn fixture() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", SOURCE);
    repo.commit_all("baseline");
    repo
}

fn record_observed(name: &str, artifact: &GraphIndexArtifact, root: &Path) {
    if let Ok(output) = std::env::var("M3_EVIDENCE_DIR") {
        let output = Path::new(&output);
        std::fs::create_dir_all(output).unwrap();
        let sources: std::collections::BTreeMap<_, _> = artifact
            .file_manifests
            .iter()
            .filter(|file| file.path.ends_with(".rs"))
            .map(|file| {
                (
                    file.path.clone(),
                    std::fs::read_to_string(root.join(&file.path)).unwrap(),
                )
            })
            .collect();
        let authoritative = build_facts(root, None).unwrap().0;
        std::fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec_pretty(
                &serde_json::json!({"artifact": artifact, "sources": sources, "authoritative_rows": authoritative.macro_invocations, "authoritative_scopes": authoritative.macro_scopes}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn full_preserves_source_scopes_attributes_repeated_sites_and_json() {
    let repo = fixture();
    let artifact = full(repo.path());
    let value = evidence(&artifact);
    assert_eq!(value["source_scope"], "physical_rust_files");
    let set: MacroInvocationEvidenceSet = serde_json::from_value(value.clone()).unwrap();
    let scopes: Vec<MacroScopeEvidence> = serde_json::from_value(value["scopes"].clone()).unwrap();
    assert_eq!(set.rows.len(), 5);
    assert_eq!(
        set.rows
            .iter()
            .map(|r| &r.site_id)
            .collect::<BTreeSet<_>>()
            .len(),
        5
    );
    for row in &set.rows {
        assert_eq!(row.source_oid, git_blob_oid(SOURCE.as_bytes()));
        assert_eq!(
            row.site_id,
            macro_invocation_site_id(
                &row.file_path,
                &row.source_oid,
                row.byte_range,
                row.line_range
            )
        );
        let scope = scopes.iter().find(|s| s.scope == row.enclosing).unwrap();
        assert_eq!(scope.source_oid, row.source_oid);
        if let Some(caller) = &row.caller_stable_symbol_id {
            assert!(artifact
                .symbols
                .iter()
                .any(|s| &s.stable_symbol_id == caller && s.byte_range == scope.byte_range));
        }
        for attribute in &row.attributes {
            let owner = scopes.iter().find(|s| s.scope == attribute.owner).unwrap();
            assert_eq!(owner.file_path, row.file_path);
            assert_eq!(owner.source_oid, row.source_oid);
            assert_eq!(
                &SOURCE[attribute.byte_range[0]..attribute.byte_range[1]],
                attribute.raw_text
            );
        }
    }
    for kind in ["file", "type", "const"] {
        let row = value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["enclosing"]["kind"] == kind)
            .unwrap();
        assert!(row["caller_stable_symbol_id"].is_null());
    }
    let module_owner = scopes
        .iter()
        .find(|s| serde_json::to_value(&s.scope.kind).unwrap() == "module")
        .unwrap();
    assert!(set
        .rows
        .iter()
        .flat_map(|r| &r.attributes)
        .any(|a| a.owner == module_owner.scope));
    set.validate(
        &artifact.graph_content_hash,
        &[MacroEvidenceSource {
            file_path: "src/lib.rs",
            bytes: SOURCE.as_bytes(),
        }],
        &scopes,
    )
    .unwrap();
    let decoded: GraphIndexArtifact =
        serde_json::from_slice(&serde_json::to_vec(&artifact).unwrap()).unwrap();
    assert_eq!(evidence(&decoded), value);
    record_observed("full", &artifact, repo.path());
}

#[test]
fn new_empty_capture_and_legacy_absence_are_distinct() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "fn plain() {}\n");
    let artifact = full(repo.path());
    let set = evidence(&artifact);
    assert_eq!(set["rows"], serde_json::json!([]));
    assert_eq!(set["scopes"], serde_json::json!([]));
    assert_eq!(set["source_scope"], "physical_rust_files");
    let mut legacy = serde_json::to_value(artifact).unwrap();
    legacy.as_object_mut().unwrap().remove("macro_invocations");
    let decoded: GraphIndexArtifact = serde_json::from_value(legacy).unwrap();
    assert!(serde_json::to_value(decoded).unwrap()["macro_invocations"].is_null());
}

#[test]
fn persisted_scope_collection_is_required_unique_and_authoritative() {
    let repo = fixture();
    let artifact = full(repo.path());
    let value = evidence(&artifact);
    let scopes: Vec<MacroScopeEvidence> = serde_json::from_value(value["scopes"].clone()).unwrap();
    let sources = [MacroEvidenceSource {
        file_path: "src/lib.rs",
        bytes: SOURCE.as_bytes(),
    }];
    for mutation in [
        "missing",
        "duplicate",
        "conflict",
        "owner_location",
        "source_oid",
    ] {
        let mut bad = value.clone();
        match mutation {
            "missing" => bad["scopes"] = serde_json::json!([]),
            "duplicate" => {
                let first = bad["scopes"][0].clone();
                bad["scopes"].as_array_mut().unwrap().push(first);
            }
            "conflict" => bad["scopes"][0]["scope"]["kind"] = "unknown".into(),
            "owner_location" => bad["scopes"][0]["byte_range"][0] = 1.into(),
            "source_oid" => bad["rows"][0]["source_oid"] = "stale".into(),
            _ => unreachable!(),
        }
        let set: MacroInvocationEvidenceSet = serde_json::from_value(bad).unwrap();
        assert!(
            set.validate(&artifact.graph_content_hash, &sources, &scopes)
                .is_err(),
            "must reject {mutation}"
        );
    }
}

#[test]
fn evidence_only_const_type_and_file_scopes_survive_without_graph_symbols() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "#![allow(dead_code)]\nregister!();\nfn demo() { const { assert!(true); }; let _: [u8; size!()]; }\n");
    let (mut facts, _) = build_facts(repo.path(), None).unwrap();
    // Physical file extraction normally supplies its real File ID. The transport
    // contract also accepts a supplied evidence-only file ID without promoting it.
    let file_scope = facts
        .macro_scopes
        .iter_mut()
        .find(|s| s.scope.kind == spur_graph::MacroScopeKind::File)
        .unwrap();
    let old = file_scope.scope.clone();
    file_scope.scope.stable_symbol_id = format!(
        "macro-scope-v1:file:{}",
        macro_invocation_site_id(
            &file_scope.file_path,
            &file_scope.source_oid,
            file_scope.byte_range,
            file_scope.line_range
        )
    );
    let replacement = file_scope.scope.clone();
    for row in &mut facts.macro_invocations {
        if row.enclosing == old {
            row.enclosing = replacement.clone();
        }
        for attribute in &mut row.attributes {
            if attribute.owner == old {
                attribute.owner = replacement.clone();
            }
        }
    }
    let artifact = artifact_from_facts(&facts, repo.path()).unwrap();
    let set: MacroInvocationEvidenceSet = serde_json::from_value(evidence(&artifact)).unwrap();
    for kind in [
        spur_graph::MacroScopeKind::File,
        spur_graph::MacroScopeKind::Const,
        spur_graph::MacroScopeKind::Type,
    ] {
        let row = set.rows.iter().find(|r| r.enclosing.kind == kind).unwrap();
        assert!(row
            .enclosing
            .stable_symbol_id
            .starts_with("macro-scope-v1:"));
        assert!(row.caller_stable_symbol_id.is_none());
        assert!(!artifact
            .symbols
            .iter()
            .any(|s| s.stable_symbol_id == row.enclosing.stable_symbol_id));
        assert!(set.scopes.iter().any(|s| s.scope == row.enclosing));
    }
    assert_eq!(set.scopes, {
        let mut scopes = facts.macro_scopes;
        scopes.sort_by(|a, b| {
            (&a.file_path, a.byte_range, &a.scope.stable_symbol_id).cmp(&(
                &b.file_path,
                b.byte_range,
                &b.scope.stable_symbol_id,
            ))
        });
        scopes
    });
}

#[test]
fn evidence_and_scope_only_edits_change_canonical_hash() {
    let repo = fixture();
    let artifact = full(repo.path());
    let original = artifact_content_hash_blake3_hex(&artifact).unwrap();
    for member in ["rows", "scopes", "graph_content_hash"] {
        let mut value = serde_json::to_value(&artifact).unwrap();
        assert!(value["macro_invocations"].is_object());
        match member {
            "rows" => value["macro_invocations"]["rows"][0]["raw_macro_path"] = "edited".into(),
            "scopes" => value["macro_invocations"]["scopes"][0]["line_range"][0] = 99.into(),
            _ => value["macro_invocations"][member] = "different-snapshot".into(),
        }
        let mutated: GraphIndexArtifact = serde_json::from_value(value).unwrap();
        assert_ne!(
            artifact_content_hash_blake3_hex(&mutated).unwrap(),
            original,
            "hash must bind {member}"
        );
    }
}

#[test]
fn dirty_incremental_replaces_removes_renames_and_moves_scopes_losslessly() {
    let repo = fixture();
    repo.write("src/keep.rs", "fn keep() { assert!(true); }\n");
    repo.write("src/remove.rs", "const OLD: () = todo!();\n");
    repo.commit_all("more files");
    let baseline = full(repo.path());
    record_observed("baseline", &baseline, repo.path());
    let old = evidence(&baseline);
    repo.write("src/lib.rs", "fn changed() { unreachable!(); }\n");
    repo.remove("src/remove.rs");
    repo.write("src/moved.rs", SOURCE);
    let (incremental, mode, stats) =
        artifact_from_facts_incremental(&baseline, repo.path()).unwrap();
    assert_eq!(mode, BuildMode::Incremental);
    assert!(stats.reused_buckets > 0);
    let next = evidence(&incremental);
    assert_eq!(next, evidence(&full(repo.path())));
    record_observed("incremental", &incremental, repo.path());
    assert_ne!(next["graph_content_hash"], old["graph_content_hash"]);
    assert_eq!(next["graph_content_hash"], incremental.graph_content_hash);
    for row in next["rows"].as_array().unwrap() {
        assert_ne!(row["file_path"], "src/remove.rs");
        if row["file_path"] == "src/lib.rs" {
            assert_eq!(row["normalized_macro_path"], "unreachable");
            assert_eq!(
                row["source_oid"],
                git_blob_oid(b"fn changed() { unreachable!(); }\n")
            );
        }
    }
    for scope in next["scopes"].as_array().unwrap() {
        assert_ne!(scope["file_path"], "src/remove.rs");
    }
    let old_keep: Vec<_> = old["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["file_path"] == "src/keep.rs")
        .collect();
    let next_keep: Vec<_> = next["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["file_path"] == "src/keep.rs")
        .collect();
    assert_eq!(old_keep, next_keep);
    repo.git(&["mv", "src/keep.rs", "src/renamed.rs"]);
    let (renamed, _, _) = artifact_from_facts_incremental(&incremental, repo.path()).unwrap();
    assert_eq!(evidence(&renamed), evidence(&full(repo.path())));
    repo.write("src/moved.rs", "fn empty() {}\n");
    let (removed, _, _) = artifact_from_facts_incremental(&renamed, repo.path()).unwrap();
    let last = evidence(&removed);
    assert_eq!(last, evidence(&full(repo.path())));
    record_observed("removed", &removed, repo.path());
    assert!(!last["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["file_path"] == "src/moved.rs"));
}

#[test]
fn stale_fact_bytes_and_dangling_function_callers_are_rejected() {
    let repo = fixture();
    let (mut facts, _) = build_facts(repo.path(), None).unwrap();
    let caller = facts
        .macro_invocations
        .iter()
        .find_map(|r| r.caller_stable_symbol_id.clone())
        .unwrap();
    facts.nodes.retain(|node| node.stable_key != caller);
    assert!(
        artifact_from_facts(&facts, repo.path()).is_err(),
        "caller must reference an actual graph symbol"
    );
    let (facts, _) = build_facts(repo.path(), None).unwrap();
    repo.write("src/lib.rs", "fn new_bytes() { todo!(); }\n");
    assert!(
        artifact_from_facts(&facts, repo.path()).is_err(),
        "evidence cannot bind stale source bytes"
    );
}

#[test]
fn incremental_never_relabels_unavailable_or_incompatible_evidence_as_current() {
    let repo = fixture();
    let baseline = full(repo.path());
    let mut stale_snapshot = baseline.clone();
    stale_snapshot
        .macro_invocations
        .as_mut()
        .unwrap()
        .graph_content_hash = "other-snapshot".into();
    assert!(artifact_from_facts_incremental(&stale_snapshot, repo.path()).is_err());
    for field in ["missing", "extractor", "schema"] {
        let mut previous = baseline.clone();
        match field {
            "missing" => previous.macro_invocations = None,
            "extractor" => {
                previous
                    .macro_invocations
                    .as_mut()
                    .unwrap()
                    .extractor_version = "old-extractor".into()
            }
            _ => previous.macro_invocations.as_mut().unwrap().schema_version = "old-schema".into(),
        }
        let (rebuilt, mode, _) = artifact_from_facts_incremental(&previous, repo.path()).unwrap();
        assert_eq!(mode, BuildMode::Full, "must recapture {field} evidence");
        assert_eq!(evidence(&rebuilt), evidence(&baseline));
    }
}
