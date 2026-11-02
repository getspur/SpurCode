#[test]
fn equal_types_and_nested_types_have_one_node_per_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct S(u8,u8,(u8,u8),fn(u8)->u8); enum E { A(u8), B(u8) }",
    )
    .unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let mut fields: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "field")
        .map(|s| s.qualified_name.as_str())
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        ["E::A::0", "E::B::0", "S::0", "S::1", "S::2", "S::3"]
    );
}

#[test]
fn physical_entries_include_owned_metadata_and_match_byte_extraction() {
    use spur_graph::extract::languages::Language;
    use spur_graph::extract::tree_sitter::BytesExtractor;
    use spur_graph::{stable_symbol_id_for, NodeKind, RelationKind};

    let source = "/* é */ struct S(#[cfg(any())] pub u8, /* next */ #[allow(dead_code)] pub(crate) Vec<(u8,u8)>, fn(u8)->u8,); enum E { A(u8), B(u8) }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), source).unwrap();
    let graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(dir.path(), None).unwrap().0,
        dir.path(),
    )
    .unwrap();
    let bytes = BytesExtractor::for_language(Language::Rust)
        .unwrap()
        .extract(std::path::Path::new("lib.rs"), source.as_bytes())
        .unwrap();
    if let Some(path) = std::env::var_os("SPUR_G1B_EXPORT_SMALL") {
        let byte_rows: Vec<_> = bytes
            .iter()
            .map(|s| {
                serde_json::json!({
                    "entity_name": s.entity_name,
                    "symbol_kind": s.symbol_kind,
                    "enclosing_scope": s.enclosing_scope,
                    "byte_range": s.byte_range,
                })
            })
            .collect();
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "source": source, "symbols": graph.symbols,
                "edges": graph.edges, "byte_symbols": byte_rows,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let expected = [
        ("S::0", "#[cfg(any())] pub u8"),
        ("S::1", "#[allow(dead_code)] pub(crate) Vec<(u8,u8)>"),
        ("S::2", "fn(u8)->u8"),
        ("E::A::0", "u8"),
        ("E::B::0", "u8"),
    ];
    let fields: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "field")
        .collect();
    assert_eq!(fields.len(), expected.len());
    for (name, text) in expected {
        let field = fields.iter().find(|f| f.qualified_name == name).unwrap();
        let [start, end] = field.byte_range;
        assert_eq!(&source[start as usize..end as usize], text);
        assert_eq!(
            field.stable_symbol_id,
            stable_symbol_id_for("lib.rs", name, NodeKind::Field, start as u64)
        );
        let owner_name = name.rsplit_once("::").unwrap().0;
        let owner = graph
            .symbols
            .iter()
            .find(|s| s.qualified_name == owner_name)
            .unwrap();
        for relation in [RelationKind::Contains, RelationKind::Defines] {
            assert!(graph.edges.iter().any(|e| e.relation == relation
                && e.source_stable_symbol_id == owner.stable_symbol_id
                && e.target_stable_symbol_id.as_deref() == Some(&field.stable_symbol_id)));
        }
        assert!(bytes.iter().any(|s| s.symbol_kind == "field"
            && s.byte_range == field.byte_range
            && s.enclosing_scope.as_deref() == Some(owner_name)));
    }
}

