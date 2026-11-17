use std::collections::HashMap;
use std::path::Path;
use std::str;

use anyhow::Context as _;
use serde_json::Value;
use tree_sitter::{Parser, Point, Query, QueryCursor, Range, StreamingIterator as _};

use crate::extract::languages::Language;
use crate::extract::markdown::extract_markdown_contents;
use crate::extract::notebook_model::{normalize_notebook, NotebookDiagnosticCode, NotebookStatus};
use crate::extract::tree_sitter::{
    compile_queries, extract_file_contents_from_tree, relative_path, CompiledQueries, FactBuilder,
};
use crate::{FileId, GraphSymbolArtifact, NodeId, NodeKind, RelationKind};

fn language_for_token(token: &str) -> Option<Language> {
    match token.to_ascii_lowercase().as_str() {
        "python" | "python3" => Some(Language::Python),
        "javascript" => Some(Language::Javascript),
        "rust" | "evcxr" => Some(Language::Rust),
        "go" | "gonb" => Some(Language::Go),
        "sql" => Some(Language::Sql),
        "ns_mermaid" | "mermaid" => Some(Language::Mermaid),
        _ => None,
    }
}

pub(crate) fn cell_is_markdown(cell: &Value) -> bool {
    cell.get("cell_type").and_then(Value::as_str) == Some("markdown")
}

pub(crate) fn resolve_cell_language(cell: &Value, root: &Value) -> Option<Language> {
    let cell_meta = cell.get("metadata");
    if let Some(code_type) = cell_meta
        .and_then(|metadata| metadata.get("spur"))
        .and_then(|spur| spur.get("code_type"))
        .and_then(Value::as_str)
    {
        return language_for_token(code_type);
    }

    let candidates = [
        cell_meta
            .and_then(|metadata| metadata.get("kernelspec"))
            .and_then(|kernelspec| kernelspec.get("name"))
            .and_then(Value::as_str),
        root.get("metadata")
            .and_then(|metadata| metadata.get("kernelspec"))
            .and_then(|kernelspec| kernelspec.get("name"))
            .and_then(Value::as_str),
        root.get("metadata")
            .and_then(|metadata| metadata.get("language_info"))
            .and_then(|language_info| language_info.get("name"))
            .and_then(Value::as_str),
    ];

    candidates
        .into_iter()
        .flatten()
        .find_map(language_for_token)
}

pub(crate) fn extract_notebook_file(
    builder: &mut FactBuilder<'_>,
    path: &Path,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let notebook = normalize_notebook(bytes);
    if notebook.status == NotebookStatus::Invalid {
        let diagnostics: Vec<_> = notebook
            .diagnostics
            .iter()
            .chain(notebook.cells.iter().flat_map(|cell| &cell.diagnostics))
            .map(|diagnostic| (diagnostic.cell_ordinal, diagnostic.code))
            .collect();
        if notebook
            .diagnostics
            .iter()
            .any(|d| d.code == NotebookDiagnosticCode::InvalidJson)
        {
            anyhow::bail!("parse .ipynb JSON: {diagnostics:?}");
        }
        anyhow::bail!("invalid .ipynb notebook inventory: {diagnostics:?}");
    }
    let source = str::from_utf8(bytes).context("read .ipynb as UTF-8")?;
    let relative_path = relative_path(builder.root(), path)?;
    let file_id = FileId(builder.next_file_id());
    let root_tree = parse_source(Language::JupyterNotebook, source)?;
    let file_node = builder.add_file_node(&relative_path, file_id, root_tree.root_node());

    let mut port_nodes = HashMap::new();
    let mut query_cache = HashMap::new();
    for cell in &notebook.cells {
        // Invalid inventory was rejected above. All ranges and graph labels use
        // original decoded bytes; only the parser sees prepared analysis bytes.
        let Some(prepared) = &cell.source else {
            continue;
        };
        let cell_source = prepared.original.as_str();
        let cell_identity_path = cell.identity_path(&relative_path);
        let cell_node = add_cell_node(
            builder,
            &relative_path,
            file_id,
            file_node,
            &cell.id,
            cell_source,
        );
        emit_declared_metadata_facts(
            builder,
            &relative_path,
            file_id,
            cell_node,
            &cell.metadata,
            cell_source,
            &mut port_nodes,
        );
        // Legacy GraphFacts has no coverage field. Callers needing notebook
        // coverage must also inspect normalize_notebook's typed diagnostics.
        match cell.language {
            Some(language) => {
                extract_cell(
                    builder,
                    CellExtractInput {
                        relative_path: &relative_path,
                        identity_relative_path: &cell_identity_path,
                        file_id,
                        parent_node: cell_node,
                        language,
                        source: cell_source,
                        analysis_source: &prepared.analysis,
                        port_nodes: &mut port_nodes,
                        query_cache: &mut query_cache,
                    },
                )?;
            }
            None if cell.cell_type.as_deref() == Some("code") => {
                tracing::warn!(
                    path = %path.display(),
                    "spur-graph: skipping notebook cell (unsupported language)"
                );
            }
            None => {}
        }
    }

    Ok(())
}

