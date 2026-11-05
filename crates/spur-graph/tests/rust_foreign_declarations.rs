use std::path::Path;

use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::{BytesExtractor, ExtractedSymbol};
use spur_graph::{
    artifact_from_facts, artifact_from_facts_incremental, build_facts, read_artifact_parquet,
    stable_symbol_id_for, write_artifact_parquet, BuildMode, GraphIndexArtifact,
    GraphSymbolArtifact, NodeKind, RelationKind, WriteOptions,
};

fn fixture(source: &str) -> (tempfile::TempDir, GraphIndexArtifact, Vec<ExtractedSymbol>) {
    fixture_at("lib.rs", source)
}

fn fixture_at(
    path: &str,
    source: &str,
) -> (tempfile::TempDir, GraphIndexArtifact, Vec<ExtractedSymbol>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
    let facts = build_facts(dir.path(), None).unwrap().0;
    let graph = artifact_from_facts(&facts, dir.path()).unwrap();
    let bytes = BytesExtractor::for_language(Language::Rust)
        .unwrap()
        .extract(Path::new(path), source.as_bytes())
        .unwrap();
    (dir, graph, bytes)
}

#[test]
fn impl_method_nested_foreign_projection_preserves_preexisting_owner() {
    // Exact supplied source, independent syn method visitor, and accepted G1
    // identity: ROOT-B-owner-review/correction-diff.json. No cfg/linkage claim.
    let text = include_str!("../../../docs/research/artifacts/2026-11-05-rust-foreign-resume/B/attempt-2/impl-method-fixture.rs");
    let path = "rest-table-gateway-ext/src/native_registration.rs";
    let (_, graph, bytes) = fixture_at(path, text);
    let method = symbol(&graph, "impl Api::get");
    assert_eq!(method.stable_symbol_id, "fa62581bd5f960a1");
    assert_eq!(method.symbol_kind, "method");
    assert_eq!(method.byte_range, [2798, 4706]);
    assert_eq!(method.line_range, [78, 121]);
    assert_eq!(method.enclosing_scope.as_deref(), Some("impl Api"));
    assert_eq!(method.anchor_hash, "16083782153702749865");
    assert_eq!(
        method.stable_symbol_id,
        stable_symbol_id_for(path, "impl Api::get", NodeKind::Method, 2798)
    );
    let block = symbol(&graph, "impl Api::get::<extern>");
    assert_eq!(parent(&graph, block), method.stable_symbol_id);
    let expected = [
        (
            "<extern>",
            NodeKind::ExternBlock,
            [2865, 3363],
            "4c131c387c0e1c0e",
        ),
        (
            "spur_native_database_weak",
            NodeKind::Function,
            [2894, 3019],
            "3e2579eb580b9af9",
        ),
        (
            "spur_native_database_lock",
            NodeKind::Function,
            [3036, 3140],
            "c4c089cf23a7f19a",
        ),
        (
            "spur_native_database_weak_destroy",
            NodeKind::Function,
            [3157, 3215],
            "11f2df3a6145ee51",
        ),
        (
            "spur_native_api",
            NodeKind::Function,
            [3232, 3287],
            "5c9a1b06faaa6417",
        ),
        (
            "spur_native_api_destroy",
            NodeKind::Function,
            [3304, 3349],
            "b643c0d3a81626d4",
        ),
    ];
    for (name, kind, span, id) in expected {
        let fqn = format!("impl Api::get::{name}");
        let row = symbol(&graph, &fqn);
        assert_eq!(row.file_path, path);
        assert_eq!(row.byte_range, span);
        assert_eq!(row.entity_name, name);
        assert_eq!(row.symbol_kind, kind.discriminator());
        assert_eq!(row.enclosing_scope.as_deref(), Some("impl Api::get"));
        assert_eq!(row.stable_symbol_id, id);
        assert_eq!(
            row.stable_symbol_id,
            stable_symbol_id_for(path, &fqn, kind, span[0] as u64)
        );
        assert_eq!(
            row.line_range,
            [
                text[..span[0]].bytes().filter(|b| *b == b'\n').count() + 1,
                text[..span[1]].bytes().filter(|b| *b == b'\n').count() + 1
            ]
        );
        let matching: Vec<_> = bytes
            .iter()
            .filter(|b| {
                b.symbol_kind == row.symbol_kind && b.byte_range == span && b.entity_name == name
            })
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].enclosing_scope, row.enclosing_scope);
        assert_eq!(matching[0].line_range, row.line_range);
        assert_eq!(matching[0].anchor_hash, row.anchor_hash);
        if kind == NodeKind::Function {
            assert_eq!(parent(&graph, row), block.stable_symbol_id);
            assert!(text[span[0]..span[1]].ends_with(';'));
            let defines: Vec<_> = graph
                .edges
                .iter()
                .filter(|e| {
                    e.relation == RelationKind::Defines
                        && e.target_stable_symbol_id.as_deref() == Some(id)
                })
                .collect();
            assert_eq!(defines.len(), 1);
            assert_eq!(defines[0].source_stable_symbol_id, block.stable_symbol_id);
        }
    }
    if let Some(output) = std::env::var_os("SPUR_G2B_METHOD_EXPORT") {
        let byte_rows: Vec<_> = bytes.iter().map(|b| serde_json::json!({"entity_name":b.entity_name,"symbol_kind":b.symbol_kind,"enclosing_scope":b.enclosing_scope,"byte_range":b.byte_range,"line_range":b.line_range})).collect();
        std::fs::write(
            output,
            serde_json::to_vec(&serde_json::json!({"graph":graph,"byte_symbols":byte_rows}))
                .unwrap(),
        )
        .unwrap();
    }
}