// Explicitly invoked with an isolated fixture tree assembled from frozen bytes.
#[test]
#[ignore]
fn export_frozen_graph() {
    let root = std::path::PathBuf::from(std::env::var_os("SPUR_G1B_ROOT").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("SPUR_G1B_EXPORT").unwrap());
    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let fixture = entry.unwrap().path();
        let graph = spur_graph::artifact_from_facts(
            &spur_graph::build_facts(&fixture, None).unwrap().0,
            &fixture,
        )
        .unwrap();
        symbols.extend(graph.symbols);
        edges.extend(graph.edges);
    }
    std::fs::write(
        output,
        serde_json::to_vec(&serde_json::json!({
            "symbols": symbols,
            "edges": edges,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn ordinal_is_physical_and_ids_include_current_offsets() {
    fn fields(source: &str) -> Vec<(String, String, usize)> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), source).unwrap();
        let graph = spur_graph::artifact_from_facts(
            &spur_graph::build_facts(dir.path(), None).unwrap().0,
            dir.path(),
        )
        .unwrap();
        let mut fields: Vec<_> = graph
            .symbols
            .into_iter()
            .filter(|s| s.symbol_kind == "field")
            .map(|s| (s.qualified_name, s.stable_symbol_id, s.byte_range[0]))
            .collect();
        fields.sort();
        fields
    }
    let original = fields("struct S(#[cfg(any())] u8,u16);");
    let removed = fields("struct S(u16);");
    assert_eq!(
        original.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
        ["S::0", "S::1"]
    );
    assert_eq!(removed[0].0, "S::0");
    assert_ne!(original[1].1, removed[0].1);

    let before = fields("struct S(u8,u16);");
    let inserted = fields("struct S(u32,u8,u16);");
    let reordered = fields("struct S(u16,u8);");
    assert_eq!(before[0].1, inserted[0].1); // same ordinal and byte start
    assert_ne!(before[1].1, inserted[1].1); // same ordinal, different byte start
    assert_eq!(before[0].1, reordered[0].1); // identity does not hash type
    assert_ne!(before[1].1, reordered[1].1); // reordered type widths shift the start
}

const DOC_SOURCE: &str = "struct S(/// line doc\n #[allow(dead_code)] u8, /** block doc */ #[allow(dead_code)] pub u16, #[allow(dead_code)] /** after attr */ u32, //// ordinary\n u64, /*** ordinary */ u128); enum E { A(/// variant doc\n u8), B(u8) }";

fn doc_expected() -> [(&'static str, &'static str); 7] {
    [
        ("S::0", "/// line doc\n #[allow(dead_code)] u8"),
        ("S::1", "/** block doc */ #[allow(dead_code)] pub u16"),
        ("S::2", "#[allow(dead_code)] /** after attr */ u32"),
        ("S::3", "u64"),
        ("S::4", "u128"),
        ("E::A::0", "/// variant doc\n u8"),
        ("E::B::0", "u8"),
    ]
}

#[test]
fn outer_doc_comments_are_owned_graph_field_spans() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), DOC_SOURCE).unwrap();
    let graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(dir.path(), None).unwrap().0,
        dir.path(),
    )
    .unwrap();
    if let Some(path) = std::env::var_os("SPUR_G1B_EXPORT_DOCS") {
        let bytes = spur_graph::extract::tree_sitter::BytesExtractor::for_language(
            spur_graph::extract::languages::Language::Rust,
        )
        .unwrap()
        .extract(std::path::Path::new("lib.rs"), DOC_SOURCE.as_bytes())
        .unwrap();
        let byte_rows: Vec<_> = bytes
            .iter()
            .map(|s| {
                serde_json::json!({
                    "entity_name": s.entity_name, "symbol_kind": s.symbol_kind,
                    "enclosing_scope": s.enclosing_scope, "byte_range": s.byte_range,
                })
            })
            .collect();
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "source": DOC_SOURCE, "symbols": graph.symbols, "edges": graph.edges,
                "byte_symbols": byte_rows,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let mut actual = Vec::new();
    for (name, text) in doc_expected() {
        let field = graph
            .symbols
            .iter()
            .find(|s| s.qualified_name == name)
            .unwrap();
        actual.push((
            name,
            &DOC_SOURCE[field.byte_range[0]..field.byte_range[1]],
            text,
        ));
    }
    assert_eq!(
        actual
            .iter()
            .map(|(name, got, _)| (*name, *got))
            .collect::<Vec<_>>(),
        doc_expected()
    );
}

#[test]
fn outer_doc_comments_are_owned_byte_field_spans() {
    use spur_graph::extract::languages::Language;
    use spur_graph::extract::tree_sitter::BytesExtractor;
    let fields = BytesExtractor::for_language(Language::Rust)
        .unwrap()
        .extract(std::path::Path::new("lib.rs"), DOC_SOURCE.as_bytes())
        .unwrap();
    let mut actual = Vec::new();
    for (name, text) in doc_expected() {
        let (owner, ordinal) = name.rsplit_once("::").unwrap();
        let field = fields
            .iter()
            .find(|s| {
                s.symbol_kind == "field"
                    && s.enclosing_scope.as_deref() == Some(owner)
                    && s.entity_name == ordinal
            })
            .unwrap();
        actual.push((
            name,
            &DOC_SOURCE[field.byte_range[0]..field.byte_range[1]],
            text,
        ));
    }
    assert_eq!(
        actual
            .iter()
            .map(|(name, got, _)| (*name, *got))
            .collect::<Vec<_>>(),
        doc_expected()
    );
}
