use std::collections::BTreeMap;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use spur_graph::extract::GraphFacts;
use spur_graph::{build_facts, GraphEdge, GraphEdgeKind, NodeId, NodeKind, RelationKind};

#[derive(Debug, Clone, Copy)]
struct EdgeExpectation {
    relation: RelationKind,
    source: &'static str,
    target: &'static str,
    edge_kind: Option<GraphEdgeKind>,
    bind_method: Option<&'static str>,
    import_path: Option<&'static str>,
}

#[derive(Debug)]
struct BenchmarkCase {
    name: &'static str,
    fixture_dir: &'static str,
    node_kind_counts: Vec<(&'static str, usize)>,
    relation_counts: Vec<(&'static str, usize)>,
    graph_edge_kind_counts: Vec<(&'static str, usize)>,
    must_have: Vec<EdgeExpectation>,
    must_not_have: Vec<EdgeExpectation>,
}

#[test]
fn semantic_benchmark_relation_contracts() {
    for case in benchmark_cases() {
        assert_benchmark_case(&case, false);
    }
}

#[test]
#[ignore = "prints current semantic benchmark counts and edge summaries"]
fn semantic_benchmark_dump_current_counts() {
    for case in benchmark_cases() {
        assert_benchmark_case(&case, true);
    }
}

fn benchmark_cases() -> Vec<BenchmarkCase> {
    vec![
        BenchmarkCase {
            name: "rust",
            fixture_dir: "rust",
            node_kind_counts: vec![
                ("Field", 2),
                ("File", 2),
                ("Function", 9),
                ("Impl", 3),
                ("Method", 6),
                ("Module", 1),
                ("Struct", 2),
                ("Trait", 2),
            ],
            relation_counts: vec![
                ("Calls", 11),
                ("Constructs", 2),
                ("Contains", 25),
                ("Defines", 8),
                ("Extends", 1),
                ("Implements", 2),
                ("Imports", 1),
                ("References", 1),
            ],
            graph_edge_kind_counts: vec![("ReferencesHof", 1)],
            must_have: vec![
                edge(RelationKind::Imports, "src/lib.rs", "helper"),
                edge(RelationKind::Calls, "run_direct", "helper"),
                edge(RelationKind::Calls, "run_method", "process"),
                edge(RelationKind::Calls, "run_scoped", "new"),
                edge(RelationKind::Calls, "run_macro", "helper"),
                edge_with_kind(
                    RelationKind::References,
                    "run_hof",
                    "normalize",
                    GraphEdgeKind::ReferencesHof,
                ),
                edge(RelationKind::Implements, "Runner for Worker", "Runner"),
                edge(RelationKind::Extends, "Runner", "Labeled"),
                edge(RelationKind::Constructs, "build_worker", "Worker"),
                edge(RelationKind::Contains, "Worker", "0"),
                edge(RelationKind::Defines, "Worker", "0"),
                edge(RelationKind::Contains, "Config", "id"),
                edge(RelationKind::Defines, "Config", "id"),
                edge(RelationKind::Contains, "src/lib.rs", "run_direct"),
            ],
            must_not_have: vec![
                edge(RelationKind::References, "run_hof", "inline_only"),
                edge(RelationKind::Constructs, "run_direct", "helper"),
            ],
        },
        BenchmarkCase {
            name: "typescript-tsx",
            fixture_dir: "typescript",
            node_kind_counts: vec![
                ("Class", 2),
                ("Enum", 1),
                ("EnumVariant", 1),
                ("File", 2),
                ("Function", 7),
                ("Interface", 1),
                ("Method", 4),
                ("TypeAlias", 2),
            ],
            relation_counts: vec![
                ("Calls", 9),
                ("Constructs", 1),
                ("Contains", 18),
                ("Defines", 5),
                ("Extends", 1),
                ("Implements", 1),
                ("Imports", 8),
                ("References", 1),
            ],
            graph_edge_kind_counts: vec![("ReferencesHof", 1)],
            must_have: vec![
                edge(RelationKind::Imports, "src/app.tsx", "BaseView"),
                edge(RelationKind::Calls, "boot", "renderThing"),
                edge(RelationKind::Calls, "boot", "mount"),
                edge(RelationKind::Constructs, "createDashboard", "Dashboard"),
                edge(RelationKind::Extends, "Dashboard", "BaseView"),
                edge(RelationKind::Implements, "Dashboard", "Renderable"),
                edge(RelationKind::Calls, "Root", "Greeting"),
                edge_with_kind(
                    RelationKind::References,
                    "renderList",
                    "renderItem",
                    GraphEdgeKind::ReferencesHof,
                ),
                edge(RelationKind::Defines, "Dashboard", "boot"),
                edge(RelationKind::Contains, "src/app.tsx", "Dashboard"),
            ],
            must_not_have: vec![
                edge(RelationKind::Calls, "Root", "div"),
                edge(RelationKind::References, "renderList", "inlineRender"),
                edge(RelationKind::Constructs, "boot", "renderThing"),
            ],
        },
        BenchmarkCase {
            name: "javascript-jsx",
            fixture_dir: "javascript",
            node_kind_counts: vec![("Class", 2), ("File", 3), ("Function", 6), ("Method", 2)],
            relation_counts: vec![
                ("Calls", 9),
                ("Constructs", 1),
                ("Contains", 10),
                ("Defines", 2),
                ("Extends", 1),
                ("Imports", 6),
                ("References", 1),
            ],
            graph_edge_kind_counts: vec![("ReferencesHof", 1)],
            must_have: vec![
                edge(RelationKind::Imports, "src/app.js", "BaseWidget"),
                edge(RelationKind::Calls, "render", "normalizeName"),
                edge(RelationKind::Calls, "render", "Badge"),
                edge(RelationKind::Constructs, "createDashboard", "Dashboard"),
                edge(RelationKind::Extends, "Dashboard", "BaseWidget"),
                edge(RelationKind::Calls, "Root", "Badge"),
                edge_with_kind(
                    RelationKind::References,
                    "runList",
                    "normalizeName",
                    GraphEdgeKind::ReferencesHof,
                ),
                edge(RelationKind::Contains, "src/app.js", "Dashboard"),
            ],
            must_not_have: vec![
                edge(RelationKind::Calls, "Root", "section"),
                edge(RelationKind::References, "runList", "inlineName"),
                edge(RelationKind::Constructs, "render", "Badge"),
            ],
        },
        BenchmarkCase {
            name: "python",
            fixture_dir: "python",
            node_kind_counts: vec![
                ("Class", 5),
                ("File", 3),
                ("Function", 5),
                ("Interface", 2),
                ("Method", 5),
            ],
            relation_counts: vec![
                ("Calls", 7),
                ("Constructs", 2),
                ("Contains", 17),
                ("Defines", 5),
                ("Extends", 2),
                ("Implements", 2),
                ("Imports", 8),
                ("References", 1),
            ],
            graph_edge_kind_counts: vec![("ReferencesHof", 1)],
            must_have: vec![
                edge(RelationKind::Imports, "src/main.py", "ConcreteService"),
                edge(RelationKind::Calls, "run", "make_message"),
                edge(RelationKind::Calls, "run", "send"),
                edge(RelationKind::Constructs, "run", "User"),
                edge(RelationKind::Constructs, "run", "ConcreteService"),
                edge(
                    RelationKind::Implements,
                    "ConcreteService",
                    "ServiceProtocol",
                ),
                edge(RelationKind::Implements, "ABCImpl", "AbstractBase"),
                edge_with_kind(
                    RelationKind::References,
                    "run_hof",
                    "normalize",
                    GraphEdgeKind::ReferencesHof,
                ),
                edge(RelationKind::Contains, "src/main.py", "run"),
            ],
            must_not_have: vec![
                edge(RelationKind::Extends, "MetaOnly", "Meta"),
                edge(RelationKind::Extends, "ConcreteService", "ServiceProtocol"),
                edge(RelationKind::Extends, "ABCImpl", "AbstractBase"),
                edge(RelationKind::References, "run_hof", "inline_only"),
                edge(RelationKind::Constructs, "run", "make_message"),
            ],
        },
        BenchmarkCase {
            name: "cpp",
            fixture_dir: "cpp",
            node_kind_counts: vec![
                ("File", 2),
                ("Function", 8),
                ("Method", 3),
                ("Module", 2),
                ("Struct", 3),
            ],
            relation_counts: vec![
                ("Calls", 9),
                ("Contains", 16),
                ("Defines", 9),
                ("Extends", 1),
                ("Imports", 7),
                ("References", 1),
            ],
            graph_edge_kind_counts: vec![("ReferencesHof", 1)],
            must_have: vec![
                edge(RelationKind::Imports, "src/main.cpp", "<algorithm>"),
                edge(RelationKind::Imports, "src/main.cpp", "Catalog"),
                edge(RelationKind::Calls, "run_catalog", "initialize"),
                edge(RelationKind::Calls, "run_catalog", "helper"),
                edge(RelationKind::Calls, "run_catalog", "load"),
                edge(RelationKind::Calls, "run_catalog", "make"),
                edge(RelationKind::Calls, "src/include/catalog.hpp", "#pragma"),
                edge(RelationKind::Extends, "CachedCatalog", "Catalog"),
                edge_with_kind(
                    RelationKind::References,
                    "run_catalog",
                    "keep_entry",
                    GraphEdgeKind::ReferencesHof,
                ),
                edge(RelationKind::Contains, "src/main.cpp", "run_catalog"),
            ],
            must_not_have: vec![
                edge(RelationKind::References, "run_catalog", "outside_std"),
                edge(RelationKind::Extends, "Plain", "Catalog"),
                edge(RelationKind::Constructs, "run_catalog", "load"),
            ],
        },
        BenchmarkCase {
            name: "markdown",
            fixture_dir: "markdown",
            node_kind_counts: vec![("File", 2), ("Section", 5)],
            relation_counts: vec![("Contains", 5), ("Links", 3)],
            graph_edge_kind_counts: vec![],
            must_have: vec![
                edge(RelationKind::Contains, "index.md", "Overview"),
                edge(RelationKind::Contains, "Overview", "Details"),
                edge(RelationKind::Contains, "Details", "Deep Dive"),
                edge(RelationKind::Links, "Overview", "guide.md"),
                edge(RelationKind::Links, "Details", "guide.md"),
                edge(RelationKind::Links, "Deep Dive", "index.md"),
            ],
            must_not_have: vec![
                edge(RelationKind::Links, "Overview", "[guide-ref]"),
                edge(RelationKind::Links, "Overview", "guide-ref"),
                edge(RelationKind::Imports, "index.md", "guide.md"),
                edge(RelationKind::Calls, "Overview", "guide.md"),
            ],
        },
        BenchmarkCase {
            name: "notebook",
            fixture_dir: "notebook",
            node_kind_counts: vec![
                ("Cell", 3),
                ("Constant", 1),
                ("File", 1),
                ("Port", 5),
                ("Section", 1),
            ],
            relation_counts: vec![
                ("Binds", 1),
                ("Calls", 3),
                ("Consumes", 2),
                ("Contains", 5),
                ("Emits", 1),
                ("Links", 1),
                ("Produces", 2),
                ("References", 3),
            ],
            graph_edge_kind_counts: vec![],
            must_have: vec![
                edge_with_bind(
                    RelationKind::Produces,
                    "cell://py",
                    "port://sales",
                    Some("declared"),
                ),
                edge_with_bind(
                    RelationKind::Consumes,
                    "cell://py",
                    "port://raw",
                    Some("declared"),
                ),
                edge_with_bind(
                    RelationKind::References,
                    "cell://py",
                    "ds://csv/raw",
                    Some("declared"),
                ),
                edge_with_bind(
                    RelationKind::Produces,
                    "cell://py",
                    "port://actual",
                    Some("actual"),
                ),
                edge_with_bind(
                    RelationKind::Consumes,
                    "cell://js",
                    "port://actual",
                    Some("actual"),
                ),
                edge_with_bind(
                    RelationKind::References,
                    "cell://py",
                    "ds://sales_orders",
                    Some("actual"),
                ),
                edge(RelationKind::Binds, "cell://py", "port://risk"),
                edge(RelationKind::Emits, "cell://py", "port://horizon"),
                edge(RelationKind::Contains, "analysis.ipynb", "cell://py"),
                edge(RelationKind::Contains, "analysis.ipynb", "cell://md"),
            ],
            must_not_have: vec![
                edge(RelationKind::Produces, "cell://py", "port://dynamic_name"),
                edge(RelationKind::Imports, "analysis.ipynb", "port://actual"),
            ],
        },
    ]
}

const fn edge(
    relation: RelationKind,
    source: &'static str,
    target: &'static str,
) -> EdgeExpectation {
    EdgeExpectation {
        relation,
        source,
        target,
        edge_kind: None,
        bind_method: None,
        import_path: None,
    }
}

const fn edge_with_kind(
    relation: RelationKind,
    source: &'static str,
    target: &'static str,
    edge_kind: GraphEdgeKind,
) -> EdgeExpectation {
    EdgeExpectation {
        relation,
        source,
        target,
        edge_kind: Some(edge_kind),
        bind_method: None,
        import_path: None,
    }
}

const fn edge_with_bind(
    relation: RelationKind,
    source: &'static str,
    target: &'static str,
    bind_method: Option<&'static str>,
) -> EdgeExpectation {
    EdgeExpectation {
        relation,
        source,
        target,
        edge_kind: None,
        bind_method,
        import_path: None,
    }
}

fn assert_benchmark_case(case: &BenchmarkCase, dump: bool) {
    let root = semantic_fixture_root().join(case.fixture_dir);
    assert!(
        root.exists(),
        "missing semantic benchmark fixture `{}` at {}; see tests/fixtures/semantic_benchmark/README.md",
        case.name,
        root.display()
    );

    let facts = build_facts(&root, None)
        .unwrap_or_else(|error| panic!("extract semantic benchmark `{}`: {error:#}", case.name))
        .0;
    if dump {
        dump_benchmark_case(case.name, &facts);
        return;
    }

    assert_eq!(
        count_node_kinds(&facts),
        expected_counts(&case.node_kind_counts),
        "{} node kind counts drifted",
        case.name
    );
    assert_eq!(
        count_relations(&facts),
        expected_counts(&case.relation_counts),
        "{} relation counts drifted",
        case.name
    );
    assert_eq!(
        count_graph_edge_kinds(&facts),
        expected_counts(&case.graph_edge_kind_counts),
        "{} graph edge kind counts drifted",
        case.name
    );

    if case.name == "rust" {
        let source = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
        assert_rust_field(&facts, &source, "Worker", "0", "pub u32");
        assert_rust_field(&facts, &source, "Config", "id", "pub id: u32");
    }

    for expected in &case.must_have {
        assert!(
            has_edge(&facts, expected),
            "{} missing expected edge {:?}; edges:\n{}",
            case.name,
            expected,
            format_edges(&facts)
        );
    }

    for forbidden in &case.must_not_have {
        assert!(
            !has_edge(&facts, forbidden),
            "{} had forbidden edge {:?}; edges:\n{}",
            case.name,
            forbidden,
            format_edges(&facts)
        );
    }
}

fn assert_rust_field(facts: &GraphFacts, source: &str, owner: &str, name: &str, text: &str) {
    let owners: Vec<_> = facts
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Struct && n.label == owner)
        .collect();
    let fields: Vec<_> = facts
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Field && n.label == name)
        .collect();
    assert_eq!(owners.len(), 1, "unique struct {owner}");
    assert_eq!(fields.len(), 1, "unique field {name}");
    let field = fields[0];
    let source_file = facts
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::File && n.label == "src/lib.rs")
        .expect("Rust fixture source file");
    assert_eq!(field.file_id, source_file.file_id, "{owner}::{name} file");
    let span = facts
        .spans
        .iter()
        .find(|s| Some(s.span_id) == field.source_span_id)
        .expect("field span");
    assert_eq!(field.file_id, Some(span.file_id));
    let start = source.find(text).expect("fixture field text");
    assert_eq!(
        (span.start_byte as usize, span.end_byte as usize),
        (start, start + text.len()),
        "{owner}::{name} span"
    );
    for relation in [RelationKind::Contains, RelationKind::Defines] {
        let owners_for_field: Vec<_> = facts
            .edges
            .iter()
            .filter(|e| e.relation == relation && e.target_node_id == Some(field.node_id))
            .map(|e| e.source_node_id)
            .collect();
        assert_eq!(
            owners_for_field,
            vec![owners[0].node_id],
            "{relation:?} direct owner of {owner}::{name}"
        );
    }
}

