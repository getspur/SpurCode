use std::path::{Path, PathBuf};

use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::BytesExtractor;
use spur_graph::{build_facts_for_paths, Confidence, GraphFacts, NodeId, RelationKind};

// The compiler-backed fixture in docs/research/artifacts/2026-11-15-rust-local-targets/
// name-collision-control.rs. Keep it inline so remote tests need no artifact sync.
const CONTROL: &str = r#"#![allow(dead_code)]
struct LocalFactory;
impl LocalFactory {
    fn new() -> Self { Self }
}
struct LocalDropper;
impl Drop for LocalDropper {
    fn drop(&mut self) {}
}
fn observed_macro() {
    let values: Vec<u8> = Vec::default();
    assert_eq!(values, Vec::new());
}
fn observed_drop() {
    let lock = std::sync::Mutex::new(1_i32);
    let guard = lock.lock().unwrap();
    drop(guard);
}
fn helper() -> usize { 7 }
fn observed_helper() -> usize { helper() }
fn main() {
    observed_macro();
    observed_drop();
    assert_eq!(observed_helper(), 7);
    println!("binding-control-ok");
}
"#;

fn extract(source: &str) -> GraphFacts {
    let path = Path::new("src/lib.rs");
    let bytes = BytesExtractor::for_language(Language::Rust)
        .unwrap()
        .extract_file_facts(path, source.as_bytes())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join(path), source).unwrap();
    let native = build_facts_for_paths(root.path(), &[PathBuf::from(path)]).unwrap();
    assert_eq!(bytes.nodes, native.nodes);
    assert_eq!(bytes.edges, native.edges);
    assert_eq!(bytes.spans, native.spans);
    assert_eq!(bytes.macro_invocations, native.macro_invocations);
    assert_eq!(bytes.macro_scopes, native.macro_scopes);
    bytes
}

fn definition(facts: &GraphFacts, source: &str, declaration: &str) -> NodeId {
    let start = source.find(declaration).unwrap() as u32;
    facts
        .nodes
        .iter()
        .find(|node| {
            facts
                .spans
                .iter()
                .any(|span| Some(span.span_id) == node.source_span_id && span.start_byte == start)
        })
        .unwrap_or_else(|| panic!("missing definition: {declaration}"))
        .node_id
}

#[expect(clippy::too_many_arguments)]
fn assert_call(
    facts: &GraphFacts,
    source: &str,
    caller: &str,
    name: &str,
    scope: Option<&str>,
    receiver: Option<&str>,
    target: Option<&str>,
) {
    let owner = definition(facts, source, &format!("fn {caller}"));
    let calls: Vec<_> = facts
        .edges
        .iter()
        .filter(|edge| {
            edge.relation == RelationKind::Calls
                && edge.source_node_id == owner
                && edge.target_label.as_deref() == Some(name)
        })
        .collect();
    let expected = target.map(|decl| definition(facts, source, decl));
    let edge = calls
        .iter()
        .find(|edge| edge.target_node_id == expected && edge.scope_text.as_deref() == scope)
        .unwrap_or_else(|| {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source, None).unwrap();
            panic!(
                "{caller}: missing {scope:?}::{name} -> {expected:?}; calls: {calls:?}; syntax: {}",
                tree.root_node().to_sexp()
            )
        });
    assert_eq!(edge.receiver_text.as_deref(), receiver, "{caller}::{name}");
    if expected.is_some() {
        assert_eq!(edge.confidence, Confidence::Heuristic);
        assert_eq!(edge.confidence_score, 0.8);
        assert_eq!(edge.bind_method.as_deref(), Some("macro_body_singleton"));
    }
}