fn symbol<'a>(graph: &'a GraphIndexArtifact, name: &str) -> &'a GraphSymbolArtifact {
    graph
        .symbols
        .iter()
        .find(|s| s.qualified_name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing physical symbol {name}; actual: {:?}",
                graph
                    .symbols
                    .iter()
                    .map(|s| &s.qualified_name)
                    .collect::<Vec<_>>()
            )
        })
}

fn parent<'a>(graph: &'a GraphIndexArtifact, child: &GraphSymbolArtifact) -> &'a str {
    let parents: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Contains
                && e.target_stable_symbol_id.as_deref() == Some(&child.stable_symbol_id)
        })
        .collect();
    assert_eq!(
        parents.len(),
        1,
        "one direct structural parent for {}",
        child.qualified_name
    );
    &parents[0].source_stable_symbol_id
}

fn assert_projection(
    graph: &GraphIndexArtifact,
    bytes: &[ExtractedSymbol],
    source: &str,
    name: &str,
    text: &str,
    scope: Option<&str>,
    kind: NodeKind,
) {
    let row = symbol(graph, name);
    let start = source.find(text).unwrap();
    assert_eq!(
        row.byte_range,
        [start, start + text.len()],
        "full range of {name}"
    );
    assert_eq!(row.symbol_kind, kind.discriminator());
    assert_eq!(
        row.enclosing_scope.as_deref(),
        scope,
        "lexical scope of {name}"
    );
    assert_eq!(
        row.stable_symbol_id,
        stable_symbol_id_for("lib.rs", name, kind, start as u64)
    );
    assert_eq!(
        row.line_range,
        [
            source[..start].bytes().filter(|b| *b == b'\n').count() + 1,
            source[..start + text.len()]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1
        ]
    );
    let matching: Vec<_> = bytes
        .iter()
        .filter(|b| {
            b.byte_range == row.byte_range
                && b.symbol_kind == row.symbol_kind
                && b.entity_name == row.entity_name
        })
        .collect();
    assert_eq!(matching.len(), 1, "one byte projection of {name}");
    assert_eq!(matching[0].enclosing_scope.as_deref(), scope);
    assert_eq!(matching[0].line_range, row.line_range);
    assert_eq!(matching[0].anchor_hash, row.anchor_hash);
}

