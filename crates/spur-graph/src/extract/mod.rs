use std::path::Path;

use anyhow::Context as _;

pub mod languages;
pub mod markdown;
pub mod mcp_tools;
pub(crate) mod notebook;
pub(crate) mod openapi;
pub(crate) mod rust_macros;
pub mod tree_sitter;

pub use tree_sitter::{build_facts, build_facts_for_paths};

use crate::{GraphEdge, GraphNode, MacroInvocationEvidence, SourceSpan};

#[derive(Debug, Clone, PartialEq)]
pub struct GraphFacts {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub spans: Vec<SourceSpan>,
    pub macro_invocations: Vec<MacroInvocationEvidence>,
}

impl GraphFacts {
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            spans: Vec::new(),
            macro_invocations: Vec::new(),
        }
    }
}

pub fn extract_notebook_facts(
    root: &Path,
    path: &Path,
    bytes: &[u8],
) -> anyhow::Result<GraphFacts> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let mut builder = tree_sitter::FactBuilder::new(&root);
    notebook::extract_notebook_file(&mut builder, path, bytes)?;
    builder.resolve_pending_edges();
    Ok(builder.into_facts())
}
