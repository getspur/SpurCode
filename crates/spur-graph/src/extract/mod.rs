use std::path::Path;

use anyhow::Context as _;

pub mod languages;
pub mod markdown;
pub mod mcp_tools;
pub(crate) mod notebook;
pub mod notebook_model;
pub mod notebook_symbols;
pub(crate) mod openapi;
mod rust_bindings;
pub(crate) mod rust_macros;
pub mod tree_sitter;

pub use tree_sitter::{build_facts, build_facts_for_paths};

/// Give selected extraction, composition and overlay tombstones one identity.
/// Missing paths retain their normalized key without becoming active sources.
pub(crate) fn normalize_selected_paths(
    root: &Path,
    paths: &[std::path::PathBuf],
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    use std::{
        collections::BTreeSet,
        io::ErrorKind,
        path::{Component, PathBuf},
    };

    // Resolve physical prefixes before consuming parents. Canonicalize cannot
    // resolve a missing suffix, so retain it while still following safe aliases.
    fn resolve(path: &Path, resolving: &mut BTreeSet<PathBuf>) -> anyhow::Result<PathBuf> {
        match path.canonicalize() {
            Ok(path) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to resolve selected path"),
        }
        let mut resolved = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => {
                    resolved.push(part);
                    match resolved.canonicalize() {
                        Ok(path) => resolved = path,
                        Err(error) if error.kind() == ErrorKind::NotFound => {
                            match resolved.symlink_metadata() {
                                Ok(metadata) if metadata.file_type().is_symlink() => {
                                    // A dangling link still determines the physical
                                    // deletion key. Detect recursive missing aliases.
                                    anyhow::ensure!(
                                        resolving.insert(resolved.clone()),
                                        "selected path has a symlink cycle"
                                    );
                                    let link = resolved.clone();
                                    let target = std::fs::read_link(&link)
                                        .context("failed to read selected symlink")?;
                                    let result =
                                        resolve(&link.parent().unwrap().join(target), resolving);
                                    resolving.remove(&link);
                                    resolved = result?;
                                }
                                Ok(_) => {}
                                Err(error) if error.kind() == ErrorKind::NotFound => {}
                                Err(error) => {
                                    return Err(error).context("failed to inspect selected path")
                                }
                            }
                        }
                        Err(error) => return Err(error).context("failed to resolve selected path"),
                    }
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    anyhow::ensure!(resolved.pop(), "selected path escapes worktree root");
                }
                _ => resolved.push(component.as_os_str()),
            }
        }
        Ok(resolved)
    }

    let root = root
        .canonicalize()
        .context("failed to resolve worktree root")?;
    let mut selected = BTreeSet::new();
    for path in paths {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        let resolved = resolve(&absolute, &mut BTreeSet::new())?;
        let normalized = resolved
            .strip_prefix(&root)
            .context("selected path escapes worktree root")?;
        normalized.to_str().context("selected path is not UTF-8")?;
        selected.insert(normalized.to_path_buf());
    }
    Ok(selected.into_iter().collect())
}

use crate::{GraphEdge, GraphNode, MacroInvocationEvidence, MacroScopeEvidence, SourceSpan};

#[derive(Debug, Clone, PartialEq)]
pub struct GraphFacts {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub spans: Vec<SourceSpan>,
    pub macro_invocations: Vec<MacroInvocationEvidence>,
    pub macro_scopes: Vec<MacroScopeEvidence>,
}

impl GraphFacts {
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            spans: Vec::new(),
            macro_invocations: Vec::new(),
            macro_scopes: Vec::new(),
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
