//! Experimental source-bound evidence, independent of the canonical graph schema.

mod analysis;
mod build_identity;
mod call_sites;
mod cga;
mod compiler_validation;
mod method_sites;
mod semantic_facts;
mod semantics;

pub use analysis::*;
pub use build_identity::*;
pub use call_sites::*;
pub use cga::*;
pub use compiler_validation::*;
pub use semantic_facts::*;
pub use semantics::*;

use std::collections::BTreeSet;
use std::path::{Component, Path};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};

use crate::{compute_graph_content_hash, git_blob_oid, GraphIndexArtifact};

pub const METHOD_CALL_SITE_SCHEMA_VERSION: &str = "panic-method-sites-v1";
pub const METHOD_CALL_SITE_EXTRACTOR_VERSION: &str = "rust-direct-method-sites-v1";

#[derive(Debug, Clone, Copy)]
pub struct PanicEvidenceSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodCallSourceScope {
    SuppliedPhysicalRustFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodCallSourceBinding {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub byte_length: usize,
    pub site_count: usize,
    pub has_parse_errors: bool,
}

/// Syntax provenance only: this does not establish execution or cfg activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodCallLexicalPhase {
    FunctionBodySyntax,
    ClosureBodySyntax,
    ConstContextSyntax,
    ItemSyntax,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodCallLexicalOwner {
    pub syntax_kind: String,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerTargetState {
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodCallSiteEvidence {
    pub site_id: String,
    pub file_path: String,
    pub source_oid: String,
    /// Half-open UTF-8 byte ranges in the supplied physical file.
    pub byte_range: [usize; 2],
    pub method_byte_range: [usize; 2],
    pub receiver_byte_range: [usize; 2],
    /// One-based inclusive lines touched by the whole call.
    pub line_range: [usize; 2],
    pub call_text: String,
    pub method_name: String,
    /// Exact receiver syntax, never an inferred receiver type.
    pub receiver_text: String,
    pub caller_stable_symbol_id: Option<String>,
    pub lexical_phase: MethodCallLexicalPhase,
    /// Innermost to outermost syntax owners; these are not graph symbol IDs.
    pub lexical_owners: Vec<MethodCallLexicalOwner>,
    pub compiler_target: CompilerTargetState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodCallSiteEvidenceSet {
    pub schema_version: String,
    pub extractor_version: String,
    pub graph_content_hash: String,
    pub source_scope: MethodCallSourceScope,
    pub sources: Vec<MethodCallSourceBinding>,
    pub sites: Vec<MethodCallSiteEvidence>,
}

impl MethodCallSiteEvidenceSet {
    /// Recollect from the trusted graph and exact bytes. Comparing every field
    /// also checks completeness, canonical order and lexical ownership, rather
    /// than merely accepting an existing symbol ID or a self-consistent span.
    pub fn validate(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[PanicEvidenceSource<'_>],
    ) -> anyhow::Result<()> {
        let expected = collect_method_call_sites(graph, sources)?;
        ensure!(
            self == &expected,
            "method-call evidence differs from canonical source-bound collection"
        );
        Ok(())
    }
}

/// Collect intact direct method-call AST nodes in the supplied physical Rust
/// files. No macro expansion, cfg evaluation, receiver typing or compiler target
/// resolution is performed. Unparsed syntax is not evidence of an absent call.
pub fn collect_method_call_sites(
    graph: &GraphIndexArtifact,
    sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<MethodCallSiteEvidenceSet> {
    ensure!(
        !graph.graph_content_hash.is_empty()
            && graph.graph_content_hash
                == compute_graph_content_hash(
                    graph
                        .file_manifests
                        .iter()
                        .map(|m| (&m.path, &m.content_oid))
                ),
        "graph snapshot hash does not match its file manifest"
    );
    let mut ordered: Vec<_> = sources.iter().collect();
    ordered.sort_by_key(|s| s.file_path);
    let mut seen = BTreeSet::new();
    let mut set = MethodCallSiteEvidenceSet {
        schema_version: METHOD_CALL_SITE_SCHEMA_VERSION.into(),
        extractor_version: METHOD_CALL_SITE_EXTRACTOR_VERSION.into(),
        graph_content_hash: graph.graph_content_hash.clone(),
        source_scope: MethodCallSourceScope::SuppliedPhysicalRustFiles,
        sources: Vec::new(),
        sites: Vec::new(),
    };
    for source in ordered {
        ensure!(
            seen.insert(source.file_path),
            "duplicate supplied source path"
        );
        let path = Path::new(source.file_path);
        ensure!(
            path.is_relative()
                && path.extension().and_then(|x| x.to_str()) == Some("rs")
                && path.components().all(|c| matches!(c, Component::Normal(_))),
            "source must name a physical worktree-relative Rust file"
        );
        let files: Vec<_> = graph
            .files
            .iter()
            .filter(|f| f.file_path == source.file_path)
            .collect();
        let manifests: Vec<_> = graph
            .file_manifests
            .iter()
            .filter(|m| m.path == source.file_path)
            .collect();
        ensure!(
            files.len() == 1 && manifests.len() == 1,
            "source requires exactly one graph file and manifest entry"
        );
        let file = files[0];
        let manifest = manifests[0];
        ensure!(
            !file.stable_file_id.is_empty()
                && file.stable_file_id == manifest.stable_file_id
                && graph
                    .files
                    .iter()
                    .filter(|f| f.stable_file_id == file.stable_file_id)
                    .count()
                    == 1
                && graph
                    .file_manifests
                    .iter()
                    .filter(|m| m.stable_file_id == file.stable_file_id)
                    .count()
                    == 1,
            "physical file identity is missing, ambiguous or inconsistent"
        );
        let oid = git_blob_oid(source.bytes);
        ensure!(
            oid == manifest.content_oid,
            "source bytes do not match graph manifest blob OID"
        );
        let text = std::str::from_utf8(source.bytes).context("Rust source is not UTF-8")?;
        let (sites, has_parse_errors) =
            method_sites::collect_in_source(graph, source.file_path, &oid, text)?;
        set.sources.push(MethodCallSourceBinding {
            file_path: source.file_path.into(),
            stable_file_id: file.stable_file_id.clone(),
            source_oid: oid,
            byte_length: source.bytes.len(),
            site_count: sites.len(),
            has_parse_errors,
        });
        set.sites.extend(sites);
    }
    set.sites
        .sort_by(|a, b| (&a.file_path, a.byte_range).cmp(&(&b.file_path, b.byte_range)));
    Ok(set)
}
