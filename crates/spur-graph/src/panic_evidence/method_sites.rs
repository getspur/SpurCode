//! Syntax collection only; deliberately independent of deduplicated Calls edges.

use std::collections::HashMap;

use anyhow::Context as _;
use tree_sitter::{Node, Parser};

use super::{
    CompilerTargetState, MethodCallLexicalOwner, MethodCallLexicalPhase, MethodCallSiteEvidence,
};
use crate::{GraphIndexArtifact, GraphSymbolArtifact};

// Preserve candidate order and count matching IDs across all graph symbols,
// including non-function symbols outside this source file.
pub(super) struct CallerSymbols<'a> {
    candidates: Vec<&'a GraphSymbolArtifact>,
    id_counts: HashMap<&'a str, usize>,
}

impl<'a> CallerSymbols<'a> {
    pub(super) fn new(graph: &'a GraphIndexArtifact, file_path: &str) -> Self {
        let candidates: Vec<_> = graph
            .symbols
            .iter()
            .filter(|s| {
                s.file_path == file_path
                    && matches!(s.symbol_kind.as_str(), "function" | "method")
                    && !s.stable_symbol_id.is_empty()
            })
            .collect();
        let mut id_counts: HashMap<_, usize> = candidates
            .iter()
            .map(|s| (s.stable_symbol_id.as_str(), 0))
            .collect();
        for symbol in &graph.symbols {
            if let Some(count) = id_counts.get_mut(symbol.stable_symbol_id.as_str()) {
                *count += 1;
            }
        }
        Self {
            candidates,
            id_counts,
        }
    }

    pub(super) fn find_unique(
        &self,
        name: &str,
        byte_range: [usize; 2],
        line_range: [usize; 2],
    ) -> Option<&'a GraphSymbolArtifact> {
        let mut candidates = self.candidates.iter().copied().filter(|s| {
            s.entity_name == name && s.byte_range == byte_range && s.line_range == line_range
        });
        let caller = candidates.next()?;
        (candidates.next().is_none()
            && self.id_counts.get(caller.stable_symbol_id.as_str()) == Some(&1))
        .then_some(caller)
    }
}

pub(super) fn collect_in_source(
    graph: &GraphIndexArtifact,
    file_path: &str,
    source_oid: &str,
    source: &str,
) -> anyhow::Result<(Vec<MethodCallSiteEvidence>, bool)> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser
        .parse(source, None)
        .context("Rust method-site parse unavailable")?;
    let root = tree.root_node();
    let mut pending = vec![root];
    let mut sites = Vec::new();
    let mut caller_symbols = None;
    while let Some(node) = pending.pop() {
        // A token tree is intentionally opaque, even if its bytes resemble Rust.
        if matches!(
            node.kind(),
            "macro_definition" | "macro_invocation" | "token_tree"
        ) {
            continue;
        }
        if let Some((method, receiver)) = method_parts(node, source) {
            let caller_symbols =
                caller_symbols.get_or_insert_with(|| CallerSymbols::new(graph, file_path));
            let (lexical_phase, caller_stable_symbol_id, lexical_owners) =
                provenance(caller_symbols, source, node);
            let byte_range = range(node);
            let method_byte_range = range(method);
            let receiver_byte_range = range(receiver);
            sites.push(MethodCallSiteEvidence {
                site_id: site_id(
                    file_path,
                    source_oid,
                    byte_range,
                    method_byte_range,
                    receiver_byte_range,
                ),
                file_path: file_path.into(),
                source_oid: source_oid.into(),
                byte_range,
                method_byte_range,
                receiver_byte_range,
                line_range: lines(source, byte_range),
                call_text: source[node.byte_range()].into(),
                method_name: source[method.byte_range()].into(),
                receiver_text: source[receiver.byte_range()].into(),
                caller_stable_symbol_id,
                lexical_phase,
                lexical_owners,
                compiler_target: CompilerTargetState::Unknown,
            });
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok((sites, root.has_error()))
}

fn method_parts<'tree>(node: Node<'tree>, source: &str) -> Option<(Node<'tree>, Node<'tree>)> {
    if node.kind() != "call_expression" || node.has_error() || !intact(node, source) {
        return None;
    }
    let mut function = node.child_by_field_name("function")?;
    if function.kind() == "generic_function" {
        function = function.child_by_field_name("function")?;
    }
    if function.kind() != "field_expression" {
        return None;
    }
    let method = function.child_by_field_name("field")?;
    let receiver = function.child_by_field_name("value")?;
    (method.kind() == "field_identifier" && intact(method, source) && intact(receiver, source))
        .then_some((method, receiver))
}

