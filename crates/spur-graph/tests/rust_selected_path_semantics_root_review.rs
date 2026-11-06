#[cfg(unix)]
mod unix {
    use spur_graph::{
        artifact_from_facts_for_paths, build_facts_for_paths, InMemoryClient, OverlayClient,
    };
    use std::{collections::HashSet, fs, path::PathBuf};

    #[test]
    fn relative_symlink_parent_and_absolute_path_select_the_same_existing_source() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("inner/deep")).unwrap();
        fs::write(
            root.path().join("entry.rs"),
            "pub struct WrongRoot(pub u8);\n",
        )
        .unwrap();
        fs::write(
            root.path().join("inner/entry.rs"),
            "pub struct IntendedInner(pub u16);\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("inner/deep", root.path().join("link")).unwrap();
        let relative = PathBuf::from("link/../entry.rs");
        let absolute = root.path().join(&relative);
        assert_eq!(
            absolute.canonicalize().unwrap(),
            root.path().join("inner/entry.rs").canonicalize().unwrap()
        );
        let relative_graph = artifact_from_facts_for_paths(
            &build_facts_for_paths(root.path(), &[relative.clone()]).unwrap(),
            root.path(),
            &[relative],
        )
        .unwrap();
        let absolute_graph = artifact_from_facts_for_paths(
            &build_facts_for_paths(root.path(), &[absolute.clone()]).unwrap(),
            root.path(),
            &[absolute],
        )
        .unwrap();
        assert_eq!(
            relative_graph, absolute_graph,
            "the same existing filesystem path selected different source bytes/owners"
        );
        assert_eq!(relative_graph.file_manifests[0].path, "inner/entry.rs");
        let (overlay, shadowed) = OverlayClient::<InMemoryClient>::extract_delta(
            root.path(),
            &[PathBuf::from("link/../entry.rs")],
        )
        .unwrap();
        assert_eq!(*overlay, absolute_graph);
        assert_eq!(shadowed, HashSet::from(["inner/entry.rs".to_owned()]));
        export(
            "symlink-parent-relative",
            &relative_graph,
            &shadowed,
            &[("inner/entry.rs", b"pub struct IntendedInner(pub u16);\n")],
        );
        export(
            "symlink-parent-absolute",
            &absolute_graph,
            &shadowed,
            &[("inner/entry.rs", b"pub struct IntendedInner(pub u16);\n")],
        );
    }

    #[test]
    fn missing_absolute_path_through_root_alias_retains_deleted_shadow() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("real");
        fs::create_dir(&root).unwrap();
        let alias = parent.path().join("root-alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let result =
            OverlayClient::<InMemoryClient>::extract_delta(&root, &[alias.join("deleted.rs")]);
        assert!(
            result.is_ok(),
            "a missing leaf beneath an internal root alias was rejected: {result:?}"
        );
        let (graph, shadowed) = result.unwrap();
        assert!(graph.file_manifests.is_empty());
        assert_eq!(shadowed, HashSet::from(["deleted.rs".to_owned()]));
        export("missing-root-alias", &graph, &shadowed, &[]);
    }

    #[test]
    fn duplicate_physical_parent_aliases_preserve_all_selected_owners() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("inner/deep")).unwrap();
        let source = b"pub struct Entry(pub u8);\npub fn entry() { println!(\"entry\"); }\n";
        fs::write(root.path().join("inner/entry.rs"), source).unwrap();
        fs::write(root.path().join("entry.rs"), "pub struct WrongRoot;\n").unwrap();
        std::os::unix::fs::symlink("inner/deep", root.path().join("link")).unwrap();
        let canonical = [PathBuf::from("inner/entry.rs")];
        let expected = artifact_from_facts_for_paths(
            &build_facts_for_paths(root.path(), &canonical).unwrap(),
            root.path(),
            &canonical,
        )
        .unwrap();
        let paths = [
            canonical[0].clone(),
            PathBuf::from("link/../entry.rs"),
            root.path().join("link/../entry.rs"),
            PathBuf::from("unused/../inner/entry.rs"),
        ];
        let graph = artifact_from_facts_for_paths(
            &build_facts_for_paths(root.path(), &paths).unwrap(),
            root.path(),
            &paths,
        )
        .unwrap();
        assert_eq!(
            graph, expected,
            "duplicate aliases changed source domain or owners"
        );
        let (overlay, shadowed) =
            OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths).unwrap();
        assert_eq!(*overlay, expected);
        assert_eq!(shadowed, HashSet::from(["inner/entry.rs".to_owned()]));
        assert!(graph.symbols.iter().any(|s| s.entity_name == "entry"));
        assert!(graph.symbols.iter().any(|s| s.entity_name == "Entry"));
        assert!(!graph.macro_invocations.as_ref().unwrap().rows.is_empty());
        export(
            "duplicate-physical-parents",
            &graph,
            &shadowed,
            &[("inner/entry.rs", source)],
        );
    }

    #[test]
    fn missing_nested_aliases_share_canonical_deleted_shadow() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("real");
        fs::create_dir_all(root.join("inner/deep")).unwrap();
        let alias = parent.path().join("root-alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        std::os::unix::fs::symlink("inner/deep", root.join("link")).unwrap();
        let paths = [
            PathBuf::from("link/../absent/deleted.rs"),
            alias.join("link/../absent/deleted.rs"),
            PathBuf::from("inner/absent/deleted.rs"),
        ];
        let facts = build_facts_for_paths(&root, &paths).unwrap();
        let selected = artifact_from_facts_for_paths(&facts, &root, &paths).unwrap();
        let (graph, shadowed) =
            OverlayClient::<InMemoryClient>::extract_delta(&root, &paths).unwrap();
        assert_eq!(*graph, selected);
        assert!(graph.file_manifests.is_empty());
        assert!(graph.rust_evidence.as_ref().unwrap().files.is_empty());
        assert_eq!(
            shadowed,
            HashSet::from(["inner/absent/deleted.rs".to_owned()])
        );
        export("missing-nested-alias", &graph, &shadowed, &[]);
    }

    #[test]
    fn missing_paths_beneath_escaping_symlink_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("absent"), root.path().join("dangling"))
            .unwrap();
        for spelling in [
            "escape/deleted.rs",
            "escape/absent/../deleted.rs",
            "dangling/deleted.rs",
        ] {
            for path in [PathBuf::from(spelling), root.path().join(spelling)] {
                let paths = [path];
                let facts = build_facts_for_paths(root.path(), &paths);
                assert!(
                    facts.is_err(),
                    "escaping missing source accepted: {paths:?}"
                );
                let overlay = OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths);
                assert!(
                    overlay.is_err(),
                    "escaping deletion shadow accepted: {paths:?}"
                );
            }
        }
    }

    #[test]
    fn missing_internal_symlink_target_retains_canonical_deleted_shadow() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("absent", root.path().join("dangling")).unwrap();
        let paths = [
            PathBuf::from("dangling/deleted.rs"),
            PathBuf::from("absent/deleted.rs"),
        ];
        let (graph, shadowed) =
            OverlayClient::<InMemoryClient>::extract_delta(root.path(), &paths).unwrap();
        assert!(graph.file_manifests.is_empty());
        assert_eq!(shadowed, HashSet::from(["absent/deleted.rs".to_owned()]));
        export("missing-internal-symlink-target", &graph, &shadowed, &[]);
    }

    fn export(
        name: &str,
        graph: &spur_graph::GraphIndexArtifact,
        shadowed: &HashSet<String>,
        inputs: &[(&str, &[u8])],
    ) {
        let sources: Vec<_> = inputs
            .iter()
            .map(|(path, bytes)| spur_graph::RustDeclarationSource {
                file_path: path,
                bytes,
            })
            .collect();
        graph.validate_rust_evidence_sources(&sources).unwrap();
        if let Ok(dir) = std::env::var("D3_EXPORT_DIR") {
            use sha2::{Digest, Sha256};
            fs::create_dir_all(&dir).unwrap();
            let mut shadowed: Vec<_> = shadowed.iter().collect();
            shadowed.sort();
            let packet = serde_json::json!({"artifact": graph, "shadowed": shadowed,
                "sources": inputs.iter().map(|(path, bytes)| serde_json::json!({"path": path,
                    "hex": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                    "sha256": format!("{:x}", Sha256::digest(bytes)),
                    "source_oid": spur_graph::git_blob_oid(bytes), "byte_length": bytes.len()})).collect::<Vec<_>>(),
                "canonical_hash": spur_graph::artifact_content_hash_blake3_hex(graph).unwrap(),
                "envelope_checksum": spur_graph::rust_evidence_checksum_blake3_hex(graph.rust_evidence.as_ref().unwrap()).unwrap(),
                "capability_digest": spur_graph::rust_evidence_capability_digest(graph).unwrap(), "source_error": null});
            fs::write(
                std::path::Path::new(&dir).join(format!("{name}.json")),
                serde_json::to_vec_pretty(&packet).unwrap(),
            )
            .unwrap();
        }
    }
}
