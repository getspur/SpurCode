use std::fs;

use spur_graph::{
    artifact_from_facts, build_facts, collect_rust_evidence, GraphIndexArtifact,
    RustDeclarationSource,
};

fn graph(inputs: &[(String, Vec<u8>)]) -> GraphIndexArtifact {
    let root = tempfile::tempdir().unwrap();
    for (path, bytes) in inputs {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap()
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

#[test]
fn stored_graph_symbol_ids_remain_globally_unambiguous() {
    let inputs = vec![("a.rs".into(), b"fn caller() { leaf(); }\n".to_vec())];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let caller = graph
        .symbols
        .iter()
        .find(|symbol| symbol.entity_name == "caller")
        .unwrap();
    for cross_file in [false, true] {
        for first in [false, true] {
            let mut changed = graph.clone();
            let mut duplicate = caller.clone();
            if cross_file {
                duplicate.file_path = "unrelated.json".into();
            }
            if first {
                changed.symbols.insert(0, duplicate);
            } else {
                changed.symbols.push(duplicate);
            }
            let error = set.validate_stored(&changed).unwrap_err();
            assert!(
                format!("{error:#}").contains("ambiguous graph symbol identity"),
                "cross_file={cross_file}, first={first}: {error:#}"
            );
            assert!(set.validate_sources(&changed, &sources(&inputs)).is_err());
        }
    }
}

#[test]
fn stored_graph_symbol_lookups_preserve_file_kind_and_range_checks() {
    let inputs = vec![
        ("a.rs".into(), b"fn caller() { leaf(); }\n".to_vec()),
        ("b.rs".into(), b"fn keep() {}\n".to_vec()),
    ];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let caller = graph
        .symbols
        .iter()
        .position(|symbol| symbol.entity_name == "caller")
        .unwrap();
    for mismatch in [
        "missing",
        "cross_file",
        "kind",
        "empty_id",
        "bytes",
        "lines",
    ] {
        let mut changed = graph.clone();
        let symbol = &mut changed.symbols[caller];
        match mismatch {
            "missing" => {
                changed.symbols.remove(caller);
            }
            "cross_file" => symbol.file_path = "unrelated.json".into(),
            "kind" => symbol.symbol_kind = "module".into(),
            "empty_id" => symbol.stable_symbol_id.clear(),
            "bytes" => symbol.byte_range = [0, inputs[0].1.len() + 1],
            "lines" => symbol.line_range = [0, 0],
            _ => unreachable!(),
        }
        assert!(set.validate_stored(&changed).is_err(), "{mismatch}");
        assert!(
            set.validate_sources(&changed, &sources(&inputs)).is_err(),
            "{mismatch}"
        );
    }
}

#[test]
fn stored_graph_symbol_validation_preserves_unrelated_duplicate_ids() {
    let inputs = vec![("a.rs".into(), b"fn caller() { leaf(); }\n".to_vec())];
    let mut graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let mut unrelated = graph.symbols[0].clone();
    unrelated.stable_symbol_id = "unrelated-non-rust-symbol".into();
    unrelated.file_path = "generated.json".into();
    unrelated.symbol_kind = "field".into();
    graph.symbols.push(unrelated.clone());
    graph.symbols.push(unrelated);
    // Only IDs belonging to checked physical Rust symbols require a unique
    // global match. Unrelated JSON duplicates are outside that validation domain.
    set.validate_stored(&graph).unwrap();
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
}

#[test]
fn source_validation_retains_each_files_original_inventory() {
    let inputs = vec![
        ("a.rs".into(), b"struct Pair(pub u8, u16);\n".to_vec()),
        (
            "b.rs".into(),
            b"unsafe extern \"C\" { pub fn foreign(x: u8); }\n".to_vec(),
        ),
        ("c.rs".into(), b"fn caller() { leaf(); }\n".to_vec()),
    ];
    let graph = graph(&inputs);
    let original = |paths: &[&str]| {
        let mut view = graph.clone();
        view.set_macro_evidence(None).unwrap();
        view.set_rust_evidence(None).unwrap();
        view.files.retain(|f| paths.contains(&f.file_path.as_str()));
        view.file_manifests
            .retain(|m| paths.contains(&m.path.as_str()));
        view.graph_content_hash = spur_graph::compute_graph_content_hash(
            view.file_manifests
                .iter()
                .map(|m| (&m.path, &m.content_oid)),
        );
        let selected: Vec<_> = sources(&inputs)
            .into_iter()
            .filter(|s| paths.contains(&s.file_path))
            .collect();
        collect_rust_evidence(&view, &selected).unwrap()
    };
    let mut set = original(&["a.rs", "c.rs"]);
    let other = original(&["b.rs"]);
    set.graph_content_hash = graph.graph_content_hash.clone();
    set.origins.extend(other.origins);
    set.origins.sort_by(|a, b| {
        (&a.manifest_version, &a.graph_content_hash)
            .cmp(&(&b.manifest_version, &b.graph_content_hash))
    });
    set.files.extend(other.files);
    set.files.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    assert_ne!(
        set.files[0].origin_graph_content_hash,
        set.files[1].origin_graph_content_hash
    );
    assert_eq!(
        set.files[0].origin_graph_content_hash,
        set.files[2].origin_graph_content_hash
    );
    // File order revisits the first original inventory after validating another.
    // Tuple fields, foreign declarations and calls keep their original headers.
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
    let mut forged = set;
    forged.files[1].declarations.graph_content_hash = graph.graph_content_hash.clone();
    assert!(forged.validate_sources(&graph, &sources(&inputs)).is_err());
}
