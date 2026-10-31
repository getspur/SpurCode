use spur_graph::{
    artifact_from_facts, build_facts, GraphEdgeKind, GraphIndexArtifact, RelationKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator as _};

const PROPOSED_QUERY: &str = r"
(token_tree
  (identifier) @call.name
  .
  (token_tree)) @call

(token_tree
  (scoped_identifier
    name: (identifier) @call.name)
  .
  (token_tree)) @call
";

const PARENTHESIZED_ARG_QUERY: &str = r#"
(token_tree
  (identifier) @call.name
  .
  (token_tree "(" ")")) @call

(token_tree
  (scoped_identifier
    name: (identifier) @call.name)
  .
  (token_tree "(" ")")) @call
"#;

const SPUR_EDGES_QUERY: &str = include_str!("../queries/rust/spur-edges.scm");

fn capture_texts(query_source: &str, source: &str, capture_name: &str) -> Vec<String> {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language).expect("configure parser");
    let tree = parser.parse(source, None).expect("parse source");
    let query = Query::new(&language, query_source).expect("compile query");
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&query, tree.root_node(), source.as_bytes());
    let mut names = Vec::new();

    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        if capture_names[capture.index as usize] == capture_name {
            names.push(
                capture
                    .node
                    .utf8_text(source.as_bytes())
                    .expect("capture text")
                    .to_owned(),
            );
        }
    }

    names
}

fn call_names(query_source: &str, source: &str) -> Vec<String> {
    capture_texts(query_source, source, "call.name")
}

fn artifact_target_label<'a>(
    artifact: &'a GraphIndexArtifact,
    edge: &'a spur_graph::GraphEdgeArtifact,
) -> Option<&'a str> {
    edge.target_label.as_deref().or_else(|| {
        edge.target_stable_symbol_id.as_deref().and_then(|id| {
            artifact
                .symbols
                .iter()
                .find(|symbol| symbol.stable_symbol_id == id)
                .map(|symbol| symbol.entity_name.as_str())
        })
    })
}

#[test]
fn proposed_macro_token_tree_query_captures_macro_body_calls() {
    let source = r#"fn caller() { json!({ "x": mermaid_subgraph(&view.nodes, &view.edges), "y": Type::bar(2) }); }"#;

    let names = call_names(PROPOSED_QUERY, source);

    assert!(names.contains(&"mermaid_subgraph".to_owned()));
    assert!(names.contains(&"bar".to_owned()));
}

#[test]
fn parenthesized_arg_query_filters_common_macro_token_false_positives() {
    let source = r#"fn caller() { json!({ "x": mermaid_subgraph(&view.nodes, &view.edges), "idx": out[0].name, "kw": if ok { 1 } else { 0 }, "variant": Some(Action::InspectPlan { plan_id }) }); }"#;

    let names = call_names(PARENTHESIZED_ARG_QUERY, source);

    assert!(names.contains(&"mermaid_subgraph".to_owned()));
    assert!(names.contains(&"Some".to_owned()));
    assert!(!names.contains(&"out".to_owned()));
    assert!(!names.contains(&"else".to_owned()));
    assert!(!names.contains(&"InspectPlan".to_owned()));
}

#[test]
fn rust_spur_edges_query_marks_macro_body_calls_with_macro_capture_names() {
    let source = r#"fn caller() { json!({ "x": mermaid_subgraph(&view.nodes, &view.edges), "y": Type::bar(2) }); }"#;

    let names = capture_texts(SPUR_EDGES_QUERY, source, "macro_call.name");

    assert!(names.contains(&"mermaid_subgraph".to_owned()));
    assert!(names.contains(&"bar".to_owned()));
}

#[test]
fn rust_query_captures_direct_macro_nodes_and_exact_paths_separately() {
    let source = r#"macro_rules! hidden { () => { panic!("hidden") }; }
fn caller() { ::std /* raw */ :: r#panic!("direct"); wrapper!(panic!("tokens")); helper(); }"#;
    assert_eq!(
        capture_texts(SPUR_EDGES_QUERY, source, "macro_invocation"),
        [
            "::std /* raw */ :: r#panic!(\"direct\")",
            "wrapper!(panic!(\"tokens\"))"
        ]
    );
    assert_eq!(
        capture_texts(SPUR_EDGES_QUERY, source, "macro_invocation.path"),
        ["::std /* raw */ :: r#panic", "wrapper"]
    );
    assert!(call_names(SPUR_EDGES_QUERY, source).contains(&"helper".to_owned()));
}

#[test]
fn rust_graph_artifact_emits_macro_body_calls_without_token_tree_noise() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("lib.rs"),
        r#"
struct Type;
impl Type {
    fn bar(_value: i32) {}
}

struct Holder {
    name: &'static str,
}

enum Action {
    InspectPlan { plan_id: u32 },
}

fn mermaid_subgraph(_nodes: &str, _edges: &str) {}

fn caller() {
    let out = [Holder { name: "node" }];
    let ok = true;
    let plan_id = 1;
    json!({
        "x": mermaid_subgraph("nodes", "edges"),
        "y": Type::bar(2),
        "idx": out[0].name,
        "kw": if ok { 1 } else { 0 },
        "variant": Some(Action::InspectPlan { plan_id }),
    });
}
"#,
    )
    .expect("write fixture");

    let facts = build_facts(dir.path(), None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, dir.path()).expect("artifact");
    let calls = artifact
        .edges
        .iter()
        .filter(|edge| {
            edge.relation == RelationKind::Calls && edge.edge_kind == Some(GraphEdgeKind::Calls)
        })
        .filter_map(|edge| artifact_target_label(&artifact, edge))
        .collect::<Vec<_>>();

    for target in ["mermaid_subgraph", "bar"] {
        assert!(
            calls.contains(&target),
            "missing macro-body Calls edge to {target}; calls: {calls:?}"
        );
    }
    for target in ["out", "else", "InspectPlan"] {
        assert!(
            !calls.contains(&target),
            "macro token-tree noise produced unexpected Calls edge to {target}; calls: {calls:?}"
        );
    }
}
