//! Direct Rust AST evidence. No expansion, cfg evaluation or Calls resolution.

use std::collections::BTreeMap;

use tree_sitter::Node;

use super::languages::DefinitionBinding;
use super::tree_sitter::{CaptureHit, FactBuilder};
use crate::{
    git_blob_oid, macro_invocation_site_id, MacroAttributeEvidence, MacroDefinitionLocation,
    MacroEvidenceSource, MacroExecutionPhase, MacroInvocationEvidence, MacroInvocationEvidenceSet,
    MacroResolutionEvidence, MacroScopeEvidence, MacroScopeKind, MacroScopeRef, MacroSinkFamily,
    MacroTestContext, NodeId, MACRO_INVOCATION_SCHEMA_VERSION,
};

pub(crate) fn emit_macro_invocations(
    builder: &mut FactBuilder<'_>,
    file_path: &str,
    file_node: NodeId,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
    captures: &[CaptureHit<'_>],
) -> anyhow::Result<()> {
    let source_oid = git_blob_oid(source.as_bytes());
    let mut scopes = BTreeMap::new();
    let mut rows = BTreeMap::new();
    for capture in captures.iter().filter(|c| c.name == "macro_invocation") {
        let node = capture.node;
        if node.has_error() || !valid_range(node, source) {
            continue;
        }
        let ancestors: Vec<_> =
            std::iter::successors(node.parent(), |node| node.parent()).collect();
        if ancestors.iter().any(|node| {
            matches!(
                node.kind(),
                "macro_definition" | "macro_invocation" | "token_tree"
            )
        }) {
            continue;
        }
        let Some(path) = captures
            .iter()
            .find(|c| {
                c.name == "macro_invocation.path"
                    && c.match_index == capture.match_index
                    && c.pattern_index == capture.pattern_index
            })
            .map(|c| c.node)
        else {
            continue;
        };
        if !valid_range(path, source) {
            continue;
        }
        let normalized_macro_path = normalized_path(path, source);
        if normalized_macro_path.is_empty() {
            continue;
        }
        let syntax_scopes: Vec<_> = ancestors
            .iter()
            .filter_map(|&node| {
                let kind = scope_kind(node)?;
                let evidence = scope_evidence(
                    builder,
                    file_path,
                    &source_oid,
                    source,
                    definitions,
                    file_node,
                    node,
                    kind,
                );
                scopes.insert(evidence.scope.stable_symbol_id.clone(), evidence.clone());
                Some((node, evidence))
            })
            .collect();
        let Some((enclosing_node, enclosing)) = syntax_scopes.first() else {
            continue;
        };
        let mut attributes = BTreeMap::new();
        for current in std::iter::once(node).chain(ancestors.iter().copied()) {
            // Outer attributes are siblings of their item/expression in the Rust AST.
            let Some((_, owner)) = syntax_scopes
                .iter()
                .find(|(scope, _)| contains(*scope, current))
            else {
                continue;
            };
            let mut previous = current.prev_named_sibling();
            while let Some(attribute) = previous {
                match attribute.kind() {
                    "attribute_item" => {
                        record_attribute(&mut attributes, attribute, &owner.scope, source)
                    }
                    "line_comment" | "block_comment" => {}
                    _ => break,
                }
                previous = attribute.prev_named_sibling();
            }
            if matches!(current.kind(), "source_file" | "declaration_list" | "block") {
                let mut cursor = current.walk();
                for attribute in current
                    .named_children(&mut cursor)
                    .filter(|n| n.kind() == "inner_attribute_item")
                {
                    record_attribute(&mut attributes, attribute, &owner.scope, source);
                }
            }
        }
        let attributes: Vec<_> = attributes.into_values().collect();
        let test_context = test_context(&attributes);
        let name = normalized_macro_path.rsplit("::").next().unwrap_or("");
        let sink_family_candidate = sink_family(name);
        let byte_range = [node.start_byte(), node.end_byte()];
        let line_range = line_range(source, byte_range);
        let resolution = if normalized_macro_path.contains("::") {
            MacroResolutionEvidence::ExplicitPathCandidate
        } else if let Some(definition) = lexical_definition(&ancestors, node, name, source) {
            MacroResolutionEvidence::LocalDefinitionCandidate {
                definition: MacroDefinitionLocation {
                    file_path: file_path.into(),
                    source_oid: source_oid.clone(),
                    byte_range: [definition.start_byte(), definition.end_byte()],
                    line_range: line_range_for_node(source, definition),
                },
            }
        } else {
            MacroResolutionEvidence::Unresolved
        };
        let execution_phase = match enclosing.scope.kind {
            _ if ancestors.iter().any(Node::is_error) || enclosing_node.has_error() => {
                MacroExecutionPhase::Unknown
            }
            MacroScopeKind::Function
                if enclosing_node
                    .child_by_field_name("body")
                    .is_some_and(|body| contains(body, node)) =>
            {
                MacroExecutionPhase::FunctionBodySyntax
            }
            MacroScopeKind::Const | MacroScopeKind::Static => {
                MacroExecutionPhase::ConstContextSyntax
            }
            MacroScopeKind::Unknown => MacroExecutionPhase::Unknown,
            _ => MacroExecutionPhase::ItemSyntax,
        };
        let row = MacroInvocationEvidence {
            site_id: macro_invocation_site_id(file_path, &source_oid, byte_range, line_range),
            file_path: file_path.into(),
            source_oid: source_oid.clone(),
            caller_stable_symbol_id: (enclosing.scope.kind == MacroScopeKind::Function)
                .then(|| enclosing.scope.stable_symbol_id.clone()),
            enclosing: enclosing.scope.clone(),
            raw_macro_path: source[path.byte_range()].into(),
            normalized_macro_path,
            byte_range,
            line_range,
            attributes,
            test_context,
            debug_assertions_candidate: sink_family_candidate
                == Some(MacroSinkFamily::DebugAssertion),
            execution_phase,
            resolution,
            sink_family_candidate,
        };
        rows.insert(byte_range, row);
    }
    // GraphFacts has rows only: the graph snapshot is bound by the artifact stage.
    // This temporary set validates same-file source/scope evidence against the
    // exact file snapshot; it is never persisted as an artifact evidence set.
    let snapshot = format!("file-source:{file_path}:{source_oid}");
    let set = MacroInvocationEvidenceSet {
        schema_version: MACRO_INVOCATION_SCHEMA_VERSION.into(),
        extractor_version: "rust-direct-macro-invocation-v1".into(),
        graph_content_hash: snapshot.clone(),
        rows: rows.into_values().collect(),
    };
    set.validate(
        &snapshot,
        &[MacroEvidenceSource {
            file_path,
            bytes: source.as_bytes(),
        }],
        &scopes.into_values().collect::<Vec<_>>(),
    )?;
    builder.record_macro_invocations(set.rows);
    Ok(())
}

fn valid_range(node: Node<'_>, source: &str) -> bool {
    !node.is_missing()
        && node.start_byte() < node.end_byte()
        && node.end_byte() <= source.len()
        && source.is_char_boundary(node.start_byte())
        && source.is_char_boundary(node.end_byte())
}

fn contains(outer: Node<'_>, inner: Node<'_>) -> bool {
    outer.start_byte() <= inner.start_byte() && inner.end_byte() <= outer.end_byte()
}

fn line_range(source: &str, [start, end]: [usize; 2]) -> [usize; 2] {
    let line = |offset| {
        1 + source.as_bytes()[..offset]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    };
    [line(start), line(end - 1)]
}

fn line_range_for_node(source: &str, node: Node<'_>) -> [usize; 2] {
    line_range(source, [node.start_byte(), node.end_byte()])
}

fn normalized_path(node: Node<'_>, source: &str) -> String {
    if matches!(node.kind(), "line_comment" | "block_comment") {
        return String::new();
    }
    if node.child_count() == 0 {
        return source[node.byte_range()].trim_start_matches("r#").into();
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .map(|child| normalized_path(child, source))
        .collect()
}

fn scope_kind(node: Node<'_>) -> Option<MacroScopeKind> {
    // A recoverable parse may expose ERROR as the root. File provenance still
    // comes from the actual file node/bytes; phase remains Unknown in that case.
    if node.parent().is_none() {
        return Some(MacroScopeKind::File);
    }
    Some(match node.kind() {
        "source_file" => MacroScopeKind::File,
        "mod_item" => MacroScopeKind::Module,
        "function_item" | "function_signature_item" => MacroScopeKind::Function,
        "const_item" | "const_block" => MacroScopeKind::Const,
        "static_item" => MacroScopeKind::Static,
        "type_item" | "associated_type" | "array_type" | "generic_type" | "tuple_type"
        | "reference_type" | "pointer_type" | "function_type" | "bounded_type"
        | "impl_trait_type" | "field_declaration" | "struct_item" | "enum_item" | "union_item"
        | "trait_item" => MacroScopeKind::Type,
        "impl_item" => MacroScopeKind::Impl,
        _ => return None,
    })
}

#[expect(clippy::too_many_arguments)]
fn scope_evidence(
    builder: &FactBuilder<'_>,
    file_path: &str,
    source_oid: &str,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
    file_node: NodeId,
    node: Node<'_>,
    mut kind: MacroScopeKind,
) -> MacroScopeEvidence {
    let binding = if kind == MacroScopeKind::File {
        Some(file_node)
    } else {
        definitions
            .iter()
            .find(|d| d.node == node)
            .map(|d| d.node_id)
    };
    let stable_id = binding.and_then(|id| builder.node_stable_key(id));
    if kind == MacroScopeKind::Function && stable_id.is_none() {
        kind = MacroScopeKind::Unknown;
    }
    let byte_range = [node.start_byte(), node.end_byte()];
    let line_range = line_range(source, byte_range);
    let stable_symbol_id = stable_id.map(str::to_owned).unwrap_or_else(|| {
        // Explicit evidence-only identity for syntax without a graph definition
        // (e.g. an array type or const block). Never invent a graph function.
        let kind_tag = match kind {
            MacroScopeKind::File => "file",
            MacroScopeKind::Module => "module",
            MacroScopeKind::Function => "function",
            MacroScopeKind::Const => "const",
            MacroScopeKind::Static => "static",
            MacroScopeKind::Type => "type",
            MacroScopeKind::Impl => "impl",
            MacroScopeKind::Unknown => "unknown",
        };
        format!(
            "macro-scope-v1:{kind_tag}:{}",
            macro_invocation_site_id(file_path, source_oid, byte_range, line_range)
        )
    });
    MacroScopeEvidence {
        scope: MacroScopeRef {
            stable_symbol_id,
            kind,
        },
        file_path: file_path.into(),
        source_oid: source_oid.into(),
        byte_range,
        line_range,
    }
}

fn record_attribute(
    attributes: &mut BTreeMap<[usize; 2], MacroAttributeEvidence>,
    node: Node<'_>,
    owner: &MacroScopeRef,
    source: &str,
) {
    if valid_range(node, source) && !node.has_error() {
        let byte_range = [node.start_byte(), node.end_byte()];
        attributes.insert(
            byte_range,
            MacroAttributeEvidence {
                raw_text: source[node.byte_range()].into(),
                owner: owner.clone(),
                byte_range,
                line_range: line_range(source, byte_range),
            },
        );
    }
}

fn test_context(attributes: &[MacroAttributeEvidence]) -> MacroTestContext {
    let mut test = false;
    for attribute in attributes {
        let raw = attribute
            .raw_text
            .trim_start_matches('#')
            .trim_start_matches('!')
            .trim_start_matches('[')
            .trim_start();
        let path = raw.split(['(', '=', ']']).next().unwrap_or("").trim();
        if matches!(path, "cfg" | "cfg_attr") {
            return MacroTestContext::Conditional;
        }
        test |= path
            .rsplit("::")
            .next()
            .is_some_and(|name| name.trim() == "test");
    }
    if test {
        MacroTestContext::TestAttributeCandidate
    } else {
        MacroTestContext::Unspecified
    }
}

fn lexical_definition<'tree>(
    ancestors: &[Node<'tree>],
    site: Node<'tree>,
    name: &str,
    source: &str,
) -> Option<Node<'tree>> {
    for &scope in ancestors {
        if !matches!(scope.kind(), "source_file" | "declaration_list" | "block") {
            continue;
        }
        let mut cursor = scope.walk();
        let candidate = scope
            .named_children(&mut cursor)
            .filter(|child| {
                child.kind() == "macro_definition"
                    && child.end_byte() <= site.start_byte()
                    && !child.has_error()
                    && valid_range(*child, source)
                    && child
                        .child_by_field_name("name")
                        .is_some_and(|id| normalized_path(id, source) == name)
            })
            .last();
        if candidate.is_some() {
            return candidate;
        }
    }
    None
}

fn sink_family(name: &str) -> Option<MacroSinkFamily> {
    Some(match name {
        "panic" | "unreachable" | "todo" | "unimplemented" => MacroSinkFamily::Panic,
        "assert" | "assert_eq" | "assert_ne" => MacroSinkFamily::Assertion,
        "debug_assert" | "debug_assert_eq" | "debug_assert_ne" => MacroSinkFamily::DebugAssertion,
        _ => return None,
    })
}