#[test]
fn foreign_block_is_structural_but_not_a_lexical_namespace() {
    let text = r#"mod m { unsafe extern "C" { pub fn imported(x:u8)->u8; } pub extern "C" fn defined(x:u8)->u8 { x } }"#;
    let (_, graph, bytes) = fixture(text);
    assert_projection(
        &graph,
        &bytes,
        text,
        "m::imported",
        "pub fn imported(x:u8)->u8;",
        Some("m"),
        NodeKind::Function,
    );
    assert_projection(
        &graph,
        &bytes,
        text,
        "m::<extern>",
        r#"unsafe extern "C" { pub fn imported(x:u8)->u8; }"#,
        Some("m"),
        NodeKind::ExternBlock,
    );
    let imported = symbol(&graph, "m::imported");
    let block = symbol(&graph, "m::<extern>");
    assert_eq!(parent(&graph, imported), block.stable_symbol_id);
    assert_eq!(parent(&graph, block), symbol(&graph, "m").stable_symbol_id);
    let defining: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Defines
                && e.target_stable_symbol_id.as_deref() == Some(&imported.stable_symbol_id)
        })
        .collect();
    assert_eq!(defining.len(), 1);
    assert_eq!(defining[0].source_stable_symbol_id, block.stable_symbol_id);
    assert_eq!(
        graph
            .symbols
            .iter()
            .filter(|s| s.symbol_kind == "extern_block")
            .count(),
        1
    );
    assert_projection(
        &graph,
        &bytes,
        text,
        "m::defined",
        r#"pub extern "C" fn defined(x:u8)->u8 { x }"#,
        Some("m"),
        NodeKind::Function,
    );
    assert_eq!(
        parent(&graph, symbol(&graph, "m::defined")),
        symbol(&graph, "m").stable_symbol_id
    );
}

#[test]
fn full_ranges_own_outer_metadata_without_stealing_neighboring_items() {
    let text = r#"// é ordinary
#[allow(dead_code)] fn previous() {}
/// block documentation
/* ordinary between metadata */
#[link(name = "fixture")]
unsafe extern "C" {
    /// function documentation
    // ordinary between docs and attribute
    #[link_name = "native"]
    pub fn r#type(x: u8) -> u8;
    /** second documentation */
    /* ordinary */
    #[cfg(any())]
    pub(crate) fn café();
    // ordinary only
    fn plain();
}
#[allow(dead_code)] fn next() {}
// ordinary only before block
extern { fn omitted(); }
"#;
    let (_, graph, bytes) = fixture(text);
    let block_text = &text[text.find("/// block documentation").unwrap()
        ..text.find("\n#[allow(dead_code)] fn next").unwrap()];
    assert_projection(
        &graph,
        &bytes,
        text,
        "<extern>",
        block_text,
        None,
        NodeKind::ExternBlock,
    );
    for (name, full) in [
        ("r#type", "/// function documentation\n    // ordinary between docs and attribute\n    #[link_name = \"native\"]\n    pub fn r#type(x: u8) -> u8;"),
        ("café", "/** second documentation */\n    /* ordinary */\n    #[cfg(any())]\n    pub(crate) fn café();"),
        ("plain", "fn plain();"),
        ("omitted", "fn omitted();"),
    ] {
        assert_projection(&graph, &bytes, text, name, full, None, NodeKind::Function);
        let owner = parent(&graph, symbol(&graph, name));
        assert!(graph.symbols.iter().any(|s| s.stable_symbol_id == owner && s.symbol_kind == "extern_block"));
    }
    let last = graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "extern_block")
        .max_by_key(|s| s.byte_range[0])
        .unwrap();
    assert_eq!(
        &text[last.byte_range[0]..last.byte_range[1]],
        "extern { fn omitted(); }"
    );
    assert_eq!(parent(&graph, last), graph.files[0].stable_file_id);
    assert!(!graph
        .symbols
        .iter()
        .any(|s| s.qualified_name.contains("<extern>::")));
}

