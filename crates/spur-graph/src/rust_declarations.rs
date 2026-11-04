//! Source-bound evidence for direct physical Rust tuple fields.
//!
//! Completeness applies only to supplied files and the covered syntax kind;
//! attributes are raw syntax, with no configured or compiled interpretation.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::{compute_graph_content_hash, git_blob_oid, GraphIndexArtifact, RelationKind};

pub const RUST_DECLARATION_SCHEMA_VERSION: &str = "spur-rust-declarations-v1";

#[derive(Debug, Clone, Copy)]
pub struct RustDeclarationSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDeclarationScope {
    SuppliedPhysicalRustFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDeclarationKind {
    TupleField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDeclarationAvailability {
    Complete { row_count: usize },
    Unknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDeclarationOwner {
    File { stable_file_id: String },
    Symbol { stable_symbol_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDeclarationText {
    pub text: String,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDeclarationFile {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub byte_length: usize,
    pub availability: RustDeclarationAvailability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustTupleFieldEvidence {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub stable_symbol_id: String,
    pub structural_owner: RustDeclarationOwner,
    pub lexical_owner: RustDeclarationOwner,
    pub ordinal: usize,
    pub full_range: [usize; 2],
    pub type_range: [usize; 2],
    pub line_range: [usize; 2],
    pub visibility: Option<RustDeclarationText>,
    pub attributes: Vec<RustDeclarationText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDeclarationEvidenceSet {
    pub schema_version: String,
    pub extractor_version: String,
    pub graph_content_hash: String,
    pub scope: RustDeclarationScope,
    pub covered_kinds: Vec<RustDeclarationKind>,
    pub sources: Vec<RustDeclarationFile>,
    pub rows: Vec<RustTupleFieldEvidence>,
}

impl RustDeclarationEvidenceSet {
    /// Recollect the full canonical set from the exact bytes and graph owners.
    pub fn validate(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[RustDeclarationSource<'_>],
    ) -> anyhow::Result<()> {
        ensure!(
            *self == collect_rust_declarations(graph, sources)?,
            "declaration evidence differs from canonical source recollection"
        );
        Ok(())
    }
}

pub fn collect_rust_declarations(
    graph: &GraphIndexArtifact,
    sources: &[RustDeclarationSource<'_>],
) -> anyhow::Result<RustDeclarationEvidenceSet> {
    ensure!(
        graph.manifest_version == crate::store::current_manifest_version(),
        "incompatible graph extractor"
    );
    let mut manifest_paths = BTreeSet::new();
    for entry in &graph.file_manifests {
        ensure!(
            manifest_paths.insert(entry.path.as_str()),
            "duplicate graph manifest path"
        );
    }
    ensure!(
        !graph.graph_content_hash.is_empty()
            && graph.graph_content_hash
                == compute_graph_content_hash(
                    graph
                        .file_manifests
                        .iter()
                        .map(|e| (&e.path, &e.content_oid))
                ),
        "stale graph content hash"
    );
    let mut set = RustDeclarationEvidenceSet {
        schema_version: RUST_DECLARATION_SCHEMA_VERSION.into(),
        extractor_version: crate::store::EXTRACTOR_VERSION.into(),
        graph_content_hash: graph.graph_content_hash.clone(),
        scope: RustDeclarationScope::SuppliedPhysicalRustFiles,
        covered_kinds: vec![RustDeclarationKind::TupleField],
        sources: Vec::new(),
        rows: Vec::new(),
    };
    let mut paths = BTreeSet::new();
    let mut ordered = sources.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|source| source.file_path);
    for source in ordered {
        let path = Path::new(source.file_path);
        ensure!(
            !source.file_path.contains("#cell:")
                && path.extension().is_some_and(|ext| ext == "rs")
                && path.components().all(|c| matches!(c, Component::Normal(_))),
            "source is not a physical Rust path"
        );
        ensure!(paths.insert(source.file_path), "duplicate source path");
        let manifest = graph
            .file_manifests
            .iter()
            .find(|e| e.path == source.file_path)
            .context("source missing from graph manifest")?;
        let file = graph
            .files
            .iter()
            .filter(|f| f.file_path == source.file_path)
            .collect::<Vec<_>>();
        ensure!(
            file.len() == 1 && file[0].stable_file_id == manifest.stable_file_id,
            "source is not an authoritative physical graph file"
        );
        let oid = git_blob_oid(source.bytes);
        ensure!(
            oid == manifest.content_oid,
            "source bytes do not match graph"
        );
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
        let tree = parser
            .parse(source.bytes, None)
            .context("Rust parser returned no tree")?;
        let start = set.rows.len();
        let utf8 = std::str::from_utf8(source.bytes).is_ok();
        if utf8 {
            collect_node(
                tree.root_node(),
                graph,
                source,
                &manifest.stable_file_id,
                &oid,
                &mut set.rows,
            )?;
        }
        let availability = if !utf8 || tree.root_node().has_error() {
            RustDeclarationAvailability::Unknown {
                reason: "invalid UTF-8 or recovered Rust syntax".into(),
            }
        } else {
            RustDeclarationAvailability::Complete {
                row_count: set.rows.len() - start,
            }
        };
        set.sources.push(RustDeclarationFile {
            file_path: source.file_path.into(),
            stable_file_id: manifest.stable_file_id.clone(),
            source_oid: oid,
            byte_length: source.bytes.len(),
            availability,
        });
    }
    Ok(set)
}

fn collect_node(
    node: Node<'_>,
    graph: &GraphIndexArtifact,
    source: &RustDeclarationSource<'_>,
    file_id: &str,
    oid: &str,
    rows: &mut Vec<RustTupleFieldEvidence>,
) -> anyhow::Result<()> {
    // Do not turn parser-recovered nodes into source facts, even when other
    // independent declarations in the same file are usable.
    if node.is_error() || node.is_missing() {
        return Ok(());
    }
    if node.kind() == "ordered_field_declaration_list" {
        let parent = node.parent().context("tuple list has no owner")?;
        if !parent.has_error() && matches!(parent.kind(), "struct_item" | "enum_variant") {
            let kind = if parent.kind() == "struct_item" {
                "struct"
            } else {
                "enum_variant"
            };
            let owners = graph
                .symbols
                .iter()
                .filter(|s| {
                    s.file_path == source.file_path
                        && s.symbol_kind == kind
                        && s.byte_range == [parent.start_byte(), parent.end_byte()]
                })
                .collect::<Vec<_>>();
            ensure!(
                owners.len() == 1,
                "tuple owner absent or ambiguous in graph"
            );
            let owner = owners[0];
            let mut ordinal = 0;
            for i in 0..node.child_count() {
                if node.field_name_for_child(i as u32) != Some("type") {
                    continue;
                }
                let ty = node.child(i as u32).context("missing tuple field type")?;
                let mut previous = ty.prev_sibling();
                let mut full_start = ty.start_byte();
                let mut full_line = ty.start_position().row + 1;
                let mut attrs = Vec::new();
                let mut visibility = None;
                while let Some(part) = previous {
                    if matches!(part.kind(), "," | "(") {
                        break;
                    }
                    let is_attr = part.kind() == "attribute_item"
                        || (matches!(part.kind(), "line_comment" | "block_comment")
                            && part.child_by_field_name("outer").is_some());
                    if is_attr || part.kind() == "visibility_modifier" {
                        full_start = part.start_byte();
                        full_line = part.start_position().row + 1;
                        let text = RustDeclarationText {
                            text: std::str::from_utf8(
                                &source.bytes[part.start_byte()..part.end_byte()],
                            )?
                            .to_owned(),
                            byte_range: [part.start_byte(), part.end_byte()],
                            line_range: [
                                part.start_position().row + 1,
                                part.end_position().row + 1,
                            ],
                        };
                        if is_attr {
                            attrs.push(text);
                        } else {
                            ensure!(
                                visibility.replace(text).is_none(),
                                "duplicate field visibility"
                            );
                        }
                    }
                    previous = part.prev_sibling();
                }
                attrs.reverse();
                let full = [full_start, ty.end_byte()];
                let fields = graph
                    .symbols
                    .iter()
                    .filter(|s| {
                        s.file_path == source.file_path
                            && s.symbol_kind == "field"
                            && s.byte_range == full
                            && s.entity_name == ordinal.to_string()
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    fields.len() == 1,
                    "tuple field absent or ambiguous in graph"
                );
                let field = fields[0];
                let parents = graph
                    .edges
                    .iter()
                    .filter(|e| {
                        e.relation == RelationKind::Contains
                            && e.target_stable_symbol_id.as_deref()
                                == Some(field.stable_symbol_id.as_str())
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    parents.len() == 1
                        && parents[0].source_stable_symbol_id == owner.stable_symbol_id,
                    "tuple graph owner disagrees with syntax"
                );
                let owner_ref = RustDeclarationOwner::Symbol {
                    stable_symbol_id: owner.stable_symbol_id.clone(),
                };
                rows.push(RustTupleFieldEvidence {
                    file_path: source.file_path.into(),
                    stable_file_id: file_id.into(),
                    source_oid: oid.into(),
                    stable_symbol_id: field.stable_symbol_id.clone(),
                    structural_owner: owner_ref.clone(),
                    lexical_owner: owner_ref,
                    ordinal,
                    full_range: full,
                    type_range: [ty.start_byte(), ty.end_byte()],
                    line_range: [full_line, ty.end_position().row + 1],
                    visibility,
                    attributes: attrs,
                });
                ordinal += 1;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_node(child, graph, source, file_id, oid, rows)?;
    }
    Ok(())
}
