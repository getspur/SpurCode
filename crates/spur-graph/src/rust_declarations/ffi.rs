//! Raw FFI source syntax; no effective ABI, linkage, cfg or executable body.

use super::*;
use crate::{stable_symbol_id_for, GraphSymbolArtifact, NodeKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RustDeclarationAbiSyntax {
    Omitted,
    Explicit { literal: RustDeclarationText },
    Inherited { extern_block_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDeclarationBody {
    AbsentForeignDeclaration,
    NonExecutableContainer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustForeignFunctionEvidence {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub stable_symbol_id: String,
    pub entity_name: String,
    pub qualified_name: String,
    pub structural_owner: RustDeclarationOwner,
    pub lexical_owner: RustDeclarationOwner,
    pub full_range: [usize; 2],
    pub line_range: [usize; 2],
    pub core_range: [usize; 2],
    pub core_line_range: [usize; 2],
    pub qualifiers: Vec<RustDeclarationText>,
    pub visibility: Option<RustDeclarationText>,
    pub attributes: Vec<RustDeclarationText>,
    pub abi: RustDeclarationAbiSyntax,
    pub body: RustDeclarationBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustExternBlockEvidence {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub stable_symbol_id: String,
    pub entity_name: String,
    pub qualified_name: String,
    pub structural_owner: RustDeclarationOwner,
    pub lexical_owner: RustDeclarationOwner,
    pub full_range: [usize; 2],
    pub line_range: [usize; 2],
    pub core_range: [usize; 2],
    pub core_line_range: [usize; 2],
    pub qualifiers: Vec<RustDeclarationText>,
    pub visibility: Option<RustDeclarationText>,
    pub attributes: Vec<RustDeclarationText>,
    pub abi: RustDeclarationAbiSyntax,
    pub body: RustDeclarationBody,
}

struct Syntax {
    full: tree_sitter::Range,
    attributes: Vec<RustDeclarationText>,
    qualifiers: Vec<RustDeclarationText>,
    visibility: Option<RustDeclarationText>,
}

fn text(node: Node<'_>, bytes: &[u8]) -> anyhow::Result<RustDeclarationText> {
    Ok(RustDeclarationText {
        text: std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])?.into(),
        byte_range: [node.start_byte(), node.end_byte()],
        line_range: [node.start_position().row + 1, node.end_position().row + 1],
    })
}

fn qualifier_tokens(
    node: Node<'_>,
    bytes: &[u8],
    out: &mut Vec<RustDeclarationText>,
) -> anyhow::Result<()> {
    if matches!(
        node.kind(),
        "unsafe" | "safe" | "extern" | "const" | "async"
    ) {
        out.push(text(node, bytes)?);
    } else if matches!(node.kind(), "function_modifiers" | "extern_modifier") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            qualifier_tokens(child, bytes, out)?;
        }
    }
    Ok(())
}

fn syntax(node: Node<'_>, bytes: &[u8]) -> anyhow::Result<Syntax> {
    let mut full = node.range();
    let mut attributes = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(part) = previous {
        let owned = match part.kind() {
            "attribute_item" => true,
            "line_comment" | "block_comment" => part.child_by_field_name("outer").is_some(),
            _ => break,
        };
        if owned {
            full.start_byte = part.start_byte();
            full.start_point = part.start_position();
            attributes.push(text(part, bytes)?);
        }
        previous = part.prev_named_sibling();
    }
    attributes.reverse();
    let mut qualifiers = Vec::new();
    let mut visibility = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        qualifier_tokens(child, bytes, &mut qualifiers)?;
        if child.kind() == "visibility_modifier" {
            ensure!(
                visibility.replace(text(child, bytes)?).is_none(),
                "duplicate visibility"
            );
        }
    }
    Ok(Syntax {
        full,
        attributes,
        qualifiers,
        visibility,
    })
}

fn symbol<'a>(
    graph: &'a GraphIndexArtifact,
    path: &str,
    range: tree_sitter::Range,
    kinds: &[&str],
) -> anyhow::Result<&'a GraphSymbolArtifact> {
    let matches = graph
        .symbols
        .iter()
        .filter(|s| {
            s.file_path == path
                && kinds.contains(&s.symbol_kind.as_str())
                && s.byte_range == [range.start_byte, range.end_byte]
        })
        .collect::<Vec<_>>();
    ensure!(
        matches.len() == 1,
        "foreign declaration or owner absent or ambiguous in graph"
    );
    let found = matches[0];
    ensure!(
        found.line_range == [range.start_point.row + 1, range.end_point.row + 1],
        "foreign graph line range disagrees with syntax"
    );
    Ok(found)
}

// Walk syntax ancestors rather than accepting an arbitrary containing symbol or
// following a forged graph edge. In particular, an impl method remains an owner.
fn lexical_owner(
    node: Node<'_>,
    graph: &GraphIndexArtifact,
    path: &str,
    file_id: &str,
) -> anyhow::Result<(RustDeclarationOwner, String)> {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        let kinds: &[&str] = match ancestor.kind() {
            "mod_item" => &["module"],
            "function_item" => &["function", "method"],
            "impl_item" => &["impl"],
            "trait_item" => &["trait"],
            "struct_item" => &["struct"],
            "enum_item" => &["enum"],
            _ => &[],
        };
        if !kinds.is_empty() {
            let owner = symbol(graph, path, ancestor.range(), kinds)?;
            return Ok((
                RustDeclarationOwner::Symbol {
                    stable_symbol_id: owner.stable_symbol_id.clone(),
                },
                owner.qualified_name.clone(),
            ));
        }
        parent = ancestor.parent();
    }
    Ok((
        RustDeclarationOwner::File {
            stable_file_id: file_id.into(),
        },
        String::new(),
    ))
}