#[test]
fn repeated_blocks_cfg_duplicates_and_nested_owners_remain_physical() {
    let text = r#"unsafe extern "C" { fn top(); }
mod m {
    unsafe extern "C" { pub fn first(x: u8) -> u8; }
    unsafe extern "C" { pub fn second(x: u8) -> u8; }
    #[cfg(a)] unsafe extern "C" { fn same(); }
    #[cfg(b)] unsafe extern "C" { fn same(); }
    mod n { fn host() { unsafe extern "C" { fn nested(); } } }
}
"#;
    let (_, graph, bytes) = fixture(text);
    assert_eq!(
        graph
            .symbols
            .iter()
            .filter(|s| s.symbol_kind == "extern_block")
            .count(),
        6
    );
    assert_projection(
        &graph,
        &bytes,
        text,
        "m::n::host::nested",
        "fn nested();",
        Some("m::n::host"),
        NodeKind::Function,
    );
    for function in graph
        .symbols
        .iter()
        .filter(|s| s.symbol_kind == "function" && s.entity_name != "host")
    {
        let block = graph
            .symbols
            .iter()
            .find(|s| s.stable_symbol_id == parent(&graph, function))
            .unwrap();
        assert_eq!(block.symbol_kind, "extern_block");
        assert!(
            block.byte_range[0] <= function.byte_range[0]
                && function.byte_range[1] <= block.byte_range[1]
        );
        assert_eq!(
            function.stable_symbol_id,
            stable_symbol_id_for(
                "lib.rs",
                &function.qualified_name,
                NodeKind::Function,
                function.byte_range[0] as u64
            )
        );
    }
    let same: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "m::same")
        .collect();
    assert_eq!(same.len(), 2);
    assert_ne!(same[0].stable_symbol_id, same[1].stable_symbol_id);
    assert_ne!(parent(&graph, same[0]), parent(&graph, same[1]));
    assert_ne!(
        parent(&graph, symbol(&graph, "m::first")),
        parent(&graph, symbol(&graph, "m::second"))
    );
    let json = serde_json::to_vec(&graph).unwrap();
    let restored: GraphIndexArtifact = serde_json::from_slice(&json).unwrap();
    let mut actual_symbols = restored.symbols.clone();
    let mut expected_symbols = graph.symbols.clone();
    actual_symbols.sort_by(|a, b| a.stable_symbol_id.cmp(&b.stable_symbol_id));
    expected_symbols.sort_by(|a, b| a.stable_symbol_id.cmp(&b.stable_symbol_id));
    assert_eq!(actual_symbols, expected_symbols);
    let edge_set = |edges: &[spur_graph::GraphEdgeArtifact]| {
        let mut rows: Vec<_> = edges
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(edge_set(&restored.edges), edge_set(&graph.edges));
    let out = tempfile::tempdir().unwrap();
    let written =
        write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let restored = read_artifact_parquet(&written).unwrap();
    let mut actual_symbols = restored.symbols.clone();
    actual_symbols.sort_by(|a, b| a.stable_symbol_id.cmp(&b.stable_symbol_id));
    assert_eq!(actual_symbols, expected_symbols);
    assert_eq!(edge_set(&restored.edges), edge_set(&graph.edges));
    if let Some(path) = std::env::var_os("SPUR_G2B_EXPORT_SMALL") {
        let byte_rows: Vec<_> = bytes.iter().map(|s| serde_json::json!({"entity_name":s.entity_name,"symbol_kind":s.symbol_kind,"enclosing_scope":s.enclosing_scope,"byte_range":s.byte_range,"line_range":s.line_range})).collect();
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"source":text,"graph":graph,"restored":restored,"byte_symbols":byte_rows})).unwrap()).unwrap();
    }
}

#[test]
fn ordinary_extern_definitions_traits_and_extern_crate_keep_their_roles() {
    let text = r#"extern crate core;
trait T { fn required(); extern "C" fn provided() {} }
pub extern "C" fn defined(x: u8) -> u8 { x }
unsafe extern "C" { fn imported(); }
"#;
    let (_, graph, _) = fixture(text);
    assert_eq!(
        graph
            .symbols
            .iter()
            .filter(|s| s.symbol_kind == "extern_block")
            .count(),
        1
    );
    assert_eq!(symbol(&graph, "T::required").symbol_kind, "method");
    assert_eq!(symbol(&graph, "T::provided").symbol_kind, "method");
    assert!(
        text[symbol(&graph, "defined").byte_range[0]..symbol(&graph, "defined").byte_range[1]]
            .ends_with("{ x }")
    );
    assert!(text
        [symbol(&graph, "imported").byte_range[0]..symbol(&graph, "imported").byte_range[1]]
        .ends_with(';'));
    assert_eq!(
        parent(&graph, symbol(&graph, "defined")),
        graph.files[0].stable_file_id
    );
}

