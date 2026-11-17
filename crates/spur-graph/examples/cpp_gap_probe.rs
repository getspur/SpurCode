//! Reproducible research probe of the real C++ extractor, without changing its policy.
//! Run with `scripts/spur-cargo run -p spur-graph --no-default-features
//! --example cpp_gap_probe -- <output.json>` from the workspace root.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::{build_facts_for_paths, BytesExtractor};
use spur_graph::extract::GraphFacts;
use std::path::{Path, PathBuf};

fn facts_json(facts: &GraphFacts) -> Value {
    json!({"nodes": facts.nodes, "edges": facts.edges, "spans": facts.spans})
}

fn syntax_calls(node: tree_sitter::Node<'_>, bytes: &[u8], out: &mut Vec<Value>) {
    if matches!(node.kind(), "call_expression" | "new_expression") {
        let callee = node.child_by_field_name("function");
        out.push(json!({
            "kind": node.kind(), "start_byte": node.start_byte(), "end_byte": node.end_byte(),
            "start_row": node.start_position().row,
            "text": node.utf8_text(bytes).unwrap_or(""),
            "callee_kind": callee.map(|n| n.kind()),
            "callee_text": callee.and_then(|n| n.utf8_text(bytes).ok()),
        }));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        syntax_calls(child, bytes, out);
    }
}

fn probe(path: &Path, bytes: &[u8]) -> Result<Value> {
    let mut extractor = BytesExtractor::for_language(Language::Cpp)?;
    let symbols: Vec<_> = extractor
        .extract(path, bytes)?
        .into_iter()
        .map(|symbol| {
            json!({
                "entity_name": symbol.entity_name, "symbol_kind": symbol.symbol_kind,
                "enclosing_scope": symbol.enclosing_scope, "byte_range": symbol.byte_range,
                "line_range": symbol.line_range, "anchor_hash": symbol.anchor_hash,
            })
        })
        .collect();
    let facts = extractor.extract_file_facts(path, bytes)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_cpp::LANGUAGE.into())?;
    let tree = parser.parse(bytes, None).context("no C++ syntax tree")?;
    let mut calls = Vec::new();
    syntax_calls(tree.root_node(), bytes, &mut calls);
    Ok(json!({
        "logical_path": path, "source_sha256": format!("{:x}", Sha256::digest(bytes)),
        "symbols": symbols, "facts": facts_json(&facts), "syntax_calls": calls,
        "parse_has_error": tree.root_node().has_error(),
    }))
}

fn main() -> Result<()> {
    let output = std::env::args()
        .nth(1)
        .context("supply the output JSON path")?;
    let cases: Vec<Value> = serde_json::from_str(include_str!("cpp_gap_probe_cases.json"))?;
    let mut fixture_results = Vec::new();
    for case in cases {
        let root = tempfile::tempdir()?;
        let cpp = case["cpp"].as_str().context("missing C++ fixture")?;
        let bytes = format!("{cpp}\n");
        std::fs::write(root.path().join("probe.cpp"), &bytes)?;
        let mut paths = vec![PathBuf::from("probe.cpp")];
        if let Some(header) = case["header"].as_str() {
            std::fs::write(root.path().join("api.hpp"), format!("{header}\n"))?;
            paths.push(PathBuf::from("api.hpp"));
        }
        let single = probe(Path::new("probe.cpp"), bytes.as_bytes())?;
        let repository = build_facts_for_paths(root.path(), &paths)?;
        fixture_results.push(json!({
            "name": case["name"], "purpose": case["purpose"], "single_file": single,
            "selected_repository": facts_json(&repository), "selected_files": paths,
        }));
    }
    let real: Vec<Value> = serde_json::from_str(include_str!("cpp_gap_probe_real.json"))?;
    let root = tempfile::tempdir()?;
    let mut real_results = Vec::new();
    let mut paths = Vec::new();
    for input in real {
        let path = PathBuf::from(input["file_path"].as_str().context("missing real path")?);
        let bytes = input["cpp"]
            .as_str()
            .context("missing real source")?
            .as_bytes();
        let result = probe(&path, bytes)?;
        anyhow::ensure!(
            result["source_sha256"] == input["source_sha256"],
            "input identity mismatch"
        );
        let disk_path = root.path().join(&path);
        std::fs::create_dir_all(disk_path.parent().context("missing parent")?)?;
        std::fs::write(disk_path, bytes)?;
        paths.push(path);
        real_results.push(json!({"revision": input["revision"], "single_file": result}));
    }
    let selected = build_facts_for_paths(root.path(), &paths)?;
    let result = json!({
        "schema": "cpp_gap_probe_v1", "provider": "spur_graph::extract (current source)",
        "fixtures": fixture_results, "real_files": real_results,
        "real_selected_repository": facts_json(&selected), "real_selected_files": paths,
        "scope": "Pinned two-file head snapshot; includes are not expanded by BytesExtractor",
    });
    let output = Path::new(&output);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, serde_json::to_vec_pretty(&result)?)?;
    println!(
        "cpp_gap_probe_v1: 14 fixtures and 2 pinned real files -> {}",
        output.display()
    );
    Ok(())
}
