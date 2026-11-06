use spur_graph::{
    artifact_from_facts_for_paths, build_facts_for_paths, InMemoryClient, OverlayClient,
};
use std::{fs, path::PathBuf};

#[test]
fn selected_parent_component_keeps_named_function() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("inner")).unwrap();
    fs::write(root.path().join("entry.rs"), "pub fn entry() {}\n").unwrap();
    let paths = [PathBuf::from("inner/../entry.rs")];
    let facts = build_facts_for_paths(root.path(), &paths).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &paths).unwrap();
    assert!(
        graph.symbols.iter().any(|s| s.entity_name == "entry"),
        "selected build silently dropped the extracted named function"
    );
}

#[test]
fn overlay_shadow_key_matches_selected_manifest() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("inner")).unwrap();
    fs::write(root.path().join("entry.rs"), "pub struct Entry;\n").unwrap();
    let paths = [PathBuf::from("inner/../entry.rs")];
    let (graph, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths).unwrap();
    assert_eq!(graph.file_manifests[0].path, "entry.rs");
    assert!(
        shadowed.contains("entry.rs"),
        "overlay shadows the input spelling instead of its emitted source: {shadowed:?}"
    );
}

#[test]
fn selected_parent_component_preserves_tuple_owner() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("inner")).unwrap();
    fs::write(root.path().join("entry.rs"), "pub struct Entry(pub u8);\n").unwrap();
    let paths = [PathBuf::from("inner/../entry.rs")];
    let facts = build_facts_for_paths(root.path(), &paths).unwrap();
    let result = artifact_from_facts_for_paths(&facts, root.path(), &paths);
    assert!(
        result.is_ok(),
        "a valid selected path lost its extracted owner: {result:?}"
    );
}

#[test]
fn overlay_parent_component_preserves_tuple_owner() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("inner")).unwrap();
    fs::write(root.path().join("entry.rs"), "pub struct Entry(pub u8);\n").unwrap();
    let paths = [PathBuf::from("inner/../entry.rs")];
    let result = OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths);
    assert!(
        result.is_ok(),
        "overlay normalized discovery disagrees with extraction: {result:?}"
    );
}

#[cfg(unix)]
#[test]
fn selected_absolute_internal_symlink_preserves_tuple_owner() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("entry.rs"), "pub struct Entry(pub u8);\n").unwrap();
    std::os::unix::fs::symlink(root.path().join("entry.rs"), root.path().join("alias.rs")).unwrap();
    let paths = [root.path().join("alias.rs")];
    let facts = build_facts_for_paths(root.path(), &paths).unwrap();
    let result = artifact_from_facts_for_paths(&facts, root.path(), &paths);
    assert!(
        result.is_ok(),
        "absolute symlink changed source identity after extraction: {result:?}"
    );
}

#[test]
fn selected_duplicate_parent_paths_preserve_functions_tuple_and_macro_owners() {
    assert_selected_identity(false);
}

#[cfg(unix)]
#[test]
fn selected_relative_and_absolute_aliases_share_extraction_and_shadow_identity() {
    assert_selected_identity(true);
}

fn assert_selected_identity(aliases: bool) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("inner")).unwrap();
    let source = "pub struct Entry(pub u8);\npub fn entry() { println!(\"entry\"); }\n";
    fs::write(root.path().join("entry.rs"), source).unwrap();
    fs::write(root.path().join("unselected.rs"), "pub fn excluded() {}\n").unwrap();
    let canonical = [PathBuf::from("entry.rs")];
    let expected = artifact_from_facts_for_paths(
        &build_facts_for_paths(root.path(), &canonical).unwrap(),
        root.path(),
        &canonical,
    )
    .unwrap();
    let mut paths = vec![
        PathBuf::from("inner/../entry.rs"),
        PathBuf::from("./entry.rs"),
        PathBuf::from("unused/../entry.rs"),
        root.path().join("entry.rs"),
    ];
    #[cfg(unix)]
    if aliases {
        std::os::unix::fs::symlink("entry.rs", root.path().join("alias.rs")).unwrap();
        paths.extend([PathBuf::from("alias.rs"), root.path().join("alias.rs")]);
    }
    let facts = build_facts_for_paths(root.path(), &paths).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &paths).unwrap();
    assert_eq!(
        graph, expected,
        "aliases or duplicates changed selected graph owners/IDs"
    );
    let (overlay, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths).unwrap();
    assert_eq!(*overlay, expected);
    assert_eq!(
        shadowed,
        std::collections::HashSet::from(["entry.rs".to_owned()])
    );
    graph
        .validate_rust_evidence_sources(&[spur_graph::RustDeclarationSource {
            file_path: "entry.rs",
            bytes: source.as_bytes(),
        }])
        .unwrap();
    assert!(graph.symbols.iter().any(|s| s.entity_name == "entry"));
    assert!(graph.symbols.iter().any(|s| s.entity_name == "Entry"));
    assert!(!graph.macro_invocations.as_ref().unwrap().rows.is_empty());
    if let Ok(dir) = std::env::var("D3_EXPORT_DIR") {
        fs::create_dir_all(&dir).unwrap();
        fs::write(std::path::Path::new(&dir).join(if aliases { "normalized-aliases.json" } else { "normalized-parents.json" }),
            serde_json::to_vec_pretty(&serde_json::json!({"artifact": graph,
                "shadowed": shadowed, "source": source, "source_oid": spur_graph::git_blob_oid(source.as_bytes())})).unwrap()).unwrap();
    }
}

#[test]
fn deleted_parent_selection_has_only_canonical_shadow_and_no_source() {
    let root = tempfile::tempdir().unwrap();
    let paths = [
        PathBuf::from("unused/../deleted.rs"),
        PathBuf::from("deleted.rs"),
    ];
    let (graph, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths).unwrap();
    assert!(graph.file_manifests.is_empty());
    assert!(graph.rust_evidence.as_ref().unwrap().files.is_empty());
    assert_eq!(
        shadowed,
        std::collections::HashSet::from(["deleted.rs".to_owned()])
    );
}