#[test]
fn transparent_lookup_resolves_qualified_names_but_preserves_unknown_dispatch() {
    let text = r#"mod m { unsafe extern "C" { fn imported(); } }
mod unrelated { fn missing() {} }
fn caller() { m::imported(); absent::missing(); }
"#;
    let (_, graph, _) = fixture(text);
    let caller = symbol(&graph, "caller");
    let imported = symbol(&graph, "m::imported");
    let calls: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Calls
                && e.source_stable_symbol_id == caller.stable_symbol_id
        })
        .collect();
    assert!(calls
        .iter()
        .any(|e| e.target_stable_symbol_id.as_deref() == Some(&imported.stable_symbol_id)));
    assert!(calls.iter().any(
        |e| e.target_label.as_deref() == Some("missing") && e.target_stable_symbol_id.is_none()
    ));
    assert!(!calls.iter().any(|e| e.target_stable_symbol_id.as_deref()
        == Some(&symbol(&graph, "unrelated::missing").stable_symbol_id)));
}

#[test]
fn stale_tuple_extractor_cannot_reuse_unchanged_file_omissions() {
    let (root, mut old, _) = fixture("unsafe extern \"C\" { fn imported(); }");
    assert_eq!(
        spur_graph::store::EXTRACTOR_VERSION,
        "2026-11-03-rust-foreign-declarations-v1"
    );
    assert_eq!(spur_graph::store::SCHEMA_VERSION, "spur-graph-schema-v12");
    old.symbols.clear();
    old.edges.clear();
    old.macro_invocations.as_mut().unwrap().extractor_version =
        "2026-11-02-rust-tuple-fields-v1".into();
    let (rebuilt, mode, _) = artifact_from_facts_incremental(&old, root.path()).unwrap();
    assert_eq!(mode, BuildMode::Full);
    assert_eq!(symbol(&rebuilt, "imported").symbol_kind, "function");
    assert_eq!(
        rebuilt
            .symbols
            .iter()
            .filter(|s| s.symbol_kind == "extern_block")
            .count(),
        1
    );
}

// Opt-in export against private copies of the supplied frozen UTF-8 sources.
#[test]
#[ignore = "requires SPUR_G2B_FROZEN_ROOT and SPUR_G2B_FROZEN_EXPORT; invoked explicitly for B evidence"]
fn export_frozen_foreign_graph() {
    let root = std::path::PathBuf::from(std::env::var_os("SPUR_G2B_FROZEN_ROOT").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("SPUR_G2B_FROZEN_EXPORT").unwrap());
    let graph = artifact_from_facts(&build_facts(&root, None).unwrap().0, &root).unwrap();
    let mut byte_rows = Vec::new();
    for file in &graph.files {
        if !file.file_path.ends_with(".rs") {
            continue;
        }
        let source = std::fs::read(root.join(&file.file_path)).unwrap();
        let rows = BytesExtractor::for_language(Language::Rust)
            .unwrap()
            .extract(Path::new(&file.file_path), &source)
            .unwrap();
        let oid = &graph
            .file_manifests
            .iter()
            .find(|m| m.path == file.file_path)
            .unwrap()
            .content_oid;
        byte_rows.extend(rows.into_iter().map(|s| serde_json::json!({"file_path":file.file_path,"source_oid":oid,"entity_name":s.entity_name,"symbol_kind":s.symbol_kind,"enclosing_scope":s.enclosing_scope,"byte_range":s.byte_range,"line_range":s.line_range})));
    }
    std::fs::write(
        output,
        serde_json::to_vec(&serde_json::json!({"graph":graph,"byte_symbols":byte_rows})).unwrap(),
    )
    .unwrap();
}