/// Document to slice with a notebook symbol's cell-relative `line_range`.
///
/// Extract stores ranges against decoded cell source, not the `.ipynb` JSON
/// wrapper. Callers that slice file bytes by those ranges read the wrong
/// document. `None` means "use the file bytes".
pub(crate) fn decoded_source_document(
    file_source: &str,
    symbol: &GraphSymbolArtifact,
) -> Option<String> {
    if !symbol.file_path.ends_with(".ipynb") {
        return None;
    }
    let wanted = notebook_cell_id_for_symbol(symbol)?;
    let notebook = normalize_notebook(file_source.as_bytes());
    if notebook.status == NotebookStatus::Invalid {
        return None;
    }
    notebook
        .cells
        .into_iter()
        .find(|cell| cell.id == wanted)?
        .source
        .map(|source| source.original)
}

fn notebook_cell_id_for_symbol(symbol: &GraphSymbolArtifact) -> Option<&str> {
    strip_cell_uri(symbol.enclosing_scope.as_deref()).or_else(|| {
        if symbol.symbol_kind == "cell" {
            strip_cell_uri(Some(symbol.entity_name.as_str()))
        } else {
            None
        }
    })
}

fn strip_cell_uri(value: Option<&str>) -> Option<&str> {
    value
        .and_then(|text| text.strip_prefix("cell://"))
        .filter(|id| !id.is_empty())
}

fn add_cell_node(
    builder: &mut FactBuilder<'_>,
    relative_path: &str,
    file_id: FileId,
    file_node: NodeId,
    cell_id: &str,
    source: &str,
) -> NodeId {
    let cell_label = format!("cell://{cell_id}");
    let cell_node = builder.add_node_with_range(
        relative_path,
        cell_label.clone(),
        cell_label,
        NodeKind::Cell,
        file_id,
        source_range(source),
    );
    builder.add_edge(file_node, Some(cell_node), RelationKind::Contains, None);
    cell_node
}

fn emit_declared_metadata_facts(
    builder: &mut FactBuilder<'_>,
    relative_path: &str,
    file_id: FileId,
    cell_node: NodeId,
    metadata: &Value,
    source: &str,
    port_nodes: &mut HashMap<String, NodeId>,
) {
    let Some(spur) = metadata.get("spur") else {
        return;
    };

    if let Some(dag) = spur.get("dag") {
        if let Some(produces) = dag.get("produces").and_then(Value::as_array) {
            for port in produces
                .iter()
                .filter_map(|entry| entry.get("port").and_then(Value::as_str))
            {
                let port_node =
                    intern_port(builder, relative_path, file_id, source, port_nodes, port);
                builder.add_edge_with_bind_method(
                    cell_node,
                    Some(port_node),
                    RelationKind::Produces,
                    None,
                    "declared",
                );
            }
        }

        if let Some(consumes) = dag.get("consumes").and_then(Value::as_array) {
            for port in consumes.iter().filter_map(Value::as_str) {
                let port_node =
                    intern_port(builder, relative_path, file_id, source, port_nodes, port);
                builder.add_edge_with_bind_method(
                    cell_node,
                    Some(port_node),
                    RelationKind::Consumes,
                    None,
                    "declared",
                );
            }
        }

        if let Some(source_ref) = dag.get("source") {
            if let (Some(kind), Some(port)) = (
                source_ref.get("kind").and_then(Value::as_str),
                source_ref.get("port").and_then(Value::as_str),
            ) {
                builder.add_edge_with_bind_method(
                    cell_node,
                    None,
                    RelationKind::References,
                    Some(format!("ds://{kind}/{port}")),
                    "declared",
                );
            }
        }
    }

    if let Some(frontend) = spur.get("frontend") {
        if let Some(binds) = frontend.get("binds").and_then(Value::as_array) {
            for port in binds.iter().filter_map(Value::as_str) {
                let port_node =
                    intern_port(builder, relative_path, file_id, source, port_nodes, port);
                builder.add_edge(cell_node, Some(port_node), RelationKind::Binds, None);
            }
        }

        if let Some(emits) = frontend.get("emits").and_then(Value::as_array) {
            for port in emits.iter().filter_map(Value::as_str) {
                let port_node =
                    intern_port(builder, relative_path, file_id, source, port_nodes, port);
                builder.add_edge(cell_node, Some(port_node), RelationKind::Emits, None);
            }
        }
    }
}

fn intern_port(
    builder: &mut FactBuilder<'_>,
    relative_path: &str,
    file_id: FileId,
    source: &str,
    port_nodes: &mut HashMap<String, NodeId>,
    port: &str,
) -> NodeId {
    if let Some(node_id) = port_nodes.get(port).copied() {
        return node_id;
    }

    let port_label = format!("port://{port}");
    let port_node = builder.add_node_with_range(
        relative_path,
        port_label.clone(),
        port_label,
        NodeKind::Port,
        file_id,
        source_range(source),
    );
    port_nodes.insert(port.to_owned(), port_node);
    port_node
}