#[test]
fn recorded_vec_constructor_never_binds_to_local_factory() {
    let facts = extract(CONTROL);
    let owner = definition(&facts, CONTROL, "fn observed_macro");
    let calls: Vec<_> = facts
        .edges
        .iter()
        .filter(|edge| {
            edge.relation == RelationKind::Calls
                && edge.source_node_id == owner
                && edge.target_label.as_deref() == Some("new")
        })
        .collect();
    assert_eq!(calls.len(), 1, "preserve the observed constructor call");
    assert_eq!(
        calls[0].target_node_id, None,
        "Vec::new must not bind LocalFactory::new"
    );
    assert_eq!(calls[0].scope_text.as_deref(), Some("Vec"));
    let helper = definition(&facts, CONTROL, "fn helper");
    let caller = definition(&facts, CONTROL, "fn observed_helper");
    assert!(facts
        .edges
        .iter()
        .any(|edge| edge.relation == RelationKind::Calls
            && edge.source_node_id == caller
            && edge.target_node_id == Some(helper)));
}

#[test]
fn local_constructor_and_bare_helper_keep_macro_heuristics() {
    let source = r#"
struct LocalFactory;
impl LocalFactory { fn new() -> Self { Self } }
fn helper() -> usize { 7 }
fn caller() { assert_eq!((LocalFactory::new(), helper()), ()); }
"#;
    let facts = extract(source);
    assert_call(
        &facts,
        source,
        "caller",
        "new",
        Some("LocalFactory"),
        None,
        Some("fn new"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        None,
        None,
        Some("fn helper"),
    );
}

#[test]
fn same_macro_keeps_distinct_qualifiers_and_foreign_leaf_unresolved() {
    let source = r#"
struct Alpha;
struct Beta;
impl Alpha { fn new() -> Self { Self } }
impl Beta { fn new() -> Self { Self } }
fn helper() {}
fn caller() { wrapper!(Alpha::new(), Beta::new(), foreign::helper(), helper()); }
"#;
    let facts = extract(source);
    assert_call(
        &facts,
        source,
        "caller",
        "new",
        Some("Alpha"),
        None,
        Some("fn new() -> Self { Self } }\nimpl Beta"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "new",
        Some("Beta"),
        None,
        Some("fn new() -> Self { Self } }\nfn helper"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        Some("foreign"),
        None,
        None,
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        None,
        None,
        Some("fn helper"),
    );
    let owner = definition(&facts, source, "fn caller");
    assert_eq!(
        facts
            .edges
            .iter()
            .filter(|edge| edge.relation == RelationKind::Calls && edge.source_node_id == owner)
            .count(),
        4
    );
}

#[test]
fn nested_token_trees_keep_each_call_scope() {
    let source = r#"
struct LocalFactory;
impl LocalFactory { fn new() -> Self { Self } }
fn helper() {}
fn caller() { wrapper!({ [Vec::new(), (LocalFactory::new(), helper())] }, foreign::helper()); }
"#;
    let facts = extract(source);
    assert_call(&facts, source, "caller", "new", Some("Vec"), None, None);
    assert_call(
        &facts,
        source,
        "caller",
        "new",
        Some("LocalFactory"),
        None,
        Some("fn new"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        None,
        None,
        Some("fn helper"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        Some("foreign"),
        None,
        None,
    );
}

#[test]
fn qualified_and_turbofish_paths_preserve_explicit_scope() {
    let source = r#"
struct LocalFactory;
impl LocalFactory { fn new() -> Self { Self } fn make<T>() -> Self { Self } }
fn helper() {}
fn caller() {
    wrapper!(::foreign::helper(), foreign::nested::helper(), Vec::<u8>::new(),
        LocalFactory::make::<u8>(), foreign::make::<u8>());
}
"#;
    let facts = extract(source);
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        Some("::foreign"),
        None,
        None,
    );
    assert_call(
        &facts,
        source,
        "caller",
        "helper",
        Some("foreign::nested"),
        None,
        None,
    );
    assert_call(
        &facts,
        source,
        "caller",
        "new",
        Some("Vec::<u8>"),
        None,
        None,
    );
    assert_call(
        &facts,
        source,
        "caller",
        "make",
        Some("LocalFactory"),
        None,
        Some("fn make"),
    );
    assert_call(
        &facts,
        source,
        "caller",
        "make",
        Some("foreign"),
        None,
        None,
    );
}
