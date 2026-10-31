//! Experimental source-bound evidence, independent of the canonical graph schema.

mod method_sites;

use serde::{Deserialize, Serialize};

use crate::GraphIndexArtifact;

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
    pub fn validate(
        &self,
        _graph: &GraphIndexArtifact,
        _sources: &[PanicEvidenceSource<'_>],
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

pub fn collect_method_call_sites(
    graph: &GraphIndexArtifact,
    sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<MethodCallSiteEvidenceSet> {
    method_sites::collect(graph, sources)
}