fn intact(node: Node<'_>, source: &str) -> bool {
    !node.is_missing()
        && node.start_byte() < node.end_byte()
        && source.get(node.byte_range()).is_some()
}

fn range(node: Node<'_>) -> [usize; 2] {
    [node.start_byte(), node.end_byte()]
}

fn lines(source: &str, [start, end]: [usize; 2]) -> [usize; 2] {
    let line = |offset| {
        1 + source.as_bytes()[..offset]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    };
    [line(start), line(end - 1)]
}

fn is_owner(node: Node<'_>) -> bool {
    node.kind().ends_with("_item")
        || node.kind().ends_with("_type")
        || matches!(
            node.kind(),
            "source_file"
                | "closure_expression"
                | "const_block"
                | "async_block"
                | "type_arguments"
                | "field_declaration"
                | "ERROR"
        )
}

fn provenance(
    caller_symbols: &CallerSymbols<'_>,
    source: &str,
    site: Node<'_>,
) -> (
    MethodCallLexicalPhase,
    Option<String>,
    Vec<MethodCallLexicalOwner>,
) {
    use MethodCallLexicalPhase::*;
    let ancestors: Vec<_> = std::iter::successors(site.parent(), |n| n.parent()).collect();
    let owners = ancestors
        .iter()
        .filter(|&&n| is_owner(n) && intact(n, source))
        .map(|&n| MethodCallLexicalOwner {
            syntax_kind: n.kind().into(),
            byte_range: range(n),
            line_range: lines(source, range(n)),
        })
        .collect();
    if ancestors.iter().any(|n| n.is_error() || n.is_missing()) {
        return (Unknown, None, owners);
    }
    if let Some(&owner) = ancestors.iter().find(|&&n| is_owner(n)) {
        // Malformed enclosing syntax cannot establish a runtime caller.
        if owner.kind() != "source_file" && owner.has_error() {
            return (Unknown, None, owners);
        }
        match owner.kind() {
            "closure_expression" => return (ClosureBodySyntax, None, owners),
            "const_item" | "static_item" | "const_block" => {
                return (ConstContextSyntax, None, owners)
            }
            "function_item" => {
                let in_body = owner.child_by_field_name("body").is_some_and(|body| {
                    body.start_byte() <= site.start_byte() && site.end_byte() <= body.end_byte()
                });
                if !in_body {
                    return (ItemSyntax, None, owners);
                }
                let name = owner
                    .child_by_field_name("name")
                    .and_then(|n| source.get(n.byte_range()));
                if let Some(caller) = name.and_then(|name| {
                    caller_symbols.find_unique(name, range(owner), lines(source, range(owner)))
                }) {
                    return (
                        FunctionBodySyntax,
                        Some(caller.stable_symbol_id.clone()),
                        owners,
                    );
                }
                return (Unknown, None, owners);
            }
            "async_block" | "ERROR" => return (Unknown, None, owners),
            _ => return (ItemSyntax, None, owners),
        }
    }
    (Unknown, None, owners)
}

fn site_id(
    path: &str,
    oid: &str,
    call: [usize; 2],
    method: [usize; 2],
    receiver: [usize; 2],
) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"spur:panic-evidence:direct-method-site:v1\0");
    for value in [path, oid] {
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    for offset in call.into_iter().chain(method).chain(receiver) {
        hash.update(&(offset as u64).to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}
