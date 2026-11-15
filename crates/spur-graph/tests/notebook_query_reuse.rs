use std::collections::HashSet;

use serde_json::json;
use spur_graph::{build_facts, NodeKind};

#[test]
fn repeated_languages_keep_each_cells_symbols_and_sections() {
    let mut cells = Vec::new();
    for repetition in 0..3 {
        cells.push(json!({"cell_type":"code","id":format!("rust-{repetition}"),
            "metadata":{"spur":{"code_type":"rust"}},"source":["fn shared_rust() {}\n"]}));
        cells.push(
            json!({"cell_type":"markdown","id":format!("markdown-{repetition}"),
            "metadata":{},"source":["# Shared section\n"]}),
        );
        cells.push(json!({"cell_type":"code","id":format!("python-{repetition}"),
            "metadata":{"spur":{"code_type":"python"}},"source":["def shared_python():\n    return 1\n"]}));
    }
    let notebook = json!({"nbformat":4,"nbformat_minor":5,
        "metadata":{"kernelspec":{"name":"python3"}},"cells":cells});
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("mixed.ipynb"),
        serde_json::to_vec(&notebook).unwrap(),
    )
    .unwrap();
    let (facts, _) = build_facts(root.path(), None).unwrap();
    for label in ["shared_rust", "shared_python"] {
        let functions: Vec<_> = facts
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function && n.label == label)
            .collect();
        assert_eq!(functions.len(), 3, "{label}");
        assert_eq!(
            functions
                .iter()
                .map(|n| n.node_id)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
    }
    assert_eq!(
        facts
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Cell)
            .count(),
        9
    );
    assert_eq!(
        facts
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Section && n.label == "Shared section")
            .count(),
        3
    );
}