fn semantic_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic_benchmark")
}

fn count_node_kinds(facts: &GraphFacts) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node in &facts.nodes {
        *counts.entry(format!("{:?}", node.kind)).or_insert(0) += 1;
    }
    counts
}

fn count_relations(facts: &GraphFacts) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for edge in &facts.edges {
        *counts.entry(format!("{:?}", edge.relation)).or_insert(0) += 1;
    }
    counts
}

fn count_graph_edge_kinds(facts: &GraphFacts) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for edge in &facts.edges {
        if let Some(kind) = edge.edge_kind {
            *counts.entry(format!("{kind:?}")).or_insert(0) += 1;
        }
    }
    counts
}

fn expected_counts(expected: &[(&str, usize)]) -> BTreeMap<String, usize> {
    expected
        .iter()
        .map(|(kind, count)| ((*kind).to_owned(), *count))
        .collect()
}

fn has_edge(facts: &GraphFacts, expected: &EdgeExpectation) -> bool {
    facts.edges.iter().any(|edge| {
        edge.relation == expected.relation
            && source_label(facts, edge) == Some(expected.source)
            && target_label(facts, edge) == Some(expected.target)
            && expected
                .edge_kind
                .map(|kind| edge.edge_kind == Some(kind))
                .unwrap_or(true)
            && expected
                .bind_method
                .map(|bind_method| edge.bind_method.as_deref() == Some(bind_method))
                .unwrap_or(true)
            && expected
                .import_path
                .map(|import_path| edge.import_path.as_deref() == Some(import_path))
                .unwrap_or(true)
    })
}

