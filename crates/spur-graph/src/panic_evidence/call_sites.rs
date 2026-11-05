//! Per-invocation syntax evidence. Ownership never implies execution.

use std::collections::BTreeMap;

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

use super::{collect_method_call_sites, MethodCallSourceScope, PanicEvidenceSource};
use crate::GraphIndexArtifact;

pub const RUST_CALL_SITE_SCHEMA_VERSION: &str = "panic-rust-call-sites-v1";
pub const RUST_CALL_SITE_EXTRACTOR_VERSION: &str = "rust-direct-call-bodies-v1";

/// Syntactic evaluation context, not a claim that the body was entered.
/// Async functions/blocks execute when polled. Ordinary closures execute when
/// invoked; invoking an async closure creates a future whose body awaits polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustBodyKind {
    Function,
    NestedFunction,
    AsyncFunction,
    NestedAsyncFunction,
    Closure,
    AsyncClosure,
    AsyncBlock,
    ConstInitializer,
    StaticInitializer,
    ConstBlock,
    /// Type/generic constants, array-repeat counts and other constant expressions.
    ConstExpression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustBodyEvidence {
    /// Snapshot-bound identity; no cross-edit stability is promised.
    pub body_id: String,
    pub source_oid: String,
    pub file_path: String,
    /// Half-open UTF-8 range of the body block or expression, excluding a
    /// function signature / closure parameters / async creation prefix.
    pub byte_range: [usize; 2],
    pub kind: RustBodyKind,
    /// Exact, unambiguous graph definition for a named function, when available.
    /// This is definition provenance, never an execution edge.
    pub definition_stable_symbol_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustCallSiteEvidence {
    pub site_id: String,
    pub source_oid: String,
    pub file_path: String,
    pub byte_range: [usize; 2],
    pub call_text: String,
    /// The innermost syntactic body; does not establish execution or a callee.
    pub body_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustCallSourceBinding {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub byte_length: usize,
    pub body_count: usize,
    pub site_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustCallSiteEvidenceSet {
    pub schema_version: String,
    pub extractor_version: String,
    pub graph_content_hash: String,
    pub source_scope: MethodCallSourceScope,
    pub sources: Vec<RustCallSourceBinding>,
    pub bodies: Vec<RustBodyEvidence>,
    pub sites: Vec<RustCallSiteEvidence>,
}

impl RustCallSiteEvidenceSet {
    /// Recollect every field from the trusted graph and exact input bytes.
    /// Rejects omissions, duplicates, reordering and forged same-file ownership.
    pub fn validate(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[PanicEvidenceSource<'_>],
    ) -> anyhow::Result<()> {
        ensure!(
            self == &collect_rust_call_sites(graph, sources)?,
            "Rust call/body evidence differs from canonical source-bound collection"
        );
        Ok(())
    }
}

/// Collect direct identifier/path and method call syntax in supplied physical
/// Rust files, including deferred bodies and constant evaluation contexts.
/// Macro token trees are opaque. No expansion, target resolution, execution or
/// whole-workspace completeness is established. Parse errors and unevaluated
/// cfg/cfg_attr conditions are rejected rather than certifying completeness.
pub fn collect_rust_call_sites(
    graph: &GraphIndexArtifact,
    sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<RustCallSiteEvidenceSet> {
    let validated = collect_method_call_sites(graph, sources)?;
    let mut set = RustCallSiteEvidenceSet {
        schema_version: RUST_CALL_SITE_SCHEMA_VERSION.into(),
        extractor_version: RUST_CALL_SITE_EXTRACTOR_VERSION.into(),
        graph_content_hash: validated.graph_content_hash,
        source_scope: validated.source_scope,
        sources: Vec::new(),
        bodies: Vec::new(),
        sites: Vec::new(),
    };
    for binding in validated.sources {
        ensure!(
            !binding.has_parse_errors,
            "parse uncertainty prevents complete Rust call/body evidence"
        );
        let source = sources
            .iter()
            .find(|s| s.file_path == binding.file_path)
            .context("validated source missing")?;
        let text = std::str::from_utf8(source.bytes)?;
        let (bodies, sites) = collect_source(graph, &binding.file_path, &binding.source_oid, text)?;
        set.sources.push(RustCallSourceBinding {
            file_path: binding.file_path,
            stable_file_id: binding.stable_file_id,
            source_oid: binding.source_oid,
            byte_length: binding.byte_length,
            body_count: bodies.len(),
            site_count: sites.len(),
        });
        set.bodies.extend(bodies);
        set.sites.extend(sites);
    }
    set.bodies.sort_by(|a, b| {
        (&a.file_path, a.byte_range, &a.body_id).cmp(&(&b.file_path, b.byte_range, &b.body_id))
    });
    set.sites
        .sort_by(|a, b| (&a.file_path, a.byte_range).cmp(&(&b.file_path, b.byte_range)));
    Ok(set)
}

fn collect_source(
    graph: &GraphIndexArtifact,
    path: &str,
    oid: &str,
    text: &str,
) -> anyhow::Result<(Vec<RustBodyEvidence>, Vec<RustCallSiteEvidence>)> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser
        .parse(text, None)
        .context("Rust call/body parse unavailable")?;
    ensure!(
        !tree.root_node().has_error(),
        "Rust call/body parse uncertainty"
    );
    let mut pending = vec![tree.root_node()];
    let mut owners = BTreeMap::new();
    let mut bodies = BTreeMap::new();
    let mut calls = Vec::new();
    while let Some(node) = pending.pop() {
        if matches!(
            node.kind(),
            "macro_definition" | "macro_invocation" | "token_tree"
        ) {
            continue;
        }
        if node.kind() == "attribute" {
            let name = node.named_child(0).map(|n| {
                let name = &text[n.byte_range()];
                name.strip_prefix("r#").unwrap_or(name)
            });
            ensure!(
                !matches!(name, Some("cfg" | "cfg_attr")),
                "unevaluated cfg/cfg_attr prevents complete Rust call/body evidence"
            );
        }
        if let Some((root, kind)) = explicit_body(node) {
            let body = body_evidence(path, oid, root, kind, definition(graph, path, text, node));
            owners.insert(node.id(), (root, body.body_id.clone()));
            bodies.insert(body.body_id.clone(), body);
        }
        if direct_call(node) {
            calls.push(node);
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    let mut sites = Vec::new();
    for call in calls {
        let body_id = call_body(call, &owners).unwrap_or_else(|root| {
            let body = body_evidence(path, oid, root, RustBodyKind::ConstExpression, None);
            let id = body.body_id.clone();
            bodies.entry(id.clone()).or_insert(body);
            id
        });
        sites.push(RustCallSiteEvidence {
            site_id: snapshot_id("call", path, oid, range(call), ""),
            source_oid: oid.into(),
            file_path: path.into(),
            byte_range: range(call),
            call_text: text[call.byte_range()].into(),
            body_id,
        });
    }
    Ok((bodies.into_values().collect(), sites))
}

fn explicit_body(node: Node<'_>) -> Option<(Node<'_>, RustBodyKind)> {
    use RustBodyKind::*;
    let (field, kind) = match node.kind() {
        "function_item" => {
            let nested = std::iter::successors(node.parent(), |n| n.parent()).any(|n| {
                matches!(
                    n.kind(),
                    "function_item" | "closure_expression" | "async_block"
                )
            });
            let mut cursor = node.walk();
            let asynchronous = node.named_children(&mut cursor).any(|n| {
                let mut modifiers = n.walk();
                n.kind() == "function_modifiers"
                    && n.children(&mut modifiers).any(|m| m.kind() == "async")
            });
            (
                "body",
                match (nested, asynchronous) {
                    (true, true) => NestedAsyncFunction,
                    (false, true) => AsyncFunction,
                    (true, false) => NestedFunction,
                    (false, false) => Function,
                },
            )
        }
        "closure_expression" => {
            let mut cursor = node.walk();
            let asynchronous = node.children(&mut cursor).any(|n| n.kind() == "async");
            ("body", if asynchronous { AsyncClosure } else { Closure })
        }
        "async_block" => ("body", AsyncBlock),
        "const_block" => ("body", ConstBlock),
        "const_item" => ("value", ConstInitializer),
        "static_item" => ("value", StaticInitializer),
        _ => return None,
    };
    // The Rust grammar exposes async/const block children without a body field.
    let root = node.child_by_field_name(field).or_else(|| {
        matches!(node.kind(), "async_block" | "const_block")
            .then(|| {
                let mut cursor = node.walk();
                let block = node
                    .named_children(&mut cursor)
                    .find(|n| n.kind() == "block");
                block
            })
            .flatten()
    })?;
    Some((root, kind))
}

fn direct_call(node: Node<'_>) -> bool {
    if node.kind() != "call_expression" {
        return false;
    }
    let Some(mut target) = node.child_by_field_name("function") else {
        return false;
    };
    if target.kind() == "generic_function" {
        let Some(inner) = target.child_by_field_name("function") else {
            return false;
        };
        target = inner;
    }
    matches!(target.kind(), "identifier" | "scoped_identifier")
        || (target.kind() == "field_expression"
            && target
                .child_by_field_name("field")
                .is_some_and(|f| f.kind() == "field_identifier"))
}

/// Find ownership by syntax ancestry, respecting item/type constant barriers.
/// Pure byte containment would incorrectly assign a local array length or a
/// nested function signature's constants to the surrounding runtime function.
fn call_body<'tree>(
    call: Node<'tree>,
    owners: &BTreeMap<usize, (Node<'tree>, String)>,
) -> Result<String, Node<'tree>> {
    let mut expression = call;
    for ancestor in std::iter::successors(call.parent(), |n| n.parent()) {
        if let Some((root, id)) = owners.get(&ancestor.id()) {
            if root.start_byte() <= call.start_byte() && call.end_byte() <= root.end_byte() {
                return Ok(id.clone());
            }
            return Err(expression);
        }
        // A repeat's element uses the surrounding context, but its length is
        // evaluated as a separate constant expression, even inside a closure
        // or async body. Preserve the entire length expression as its root.
        if ancestor.kind() == "array_expression" {
            if let Some(length) = ancestor.child_by_field_name("length") {
                if length.start_byte() <= call.start_byte() && call.end_byte() <= length.end_byte()
                {
                    return Err(length);
                }
            }
        }
        if ancestor.kind().ends_with("_item")
            || ancestor.kind().ends_with("_type")
            || matches!(
                ancestor.kind(),
                "type_arguments" | "enum_variant" | "const_parameter" | "field_declaration"
            )
        {
            return Err(expression);
        }
        if ancestor.kind().ends_with("_expression") || ancestor.kind() == "block" {
            expression = ancestor;
        }
    }
    Err(expression)
}

fn body_evidence(
    path: &str,
    oid: &str,
    root: Node<'_>,
    kind: RustBodyKind,
    definition_stable_symbol_id: Option<String>,
) -> RustBodyEvidence {
    RustBodyEvidence {
        body_id: snapshot_id("body", path, oid, range(root), &format!("{kind:?}")),
        source_oid: oid.into(),
        file_path: path.into(),
        byte_range: range(root),
        kind,
        definition_stable_symbol_id,
    }
}

fn definition(
    graph: &GraphIndexArtifact,
    path: &str,
    text: &str,
    node: Node<'_>,
) -> Option<String> {
    if node.kind() != "function_item" {
        return None;
    }
    let name = &text[node.child_by_field_name("name")?.byte_range()];
    let line = |offset| {
        1 + text.as_bytes()[..offset]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    };
    let candidates: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| {
            s.file_path == path
                && s.entity_name == name
                && matches!(s.symbol_kind.as_str(), "function" | "method")
                && s.byte_range == range(node)
                && s.line_range == [line(node.start_byte()), line(node.end_byte() - 1)]
                && !s.stable_symbol_id.is_empty()
        })
        .collect();
    let [symbol] = candidates.as_slice() else {
        return None;
    };
    (graph
        .symbols
        .iter()
        .filter(|s| s.stable_symbol_id == symbol.stable_symbol_id)
        .count()
        == 1)
        .then(|| symbol.stable_symbol_id.clone())
}

fn range(node: Node<'_>) -> [usize; 2] {
    [node.start_byte(), node.end_byte()]
}

pub(crate) fn snapshot_id(
    domain: &str,
    path: &str,
    oid: &str,
    range: [usize; 2],
    kind: &str,
) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"spur:panic-evidence:rust-call-bodies:v1\0");
    for value in [domain, path, oid, kind] {
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    for offset in range {
        hash.update(&(offset as u64).to_le_bytes());
    }
    hash.finalize().to_hex().to_string()
}