fn source_range(source: &str) -> Range {
    Range {
        start_byte: 0,
        end_byte: source.len(),
        start_point: Point { row: 0, column: 0 },
        end_point: end_point(source),
    }
}

fn end_point(source: &str) -> Point {
    let mut row = 0;
    let mut column = 0;
    for byte in source.bytes() {
        if byte == b'\n' {
            row += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    Point { row, column }
}

struct CellExtractInput<'a> {
    relative_path: &'a str,
    identity_relative_path: &'a str,
    file_id: FileId,
    parent_node: NodeId,
    language: Language,
    source: &'a str,
    analysis_source: &'a str,
    port_nodes: &'a mut HashMap<String, NodeId>,
    query_cache: &'a mut HashMap<Language, CompiledQueries>,
}

fn extract_cell(builder: &mut FactBuilder<'_>, input: CellExtractInput<'_>) -> anyhow::Result<()> {
    let CellExtractInput {
        relative_path,
        identity_relative_path,
        file_id,
        parent_node,
        language,
        source,
        analysis_source,
        port_nodes,
        query_cache,
    } = input;
    let config = language.config();
    // Query definitions depend only on the language. Keep cursors and parsed
    // trees local to each cell while reusing queries within this notebook.
    let queries = match query_cache.entry(language) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(compile_queries(&config, language)?)
        }
    };
    let tree = parse_source(language, analysis_source)?;

    if language == Language::Markdown {
        let mut inline_parser = markdown_inline_parser(&config)?;
        return extract_markdown_contents(
            builder,
            &config,
            identity_relative_path,
            file_id,
            parent_node,
            source,
            tree.root_node(),
            queries,
            inline_parser.as_mut(),
        );
    }

    emit_actual_notebook_facts(
        builder,
        ActualNotebookFactInput {
            relative_path,
            file_id,
            cell_node: parent_node,
            query: queries.spur_notebook_facts.as_ref(),
            source,
            root_node: tree.root_node(),
        },
        port_nodes,
    )?;

    extract_file_contents_from_tree(
        builder,
        language.label(),
        &config,
        identity_relative_path,
        file_id,
        parent_node,
        source,
        tree.root_node(),
        queries,
    )
}

struct ActualNotebookFactInput<'a, 'tree> {
    relative_path: &'a str,
    file_id: FileId,
    cell_node: NodeId,
    query: Option<&'a Query>,
    source: &'a str,
    root_node: tree_sitter::Node<'tree>,
}

fn emit_actual_notebook_facts(
    builder: &mut FactBuilder<'_>,
    input: ActualNotebookFactInput<'_, '_>,
    port_nodes: &mut HashMap<String, NodeId>,
) -> anyhow::Result<()> {
    let Some(query) = input.query else {
        return Ok(());
    };

    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, input.root_node, input.source.as_bytes());
    while let Some(query_match) = matches.next() {
        let mut produces = false;
        let mut consumes = false;
        let mut port_name = None;
        let mut table_call = None;

        for capture in query_match.captures {
            let capture_name = capture_names[capture.index as usize];
            match capture_name {
                "port.produce" => produces = true,
                "port.consume" => consumes = true,
                "port.name" | "port.get.name" => {
                    port_name = string_literal_value(input.source, capture.node);
                }
                "table.call" => {
                    table_call = capture
                        .node
                        .utf8_text(input.source.as_bytes())
                        .ok()
                        .map(str::to_owned);
                }
                _ => {}
            }
        }

        if produces {
            if let Some(port) = port_name.as_deref() {
                let port_node = intern_port(
                    builder,
                    input.relative_path,
                    input.file_id,
                    input.source,
                    port_nodes,
                    port,
                );
                builder.add_edge_with_bind_method(
                    input.cell_node,
                    Some(port_node),
                    RelationKind::Produces,
                    None,
                    "actual",
                );
            } else {
                builder.add_edge_with_bind_method(
                    input.cell_node,
                    None,
                    RelationKind::References,
                    Some("opaque_put".to_owned()),
                    "actual",
                );
            }
        }

        if consumes {
            if let Some(port) = port_name.as_deref() {
                let port_node = intern_port(
                    builder,
                    input.relative_path,
                    input.file_id,
                    input.source,
                    port_nodes,
                    port,
                );
                builder.add_edge_with_bind_method(
                    input.cell_node,
                    Some(port_node),
                    RelationKind::Consumes,
                    None,
                    "actual",
                );
            }
        }

        if let Some(call_name) = table_call
            .as_deref()
            .filter(|name| is_table_function_call(name))
        {
            builder.add_edge_with_bind_method(
                input.cell_node,
                None,
                RelationKind::References,
                Some(format!("ds://{call_name}")),
                "actual",
            );
        }
    }

    Ok(())
}

fn string_literal_value(source: &str, node: tree_sitter::Node<'_>) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?.trim();
    let (start, quote) = text
        .char_indices()
        .find(|(_, ch)| matches!(ch, '"' | '\'' | '`'))?;
    let end = text.rfind(quote)?;
    if end <= start {
        return None;
    }
    Some(text[start + quote.len_utf8()..end].to_owned())
}