fn source_label<'a>(facts: &'a GraphFacts, edge: &GraphEdge) -> Option<&'a str> {
    node_label(facts, edge.source_node_id)
}

fn target_label<'a>(facts: &'a GraphFacts, edge: &'a GraphEdge) -> Option<&'a str> {
    edge.target_node_id
        .and_then(|node_id| node_label(facts, node_id))
        .or(edge.target_label.as_deref())
}

fn node_label(facts: &GraphFacts, node_id: NodeId) -> Option<&str> {
    facts
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .map(|node| node.label.as_str())
}

fn format_edges(facts: &GraphFacts) -> String {
    let mut rows = facts
        .edges
        .iter()
        .map(|edge| {
            format!(
                "{:?}: {} -> {} kind={:?} bind={:?} import_path={:?} confidence={:?} score={:.2}",
                edge.relation,
                source_label(facts, edge).unwrap_or("<missing-source>"),
                target_label(facts, edge).unwrap_or("<missing-target>"),
                edge.edge_kind,
                edge.bind_method,
                edge.import_path,
                edge.confidence,
                edge.confidence_score,
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.join("\n")
}

fn dump_benchmark_case(name: &str, facts: &GraphFacts) {
    eprintln!("=== semantic benchmark: {name} ===");
    eprintln!(
        "node_kind_counts: {}",
        format_count_array(count_node_kinds(facts))
    );
    eprintln!(
        "relation_counts: {}",
        format_count_array(count_relations(facts))
    );
    eprintln!(
        "graph_edge_kind_counts: {}",
        format_count_array(count_graph_edge_kinds(facts))
    );
    eprintln!("edges:\n{}", format_edges(facts));
}

fn format_count_array(counts: BTreeMap<String, usize>) -> String {
    let entries = counts
        .into_iter()
        .map(|(kind, count)| format!("(\"{kind}\", {count})"))
        .collect::<Vec<_>>();
    format!("[{}]", entries.join(", "))
}
