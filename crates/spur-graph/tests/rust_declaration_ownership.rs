use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::BytesExtractor;
use spur_graph::{artifact_from_facts, build_facts, stable_symbol_id_for, NodeKind, RelationKind};

#[test]
fn repeated_variant_fields_use_their_immediate_variant() {
    let source = "enum E { A { x:u8 }, B { x:u8 } } fn unrelated() {}";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), source).unwrap();
    let facts = build_facts(dir.path(), None).unwrap().0;
    let graph = artifact_from_facts(&facts, dir.path()).unwrap();
    let mut fields: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "field")
        .collect();
    fields.sort_by_key(|s| s.byte_range);
    assert_eq!(
        fields
            .iter()
            .map(|s| s.qualified_name.as_str())
            .collect::<Vec<_>>(),
        ["E::A::x", "E::B::x"]
    );
    for (field, parent_name) in fields.iter().zip(["E::A", "E::B"]) {
        let parent = graph
            .symbols
            .iter()
            .find(|s| s.qualified_name == parent_name)
            .unwrap();
        assert_eq!(
            field.enclosing_scope.as_deref(),
            parent_name.rsplit("::").next()
        );
        assert_eq!(
            field.stable_symbol_id,
            stable_symbol_id_for(
                "lib.rs",
                &field.qualified_name,
                NodeKind::Field,
                field.byte_range[0] as u64
            )
        );
        for relation in [RelationKind::Contains, RelationKind::Defines] {
            let owners: Vec<_> = graph
                .edges
                .iter()
                .filter(|e| {
                    e.relation == relation
                        && e.target_stable_symbol_id.as_deref() == Some(&field.stable_symbol_id)
                })
                .map(|e| e.source_stable_symbol_id.as_str())
                .collect();
            assert_eq!(owners, [parent.stable_symbol_id.as_str()]);
        }
    }
    let unrelated = graph
        .symbols
        .iter()
        .find(|s| s.qualified_name == "unrelated")
        .unwrap();
    assert_eq!(
        unrelated.stable_symbol_id,
        stable_symbol_id_for(
            "lib.rs",
            "unrelated",
            NodeKind::Function,
            source.find("fn unrelated").unwrap() as u64
        )
    );
}

#[test]
fn nested_modules_and_impls_keep_their_owners() {
    let dir = tempfile::tempdir().unwrap();
    let source =
        "mod m { enum E { A { x:u8 }, B { x:u8 } } struct S { x:u8 } impl S { fn f(&self) {} } }";
    std::fs::write(dir.path().join("lib.rs"), source).unwrap();
    let facts = build_facts(dir.path(), None).unwrap().0;
    let graph = artifact_from_facts(&facts, dir.path()).unwrap();
    for name in ["m::E::A::x", "m::E::B::x", "m::S::x", "m::impl S::f"] {
        assert!(
            graph.symbols.iter().any(|s| s.qualified_name == name),
            "missing {name}"
        );
    }
}

#[test]
fn frozen_source_graph_and_byte_owners_agree() {
    let source = include_str!("../../../docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/fixtures/enum-ownership.rs");
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), source).unwrap();
    let graph = artifact_from_facts(&build_facts(dir.path(), None).unwrap().0, dir.path()).unwrap();
    let bytes = BytesExtractor::for_language(Language::Rust)
        .unwrap()
        .extract(std::path::Path::new("src/lib.rs"), source.as_bytes())
        .unwrap();
    let byte_fields: Vec<_> = bytes
        .iter()
        .filter(|s| s.symbol_kind == "field")
        .map(|s| (s.byte_range, s.enclosing_scope.as_deref()))
        .collect();
    let graph_fields: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "field")
        .map(|s| {
            let edge = graph
                .edges
                .iter()
                .find(|e| {
                    e.relation == RelationKind::Contains
                        && e.target_stable_symbol_id.as_deref() == Some(&s.stable_symbol_id)
                })
                .unwrap();
            let owner = graph
                .symbols
                .iter()
                .find(|p| p.stable_symbol_id == edge.source_stable_symbol_id)
                .unwrap();
            (s.byte_range, Some(owner.qualified_name.as_str()))
        })
        .collect();
    if let Some(path) = std::env::var_os("SPUR_G1A_EXPORT") {
        let byte_rows: Vec<_> = bytes.iter().map(|s| serde_json::json!({"entity_name":s.entity_name,"symbol_kind":s.symbol_kind,"enclosing_scope":s.enclosing_scope,"byte_range":s.byte_range})).collect();
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"source":source,"symbols":graph.symbols,"edges":graph.edges,"byte_symbols":byte_rows})).unwrap()).unwrap();
    }
    assert_eq!(byte_fields, graph_fields);
    assert_eq!(
        byte_fields,
        [([13, 17], Some("E::A")), ([25, 29], Some("E::B"))]
    );
}