fn is_table_function_call(name: &str) -> bool {
    if !name.contains('_') {
        return false;
    }
    if !name
        .bytes()
        .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    {
        return false;
    }
    let mut parts = name.split('_');
    let Some(first) = parts.next() else {
        return false;
    };
    !first.is_empty()
        && first
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| !part.is_empty())
}

fn parse_source(language: Language, source: &str) -> anyhow::Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let ts_language = language.tree_sitter_language();
    parser
        .set_language(&ts_language)
        .with_context(|| format!("set {} parser language", language.label()))?;
    parser
        .parse(source.as_bytes(), None)
        .with_context(|| format!("parse {} notebook cell", language.label()))
}

fn markdown_inline_parser(
    config: &crate::extract::languages::LanguageConfig,
) -> anyhow::Result<Option<Parser>> {
    let Some(inline_language) = config.inline_language.as_ref() else {
        return Ok(None);
    };
    let mut parser = Parser::new();
    parser
        .set_language(inline_language)
        .context("set markdown inline parser language")?;
    Ok(Some(parser))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::extract::tree_sitter::FactBuilder;
    use crate::extract::GraphFacts;
    use crate::{NodeKind, RelationKind};

    #[test]
    fn code_type_takes_precedence() {
        let cell = json!({"cell_type":"code","metadata":{"spur":{"code_type":"javascript"}}});
        let root = json!({"metadata":{"kernelspec":{"name":"python3"}}});

        assert_eq!(
            resolve_cell_language(&cell, &root),
            Some(Language::Javascript)
        );
    }

    #[test]
    fn falls_back_to_notebook_kernelspec_then_language_info() {
        let cell = json!({"cell_type":"code","metadata":{}});
        let root = json!({"metadata":{"kernelspec":{"name":"python3"}}});
        assert_eq!(resolve_cell_language(&cell, &root), Some(Language::Python));

        let root2 = json!({"metadata":{"language_info":{"name":"rust"}}});
        assert_eq!(resolve_cell_language(&cell, &root2), Some(Language::Rust));
    }

    #[test]
    fn unknown_and_unsupported_languages_resolve_to_none() {
        let root = json!({"metadata":{}});
        for kernel in ["julia", "r", "ir", "haskell"] {
            let cell = json!({"cell_type":"code","metadata":{"spur":{"code_type":kernel}}});
            assert_eq!(resolve_cell_language(&cell, &root), None, "{kernel}");
        }
    }

    #[test]
    fn explicit_unmapped_code_type_does_not_fall_through_to_kernel() {
        let root = json!({"metadata":{"kernelspec":{"name":"python3"}}});
        for token in ["julia", "r", "ir", "haskell"] {
            let cell = json!({"cell_type":"code","metadata":{"spur":{"code_type":token}}});
            assert_eq!(
                resolve_cell_language(&cell, &root),
                None,
                "{token} must skip, not inherit python3"
            );
        }
    }

    #[test]
    fn ns_mermaid_code_type_does_not_inherit_python_kernel() {
        let root = json!({"metadata":{"kernelspec":{"name":"python3"}}});
        let cell = json!({"cell_type":"code","metadata":{"spur":{"code_type":"ns_mermaid"}}});
        assert_eq!(
            resolve_cell_language(&cell, &root),
            Some(Language::Mermaid),
            "ns_mermaid is Mermaid, never Python"
        );
    }

    #[test]
    fn mcp_tokens_map_even_when_notebook_kernel_is_python() {
        let root = json!({"metadata":{"kernelspec":{"name":"python3"}}});
        for (token, language) in [
            ("go", Language::Go),
            ("gonb", Language::Go),
            ("sql", Language::Sql),
            ("ns_mermaid", Language::Mermaid),
            ("mermaid", Language::Mermaid),
        ] {
            let cell = json!({"cell_type":"code","metadata":{"spur":{"code_type":token}}});
            assert_eq!(
                resolve_cell_language(&cell, &root),
                Some(language),
                "{token}"
            );
        }
    }

    #[test]
    fn detects_markdown_cells() {
        assert!(cell_is_markdown(&json!({"cell_type":"markdown"})));
        assert!(!cell_is_markdown(&json!({"cell_type":"code"})));
    }

    #[test]
    fn ns_mermaid_cell_extracts_flowchart_vertices_under_python_kernel() {
        let nb = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"kernelspec": {"name": "python3"}},
            "cells": [
                {
                    "cell_type": "code",
                    "id": "diagram",
                    "metadata": {"spur": {"code_type": "ns_mermaid"}},
                    "source": ["flowchart TD\n", "    SPEC[spec]\n", "    CHECK[check]\n", "    SPEC --> CHECK\n"]
                }
            ]
        });
        let bytes = serde_json::to_vec(&nb).expect("serialize notebook");
        let facts = run_notebook_extraction(Path::new("design.ipynb"), &bytes)
            .expect("ns_mermaid cell extracts");
        assert!(
            facts
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::Module && node.label == "SPEC"),
            "expected Module SPEC, got {:?}",
            facts
                .nodes
                .iter()
                .map(|node| (node.label.clone(), node.kind))
                .collect::<Vec<_>>()
        );
        assert!(facts
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::Module && node.label == "CHECK"));
        assert!(facts
            .nodes
            .iter()
            .any(|node| { node.kind == NodeKind::Cell && node.label == "cell://diagram" }));
    }

    #[test]
    fn mermaid_vertex_read_document_is_decoded_cell_source_not_json() {
        let nb = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"kernelspec": {"name": "python3"}},
            "cells": [
                {
                    "cell_type": "code",
                    "id": "diagram",
                    "metadata": {"spur": {"code_type": "ns_mermaid"}},
                    "source": ["flowchart TD\n", "    SPEC[spec]\n", "    CHECK[check]\n", "    SPEC --> CHECK\n"]
                }
            ]
        });
        let bytes = serde_json::to_vec(&nb).expect("serialize notebook");
        let file_source = String::from_utf8(bytes).expect("utf8 notebook");
        let symbol = GraphSymbolArtifact {
            stable_symbol_id: "spec".to_owned(),
            file_path: "design.ipynb".to_owned(),
            byte_range: [0, 4],
            line_range: [2, 2],
            entity_name: "SPEC".to_owned(),
            qualified_name: "cell://diagram::SPEC".to_owned(),
            symbol_kind: "module".to_owned(),
            anchor_hash: "0".to_owned(),
            enclosing_scope: Some("cell://diagram".to_owned()),
        };
        let document = decoded_source_document(&file_source, &symbol)
            .expect("notebook symbols must expose decoded cell source");
        assert!(
            document.contains("SPEC[spec]"),
            "decoded cell source should contain mermaid vertex text, got {document:?}"
        );
        assert!(
            !document.contains("\"cell_type\""),
            "decoded cell source must not be the JSON wrapper, got {document:?}"
        );
    }

    #[test]
    fn missing_cell_id_is_content_stable_across_reorder() {
        let source_a = "def alpha():\n    return 1\n";
        let source_b = "def beta():\n    return 2\n";
        let cell = |source: &str| {
            json!({
                "cell_type": "code",
                "metadata": {"spur": {"code_type": "python"}},
                "source": [source]
            })
        };
        let facts_ab = {
            let nb = json!({
                "nbformat": 4,
                "nbformat_minor": 5,
                "metadata": {},
                "cells": [cell(source_a), cell(source_b)]
            });
            let bytes = serde_json::to_vec(&nb).unwrap();
            run_notebook_extraction(Path::new("nb.ipynb"), &bytes).unwrap()
        };
        let facts_ba = {
            let nb = json!({
                "nbformat": 4,
                "nbformat_minor": 5,
                "metadata": {},
                "cells": [cell(source_b), cell(source_a)]
            });
            let bytes = serde_json::to_vec(&nb).unwrap();
            run_notebook_extraction(Path::new("nb.ipynb"), &bytes).unwrap()
        };
        let key = |facts: &GraphFacts, name: &str| {
            facts
                .nodes
                .iter()
                .find(|node| node.label == name)
                .map(|node| node.stable_key.clone())
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        assert_eq!(key(&facts_ab, "alpha"), key(&facts_ba, "alpha"));
        assert_eq!(key(&facts_ab, "beta"), key(&facts_ba, "beta"));
        assert_ne!(key(&facts_ab, "alpha"), key(&facts_ab, "beta"));
    }

    #[test]
    fn extracts_python_def_and_markdown_heading_as_transitively_contained_children() {
        let nb = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"kernelspec": {"name": "python3"}},
            "cells": [
                {
                    "cell_type": "code",
                    "metadata": {},
                    "source": ["def load_df():\n", "    return 1\n"]
                },
                {
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": ["# Analysis\n"]
                }
            ]
        });
        let bytes = serde_json::to_vec(&nb).expect("serialize notebook");

        let facts =
            run_notebook_extraction(Path::new("nb.ipynb"), &bytes).expect("notebook extracts");

        assert!(has_symbol(&facts, "load_df"));
        assert!(has_section(&facts, "Analysis"));
        assert!(all_non_file_nodes_reachable_from_file_via_contains(
            &facts, "nb.ipynb"
        ));
        assert!(all_symbols_contained_by_cells(&facts));
        assert!(!file_directly_contains_symbols(&facts, "nb.ipynb"));
    }

    #[test]
    fn each_cell_gets_a_cell_container_node() {
        let nb = serde_json::to_vec(&serde_json::json!({
            "nbformat": 4, "nbformat_minor": 5, "metadata": {},
            "cells": [
                {"cell_type":"code","id":"a3f1","source":["def load():\n"," pass\n"],"metadata":{},"outputs":[],"execution_count":null},
                {"cell_type":"code","id":"b2c9","source":["x = load()\n"],"metadata":{},"outputs":[],"execution_count":null}
            ]
        }))
        .unwrap();
        let mut builder = FactBuilder::new(Path::new("/nb"));
        extract_notebook_file(&mut builder, Path::new("/nb/app.ipynb"), &nb).unwrap();
        let facts = builder.into_facts();
        let cell_nodes: Vec<_> = facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Cell)
            .collect();
        assert_eq!(cell_nodes.len(), 2);
        let labels: Vec<&str> = cell_nodes.iter().map(|node| node.label.as_str()).collect();
        assert!(labels.contains(&"cell://a3f1"));
        assert!(labels.contains(&"cell://b2c9"));
    }

    #[test]
    fn same_cell_local_symbol_offsets_get_distinct_stable_keys() {
        let source = "const callTool = (globalThis as any).callTool;\n";
        let nb = serde_json::to_vec(&serde_json::json!({
            "nbformat": 4, "nbformat_minor": 5, "metadata": {},
            "cells": [
                {"cell_type":"code","id":"preview","source":[source],"metadata":{"spur":{"code_type":"javascript"}},"outputs":[],"execution_count":null},
                {"cell_type":"code","id":"controls","source":[source],"metadata":{"spur":{"code_type":"javascript"}},"outputs":[],"execution_count":null}
            ]
        }))
        .unwrap();
        let mut builder = FactBuilder::new(Path::new("/nb"));
        extract_notebook_file(&mut builder, Path::new("/nb/app.ipynb"), &nb).unwrap();
        let facts = builder.into_facts();
        let call_tool_constants: Vec<_> = facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Constant && node.label == "callTool")
            .collect();

        assert_eq!(call_tool_constants.len(), 2);
        assert_ne!(
            call_tool_constants[0].stable_key,
            call_tool_constants[1].stable_key
        );
    }

    #[test]
    fn notebook_metadata_emits_declared_dag_and_frontend_semantic_facts() {
        let nb = serde_json::to_vec(&serde_json::json!({
            "nbformat":4,"nbformat_minor":5,"metadata":{},
            "cells":[{
                "cell_type":"code","id":"a3f1","source":["pass\n"],"outputs":[],"execution_count":null,
                "metadata":{"spur":{"version":7,
                    "dag":{"produces":[{"port":"sales","repr":"arrow"}],"consumes":["raw"],
                            "source":{"kind":"csv","port":"raw"}},
                    "frontend":{"binds":["risk"],"emits":["horizon"]}}}
            },{
                "cell_type":"code","id":"b2c9","source":["pass\n"],"outputs":[],"execution_count":null,
                "metadata":{"spur":{"version":7,
                    "dag":{"consumes":["sales"]}}}
            }]
        }))
        .unwrap();
        let mut builder = FactBuilder::new(Path::new("/nb"));
        extract_notebook_file(&mut builder, Path::new("/nb/app.ipynb"), &nb).unwrap();
        let f = builder.into_facts();
        let has = |rel, label: &str, bm: Option<&str>| {
            f.edges.iter().any(|edge| {
                let target_label = edge
                    .target_node_id
                    .and_then(|target| f.nodes.iter().find(|node| node.node_id == target))
                    .map(|node| node.label.as_str())
                    .or(edge.target_label.as_deref());
                edge.relation == rel
                    && target_label == Some(label)
                    && edge.bind_method.as_deref() == bm
            })
        };
        assert!(has(
            RelationKind::Produces,
            "port://sales",
            Some("declared")
        ));
        assert!(has(RelationKind::Consumes, "port://raw", Some("declared")));
        assert!(has(
            RelationKind::Consumes,
            "port://sales",
            Some("declared")
        ));
        assert!(has(
            RelationKind::References,
            "ds://csv/raw",
            Some("declared")
        ));
        assert!(has(RelationKind::Binds, "port://risk", None));
        assert!(has(RelationKind::Emits, "port://horizon", None));
        assert_eq!(
            f.nodes
                .iter()
                .filter(|node| node.kind == NodeKind::Port && node.label == "port://sales")
                .count(),
            1
        );
    }

    #[test]
    fn notebook_source_emits_actual_put_get_dynamic_put_and_table_reference_facts() {
        let nb = serde_json::to_vec(&serde_json::json!({
            "nbformat":4,"nbformat_minor":5,"metadata":{"kernelspec":{"name":"python3"}},
            "cells":[
              {"cell_type":"code","id":"py","source":["spur.put(\"x\", df)\nspur.put(dyn_name, df)\nsales_orders(limit=10)\n"],
               "outputs":[],"execution_count":null,"metadata":{}},
              {"cell_type":"code","id":"js","source":["const v = spur.get(\"x\");\n"],
               "outputs":[],"execution_count":null,"metadata":{"spur":{"code_type":"javascript"}}}
            ]
        }))
        .unwrap();
        let mut builder = FactBuilder::new(Path::new("/nb"));
        extract_notebook_file(&mut builder, Path::new("/nb/app.ipynb"), &nb).unwrap();
        let f = builder.into_facts();
        let has = |rel, label: &str, bm: Option<&str>| {
            f.edges.iter().any(|edge| {
                let target_label = edge
                    .target_node_id
                    .and_then(|target| f.nodes.iter().find(|node| node.node_id == target))
                    .map(|node| node.label.as_str())
                    .or(edge.target_label.as_deref());
                edge.relation == rel
                    && target_label == Some(label)
                    && edge.bind_method.as_deref() == bm
            })
        };

        assert_eq!(
            f.nodes
                .iter()
                .filter(|node| node.kind == NodeKind::Port && node.label == "port://x")
                .count(),
            1
        );
        assert!(has(RelationKind::Produces, "port://x", Some("actual")));
        assert!(has(RelationKind::Consumes, "port://x", Some("actual")));
        assert!(has(
            RelationKind::References,
            "ds://sales_orders",
            Some("actual")
        ));
        assert!(has(RelationKind::References, "opaque_put", Some("actual")));
    }

    #[test]
    fn skips_unknown_language_cells() {
        let nb = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {},
            "cells": [
                {
                    "cell_type": "code",
                    "metadata": {"spur": {"code_type": "julia"}},
                    "source": ["function nope()\nend\n"]
                }
            ]
        });
        let bytes = serde_json::to_vec(&nb).expect("serialize notebook");

        let facts = run_notebook_extraction(Path::new("nb.ipynb"), &bytes)
            .expect("unknown language cell is skipped");

        assert!(facts
            .nodes
            .iter()
            .any(|node| { node.kind == NodeKind::File && node.label == "nb.ipynb" }));
        assert!(!has_symbol(&facts, "nope"));
    }

    #[test]
    fn malformed_json_returns_error() {
        let err = run_notebook_extraction(Path::new("nb.ipynb"), b"{bad json")
            .expect_err("malformed JSON should be an error");

        assert!(
            err.to_string().contains("parse .ipynb JSON"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn leading_rust_dep_recovers_function_at_original_multibyte_crlf_span() {
        let source = ":dep sha2 = \"0.10\" # café\r\nfn café() {}\r\n";
        let bytes =
            serde_json::to_vec(&json!({"metadata":{"kernelspec":{"name":"evcxr"}},"cells":[
                {"id":" rust ","cell_type":"code","source":source,"metadata":{}}
            ]}))
            .unwrap();
        let facts = run_notebook_extraction(Path::new("nb.ipynb"), &bytes).unwrap();
        let function = facts
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.label == "café")
            .expect("leading :dep must not hide the function");
        let span = facts
            .spans
            .iter()
            .find(|s| Some(s.span_id) == function.source_span_id)
            .unwrap();
        assert_eq!(span.start_byte as usize, source.find("fn café").unwrap());
        assert_eq!(
            &source[span.start_byte as usize..span.end_byte as usize],
            "fn café() {}"
        );
        assert_eq!(span.start_line, 2);
        let masked = source.as_bytes()[..source.find("fn café").unwrap()]
            .iter()
            .map(|b| {
                if matches!(*b, b'\r' | b'\n') {
                    *b as char
                } else {
                    ' '
                }
            })
            .collect::<String>()
            + "fn café() {}\r\n";
        let ordinary =
            serde_json::to_vec(&json!({"metadata":{"kernelspec":{"name":"rust"}},"cells":[
                {"id":"rust","cell_type":"code","source":masked,"metadata":{}}
            ]}))
            .unwrap();
        let before = run_notebook_extraction(Path::new("nb.ipynb"), &ordinary).unwrap();
        let old = before
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.label == "café")
            .unwrap();
        assert_eq!(function.stable_key, old.stable_key);
    }

    #[test]
    fn valid_graph_stable_keys_keep_legacy_cell_paths_and_source_decoding() {
        let source = "def helper():\n    return 1\n";
        let base = format!("cell-{}", blake3::hash(source.as_bytes()).to_hex());
        let bytes = serde_json::to_vec(&json!({"metadata":{"kernelspec":{"name":"python3"}},"cells":[
            {"id":" explicit ","cell_type":"code","source":["def helper():\n","    return 1\n"]},
            {"cell_type":"code","source":source},
            {"cell_type":"code","source":source}
        ]})).unwrap();
        let facts = run_notebook_extraction(Path::new("nb.ipynb"), &bytes).unwrap();
        for id in ["explicit".to_owned(), base.clone(), format!("{base}-2")] {
            let identity = format!("nb.ipynb#cell:{id}");
            let expected =
                crate::identity::stable_symbol_id_for(&identity, "helper", NodeKind::Function, 0);
            assert!(
                facts
                    .nodes
                    .iter()
                    .any(|n| n.label == "helper" && n.stable_key == expected),
                "legacy stable key {id}: {:?}",
                facts.nodes
            );
        }
    }

    #[test]
    fn graph_rejects_invalid_inventory_instead_of_clean_empty_or_duplicate_keys() {
        for nb in [
            json!({}),
            json!({"cells":[{"cell_type":"code","source":["fn f() {}",42]}]}),
            json!({"cells":[{"id":"a","cell_type":"raw","source":""},{"id":" a ","cell_type":"raw","source":""}]}),
        ] {
            assert!(
                run_notebook_extraction(Path::new("nb.ipynb"), &serde_json::to_vec(&nb).unwrap())
                    .is_err(),
                "{nb}"
            );
        }
    }

    #[test]
    fn raw_byte_strings_and_nested_comments_recover_function_through_graph() {
        for source in [
            "let text = br##\"\n:load not_a_command.rs\n\"##;\nfn f() {}\n",
            "/* outer /* inner */\n:load not_a_command.rs\n*/\nfn f() {}",
        ] {
            let bytes =
                serde_json::to_vec(&json!({"metadata":{"kernelspec":{"name":"evcxr"}},"cells":[
                    {"id":"rust","cell_type":"code","source":source}
                ]}))
                .unwrap();
            let facts = run_notebook_extraction(Path::new("nb.ipynb"), &bytes).unwrap();
            assert!(has_symbol(&facts, "f"), "{source}");
        }
    }

    #[test]
    fn graph_rejects_effective_id_collisions_across_explicit_and_anonymous_cells() {
        let id = format!("cell-{}", blake3::hash(b"fn f() {}\n").to_hex());
        let bytes = serde_json::to_vec(&json!({"cells":[
            {"cell_type":"code","source":"fn f() {}\n"},
            {"id":id,"cell_type":"raw","source":"other"}
        ]}))
        .unwrap();
        assert!(run_notebook_extraction(Path::new("nb.ipynb"), &bytes).is_err());
    }

    #[test]
    fn direct_notebook_facts_match_batch_extraction() {
        let nb = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"kernelspec": {"name": "python3"}},
            "cells": [
                {
                    "cell_type": "code",
                    "id": "a3f1",
                    "metadata": {},
                    "source": ["def load():\n", "    return 1\n"]
                },
                {
                    "cell_type": "code",
                    "id": "b2c9",
                    "metadata": {},
                    "source": ["x = load()\n"]
                }
            ]
        });
        let bytes = serde_json::to_vec(&nb).expect("serialize notebook");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nb.ipynb");
        std::fs::write(&path, &bytes).expect("write notebook");

        let batch = crate::extract::build_facts_for_paths(dir.path(), std::slice::from_ref(&path))
            .expect("batch notebook facts");
        let direct = crate::extract::extract_notebook_facts(dir.path(), &path, &bytes)
            .expect("direct notebook facts");

        assert_eq!(direct, batch);
    }

    fn run_notebook_extraction(path: &Path, bytes: &[u8]) -> anyhow::Result<GraphFacts> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(path);
        let mut builder = FactBuilder::new(dir.path());
        extract_notebook_file(&mut builder, &path, bytes)?;
        Ok(builder.into_facts())
    }

    fn has_symbol(facts: &GraphFacts, label: &str) -> bool {
        facts.nodes.iter().any(|node| {
            node.label == label && matches!(node.kind, NodeKind::Function | NodeKind::Method)
        })
    }

    fn has_section(facts: &GraphFacts, label: &str) -> bool {
        facts
            .nodes
            .iter()
            .any(|node| node.label == label && node.kind == NodeKind::Section)
    }

    fn all_non_file_nodes_reachable_from_file_via_contains(
        facts: &GraphFacts,
        file_label: &str,
    ) -> bool {
        let file_node = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == file_label)
            .expect("file node")
            .node_id;
        let mut reachable = HashSet::from([file_node]);
        loop {
            let before = reachable.len();
            for edge in facts
                .edges
                .iter()
                .filter(|edge| edge.relation == RelationKind::Contains)
            {
                if reachable.contains(&edge.source_node_id) {
                    if let Some(target) = edge.target_node_id {
                        reachable.insert(target);
                    }
                }
            }
            if reachable.len() == before {
                break;
            }
        }

        facts
            .nodes
            .iter()
            .filter(|node| node.kind != NodeKind::File)
            .all(|node| reachable.contains(&node.node_id))
    }

    fn all_symbols_contained_by_cells(facts: &GraphFacts) -> bool {
        let cell_nodes: HashSet<_> = facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Cell)
            .map(|node| node.node_id)
            .collect();

        facts
            .nodes
            .iter()
            .filter(|node| !matches!(node.kind, NodeKind::File | NodeKind::Cell))
            .all(|node| {
                facts.edges.iter().any(|edge| {
                    edge.relation == RelationKind::Contains
                        && edge.target_node_id == Some(node.node_id)
                        && cell_nodes.contains(&edge.source_node_id)
                })
            })
    }

    fn file_directly_contains_symbols(facts: &GraphFacts, file_label: &str) -> bool {
        let file_node = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == file_label)
            .expect("file node")
            .node_id;
        let symbol_nodes: HashSet<_> = facts
            .nodes
            .iter()
            .filter(|node| !matches!(node.kind, NodeKind::File | NodeKind::Cell))
            .map(|node| node.node_id)
            .collect();

        facts.edges.iter().any(|edge| {
            edge.relation == RelationKind::Contains
                && edge.source_node_id == file_node
                && edge
                    .target_node_id
                    .is_some_and(|target| symbol_nodes.contains(&target))
        })
    }
}
