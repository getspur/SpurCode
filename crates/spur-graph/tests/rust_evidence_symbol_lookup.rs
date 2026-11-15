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
            "cross_file" => symbol.file_path = "b.rs".into(),
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
