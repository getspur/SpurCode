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