fn check_owner(
    graph: &GraphIndexArtifact,
    child: &GraphSymbolArtifact,
    owner: &RustDeclarationOwner,
) -> anyhow::Result<()> {
    let id = match owner {
        RustDeclarationOwner::File { stable_file_id } => stable_file_id,
        RustDeclarationOwner::Symbol { stable_symbol_id } => stable_symbol_id,
    };
    let parents = graph
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Contains
                && e.target_stable_symbol_id.as_deref() == Some(child.stable_symbol_id.as_str())
        })
        .collect::<Vec<_>>();
    ensure!(
        parents.len() == 1 && parents[0].source_stable_symbol_id == *id,
        "foreign graph owner disagrees with syntax"
    );
    Ok(())
}

fn check_identity(
    symbol: &GraphSymbolArtifact,
    name: &str,
    prefix: &str,
    kind: NodeKind,
) -> anyhow::Result<()> {
    let qualified = if prefix.is_empty() {
        name.into()
    } else {
        format!("{prefix}::{name}")
    };
    ensure!(
        symbol.entity_name == name
            && symbol.qualified_name == qualified
            && symbol.stable_symbol_id
                == stable_symbol_id_for(
                    &symbol.file_path,
                    &qualified,
                    kind,
                    symbol.byte_range[0] as u64
                ),
        "foreign graph identity disagrees with syntax"
    );
    Ok(())
}

pub(super) fn collect(
    node: Node<'_>,
    graph: &GraphIndexArtifact,
    source: &RustDeclarationSource<'_>,
    file_id: &str,
    oid: &str,
    set: &mut RustDeclarationEvidenceSet,
) -> anyhow::Result<()> {
    if node.is_error() || node.is_missing() {
        return Ok(());
    }
    if node.kind() == "foreign_mod_item" && !node.has_error() {
        let raw = syntax(node, source.bytes)?;
        let block = symbol(graph, source.file_path, raw.full, &["extern_block"])?;
        let (owner, prefix) = lexical_owner(node, graph, source.file_path, file_id)?;
        check_owner(graph, block, &owner)?;
        check_identity(block, "<extern>", &prefix, NodeKind::ExternBlock)?;
        let mut abi = RustDeclarationAbiSyntax::Omitted;
        let mut cursor = node.walk();
        for modifier in node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == "extern_modifier")
        {
            let mut inner = modifier.walk();
            for literal in modifier
                .named_children(&mut inner)
                .filter(|n| n.kind() == "string_literal")
            {
                abi = RustDeclarationAbiSyntax::Explicit {
                    literal: text(literal, source.bytes)?,
                };
            }
        }
        set.extern_blocks.push(RustExternBlockEvidence {
            file_path: source.file_path.into(),
            stable_file_id: file_id.into(),
            source_oid: oid.into(),
            stable_symbol_id: block.stable_symbol_id.clone(),
            entity_name: block.entity_name.clone(),
            qualified_name: block.qualified_name.clone(),
            structural_owner: owner.clone(),
            lexical_owner: owner.clone(),
            full_range: [raw.full.start_byte, raw.full.end_byte],
            line_range: [raw.full.start_point.row + 1, raw.full.end_point.row + 1],
            core_range: [node.start_byte(), node.end_byte()],
            core_line_range: [node.start_position().row + 1, node.end_position().row + 1],
            qualifiers: raw.qualifiers,
            visibility: raw.visibility,
            attributes: raw.attributes,
            abi,
            body: RustDeclarationBody::NonExecutableContainer,
        });
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for function in body
                .named_children(&mut cursor)
                .filter(|n| n.kind() == "function_signature_item" && !n.has_error())
            {
                let raw = syntax(function, source.bytes)?;
                let found = symbol(graph, source.file_path, raw.full, &["function"])?;
                let structural_owner = RustDeclarationOwner::Symbol {
                    stable_symbol_id: block.stable_symbol_id.clone(),
                };
                check_owner(graph, found, &structural_owner)?;
                let name = function
                    .child_by_field_name("name")
                    .context("foreign function has no name")?;
                check_identity(
                    found,
                    &text(name, source.bytes)?.text,
                    &prefix,
                    NodeKind::Function,
                )?;
                set.foreign_functions.push(RustForeignFunctionEvidence {
                    file_path: source.file_path.into(),
                    stable_file_id: file_id.into(),
                    source_oid: oid.into(),
                    stable_symbol_id: found.stable_symbol_id.clone(),
                    entity_name: found.entity_name.clone(),
                    qualified_name: found.qualified_name.clone(),
                    structural_owner,
                    lexical_owner: owner.clone(),
                    full_range: [raw.full.start_byte, raw.full.end_byte],
                    line_range: [raw.full.start_point.row + 1, raw.full.end_point.row + 1],
                    core_range: [function.start_byte(), function.end_byte()],
                    core_line_range: [
                        function.start_position().row + 1,
                        function.end_position().row + 1,
                    ],
                    qualifiers: raw.qualifiers,
                    visibility: raw.visibility,
                    attributes: raw.attributes,
                    abi: RustDeclarationAbiSyntax::Inherited {
                        extern_block_id: block.stable_symbol_id.clone(),
                    },
                    body: RustDeclarationBody::AbsentForeignDeclaration,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, graph, source, file_id, oid, set)?;
    }
    Ok(())
}
