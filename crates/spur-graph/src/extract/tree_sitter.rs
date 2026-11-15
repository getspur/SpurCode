use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str;

use anyhow::{anyhow, Context as _};
use indicatif::ProgressBar;
use thiserror::Error;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator as _};

use crate::discovery::discover_files;
use crate::extract::languages::{
    all_supported_extensions, emit_definitions, emit_definitions_with_parents, emit_edges,
    emit_rust_dyn_trait_edges, extracted_symbols, language_registry, Language, LanguageConfig,
    LanguageDescriptor,
};
use crate::extract::markdown::extract_markdown_file;
use crate::extract::mcp_tools::emit_mcp_tools;
use crate::extract::openapi::{extract_openapi_document, MAX_STRUCTURED_EXTRACT_BYTES};
use crate::extract::GraphFacts;
use crate::{
    Confidence, EdgeId, EvidenceId, FileId, GraphEdge, GraphEdgeKind, GraphNode, NodeId, NodeKind,
    RelationKind, RunId, SourceSpan, SpanId,
};

#[derive(Debug, Clone)]
pub(crate) struct PendingEdge {
    pub(crate) source: NodeId,
    pub(crate) target_name: String,
    pub(crate) import_path: Option<String>,
    pub(crate) relation: RelationKind,
    pub(crate) edge_kind: Option<GraphEdgeKind>,
    pub(crate) origin: CallOrigin,
    pub(crate) receiver_text: Option<String>,
    pub(crate) scope_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallOrigin {
    Expression,
    MacroBody,
}

#[derive(Debug)]
pub(crate) struct FactBuilder<'a> {
    root: &'a Path,
    facts: GraphFacts,
    next_node: u64,
    next_edge: u64,
    next_file: u64,
    next_span: u64,
    pub(crate) pending_edges: Vec<PendingEdge>,
    pub(crate) reexport_edges: Vec<PendingEdge>,
    symbol_index: BTreeMap<String, Vec<NodeId>>,
    edge_index: HashSet<EdgeDedupKey>,
    qualified_symbol_index: BTreeMap<String, Vec<NodeId>>,
}

#[derive(Debug, Clone)]
pub(crate) struct CaptureHit<'tree> {
    pub(crate) name: String,
    pub(crate) node: Node<'tree>,
    pub(crate) pattern_index: usize,
    pub(crate) match_index: usize,
}

pub(crate) struct CompiledQueries {
    pub(crate) tags: Query,
    pub(crate) symbols: Query,
    pub(crate) spur_edges: Option<Query>,
    pub(crate) jsx_edges: Option<Query>,
    pub(crate) inline_spur_edges: Option<Query>,
    pub(crate) spur_notebook_facts: Option<Query>,
}

impl CompiledQueries {
    fn empty(language: &tree_sitter::Language) -> anyhow::Result<Self> {
        Ok(Self {
            tags: Query::new(language, "").context("compile empty tags query")?,
            symbols: Query::new(language, "").context("compile empty symbols query")?,
            spur_edges: None,
            jsx_edges: None,
            inline_spur_edges: None,
            spur_notebook_facts: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolQueryPolicy {
    ReuseTags,
    Dedicated(&'static str),
}

impl SymbolQueryPolicy {
    fn source(self, tags_source: &'static str) -> &'static str {
        match self {
            Self::ReuseTags => tags_source,
            Self::Dedicated(source) => source,
        }
    }
}

fn symbol_query_policy(language: Language) -> SymbolQueryPolicy {
    match language {
        Language::Rust
        | Language::Python
        | Language::TypeScript
        | Language::Tsx
        | Language::Javascript
        | Language::C
        | Language::Cpp
        | Language::Go
        | Language::Hcl
        | Language::Terraform
        | Language::Lua
        | Language::Shell
        | Language::Sql
        | Language::Json
        | Language::Toml
        | Language::Yaml
        | Language::Mermaid
        | Language::Html
        | Language::Css
        | Language::JupyterNotebook => SymbolQueryPolicy::ReuseTags,
        Language::Markdown => {
            SymbolQueryPolicy::Dedicated(include_str!("../../queries/markdown/symbols.scm"))
        }
    }
}

#[derive(Debug)]
pub(crate) struct LanguageFileGroup {
    pub(crate) label: &'static str,
    pub(crate) language: Language,
    pub(crate) config: LanguageConfig,
    pub(crate) files: Vec<PathBuf>,
}

type GroupAccumulator = BTreeMap<&'static str, (Language, fn() -> LanguageConfig, Vec<PathBuf>)>;
type EdgeDedupKey = (
    NodeId,
    Option<NodeId>,
    RelationKind,
    Option<String>,
    Option<GraphEdgeKind>,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone)]
struct EdgeMetadata {
    confidence: Confidence,
    confidence_score: f32,
    import_path: Option<String>,
    receiver_text: Option<String>,
    scope_text: Option<String>,
    bind_method: Option<&'static str>,
}

struct PendingResolutionIndexes<'a> {
    singleton_symbols_by_label: &'a HashMap<String, NodeId>,
    ambiguous_symbols_by_label: &'a HashMap<String, usize>,
    files_by_label: &'a HashMap<String, NodeId>,
    pending_imports_by_source: &'a HashMap<NodeId, Vec<PendingEdge>>,
    file_by_id: &'a HashMap<NodeId, &'a str>,
    node_kind_by_id: &'a HashMap<NodeId, NodeKind>,
    enclosing_scope_by_id: &'a HashMap<NodeId, String>,
    qualified_name_by_id: &'a HashMap<NodeId, String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportOriginClassification {
    Internal,
    External {
        origin: String,
        path: String,
        name: String,
    },
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub(crate) struct ImportWorkspaceIndex {
    pub(crate) rust_crates: HashSet<String>,
    pub(crate) javascript_path_aliases: HashSet<String>,
    pub(crate) javascript_workspace_roots: HashSet<String>,
    pub(crate) python_modules: HashSet<String>,
}

/// Maximum import re-export hops followed before leaving the edge unresolved.
const MAX_REEXPORT_FOLLOW_DEPTH: usize = 8;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("invalid utf-8: {0}")]
    InvalidUtf8(str::Utf8Error),
    #[error("parser setup failed: {0}")]
    Setup(String),
    #[error("tree-sitter parse returned no tree")]
    NoTree,
    #[error("extraction failed: {0}")]
    Extraction(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSymbol {
    pub entity_name: String,
    pub symbol_kind: String,
    pub enclosing_scope: Option<String>,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
    pub anchor_hash: String,
    pub tokens: Vec<String>,
}

pub struct BytesExtractor {
    language: Language,
    config: LanguageConfig,
    parser: Parser,
    queries: CompiledQueries,
    markdown_inline_parser: Option<Parser>,
}

// The C++ tags query (also used for symbols) does not capture this
// linkage-wrapped array declarator, and cpp/spur-edges has no definitions for
// its includes or numeric initializer. Recognize the entire generated file,
// rather than trusting a filename, generated marker, size, or sampled prefix.
// Keep graph-fact extraction on the AST path: non-definition captures still
// matter there. The AST-oracle tests must remain empty if these queries change.
fn is_generated_icu_numeric_data(bytes: &[u8]) -> bool {
    let Some(mut remaining) = bytes.strip_prefix(
        concat!(
            "\n// This file is generated by scripts/inline-data.py, do not edit it manually //\n\n",
            "#include \"unicode/utypes.h\"\n",
            "#include \"unicode/udata.h\"\n",
            "#include \"unicode/uversion.h\"\n\n",
            "extern \"C\" U_EXPORT const unsigned char U_ICUDATA_ENTRY_POINT [] = {\n"
        )
        .as_bytes(),
    ) else {
        return false;
    };

    loop {
        remaining = remaining.trim_ascii_start();
        let digit_count = remaining
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let digits = &remaining[..digit_count];
        // Canonical decimal bytes only: leading zeroes could be C++ octal,
        // and checked u8 arithmetic rejects overflow without a size threshold.
        if digits.is_empty()
            || (digits.len() > 1 && digits[0] == b'0')
            || digits
                .iter()
                .try_fold(0u8, |value, digit| {
                    value.checked_mul(10)?.checked_add(*digit - b'0')
                })
                .is_none()
        {
            return false;
        }
        remaining = remaining[digit_count..].trim_ascii_start();
        if let Some(after_comma) = remaining.strip_prefix(b",") {
            remaining = after_comma.trim_ascii_start();
            if !remaining.starts_with(b"}") {
                continue;
            }
        }
        return remaining
            .strip_prefix(b"};")
            .is_some_and(|trailing| trailing.trim_ascii_start().is_empty());
    }
}

impl BytesExtractor {
    pub fn for_language(language: Language) -> Result<Self, ExtractError> {
        Self::new(language, language.config())
    }

    fn new(language: Language, config: LanguageConfig) -> Result<Self, ExtractError> {
        if language == Language::JupyterNotebook {
            let queries = CompiledQueries::empty(&config.language)
                .map_err(|err| ExtractError::Setup(err.to_string()))?;
            return Ok(Self {
                language,
                config,
                parser: Parser::new(),
                queries,
                markdown_inline_parser: None,
            });
        }

        let mut parser = Parser::new();
        parser
            .set_language(&config.language)
            .map_err(|err| ExtractError::Setup(err.to_string()))?;
        let queries = compile_queries(&config, language)
            .map_err(|err| ExtractError::Setup(err.to_string()))?;
        let markdown_inline_parser = if language == Language::Markdown {
            let mut parser = Parser::new();
            if let Some(inline_language) = config.inline_language.as_ref() {
                parser
                    .set_language(inline_language)
                    .map_err(|err| ExtractError::Setup(err.to_string()))?;
            }
            Some(parser)
        } else {
            None
        };
        Ok(Self {
            language,
            config,
            parser,
            queries,
            markdown_inline_parser,
        })
    }

    pub fn extract(
        &mut self,
        _logical_path: &Path,
        bytes: &[u8],
    ) -> Result<Vec<ExtractedSymbol>, ExtractError> {
        if self.language == Language::JupyterNotebook {
            return Ok(Vec::new());
        }

        let source = str::from_utf8(bytes).map_err(ExtractError::InvalidUtf8)?;
        if matches!(self.language, Language::Json | Language::Yaml)
            && source.len() > MAX_STRUCTURED_EXTRACT_BYTES
        {
            return Ok(Vec::new());
        }
        if self.language == Language::Cpp && is_generated_icu_numeric_data(bytes) {
            return Ok(Vec::new());
        }
        let tree = self
            .parser
            .parse(source.as_bytes(), None)
            .ok_or(ExtractError::NoTree)?;
        let mut captures = run_query(&self.queries.symbols, tree.root_node(), source);
        if let Some(spur_edges) = self.queries.spur_edges.as_ref() {
            // Pattern/match indices are query-local. Keep name lookup keys
            // disjoint when projecting definitions from both capture groups.
            captures.extend(
                run_query(spur_edges, tree.root_node(), source)
                    .into_iter()
                    .map(|mut capture| {
                        capture.pattern_index += self.queries.symbols.pattern_count();
                        capture
                    }),
            );
        }
        Ok(extracted_symbols(&self.config, source, &captures))
    }

    /// Extract native graph facts for one supplied file without filesystem or
    /// repository access, using this extractor's configured language.
    ///
    /// The logical path need not exist. It must be nonempty UTF-8 and relative,
    /// with no parent components or drive prefix. Separators and harmless `.`
    /// components are normalized lexically. Each call starts fresh numeric IDs;
    /// pending-edge resolution considers only this file, so references needing
    /// other files may remain unresolved with their native evidence intact.
    pub fn extract_file_facts(
        &mut self,
        logical_path: &Path,
        bytes: &[u8],
    ) -> Result<GraphFacts, ExtractError> {
        let logical_path = logical_path
            .to_str()
            .ok_or_else(|| ExtractError::Extraction("logical path must be UTF-8".into()))?;
        // Native fact paths use slash separators, including on Windows. Validate
        // that spelling so a backslash cannot hide a root or parent component.
        let logical_path = logical_path.replace('\\', "/");
        let mut path = PathBuf::new();
        for component in Path::new(&logical_path).components() {
            match component {
                Component::Normal(part) => path.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(ExtractError::Extraction(
                        "logical path must be relative with no parent components".into(),
                    ));
                }
            }
        }
        if path.as_os_str().is_empty() {
            return Err(ExtractError::Extraction(
                "logical path must name a file".into(),
            ));
        }
        let path_bytes = path.as_os_str().as_encoded_bytes();
        if path_bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && path_bytes.get(1) == Some(&b':')
        {
            return Err(ExtractError::Extraction(
                "logical path must not have a drive prefix".into(),
            ));
        }

        let mut builder = FactBuilder::new(Path::new(""));
        self.extract_graph_facts(&mut builder, &path, bytes)?;
        builder.resolve_pending_edges();
        Ok(builder.into_facts())
    }

    pub(crate) fn extract_graph_facts(
        &mut self,
        builder: &mut FactBuilder<'_>,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), ExtractError> {
        if self.language == Language::JupyterNotebook {
            return crate::extract::notebook::extract_notebook_file(builder, path, bytes)
                .map_err(|err| ExtractError::Extraction(err.to_string()));
        }

        let source = str::from_utf8(bytes).map_err(ExtractError::InvalidUtf8)?;
        let tree = self
            .parser
            .parse(source.as_bytes(), None)
            .ok_or(ExtractError::NoTree)?;
        if self.language == Language::Markdown {
            extract_markdown_file(
                builder,
                &self.config,
                path,
                source,
                tree.root_node(),
                &self.queries,
                self.markdown_inline_parser.as_mut(),
            )
        } else {
            extract_file_from_tree(
                builder,
                self.language.label(),
                &self.config,
                path,
                source,
                tree.root_node(),
                &self.queries,
            )
        }
        .map_err(|err| ExtractError::Extraction(err.to_string()))
    }
}

impl<'a> FactBuilder<'a> {
    pub(crate) fn new(root: &'a Path) -> Self {
        Self {
            root,
            facts: GraphFacts::empty(),
            next_node: 1,
            next_edge: 1,
            next_file: 1,
            next_span: 1,
            pending_edges: Vec::new(),
            reexport_edges: Vec::new(),
            symbol_index: BTreeMap::new(),
            edge_index: HashSet::new(),
            qualified_symbol_index: BTreeMap::new(),
        }
    }

    pub(crate) fn root(&self) -> &'a Path {
        self.root
    }

    pub(crate) fn into_facts(self) -> GraphFacts {
        self.facts
    }

    pub(crate) fn node_stable_key(&self, node_id: NodeId) -> Option<&str> {
        self.facts
            .nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .map(|node| node.stable_key.as_str())
    }

    pub(crate) fn record_macro_invocations(
        &mut self,
        rows: Vec<crate::MacroInvocationEvidence>,
        scopes: Vec<crate::MacroScopeEvidence>,
    ) {
        self.facts.macro_invocations.extend(rows);
        self.facts.macro_scopes.extend(scopes);
    }

    pub(crate) fn next_file_id(&mut self) -> u64 {
        let next = self.next_file;
        self.next_file += 1;
        next
    }

    pub(crate) fn add_file_node(
        &mut self,
        relative_path: &str,
        file_id: FileId,
        root_node: Node<'_>,
    ) -> NodeId {
        self.add_node(
            relative_path,
            relative_path.to_owned(),
            relative_path.to_owned(),
            NodeKind::File,
            file_id,
            root_node,
        )
    }

    pub(crate) fn add_node(
        &mut self,
        relative_path: &str,
        label: String,
        fqn: String,
        kind: NodeKind,
        file_id: FileId,
        node: Node<'_>,
    ) -> NodeId {
        self.add_node_with_range(relative_path, label, fqn, kind, file_id, node.range())
    }

    pub(crate) fn add_node_with_range(
        &mut self,
        relative_path: &str,
        label: String,
        fqn: String,
        kind: NodeKind,
        file_id: FileId,
        range: tree_sitter::Range,
    ) -> NodeId {
        let node_id = NodeId(self.next_node);
        self.next_node += 1;
        let span_id = SpanId(self.next_span);
        self.next_span += 1;
        let impl_identity;
        let identity_fqn = if kind == NodeKind::Impl {
            impl_identity = impl_identity_fqn(&fqn);
            impl_identity.as_str()
        } else {
            &fqn
        };
        let stable_key = crate::identity::stable_symbol_id_for(
            relative_path,
            identity_fqn,
            kind,
            range.start_byte as u64,
        );

        self.facts.spans.push(SourceSpan {
            span_id,
            file_id,
            start_byte: range.start_byte as u32,
            end_byte: range.end_byte as u32,
            start_line: range.start_point.row as u32 + 1,
            end_line: range.end_point.row as u32 + 1,
        });
        self.facts.nodes.push(GraphNode {
            node_id,
            stable_key,
            label: label.clone(),
            kind,
            file_id: Some(file_id),
            source_span_id: Some(span_id),
            first_seen_run_id: RunId(1),
        });
        if !matches!(kind, NodeKind::File | NodeKind::McpTool) {
            self.symbol_index.entry(label).or_default().push(node_id);
            self.qualified_symbol_index
                .entry(fqn)
                .or_default()
                .push(node_id);
        }
        node_id
    }

    pub(crate) fn add_edge(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
        relation: RelationKind,
        target_label: Option<String>,
    ) {
        self.add_edge_with_kind(source, target, relation, target_label, None);
    }

    pub(crate) fn add_edge_with_kind(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
        relation: RelationKind,
        target_label: Option<String>,
        edge_kind: Option<GraphEdgeKind>,
    ) {
        let (confidence, confidence_score) = confidence_for_edge(relation, edge_kind);
        self.add_edge_with_metadata(
            source,
            target,
            relation,
            target_label,
            edge_kind,
            EdgeMetadata {
                confidence,
                confidence_score,
                import_path: None,
                receiver_text: None,
                scope_text: None,
                bind_method: None,
            },
        );
    }

    pub(crate) fn add_edge_with_bind_method(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
        relation: RelationKind,
        target_label: Option<String>,
        bind_method: &'static str,
    ) {
        let (confidence, confidence_score) = confidence_for_edge(relation, None);
        self.add_edge_with_metadata(
            source,
            target,
            relation,
            target_label,
            None,
            EdgeMetadata {
                confidence,
                confidence_score,
                import_path: None,
                receiver_text: None,
                scope_text: None,
                bind_method: Some(bind_method),
            },
        );
    }

    fn add_pending_edge(&mut self, edge: &PendingEdge, target: Option<NodeId>) {
        self.add_pending_edge_with_bind_method(edge, target, None);
    }

    fn add_pending_edge_as(
        &mut self,
        edge: &PendingEdge,
        target: Option<NodeId>,
        relation: RelationKind,
        bind_method: Option<&'static str>,
    ) {
        let metadata = metadata_for_pending_edge(edge, target, bind_method);
        self.add_edge_with_metadata(
            edge.source,
            target,
            relation,
            Some(edge.target_name.clone()),
            edge.edge_kind,
            metadata,
        );
    }

    fn add_pending_edge_with_bind_method(
        &mut self,
        edge: &PendingEdge,
        target: Option<NodeId>,
        bind_method: Option<&'static str>,
    ) {
        let metadata = metadata_for_pending_edge(edge, target, bind_method);
        self.add_edge_with_metadata(
            edge.source,
            target,
            edge.relation,
            Some(edge.target_name.clone()),
            edge.edge_kind,
            metadata,
        );
    }

    fn add_edge_with_metadata(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
        relation: RelationKind,
        target_label: Option<String>,
        edge_kind: Option<GraphEdgeKind>,
        metadata: EdgeMetadata,
    ) {
        if !self.edge_index.insert((
            source,
            target,
            relation,
            target_label.clone(),
            edge_kind,
            metadata.receiver_text.clone(),
            metadata.scope_text.clone(),
        )) {
            return;
        }
        let edge_id = EdgeId(self.next_edge);
        self.next_edge += 1;
        self.facts.edges.push(GraphEdge {
            edge_id,
            source_node_id: source,
            target_node_id: target,
            relation,
            target_label,
            import_path: metadata.import_path,
            receiver_text: metadata.receiver_text,
            scope_text: metadata.scope_text,
            confidence: metadata.confidence,
            confidence_score: metadata.confidence_score,
            edge_kind,
            bind_method: metadata.bind_method.map(str::to_string),
            evidence_id: EvidenceId(edge_id.get()),
            directed: true,
            change_kind: None,
        });
    }

    pub(crate) fn resolve_pending_edges(&mut self) {
        let mut singleton_symbols_by_label: HashMap<String, NodeId> = HashMap::new();
        let mut ambiguous_symbols_by_label: HashMap<String, usize> = HashMap::new();
        for (label, ids) in &self.symbol_index {
            match ids.as_slice() {
                [id] => {
                    singleton_symbols_by_label.insert(label.clone(), *id);
                }
                [] => {}
                _ => {
                    ambiguous_symbols_by_label.insert(label.clone(), ids.len());
                }
            }
        }
        let mut files_by_label: HashMap<String, NodeId> = HashMap::new();
        for node in &self.facts.nodes {
            if node.kind == NodeKind::File {
                files_by_label
                    .entry(node.label.clone())
                    .or_insert(node.node_id);
            }
        }
        let node_kind_by_id: HashMap<NodeId, NodeKind> = self
            .facts
            .nodes
            .iter()
            .map(|node| (node.node_id, node.kind))
            .collect();
        let file_path_by_file_node: HashMap<NodeId, String> = self
            .facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File)
            .map(|node| (node.node_id, node.label.clone()))
            .collect();
        let (qualified_symbols_by_name, enclosing_scope_by_id, file_by_id) = {
            let nodes_by_id: HashMap<_, _> = self
                .facts
                .nodes
                .iter()
                .map(|node| (node.node_id, node))
                .collect();
            let parent_by_target = parent_by_target(&self.facts);
            (
                qualified_symbols_by_name_from_maps(&self.facts, &nodes_by_id, &parent_by_target),
                enclosing_scope_by_id(&self.facts, &nodes_by_id, &parent_by_target),
                file_by_id_from_maps(
                    &self.facts,
                    &nodes_by_id,
                    &parent_by_target,
                    &file_path_by_file_node,
                ),
            )
        };
        let qualified_name_by_id = qualified_name_by_id_from_index(&qualified_symbols_by_name);
        let pending = std::mem::take(&mut self.pending_edges);
        let reexports = std::mem::take(&mut self.reexport_edges);
        let mut pending_imports_by_source: HashMap<NodeId, Vec<PendingEdge>> = HashMap::new();
        for edge in pending.iter().chain(reexports.iter()) {
            if edge.relation == RelationKind::Imports {
                pending_imports_by_source
                    .entry(edge.source)
                    .or_default()
                    .push(edge.clone());
            }
        }
        let indexes = PendingResolutionIndexes {
            singleton_symbols_by_label: &singleton_symbols_by_label,
            ambiguous_symbols_by_label: &ambiguous_symbols_by_label,
            files_by_label: &files_by_label,
            pending_imports_by_source: &pending_imports_by_source,
            file_by_id: &file_by_id,
            node_kind_by_id: &node_kind_by_id,
            enclosing_scope_by_id: &enclosing_scope_by_id,
            qualified_name_by_id: &qualified_name_by_id,
        };
        let mut ambiguous_unresolved = 0usize;
        let mut phantom_blocked_references = 0usize;
        let mut phantom_blocked_calls = 0usize;
        for edge in pending {
            if edge.relation == RelationKind::Links {
                // URL text is evidence, not a symbol name. A later dedicated
                // URL/file resolver may bind it, but bare-symbol lookup must not.
                self.add_pending_edge(&edge, None);
            } else if edge.edge_kind == Some(GraphEdgeKind::CallsDyn) {
                let mut candidates = Vec::new();
                if let Some(indexed) = self.qualified_symbol_index.get(&edge.target_name) {
                    candidates.extend(indexed.iter().copied());
                }
                if let Some(indexed) = qualified_symbols_by_name.get(&edge.target_name) {
                    candidates.extend(indexed.iter().copied());
                }
                candidates.extend(trait_method_candidates(&self.facts, &edge.target_name));
                candidates.sort_by_key(|id| id.get());
                candidates.dedup();
                match candidates.as_slice() {
                    [target]
                        if *target != edge.source
                            && matches!(
                                node_kind_by_id.get(target).copied(),
                                Some(NodeKind::Method)
                            ) =>
                    {
                        self.add_pending_edge(&edge, Some(*target));
                    }
                    candidates if candidates.len() > 1 => {
                        ambiguous_unresolved += 1;
                        tracing::debug!(
                            target_label = %edge.target_name,
                            candidates = candidates.len(),
                            "spur-graph: ambiguous dyn trait call target; leaving unresolved"
                        );
                        self.add_pending_edge(&edge, None);
                    }
                    _ => {
                        self.add_pending_edge(&edge, None);
                    }
                }
            } else if edge.edge_kind == Some(GraphEdgeKind::ReferencesAddress) {
                resolve_address_reference_edge(self, &edge, &indexes, &mut ambiguous_unresolved);
            } else if edge.relation == RelationKind::References {
                let allow_reference_fallbacks = reference_fallbacks_allowed(&edge, &file_by_id);
                if let Some(target) = singleton_symbols_by_label.get(&edge.target_name).copied() {
                    if target != edge.source
                        && matches!(
                            node_kind_by_id.get(&target).copied(),
                            Some(NodeKind::Function | NodeKind::Method)
                        )
                    {
                        let provably_unsafe = matches!(
                            (file_by_id.get(&edge.source).copied(), file_by_id.get(&target).copied()),
                            (Some(src_file), Some(tgt_file))
                                if !function_singleton_safe(src_file, tgt_file)
                        );
                        if provably_unsafe {
                            phantom_blocked_references += 1;
                            self.add_pending_edge(&edge, None);
                        } else {
                            self.add_pending_edge(&edge, Some(target));
                        }
                    } else if target != edge.source && allow_reference_fallbacks {
                        self.add_pending_edge(&edge, Some(target));
                    }
                } else if allow_reference_fallbacks {
                    self.add_pending_edge(&edge, None);
                }
            } else if edge.relation == RelationKind::Calls {
                let candidates = qualified_edge_candidates(&edge, &qualified_symbols_by_name);
                match candidates.as_slice() {
                    [target] if *target != edge.source => {
                        let kind = node_kind_by_id.get(target).copied();
                        if matches!(
                            kind,
                            Some(NodeKind::Struct | NodeKind::EnumVariant | NodeKind::Class)
                        ) {
                            self.add_pending_edge_as(
                                &edge,
                                Some(*target),
                                RelationKind::Constructs,
                                None,
                            );
                        } else if matches!(kind, Some(NodeKind::Function | NodeKind::Method)) {
                            self.add_pending_edge_with_bind_method(
                                &edge,
                                Some(*target),
                                Some("fqn"),
                            );
                        } else {
                            self.add_pending_edge(&edge, None);
                        }
                    }
                    candidates if candidates.len() > 1 => {
                        ambiguous_unresolved += 1;
                        tracing::debug!(
                            target_label = %edge.target_name,
                            candidates = candidates.len(),
                            "spur-graph: ambiguous qualified pending edge target; leaving unresolved"
                        );
                        self.add_pending_edge(&edge, None);
                    }
                    _ => {
                        resolve_call_edge_after_qualified_miss(
                            self,
                            &edge,
                            &indexes,
                            &mut ambiguous_unresolved,
                            &mut phantom_blocked_calls,
                        );
                    }
                }
            } else if relational_target_kinds(edge.relation).is_some()
                || matches!(
                    edge.relation,
                    RelationKind::Imports | RelationKind::Constructs
                )
            {
                resolve_bare_pending_edge(
                    self,
                    &edge,
                    &indexes,
                    &mut ambiguous_unresolved,
                    &mut phantom_blocked_calls,
                );
            } else if let Some(candidates) =
                ambiguous_symbols_by_label.get(&edge.target_name).copied()
            {
                ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    candidates,
                    "spur-graph: ambiguous pending edge target; leaving unresolved"
                );
                self.add_pending_edge(&edge, None);
            } else {
                resolve_bare_pending_edge(
                    self,
                    &edge,
                    &indexes,
                    &mut ambiguous_unresolved,
                    &mut phantom_blocked_calls,
                );
            }
        }
        if ambiguous_unresolved > 0 {
            tracing::info!(
                ambiguous_unresolved,
                "spur-graph: left ambiguous pending edges unresolved"
            );
        }
        if phantom_blocked_references > 0 || phantom_blocked_calls > 0 {
            tracing::info!(
                phantom_blocked_references,
                phantom_blocked_calls,
                "spur-graph: crate-scope gate (measurement-only) would have blocked phantom singleton binds"
            );
        }
    }
}

fn resolve_address_reference_edge(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    ambiguous_unresolved: &mut usize,
) {
    let candidates = address_reference_candidates(builder, edge, indexes);
    let source_directory = file_path_for_node(builder, edge.source, indexes)
        .map(|path| parent_directory(&path).to_owned());
    let module_scoped = candidates
        .iter()
        .copied()
        .filter(|target| {
            source_directory.as_deref().is_some_and(|source_directory| {
                file_path_for_node(builder, *target, indexes)
                    .is_some_and(|path| parent_directory(&path) == source_directory)
            })
        })
        .collect::<Vec<_>>();
    match module_scoped.as_slice() {
        [target] => {
            builder.add_pending_edge_with_bind_method(
                edge,
                Some(*target),
                Some("address_module_scope"),
            );
        }
        [] => match candidates.as_slice() {
            [target] => {
                builder.add_pending_edge_with_bind_method(
                    edge,
                    Some(*target),
                    Some("address_singleton"),
                );
            }
            [] => {
                builder.add_pending_edge(edge, None);
            }
            candidates => {
                *ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    candidates = candidates.len(),
                    "spur-graph: ambiguous address reference target; leaving unresolved"
                );
                builder.add_pending_edge(edge, None);
            }
        },
        duplicates => {
            *ambiguous_unresolved += 1;
            tracing::debug!(
                target_label = %edge.target_name,
                candidates = duplicates.len(),
                "spur-graph: duplicate address within one module directory; leaving unresolved"
            );
            builder.add_pending_edge(edge, None);
        }
    }
}

fn address_reference_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let Some(source_family) =
        file_path_for_node(builder, edge.source, indexes).and_then(|path| language_family(&path))
    else {
        return Vec::new();
    };
    let mut candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            matches!(
                indexes.node_kind_by_id.get(target).copied(),
                Some(NodeKind::Resource | NodeKind::Constant)
            )
        })
        .filter(|target| {
            file_path_for_node(builder, *target, indexes)
                .as_deref()
                .and_then(language_family)
                == Some(source_family)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn parent_directory(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

fn reference_fallbacks_allowed(edge: &PendingEdge, file_by_id: &HashMap<NodeId, &str>) -> bool {
    edge.edge_kind != Some(GraphEdgeKind::ReferencesHof)
        || file_by_id
            .get(&edge.source)
            .is_some_and(|path| path.ends_with(".sql"))
}

fn resolve_call_edge_after_qualified_miss(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    ambiguous_unresolved: &mut usize,
    phantom_blocked_calls: &mut usize,
) {
    let candidates = method_scope_candidates(
        edge,
        builder.symbol_index.get(&edge.target_name),
        indexes.node_kind_by_id,
        indexes.enclosing_scope_by_id,
    );
    match candidates.as_slice() {
        [target] => {
            builder.add_pending_edge_with_bind_method(edge, Some(*target), Some("scope_match"));
        }
        candidates if candidates.len() > 1 => {
            *ambiguous_unresolved += 1;
            tracing::debug!(
                target_label = %edge.target_name,
                candidates = candidates.len(),
                "spur-graph: ambiguous scoped method pending edge target; leaving unresolved"
            );
            builder.add_pending_edge(edge, None);
        }
        _ if edge.scope_text.is_some()
            && edge.receiver_text.is_none()
            && indexes
                .file_by_id
                .get(&edge.source)
                .is_some_and(|path| language_family(path) == Some("rust")) =>
        {
            // A failed explicit Rust path is not evidence for an unrelated
            // bare-name singleton (including newly represented foreign items).
            builder.add_pending_edge(edge, None);
        }
        _ => {
            resolve_bare_pending_edge(
                builder,
                edge,
                indexes,
                ambiguous_unresolved,
                phantom_blocked_calls,
            );
        }
    }
}

fn resolve_bare_pending_edge(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    ambiguous_unresolved: &mut usize,
    phantom_blocked_calls: &mut usize,
) {
    if edge.relation == RelationKind::Calls {
        if resolve_javascript_call_via_import_edge(builder, edge, indexes, ambiguous_unresolved) {
            return;
        }
        let candidates = callable_symbol_candidates(builder, edge, indexes);
        if let Some(target) =
            same_file_duplicate_function_candidate(edge, Some(&candidates), indexes)
        {
            builder.add_pending_edge_with_bind_method(
                edge,
                Some(target),
                Some("same_file_duplicate"),
            );
            return;
        }

        match candidates.as_slice() {
            [target] if *target != edge.source => {
                resolve_singleton_bare_target(
                    builder,
                    edge,
                    *target,
                    indexes,
                    phantom_blocked_calls,
                );
                return;
            }
            candidates if candidates.len() > 1 => {
                *ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    candidates = candidates.len(),
                    "spur-graph: ambiguous callable pending edge target; leaving unresolved"
                );
                builder.add_pending_edge(edge, None);
                return;
            }
            _ => {}
        }
    }

    if edge.relation == RelationKind::Imports {
        if let Some(source_file) = indexes.file_by_id.get(&edge.source).copied() {
            if is_web_import_source_file(source_file) {
                let target = edge
                    .import_path
                    .as_deref()
                    .and_then(|path| web_import_target_path(source_file, path))
                    .and_then(|path| indexes.files_by_label.get(&path).copied())
                    .filter(|target| *target != edge.source);
                if let Some(target) = target {
                    builder.add_pending_edge_with_bind_method(
                        edge,
                        Some(target),
                        Some("import_path"),
                    );
                } else {
                    builder.add_pending_edge(edge, None);
                }
                return;
            }
        }

        let python_bare_import_path = is_python_bare_import_path(edge, indexes);
        if !python_bare_import_path {
            let path_candidates = module_path_resolution_candidates(builder, edge, indexes);
            match path_candidates.as_slice() {
                [target] if *target != edge.source => {
                    builder.add_pending_edge_with_bind_method(
                        edge,
                        Some(*target),
                        Some("import_path"),
                    );
                    return;
                }
                candidates if candidates.len() > 1 => {
                    *ambiguous_unresolved += 1;
                    tracing::debug!(
                        target_label = %edge.target_name,
                        import_path = ?edge.import_path,
                        candidates = candidates.len(),
                        "spur-graph: ambiguous module-path import target; leaving unresolved"
                    );
                    builder.add_pending_edge(edge, None);
                    return;
                }
                _ => {}
            }
        }

        let fallback_edge;
        let resolution_edge = if python_bare_import_path {
            fallback_edge = Some(edge_without_import_path(edge));
            fallback_edge.as_ref().expect("fallback edge")
        } else {
            edge
        };
        let candidates = if edge.import_path.is_some() && !python_bare_import_path {
            type_like_import_resolution_candidates(builder, edge, indexes)
        } else {
            import_resolution_candidates(builder, resolution_edge, indexes)
        };
        match candidates.as_slice() {
            [target] if *target != edge.source => {
                builder.add_pending_edge(resolution_edge, Some(*target));
                return;
            }
            _ => {}
        }

        match reexport_resolution_candidates(builder, edge, indexes) {
            ReexportFollowResult::Resolved(candidates) => match candidates.as_slice() {
                [target] if *target != edge.source => {
                    builder.add_pending_edge_with_bind_method(
                        edge,
                        Some(*target),
                        Some("import_path"),
                    );
                    return;
                }
                candidates if candidates.len() > 1 => {
                    *ambiguous_unresolved += 1;
                    tracing::debug!(
                        target_label = %edge.target_name,
                        import_path = ?edge.import_path,
                        candidates = candidates.len(),
                        "spur-graph: ambiguous re-export import target; leaving unresolved"
                    );
                    add_pending_import_without_python_bare_path(
                        builder,
                        edge,
                        None,
                        python_bare_import_path,
                    );
                    return;
                }
                _ => {}
            },
            ReexportFollowResult::Blocked => {
                *ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    import_path = ?edge.import_path,
                    "spur-graph: blocked re-export import target; leaving unresolved"
                );
                add_pending_import_without_python_bare_path(
                    builder,
                    edge,
                    None,
                    python_bare_import_path,
                );
                return;
            }
            ReexportFollowResult::NoMatch => {}
        }

        match candidates.as_slice() {
            candidates if candidates.len() > 1 => {
                *ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    candidates = candidates.len(),
                    "spur-graph: ambiguous import pending edge target; leaving unresolved"
                );
                builder.add_pending_edge(resolution_edge, None);
                return;
            }
            _ => {
                builder.add_pending_edge(resolution_edge, None);
                return;
            }
        }
    }

    if edge.relation == RelationKind::Constructs {
        let candidates = constructs_symbol_candidates(builder, edge, indexes);
        match candidates.as_slice() {
            [target] => {
                if constructs_language_family_allows(builder, edge, *target, indexes) {
                    builder.add_pending_edge_with_bind_method(
                        edge,
                        Some(*target),
                        Some("constructs_type_singleton"),
                    );
                } else {
                    builder.add_pending_edge(edge, None);
                }
                return;
            }
            candidates if candidates.len() > 1 => {
                *ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = %edge.target_name,
                    candidates = candidates.len(),
                    "spur-graph: ambiguous constructs pending edge target; leaving unresolved"
                );
                builder.add_pending_edge(edge, None);
                return;
            }
            _ => {
                builder.add_pending_edge(edge, None);
                return;
            }
        }
    }

    if let Some(allowed) = relational_target_kinds(edge.relation) {
        let candidates = relational_symbol_candidates(builder, edge, indexes, allowed);
        match candidates.as_slice() {
            [target] if *target != edge.source => {
                builder.add_pending_relational_edge(edge, *target, indexes);
                return;
            }
            cands if cands.len() > 1 => {
                *ambiguous_unresolved += 1;
                builder.add_pending_edge(edge, None);
                return;
            }
            _ => {
                builder.add_pending_edge(edge, None);
                return;
            }
        }
    }

    if let Some(candidates) = indexes
        .ambiguous_symbols_by_label
        .get(&edge.target_name)
        .copied()
    {
        if let Some(target) = same_file_duplicate_function_candidate(
            edge,
            builder.symbol_index.get(&edge.target_name),
            indexes,
        ) {
            builder.add_pending_edge_with_bind_method(
                edge,
                Some(target),
                Some("same_file_duplicate"),
            );
            return;
        }

        *ambiguous_unresolved += 1;
        tracing::debug!(
            target_label = %edge.target_name,
            candidates,
            "spur-graph: ambiguous pending edge target; leaving unresolved"
        );
        builder.add_pending_edge(edge, None);
        return;
    }

    let Some(target) = indexes
        .singleton_symbols_by_label
        .get(&edge.target_name)
        .copied()
        .or_else(|| indexes.files_by_label.get(&edge.target_name).copied())
    else {
        builder.add_pending_edge(edge, None);
        return;
    };

    if target == edge.source {
        return;
    }

    resolve_singleton_bare_target(builder, edge, target, indexes, phantom_blocked_calls);
}

fn resolve_singleton_bare_target(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
    phantom_blocked_calls: &mut usize,
) {
    match indexes.node_kind_by_id.get(&target).copied() {
        Some(NodeKind::Method) => {
            if edge.relation == RelationKind::Calls
                && method_scope_matches(edge, target, indexes.enclosing_scope_by_id)
            {
                builder.add_pending_edge_with_bind_method(edge, Some(target), Some("scope_match"));
            } else if edge.relation == RelationKind::Calls {
                let file_for_node = |node_id| {
                    indexes.file_by_id.get(&node_id).copied().or_else(|| {
                        let file_id = builder
                            .facts
                            .nodes
                            .iter()
                            .find(|node| node.node_id == node_id)?
                            .file_id?;
                        builder
                            .facts
                            .nodes
                            .iter()
                            .find(|node| {
                                node.kind == NodeKind::File && node.file_id == Some(file_id)
                            })
                            .map(|node| node.label.as_str())
                    })
                };
                let same_crate_safe = matches!(
                    (file_for_node(edge.source), file_for_node(target)),
                    (Some(src_file), Some(tgt_file)) if function_singleton_safe(src_file, tgt_file)
                );
                let builtin_method = file_for_node(edge.source)
                    .and_then(|src_file| {
                        crate::extract::languages::Language::from_path(std::path::Path::new(
                            src_file,
                        ))
                    })
                    .is_some_and(|lang| {
                        lang.builtin_method_names()
                            .binary_search(&edge.target_name.as_str())
                            .is_ok()
                    });
                if same_crate_safe && !builtin_method {
                    builder.add_pending_edge_with_bind_method(
                        edge,
                        Some(target),
                        Some("method_crate_singleton"),
                    );
                } else {
                    builder.add_pending_edge(edge, None);
                }
            } else {
                builder.add_pending_edge(edge, Some(target));
            }
        }
        Some(NodeKind::Function) if edge.relation == RelationKind::Calls => {
            let file_for_node = |node_id| {
                indexes.file_by_id.get(&node_id).copied().or_else(|| {
                    let file_id = builder
                        .facts
                        .nodes
                        .iter()
                        .find(|node| node.node_id == node_id)?
                        .file_id?;
                    builder
                        .facts
                        .nodes
                        .iter()
                        .find(|node| node.kind == NodeKind::File && node.file_id == Some(file_id))
                        .map(|node| node.label.as_str())
                })
            };
            let provably_unsafe = matches!(
                (file_for_node(edge.source), file_for_node(target)),
                (Some(src_file), Some(tgt_file)) if !function_singleton_safe(src_file, tgt_file)
            );
            if provably_unsafe {
                *phantom_blocked_calls += 1;
                builder.add_pending_edge(edge, None);
            } else {
                builder.add_pending_edge_with_bind_method(edge, Some(target), Some("singleton"));
            }
        }
        _ => {
            if edge.relation == RelationKind::Calls
                && matches!(
                    indexes.node_kind_by_id.get(&target).copied(),
                    Some(NodeKind::Struct | NodeKind::EnumVariant | NodeKind::Class)
                )
            {
                builder.add_pending_edge_as(edge, Some(target), RelationKind::Constructs, None);
            } else if should_reclassify_python_extends_as_implements(edge, target, indexes) {
                builder.add_pending_edge_as(
                    edge,
                    Some(target),
                    RelationKind::Implements,
                    Some("relational"),
                );
            } else if edge.relation == RelationKind::Calls {
                // P1a: calls only resolve to callable symbols (handled above) or
                // constructible types. Other singleton kinds are misresolutions.
                builder.add_pending_edge(edge, None);
            } else {
                builder.add_pending_edge(edge, Some(target));
            }
        }
    }
}

fn resolve_javascript_call_via_import_edge(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    ambiguous_unresolved: &mut usize,
) -> bool {
    let Some(source_file) = file_path_for_node(builder, edge.source, indexes) else {
        return false;
    };
    if import_language_family(&source_file) != Some(ImportLanguageFamily::Javascript) {
        return false;
    }

    let Some(source_file_node) = builder
        .facts
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::File && node.label == source_file)
        .map(|node| node.node_id)
    else {
        return false;
    };

    let Some(imports) = indexes.pending_imports_by_source.get(&source_file_node) else {
        return false;
    };

    let mut candidates = imports
        .iter()
        .filter(|import| import.target_name == edge.target_name)
        .flat_map(|import| module_path_resolution_candidates(builder, import, indexes))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();

    match candidates.as_slice() {
        [target] if *target != edge.source => {
            builder.add_pending_edge_with_bind_method(edge, Some(*target), Some("import_path"));
            return true;
        }
        candidates if candidates.len() > 1 => {
            *ambiguous_unresolved += 1;
            tracing::debug!(
                target_label = %edge.target_name,
                candidates = candidates.len(),
                "spur-graph: ambiguous javascript import-based call target; leaving unresolved"
            );
            builder.add_pending_edge(edge, None);
            return true;
        }
        _ => {}
    }

    false
}

fn callable_symbol_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let mut candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_callable_target_kind)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn relational_target_kinds(relation: RelationKind) -> Option<&'static [NodeKind]> {
    match relation {
        RelationKind::Implements => Some(&[NodeKind::Trait, NodeKind::Interface]),
        RelationKind::Extends => Some(&[NodeKind::Trait, NodeKind::Interface, NodeKind::Class]),
        _ => None,
    }
}

fn constructs_target_kinds() -> &'static [NodeKind] {
    &[
        NodeKind::Struct,
        NodeKind::Enum,
        NodeKind::EnumVariant,
        NodeKind::Class,
    ]
}

fn constructs_symbol_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let allowed = constructs_target_kinds();
    let mut candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(|kind| allowed.contains(&kind))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn relational_symbol_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    allowed: &[NodeKind],
) -> Vec<NodeId> {
    let source_language_family =
        file_path_for_node(builder, edge.source, indexes).and_then(|path| language_family(&path));
    let mut candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(|kind| allowed.contains(&kind))
        })
        .filter(|target| match source_language_family {
            None => true,
            Some(source_family) => {
                file_path_for_node(builder, *target, indexes)
                    .as_deref()
                    .and_then(language_family)
                    == Some(source_family)
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

impl FactBuilder<'_> {
    fn add_pending_relational_edge(
        &mut self,
        edge: &PendingEdge,
        target: NodeId,
        indexes: &PendingResolutionIndexes<'_>,
    ) {
        if should_reclassify_python_extends_as_implements(edge, target, indexes) {
            self.add_pending_edge_as(
                edge,
                Some(target),
                RelationKind::Implements,
                Some("relational"),
            );
        } else {
            self.add_pending_edge_with_bind_method(edge, Some(target), Some("relational"));
        }
    }
}

fn constructs_language_family_allows(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> bool {
    let Some(src_file) = file_path_for_node(builder, edge.source, indexes) else {
        return false;
    };
    let Some(tgt_file) = file_path_for_node(builder, target, indexes) else {
        return false;
    };
    if src_file == tgt_file {
        return true;
    }
    matches!(
        (language_family(&src_file), language_family(&tgt_file)),
        (Some(src_family), Some(tgt_family)) if src_family == tgt_family
    )
}

fn file_path_for_node(
    builder: &FactBuilder<'_>,
    node_id: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> Option<String> {
    indexes
        .file_by_id
        .get(&node_id)
        .map(|path| (*path).to_owned())
        .or_else(|| {
            let file_id = builder
                .facts
                .nodes
                .iter()
                .find(|node| node.node_id == node_id)?
                .file_id?;
            builder
                .facts
                .nodes
                .iter()
                .find(|node| node.kind == NodeKind::File && node.file_id == Some(file_id))
                .map(|node| node.label.clone())
        })
}

fn should_reclassify_python_extends_as_implements(
    edge: &PendingEdge,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> bool {
    edge.relation == RelationKind::Extends
        && indexes.node_kind_by_id.get(&target).copied() == Some(NodeKind::Interface)
        && indexes
            .file_by_id
            .get(&edge.source)
            .is_some_and(|path| is_python_path(path))
        && indexes
            .file_by_id
            .get(&target)
            .is_some_and(|path| is_python_path(path))
}

fn is_python_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
}

fn import_resolution_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let raw_candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_candidate_kind)
        })
        .collect::<Vec<_>>();

    let mut candidates = raw_candidates
        .iter()
        .copied()
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_type_like_candidate_kind)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = raw_candidates
            .into_iter()
            .filter(|target| {
                indexes
                    .node_kind_by_id
                    .get(target)
                    .copied()
                    .is_some_and(is_import_resolution_fallback_candidate_kind)
            })
            .collect();
    }

    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn type_like_import_resolution_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let Some(source_file) = indexes.file_by_id.get(&edge.source).copied() else {
        return Vec::new();
    };
    let mut candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_type_like_candidate_kind)
        })
        .filter(|target| {
            indexes
                .file_by_id
                .get(target)
                .copied()
                .is_some_and(|target_file| {
                    import_path_language_family_allows(source_file, target_file)
                })
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn module_path_resolution_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let Some(import_path) = edge.import_path.as_deref().map(str::trim) else {
        return Vec::new();
    };
    if import_path.is_empty() {
        return Vec::new();
    }
    let Some(source_file) = indexes.file_by_id.get(&edge.source).copied() else {
        return Vec::new();
    };

    let raw_candidates = builder
        .symbol_index
        .get(&edge.target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| *target != edge.source)
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_candidate_kind)
        })
        .filter(|target| {
            let Some(target_file) = indexes.file_by_id.get(target).copied() else {
                return false;
            };
            import_path_language_family_allows(source_file, target_file)
                && import_path_matches_target(
                    import_path,
                    edge,
                    source_file,
                    target_file,
                    *target,
                    indexes,
                )
        })
        .collect::<Vec<_>>();

    narrow_import_candidates(raw_candidates, indexes, true)
}

fn narrow_import_candidates(
    raw_candidates: Vec<NodeId>,
    indexes: &PendingResolutionIndexes<'_>,
    allow_fallback_candidates: bool,
) -> Vec<NodeId> {
    let mut candidates = raw_candidates
        .iter()
        .copied()
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_type_like_candidate_kind)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() && allow_fallback_candidates {
        candidates = raw_candidates
            .into_iter()
            .filter(|target| {
                indexes
                    .node_kind_by_id
                    .get(target)
                    .copied()
                    .is_some_and(is_import_resolution_fallback_candidate_kind)
            })
            .collect();
    }

    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn is_python_bare_import_path(edge: &PendingEdge, indexes: &PendingResolutionIndexes<'_>) -> bool {
    let Some(import_path) = edge.import_path.as_deref() else {
        return false;
    };
    if !is_bare_single_segment_import_path(import_path) {
        return false;
    }
    indexes
        .file_by_id
        .get(&edge.source)
        .and_then(|source_file| import_language_family(source_file))
        == Some(ImportLanguageFamily::Python)
}

fn is_bare_single_segment_import_path(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && !path.starts_with('.')
        && !path.contains("::")
        && !path.contains('.')
        && !path.contains('/')
        && !path.contains('\\')
        && !path.contains('{')
}

fn add_pending_import_without_python_bare_path(
    builder: &mut FactBuilder<'_>,
    edge: &PendingEdge,
    target: Option<NodeId>,
    scrub_import_path: bool,
) {
    if scrub_import_path {
        let fallback_edge = edge_without_import_path(edge);
        builder.add_pending_edge(&fallback_edge, target);
    } else {
        builder.add_pending_edge(edge, target);
    }
}

fn edge_without_import_path(edge: &PendingEdge) -> PendingEdge {
    let mut edge = edge.clone();
    edge.import_path = None;
    edge
}

enum ReexportFollowResult {
    NoMatch,
    Resolved(Vec<NodeId>),
    Blocked,
}

fn reexport_resolution_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> ReexportFollowResult {
    let mut visited = HashSet::new();
    reexport_resolution_candidates_at_depth(builder, edge, indexes, 0, &mut visited)
}

fn reexport_resolution_candidates_at_depth(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    if depth > MAX_REEXPORT_FOLLOW_DEPTH {
        return ReexportFollowResult::Blocked;
    }
    if edge
        .import_path
        .as_deref()
        .is_none_or(|path| path.trim().is_empty())
    {
        return ReexportFollowResult::NoMatch;
    }
    let sources = reexport_source_candidates(builder, edge, indexes);
    follow_reexport_sources(
        builder,
        &sources,
        &edge.target_name,
        indexes,
        depth,
        visited,
    )
}

fn follow_reexport_sources(
    builder: &FactBuilder<'_>,
    sources: &[NodeId],
    target_name: &str,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    if sources.is_empty() {
        return ReexportFollowResult::NoMatch;
    }

    let mut candidates = Vec::new();
    let mut matched = false;
    for source in sources {
        match follow_reexported_name(builder, *source, target_name, indexes, depth, visited) {
            ReexportFollowResult::NoMatch => {}
            ReexportFollowResult::Resolved(mut resolved) => {
                matched = true;
                candidates.append(&mut resolved);
            }
            ReexportFollowResult::Blocked => return ReexportFollowResult::Blocked,
        }
    }

    if !matched {
        return ReexportFollowResult::NoMatch;
    }
    finalize_reexport_candidates(candidates)
}

fn follow_reexported_name(
    builder: &FactBuilder<'_>,
    source: NodeId,
    target_name: &str,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    if depth > MAX_REEXPORT_FOLLOW_DEPTH {
        return ReexportFollowResult::Blocked;
    }
    let key = (source, target_name.to_owned());
    if !visited.insert(key.clone()) {
        return ReexportFollowResult::Blocked;
    }

    let result =
        follow_reexported_name_inner(builder, source, target_name, indexes, depth, visited);
    visited.remove(&key);
    result
}

fn follow_reexported_name_inner(
    builder: &FactBuilder<'_>,
    source: NodeId,
    target_name: &str,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    let Some(imports) = indexes.pending_imports_by_source.get(&source) else {
        return ReexportFollowResult::NoMatch;
    };

    let mut candidates = Vec::new();
    let mut matched = false;
    for import in imports {
        let result = if import.target_name == target_name {
            resolve_reexport_edge_target(builder, import, indexes, depth + 1, visited)
        } else if is_glob_reexport(import) {
            resolve_glob_reexport_targets(builder, import, target_name, indexes, depth + 1, visited)
        } else {
            ReexportFollowResult::NoMatch
        };

        match result {
            ReexportFollowResult::NoMatch => {}
            ReexportFollowResult::Resolved(mut resolved) => {
                matched = true;
                candidates.append(&mut resolved);
            }
            ReexportFollowResult::Blocked => return ReexportFollowResult::Blocked,
        }
    }

    if !matched {
        return ReexportFollowResult::NoMatch;
    }
    finalize_reexport_candidates(candidates)
}

fn resolve_reexport_edge_target(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    if depth > MAX_REEXPORT_FOLLOW_DEPTH {
        return ReexportFollowResult::Blocked;
    }

    let candidates = module_path_resolution_candidates(builder, edge, indexes);
    match candidates.as_slice() {
        [_] => ReexportFollowResult::Resolved(candidates),
        candidates if candidates.len() > 1 => ReexportFollowResult::Blocked,
        _ => reexport_resolution_candidates_at_depth(builder, edge, indexes, depth, visited),
    }
}

fn resolve_glob_reexport_targets(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    target_name: &str,
    indexes: &PendingResolutionIndexes<'_>,
    depth: usize,
    visited: &mut HashSet<(NodeId, String)>,
) -> ReexportFollowResult {
    if depth > MAX_REEXPORT_FOLLOW_DEPTH {
        return ReexportFollowResult::Blocked;
    }

    let sources = reexport_source_candidates(builder, edge, indexes);
    if sources.is_empty() {
        return ReexportFollowResult::NoMatch;
    }

    let mut candidates =
        direct_reexported_symbol_candidates(builder, target_name, &sources, indexes);
    let mut matched = !candidates.is_empty();
    for source in sources {
        match follow_reexported_name(builder, source, target_name, indexes, depth, visited) {
            ReexportFollowResult::NoMatch => {}
            ReexportFollowResult::Resolved(mut resolved) => {
                matched = true;
                candidates.append(&mut resolved);
            }
            ReexportFollowResult::Blocked => return ReexportFollowResult::Blocked,
        }
    }

    if !matched {
        return ReexportFollowResult::NoMatch;
    }
    finalize_reexport_candidates(candidates)
}

fn finalize_reexport_candidates(mut candidates: Vec<NodeId>) -> ReexportFollowResult {
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    match candidates.len() {
        0 => ReexportFollowResult::NoMatch,
        1 => ReexportFollowResult::Resolved(candidates),
        _ => ReexportFollowResult::Blocked,
    }
}

fn is_glob_reexport(edge: &PendingEdge) -> bool {
    edge.target_name == "*"
        || edge.import_path.as_deref().is_some_and(|path| {
            let path = path.trim();
            path.ends_with("::*") || path.ends_with(".*")
        })
}

fn reexport_source_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let Some(import_path) = edge.import_path.as_deref().map(str::trim) else {
        return Vec::new();
    };
    if import_path.is_empty() {
        return Vec::new();
    }
    let Some(source_file) = indexes.file_by_id.get(&edge.source).copied() else {
        return Vec::new();
    };

    let mut candidates = match import_language_family(source_file) {
        Some(ImportLanguageFamily::Rust) => {
            rust_reexport_source_candidates(builder, edge, import_path, source_file, indexes)
        }
        Some(ImportLanguageFamily::Python) => {
            python_reexport_source_candidates(builder, edge, import_path, source_file)
        }
        Some(ImportLanguageFamily::Javascript) => {
            javascript_reexport_source_candidates(builder, import_path, source_file)
        }
        None => Vec::new(),
    };
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn rust_reexport_source_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    import_path: &str,
    source_file: &str,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let Some(segments) =
        rust_absolute_import_segments(import_path, &edge.target_name, edge, source_file, indexes)
    else {
        return Vec::new();
    };
    let module_segments = reexport_module_segments(&segments, &edge.target_name);
    rust_source_candidates_for_module_segments(builder, source_file, &module_segments, indexes)
}

fn reexport_module_segments(segments: &[String], target_name: &str) -> Vec<String> {
    let mut module_segments = segments.to_vec();
    if module_segments
        .last()
        .is_some_and(|segment| segment == target_name || segment == "*")
    {
        module_segments.pop();
    }
    module_segments
}

fn rust_source_candidates_for_module_segments(
    builder: &FactBuilder<'_>,
    source_file: &str,
    module_segments: &[String],
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let qualified_module = module_segments.join("::");
    builder
        .facts
        .nodes
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::File
                if path_scope(source_file) == path_scope(&node.label)
                    && rust_module_file_matches(source_file, &node.label, module_segments) =>
            {
                Some(node.node_id)
            }
            NodeKind::Module
                if indexes
                    .qualified_name_by_id
                    .get(&node.node_id)
                    .is_some_and(|qualified_name| qualified_name == &qualified_module)
                    && indexes.file_by_id.get(&node.node_id).copied().is_some_and(
                        |module_file| path_scope(source_file) == path_scope(module_file),
                    ) =>
            {
                Some(node.node_id)
            }
            _ => None,
        })
        .collect()
}

fn python_reexport_source_candidates(
    builder: &FactBuilder<'_>,
    edge: &PendingEdge,
    import_path: &str,
    source_file: &str,
) -> Vec<NodeId> {
    let module_candidates =
        python_import_module_candidates(import_path, &edge.target_name, source_file);
    builder
        .facts
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .filter(|node| {
            module_candidates
                .iter()
                .any(|candidate| candidate.matches_python_file(&node.label))
        })
        .map(|node| node.node_id)
        .collect()
}

fn javascript_reexport_source_candidates(
    builder: &FactBuilder<'_>,
    import_path: &str,
    source_file: &str,
) -> Vec<NodeId> {
    let module_candidates = javascript_import_module_candidates(import_path, source_file);
    builder
        .facts
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .filter(|node| {
            module_candidates
                .iter()
                .any(|candidate| candidate.matches_javascript_file(&node.label))
        })
        .map(|node| node.node_id)
        .collect()
}

fn direct_reexported_symbol_candidates(
    builder: &FactBuilder<'_>,
    target_name: &str,
    sources: &[NodeId],
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<NodeId> {
    let raw_candidates = builder
        .symbol_index
        .get(target_name)
        .into_iter()
        .flat_map(|ids| ids.iter().copied())
        .filter(|target| {
            indexes
                .node_kind_by_id
                .get(target)
                .copied()
                .is_some_and(is_import_resolution_candidate_kind)
        })
        .filter(|target| {
            sources
                .iter()
                .any(|source| target_belongs_to_reexport_source(*source, *target, indexes))
        })
        .collect::<Vec<_>>();

    narrow_import_candidates(raw_candidates, indexes, true)
}

fn target_belongs_to_reexport_source(
    source: NodeId,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> bool {
    match indexes.node_kind_by_id.get(&source).copied() {
        Some(NodeKind::File) => indexes.file_by_id.get(&source) == indexes.file_by_id.get(&target),
        Some(NodeKind::Module) => {
            let Some(source_file) = indexes.file_by_id.get(&source) else {
                return false;
            };
            let Some(target_file) = indexes.file_by_id.get(&target) else {
                return false;
            };
            if source_file != target_file {
                return false;
            }
            let Some(source_qualified_name) = indexes.qualified_name_by_id.get(&source) else {
                return false;
            };
            indexes
                .qualified_name_by_id
                .get(&target)
                .is_some_and(|target_qualified_name| {
                    target_qualified_name
                        .strip_prefix(source_qualified_name)
                        .is_some_and(|suffix| suffix.starts_with("::"))
                })
        }
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportLanguageFamily {
    Rust,
    Python,
    Javascript,
}

pub(crate) fn is_web_import_source_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "css"
            )
        })
}

fn is_local_web_import_path(import_path: &str) -> bool {
    let import_path = import_path.trim();
    if import_path.is_empty() || import_path.starts_with("//") {
        return false;
    }
    if import_path.starts_with('#') {
        return true;
    }
    let Some((scheme, _)) = import_path.split_once(':') else {
        return true;
    };
    !scheme.starts_with(|character: char| character.is_ascii_alphabetic())
        || !scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

pub(crate) fn web_import_target_path(source_file: &str, import_path: &str) -> Option<String> {
    if !is_web_import_source_file(source_file) || !is_local_web_import_path(import_path) {
        return None;
    }

    let import_path = import_path.trim();
    // A leading slash is document-root-relative, not repository-root-relative.
    // Without a configured web root there is no sound workspace file binding.
    if import_path.starts_with('/') {
        return None;
    }
    let import_path = import_path.split(['?', '#']).next().unwrap_or_default();
    if import_path.is_empty() {
        return None;
    }
    let path = Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(import_path);

    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(segment) => {
                normalized.push(segment.to_string_lossy().into_owned())
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop()?;
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    (!normalized.is_empty()).then(|| normalized.join("/"))
}

fn import_path_language_family_allows(source_file: &str, target_file: &str) -> bool {
    let Some(source_family) = import_language_family(source_file) else {
        return false;
    };
    import_language_family(target_file) == Some(source_family)
}

fn import_language_family(path: &str) -> Option<ImportLanguageFamily> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("rs") => Some(ImportLanguageFamily::Rust),
        Some("py") => Some(ImportLanguageFamily::Python),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => Some(ImportLanguageFamily::Javascript),
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn classify_import_origin(
    import_path: &str,
    source_file: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> ImportOriginClassification {
    let import_path = import_path.trim();
    if is_web_import_source_file(source_file) {
        return if import_path.to_ascii_lowercase().starts_with("data:")
            || is_local_web_import_path(import_path)
        {
            ImportOriginClassification::Internal
        } else {
            external_import_origin(import_path, import_path, import_path)
        };
    }
    match import_language_family(source_file) {
        Some(ImportLanguageFamily::Rust) => {
            classify_rust_import_origin(import_path, workspace_index)
        }
        Some(ImportLanguageFamily::Python) => {
            classify_python_import_origin(import_path, workspace_index)
        }
        Some(ImportLanguageFamily::Javascript) => {
            classify_javascript_import_origin(import_path, workspace_index)
        }
        None => external_import_origin(import_path, import_path, import_path),
    }
}

fn classify_rust_import_origin(
    import_path: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> ImportOriginClassification {
    let segments = rust_import_path_origin_segments(import_path);
    let Some(first) = segments.first() else {
        return external_import_origin("", import_path, "");
    };
    if matches!(first.as_str(), "crate" | "self" | "super" | "$crate")
        || workspace_index.rust_crates.contains(first)
    {
        return ImportOriginClassification::Internal;
    }

    let origin = if matches!(first.as_str(), "std" | "core" | "alloc") {
        "std"
    } else {
        first
    };
    let name = segments.last().map_or(first.as_str(), String::as_str);
    external_import_origin(origin, &segments.join("::"), name)
}

fn rust_import_path_origin_segments(import_path: &str) -> Vec<String> {
    let mut path = import_path.trim();
    if let Some((prefix, _)) = path.split_once('{') {
        path = prefix.trim_end_matches(':').trim();
    }
    path.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.trim_start_matches("r#").to_owned())
        .collect()
}

fn classify_javascript_import_origin(
    import_path: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> ImportOriginClassification {
    if import_path.starts_with("./")
        || import_path.starts_with("../")
        || import_path.starts_with('/')
    {
        return ImportOriginClassification::Internal;
    }
    if workspace_index
        .javascript_path_aliases
        .iter()
        .any(|alias| javascript_path_alias_matches(import_path, alias))
        || workspace_index
            .javascript_workspace_roots
            .iter()
            .any(|root| path_has_segment_prefix(import_path, root, '/'))
    {
        return ImportOriginClassification::Internal;
    }

    let (origin, name) = javascript_external_identity(import_path);
    external_import_origin(&origin, import_path, &name)
}

fn javascript_path_alias_matches(import_path: &str, alias: &str) -> bool {
    let alias = alias.trim();
    if alias.is_empty() {
        return false;
    }
    let prefix = alias.strip_suffix('*').unwrap_or(alias);
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return false;
    }
    import_path == prefix || import_path.starts_with(&format!("{prefix}/"))
}

fn javascript_external_identity(import_path: &str) -> (String, String) {
    let segments = split_path_like_segments(import_path);
    if import_path.starts_with('@') && segments.len() >= 2 {
        return (
            format!("{}/{}", segments[0], segments[1]),
            segments
                .last()
                .cloned()
                .unwrap_or_else(|| import_path.to_owned()),
        );
    }
    let origin = segments
        .first()
        .cloned()
        .unwrap_or_else(|| import_path.to_owned());
    let name = segments.last().cloned().unwrap_or_else(|| origin.clone());
    (origin, name)
}

fn classify_python_import_origin(
    import_path: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> ImportOriginClassification {
    if import_path.starts_with('.') || python_import_matches_workspace(import_path, workspace_index)
    {
        return ImportOriginClassification::Internal;
    }

    let segments = split_module_path(import_path, '.');
    let origin = segments
        .first()
        .cloned()
        .unwrap_or_else(|| import_path.to_owned());
    let name = segments.last().cloned().unwrap_or_else(|| origin.clone());
    external_import_origin(&origin, import_path, &name)
}

fn python_import_matches_workspace(
    import_path: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> bool {
    workspace_index
        .python_modules
        .iter()
        .any(|module| path_has_segment_prefix(import_path, module, '.'))
}

fn path_has_segment_prefix(path: &str, prefix: &str, separator: char) -> bool {
    let prefix = prefix.trim().trim_matches(separator);
    if prefix.is_empty() {
        return false;
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with(separator))
}

fn external_import_origin(origin: &str, path: &str, name: &str) -> ImportOriginClassification {
    ImportOriginClassification::External {
        origin: origin.to_owned(),
        path: path.to_owned(),
        name: name.to_owned(),
    }
}

fn import_path_matches_target(
    import_path: &str,
    edge: &PendingEdge,
    source_file: &str,
    target_file: &str,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> bool {
    match import_language_family(source_file) {
        Some(ImportLanguageFamily::Rust) => rust_import_path_matches_target(
            import_path,
            edge,
            source_file,
            target_file,
            target,
            indexes,
        ),
        Some(ImportLanguageFamily::Python) => {
            python_import_path_matches_target(import_path, edge, source_file, target_file)
        }
        Some(ImportLanguageFamily::Javascript) => {
            javascript_import_path_matches_target(import_path, source_file, target_file)
        }
        None => false,
    }
}

fn rust_import_path_matches_target(
    import_path: &str,
    edge: &PendingEdge,
    source_file: &str,
    target_file: &str,
    target: NodeId,
    indexes: &PendingResolutionIndexes<'_>,
) -> bool {
    let Some(segments) =
        rust_absolute_import_segments(import_path, &edge.target_name, edge, source_file, indexes)
    else {
        return false;
    };
    if segments.is_empty() {
        return false;
    }
    if path_scope(source_file) != path_scope(target_file) {
        return false;
    }
    let qualified_import_path = segments.join("::");
    if indexes
        .qualified_name_by_id
        .get(&target)
        .is_some_and(|qualified_name| qualified_name == &qualified_import_path)
    {
        return true;
    }
    if segments
        .last()
        .is_none_or(|segment| segment != &edge.target_name)
    {
        return false;
    }
    rust_module_file_matches(source_file, target_file, &segments[..segments.len() - 1])
}

fn rust_absolute_import_segments(
    import_path: &str,
    target_name: &str,
    edge: &PendingEdge,
    source_file: &str,
    indexes: &PendingResolutionIndexes<'_>,
) -> Option<Vec<String>> {
    let segments = rust_import_path_segments(import_path, target_name);
    let (first, rest) = segments.split_first()?;
    let current_module = rust_current_module_segments(edge, source_file, indexes);
    let mut resolved = match first.as_str() {
        "crate" => rest.to_vec(),
        "self" => {
            let mut segments = current_module;
            segments.extend_from_slice(rest);
            segments
        }
        "super" => {
            let mut segments = current_module;
            segments.pop();
            let mut remainder = rest;
            while let Some((next, next_rest)) = remainder.split_first() {
                if next != "super" {
                    break;
                }
                segments.pop();
                remainder = next_rest;
            }
            segments.extend_from_slice(remainder);
            segments
        }
        root if rust_source_crate_name(source_file).as_deref() == Some(root) => rest.to_vec(),
        _ => segments,
    };
    if resolved.last().is_none_or(|segment| segment != target_name) {
        resolved.push(target_name.to_owned());
    }
    Some(resolved)
}

fn rust_import_path_segments(import_path: &str, target_name: &str) -> Vec<String> {
    let mut path = import_path.trim();
    if let Some((prefix, _)) = path.split_once('{') {
        path = prefix.trim_end_matches(':').trim();
    }
    let mut segments = path
        .split("::")
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.trim_start_matches("r#").to_owned())
        .collect::<Vec<_>>();
    if segments.last().is_none_or(|segment| segment != target_name) {
        segments.push(target_name.to_owned());
    }
    segments
}

fn rust_current_module_segments(
    edge: &PendingEdge,
    source_file: &str,
    indexes: &PendingResolutionIndexes<'_>,
) -> Vec<String> {
    if indexes.node_kind_by_id.get(&edge.source).copied() == Some(NodeKind::Module) {
        if let Some(qualified_name) = indexes.qualified_name_by_id.get(&edge.source) {
            return qualified_name
                .split("::")
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    rust_file_module_segments(source_file)
}

fn rust_file_module_segments(source_file: &str) -> Vec<String> {
    let Some(src_relative) = rust_src_relative_path(source_file) else {
        return Vec::new();
    };
    if matches!(src_relative, "lib.rs" | "main.rs") {
        return Vec::new();
    }
    let module_path = src_relative
        .strip_suffix("/mod.rs")
        .or_else(|| src_relative.strip_suffix(".rs"))
        .unwrap_or(src_relative);
    module_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn rust_src_relative_path(source_file: &str) -> Option<&str> {
    source_file
        .strip_prefix("src/")
        .or_else(|| source_file.split_once("/src/").map(|(_, path)| path))
}

fn rust_crate_root(source_file: &str) -> Option<&str> {
    if source_file.starts_with("src/") {
        return Some("");
    }
    source_file.split_once("/src/").map(|(root, _)| root)
}

fn rust_source_crate_name(source_file: &str) -> Option<String> {
    let root = rust_crate_root(source_file)?;
    let crate_dir = root
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())?;
    Some(crate_dir.replace('-', "_"))
}

fn rust_module_file_matches(
    source_file: &str,
    target_file: &str,
    module_segments: &[String],
) -> bool {
    let Some(crate_root) = rust_crate_root(source_file) else {
        return false;
    };
    let src_prefix = if crate_root.is_empty() {
        "src".to_owned()
    } else {
        format!("{crate_root}/src")
    };
    if module_segments.is_empty() {
        return target_file == format!("{src_prefix}/lib.rs")
            || target_file == format!("{src_prefix}/main.rs");
    }
    let module_path = module_segments.join("/");
    target_file == format!("{src_prefix}/{module_path}.rs")
        || target_file == format!("{src_prefix}/{module_path}/mod.rs")
}

fn python_import_path_matches_target(
    import_path: &str,
    edge: &PendingEdge,
    source_file: &str,
    target_file: &str,
) -> bool {
    python_import_module_candidates(import_path, &edge.target_name, source_file)
        .into_iter()
        .any(|candidate| candidate.matches_python_file(target_file))
}

enum PythonModuleCandidate {
    Exact(Vec<String>),
    Suffix(Vec<String>),
}

impl PythonModuleCandidate {
    fn matches_python_file(&self, target_file: &str) -> bool {
        match self {
            Self::Exact(segments) => module_file_matches_exact(target_file, segments, &["py"]),
            Self::Suffix(segments) => module_file_matches_suffix(target_file, segments, &["py"]),
        }
    }
}

fn python_import_module_candidates(
    import_path: &str,
    target_name: &str,
    source_file: &str,
) -> Vec<PythonModuleCandidate> {
    let import_path = import_path.trim();
    if import_path.starts_with('.') {
        let dot_count = import_path.chars().take_while(|ch| *ch == '.').count();
        let rest = import_path[dot_count..].trim_start_matches('.');
        let mut base = path_dir_segments(source_file);
        for _ in 1..dot_count {
            base.pop();
        }
        let mut module = base.clone();
        module.extend(split_module_path(rest, '.'));
        let mut with_target = module.clone();
        with_target.push(target_name.to_owned());
        return vec![
            PythonModuleCandidate::Exact(module),
            PythonModuleCandidate::Exact(with_target),
        ];
    }

    let module = split_module_path(import_path, '.');
    let mut candidates = vec![PythonModuleCandidate::Suffix(module.clone())];
    if module.last().is_some_and(|segment| segment == target_name) && module.len() > 1 {
        candidates.push(PythonModuleCandidate::Suffix(
            module[..module.len() - 1].to_vec(),
        ));
    } else {
        let mut with_target = module;
        with_target.push(target_name.to_owned());
        candidates.push(PythonModuleCandidate::Suffix(with_target));
    }
    candidates
}

fn javascript_import_path_matches_target(
    import_path: &str,
    source_file: &str,
    target_file: &str,
) -> bool {
    javascript_import_module_candidates(import_path, source_file)
        .into_iter()
        .any(|candidate| candidate.matches_javascript_file(target_file))
}

enum JavascriptModuleCandidate {
    Exact(Vec<String>),
    Suffix(Vec<String>),
}

impl JavascriptModuleCandidate {
    fn matches_javascript_file(&self, target_file: &str) -> bool {
        let extensions = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
        match self {
            Self::Exact(segments) => {
                module_file_matches_exact(target_file, segments, &extensions)
                    || module_file_with_known_extension_matches_exact(target_file, segments)
            }
            Self::Suffix(segments) => {
                module_file_matches_suffix(target_file, segments, &extensions)
                    || module_file_with_known_extension_matches_suffix(target_file, segments)
            }
        }
    }
}

fn javascript_import_module_candidates(
    import_path: &str,
    source_file: &str,
) -> Vec<JavascriptModuleCandidate> {
    let import_path = import_path.trim();
    let path = import_path.strip_prefix("@/").unwrap_or(import_path);
    let segments = split_path_like_segments(path);
    if path.starts_with("./") || path.starts_with("../") {
        let mut base = path_dir_segments(source_file);
        base.extend(segments);
        normalize_module_segments(&mut base);
        vec![JavascriptModuleCandidate::Exact(base)]
    } else {
        vec![JavascriptModuleCandidate::Suffix(segments)]
    }
}

fn split_module_path(path: &str, separator: char) -> Vec<String> {
    path.split(separator)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn split_path_like_segments(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn path_dir_segments(path: &str) -> Vec<String> {
    path.rsplit_once('/')
        .map(|(dir, _)| split_path_like_segments(dir))
        .unwrap_or_default()
}

fn normalize_module_segments(segments: &mut Vec<String>) {
    let mut normalized = Vec::new();
    for segment in std::mem::take(segments) {
        match segment.as_str() {
            "." => {}
            ".." => {
                normalized.pop();
            }
            _ => normalized.push(segment),
        }
    }
    *segments = normalized;
}

fn module_file_matches_exact(
    target_file: &str,
    module_segments: &[String],
    extensions: &[&str],
) -> bool {
    if module_segments.is_empty() {
        return false;
    }
    extensions.iter().any(|extension| {
        target_file == format!("{}.{}", module_segments.join("/"), extension)
            || target_file == format!("{}/__init__.{}", module_segments.join("/"), extension)
            || target_file == format!("{}/index.{}", module_segments.join("/"), extension)
    })
}

fn module_file_matches_suffix(
    target_file: &str,
    module_segments: &[String],
    extensions: &[&str],
) -> bool {
    if module_segments.is_empty() {
        return false;
    }
    extensions.iter().any(|extension| {
        let module_path = module_segments.join("/");
        target_file == format!("{module_path}.{extension}")
            || target_file.ends_with(&format!("/{module_path}.{extension}"))
            || target_file == format!("{module_path}/__init__.{extension}")
            || target_file.ends_with(&format!("/{module_path}/__init__.{extension}"))
            || target_file == format!("{module_path}/index.{extension}")
            || target_file.ends_with(&format!("/{module_path}/index.{extension}"))
    })
}

fn module_file_with_known_extension_matches_exact(
    target_file: &str,
    module_segments: &[String],
) -> bool {
    let Some((last, extension)) = module_segments
        .last()
        .and_then(|segment| segment.rsplit_once('.'))
    else {
        return false;
    };
    if !matches!(extension, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
        return false;
    }
    let mut expected = module_segments[..module_segments.len() - 1].to_vec();
    expected.push(format!("{last}.{extension}"));
    target_file == expected.join("/")
}

fn module_file_with_known_extension_matches_suffix(
    target_file: &str,
    module_segments: &[String],
) -> bool {
    let Some((last, extension)) = module_segments
        .last()
        .and_then(|segment| segment.rsplit_once('.'))
    else {
        return false;
    };
    if !matches!(extension, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
        return false;
    }
    let mut expected = module_segments[..module_segments.len() - 1].to_vec();
    expected.push(format!("{last}.{extension}"));
    let expected = expected.join("/");
    target_file == expected || target_file.ends_with(&format!("/{expected}"))
}

fn is_callable_target_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Struct
            | NodeKind::EnumVariant
            | NodeKind::Macro
    )
}

fn is_import_resolution_candidate_kind(kind: NodeKind) -> bool {
    is_import_resolution_type_like_candidate_kind(kind)
        || is_import_resolution_fallback_candidate_kind(kind)
}

fn is_import_resolution_type_like_candidate_kind(kind: NodeKind) -> bool {
    // Impl blocks share the type's import surface; they are containers, not
    // independently importable definitions.
    matches!(
        kind,
        NodeKind::Module
            | NodeKind::Function
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Struct
            | NodeKind::Enum
            | NodeKind::Trait
            | NodeKind::TypeAlias
            | NodeKind::Macro
    )
}

pub(crate) fn is_import_resolution_fallback_candidate_kind(kind: NodeKind) -> bool {
    match kind {
        NodeKind::Module => false,
        NodeKind::ExternBlock => false,
        NodeKind::Function => false,
        NodeKind::Class => false,
        NodeKind::Interface => false,
        NodeKind::Struct => false,
        NodeKind::Impl => false,
        NodeKind::Trait => false,
        NodeKind::Enum => false,
        NodeKind::EnumVariant => true,
        NodeKind::File => false,
        NodeKind::External => false,
        NodeKind::Method => false,
        NodeKind::Field => false,
        NodeKind::Constant => true,
        NodeKind::TypeAlias => false,
        NodeKind::Macro => false,
        NodeKind::Section => false,
        NodeKind::Commit => false,
        NodeKind::McpTool => false,
        NodeKind::Cell => false,
        NodeKind::Port => false,
        NodeKind::Resource => false,
    }
}

fn same_file_duplicate_function_candidate(
    edge: &PendingEdge,
    candidates: Option<&Vec<NodeId>>,
    indexes: &PendingResolutionIndexes<'_>,
) -> Option<NodeId> {
    if edge.relation != RelationKind::Calls {
        return None;
    }

    let mut candidates = candidates?
        .iter()
        .copied()
        .filter(|target| *target != edge.source)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    if candidates.len() < 2 {
        return None;
    }

    if !candidates.iter().all(|target| {
        matches!(
            indexes.node_kind_by_id.get(target),
            Some(NodeKind::Function)
        )
    }) {
        return None;
    }

    let first_file = indexes.file_by_id.get(&candidates[0]).copied()?;
    if !candidates
        .iter()
        .all(|target| indexes.file_by_id.get(target).copied() == Some(first_file))
    {
        return None;
    }

    let first_qualified_name = indexes.qualified_name_by_id.get(&candidates[0])?;
    if !candidates
        .iter()
        .all(|target| indexes.qualified_name_by_id.get(target) == Some(first_qualified_name))
    {
        return None;
    }

    Some(candidates[0])
}

pub(crate) fn function_singleton_safe(src_file: &str, tgt_file: &str) -> bool {
    if src_file == tgt_file {
        return true;
    }
    let src_crate = path_scope(src_file);
    let tgt_crate = path_scope(tgt_file);
    if src_crate.is_none() || src_crate != tgt_crate {
        return false;
    }
    // Same crate scope is not enough: one crates/<name> tree can hold multiple
    // languages (spur-notebook = Rust rest-table-gateway + TS jute-notebook).
    // A call never crosses a language boundary, so require the same family.
    matches!(
        (language_family(src_file), language_family(tgt_file)),
        (Some(a), Some(b)) if a == b
    )
}

pub(crate) fn language_family(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    if ext == path {
        return None;
    }
    Some(match ext {
        "rs" => "rust",
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "js",
        "py" | "pyi" => "python",
        "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "hxx" => "cpp",
        "md" | "markdown" => "markdown",
        "tf" | "hcl" => "hcl",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "mmd" | "mermaid" => "mermaid",
        _ => return None,
    })
}

fn path_crate(path: &str) -> Option<&str> {
    let stripped = path.strip_prefix("crates/")?;
    let end = stripped.find('/')?;
    if end == 0 {
        return None;
    }
    Some(&stripped[..end])
}

fn path_scope(path: &str) -> Option<&str> {
    if let Some(krate) = path_crate(path) {
        return Some(krate);
    }
    if path.starts_with("crates/") {
        return None;
    }
    path.split('/').next().filter(|segment| !segment.is_empty())
}

fn method_scope_candidates(
    edge: &PendingEdge,
    candidates: Option<&Vec<NodeId>>,
    node_kind_by_id: &HashMap<NodeId, NodeKind>,
    enclosing_scope_by_id: &HashMap<NodeId, String>,
) -> Vec<NodeId> {
    if edge.scope_text.is_none() {
        return Vec::new();
    }
    let Some(candidates) = candidates else {
        return Vec::new();
    };

    let mut matches = candidates
        .iter()
        .copied()
        .filter(|target| *target != edge.source)
        .filter(|target| matches!(node_kind_by_id.get(target).copied(), Some(NodeKind::Method)))
        .filter(|target| method_scope_matches(edge, *target, enclosing_scope_by_id))
        .collect::<Vec<_>>();
    matches.sort_by_key(|id| id.get());
    matches.dedup();
    matches
}

fn qualified_edge_candidates(
    edge: &PendingEdge,
    qualified_symbols_by_name: &HashMap<String, Vec<NodeId>>,
) -> Vec<NodeId> {
    let mut candidates = Vec::new();
    if edge.target_name.contains("::") {
        if let Some(indexed) = qualified_symbols_by_name.get(&edge.target_name) {
            candidates.extend(indexed.iter().copied());
        }
    }
    if let Some(scope_text) = edge.scope_text.as_deref() {
        let qualified_name = format!("{}::{}", scope_text.trim(), edge.target_name);
        if let Some(indexed) = qualified_symbols_by_name.get(&qualified_name) {
            candidates.extend(indexed.iter().copied());
        }
    }
    candidates.sort_by_key(|id| id.get());
    candidates.dedup();
    candidates
}

fn method_scope_matches(
    edge: &PendingEdge,
    target: NodeId,
    enclosing_scope_by_id: &HashMap<NodeId, String>,
) -> bool {
    let Some(scope_text) = edge.scope_text.as_deref() else {
        return false;
    };
    let Some(enclosing_scope) = enclosing_scope_by_id.get(&target) else {
        return false;
    };
    let matched_scope = if scope_text.trim() == "Self" {
        enclosing_scope_by_id
            .get(&edge.source)
            .cloned()
            .unwrap_or_else(|| canonical_method_scope_text(scope_text))
    } else {
        canonical_method_scope_text(scope_text)
    };
    method_scope_text_matches(&matched_scope, enclosing_scope)
}

fn method_scope_text_matches(matched_scope: &str, enclosing_scope: &str) -> bool {
    if matched_scope == enclosing_scope {
        return true;
    }
    let Some(receiver_type) = matched_scope.strip_prefix("impl ") else {
        return false;
    };
    let Some((_, impl_self_type)) = enclosing_scope
        .strip_prefix("impl ")
        .and_then(|scope| scope.rsplit_once(" for "))
    else {
        return false;
    };
    receiver_type == impl_self_type
}

fn canonical_method_scope_text(scope_text: &str) -> String {
    let trimmed = scope_text.trim();
    if trimmed.starts_with("impl ") {
        return trimmed.to_owned();
    }
    if let Some((self_ty, trait_ty)) = qualified_trait_scope(trimmed) {
        return format!("impl {trait_ty} for {self_ty}");
    }
    format!("impl {trimmed}")
}

fn qualified_trait_scope(scope_text: &str) -> Option<(&str, &str)> {
    let inner = scope_text.strip_prefix('<')?.strip_suffix('>')?.trim();
    let (self_ty, trait_ty) = inner.split_once(" as ")?;
    Some((self_ty.trim(), trait_ty.trim()))
}

fn enclosing_scope_by_id(
    facts: &GraphFacts,
    nodes_by_id: &HashMap<NodeId, &GraphNode>,
    parent_by_target: &HashMap<NodeId, NodeId>,
) -> HashMap<NodeId, String> {
    facts
        .nodes
        .iter()
        .filter_map(|node| {
            let mut parent = parent_by_target
                .get(&node.node_id)
                .and_then(|id| nodes_by_id.get(id).copied())?;
            let mut seen = HashSet::from([node.node_id]);
            while parent.kind == NodeKind::ExternBlock {
                if !seen.insert(parent.node_id) {
                    return None;
                }
                parent = parent_by_target
                    .get(&parent.node_id)
                    .and_then(|id| nodes_by_id.get(id).copied())?;
            }
            match parent.kind {
                NodeKind::File => None,
                NodeKind::Impl => Some((
                    node.node_id,
                    format!(
                        "impl {}",
                        parent.label.strip_prefix("impl ").unwrap_or(&parent.label)
                    ),
                )),
                _ => Some((node.node_id, parent.label.clone())),
            }
        })
        .collect()
}

fn file_by_id_from_maps<'a>(
    facts: &GraphFacts,
    nodes_by_id: &HashMap<NodeId, &GraphNode>,
    parent_by_target: &HashMap<NodeId, NodeId>,
    file_path_by_file_node: &'a HashMap<NodeId, String>,
) -> HashMap<NodeId, &'a str> {
    facts
        .nodes
        .iter()
        .filter_map(|node| {
            let file_node_id = file_node_id_for(node, nodes_by_id, parent_by_target)?;
            let file_path = file_path_by_file_node.get(&file_node_id)?;
            Some((node.node_id, file_path.as_str()))
        })
        .collect()
}

fn file_node_id_for(
    node: &GraphNode,
    nodes_by_id: &HashMap<NodeId, &GraphNode>,
    parent_by_target: &HashMap<NodeId, NodeId>,
) -> Option<NodeId> {
    if node.kind == NodeKind::File {
        return Some(node.node_id);
    }

    let mut current = node;
    let mut seen = HashSet::new();
    seen.insert(node.node_id);
    while let Some(parent) = parent_by_target
        .get(&current.node_id)
        .and_then(|id| nodes_by_id.get(id).copied())
    {
        if !seen.insert(parent.node_id) {
            return None;
        }
        if parent.kind == NodeKind::File {
            return Some(parent.node_id);
        }
        current = parent;
    }
    None
}

fn metadata_for_pending_edge(
    edge: &PendingEdge,
    target: Option<NodeId>,
    bind_method: Option<&'static str>,
) -> EdgeMetadata {
    if edge.origin == CallOrigin::MacroBody
        && edge.relation == RelationKind::Calls
        && target.is_some()
    {
        return EdgeMetadata {
            confidence: Confidence::Heuristic,
            confidence_score: 0.8,
            import_path: edge.import_path.clone(),
            receiver_text: edge.receiver_text.clone(),
            scope_text: edge.scope_text.clone(),
            bind_method: Some("macro_body_singleton"),
        };
    }

    let (mut confidence, mut confidence_score) = confidence_for_edge(edge.relation, edge.edge_kind);
    let mut bind_method = bind_method;
    if edge.relation == RelationKind::Calls
        && edge.scope_text.is_some()
        && !matches!(bind_method, Some("fqn" | "scope_match"))
    {
        confidence = Confidence::Heuristic;
        confidence_score = 0.8;
        if target.is_some() && matches!(bind_method, Some("singleton")) {
            bind_method = Some("bare_qualified_singleton");
        }
    }
    if edge.edge_kind == Some(GraphEdgeKind::ReferencesAddress)
        && target.is_some()
        && matches!(bind_method, Some("address_module_scope"))
    {
        confidence = Confidence::Heuristic;
        confidence_score = 0.8;
    }

    EdgeMetadata {
        confidence,
        confidence_score,
        import_path: edge.import_path.clone(),
        receiver_text: edge.receiver_text.clone(),
        scope_text: edge.scope_text.clone(),
        bind_method,
    }
}

fn confidence_for_edge(
    relation: RelationKind,
    edge_kind: Option<GraphEdgeKind>,
) -> (Confidence, f32) {
    if edge_kind == Some(GraphEdgeKind::CallsDyn) {
        return (Confidence::Heuristic, 0.8);
    }
    match relation {
        RelationKind::Contains | RelationKind::Calls | RelationKind::Constructs => {
            (Confidence::SyntaxExact, 1.0)
        }
        RelationKind::Imports | RelationKind::Links => (Confidence::Heuristic, 0.8),
        _ => (Confidence::Heuristic, 0.5),
    }
}

pub fn build_facts(
    root: &Path,
    progress: Option<ProgressBar>,
) -> anyhow::Result<(GraphFacts, BTreeMap<&'static str, usize>)> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let (groups, file_counts) = discover_language_groups(&root)?;
    let extract_groups: Vec<_> = groups
        .into_iter()
        .map(|group| (group.language, group.label, group.config, group.files))
        .collect();
    let facts = extract_files(&root, extract_groups, progress.as_ref())?;
    Ok((facts, file_counts))
}

pub fn build_facts_for_paths(root: &Path, files: &[PathBuf]) -> anyhow::Result<GraphFacts> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let groups = language_groups_for_paths(&root, files)?;
    let extract_groups: Vec<_> = groups
        .into_iter()
        .map(|group| (group.language, group.label, group.config, group.files))
        .collect();
    extract_files(&root, extract_groups, None)
}

fn extract_files(
    root: &Path,
    groups: Vec<(Language, &'static str, LanguageConfig, Vec<PathBuf>)>,
    progress: Option<&ProgressBar>,
) -> anyhow::Result<GraphFacts> {
    let mut builder = FactBuilder::new(root);

    for (language, label, config, files) in groups {
        let mut extractor = BytesExtractor::new(language, config).map_err(|err| {
            anyhow!("failed to configure tree-sitter parser for `{label}`: {err}")
        })?;
        for path in files {
            if let Some(progress) = progress {
                progress.set_message(progress_file_message(root, &path));
            }
            let source_bytes = match fs::read(&path) {
                Ok(source) => source,
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "spur-graph: skipping file (read failed)"
                    );
                    if let Some(progress) = progress {
                        progress.inc(1);
                    }
                    continue;
                }
            };
            if let Err(err) = extractor.extract_graph_facts(&mut builder, &path, &source_bytes) {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "spur-graph: skipping file (extraction failed)"
                );
            }
            if let Some(progress) = progress {
                progress.inc(1);
            }
        }
    }
    builder.resolve_pending_edges();
    Ok(builder.facts)
}

fn progress_file_message(root: &Path, path: &Path) -> String {
    let display_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    truncate_progress_message(&display_path)
}

fn truncate_progress_message(message: &str) -> String {
    const MAX_CHARS: usize = 96;
    if message.chars().count() <= MAX_CHARS {
        return message.to_owned();
    }

    let tail: String = message
        .chars()
        .rev()
        .take(MAX_CHARS.saturating_sub(3))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}")
}

fn extract_file_from_tree(
    builder: &mut FactBuilder<'_>,
    language_label: &str,
    config: &crate::extract::languages::LanguageConfig,
    path: &Path,
    source: &str,
    root_node: Node<'_>,
    queries: &CompiledQueries,
) -> anyhow::Result<()> {
    let relative_path = relative_path(builder.root(), path)?;
    let file_id = FileId(builder.next_file_id());
    let file_node = builder.add_file_node(&relative_path, file_id, root_node);

    extract_file_contents_from_tree(
        builder,
        language_label,
        config,
        &relative_path,
        file_id,
        file_node,
        source,
        root_node,
        queries,
    )
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn extract_file_contents_from_tree(
    builder: &mut FactBuilder<'_>,
    language_label: &str,
    config: &crate::extract::languages::LanguageConfig,
    relative_path: &str,
    file_id: FileId,
    file_node: NodeId,
    source: &str,
    root_node: Node<'_>,
    queries: &CompiledQueries,
) -> anyhow::Result<()> {
    if matches!(language_label, "json" | "yaml")
        && extract_openapi_document(
            builder,
            relative_path,
            file_id,
            file_node,
            source,
            root_node,
            language_label,
        )
    {
        return Ok(());
    }
    if matches!(language_label, "json" | "yaml") && source.len() > MAX_STRUCTURED_EXTRACT_BYTES {
        return Ok(());
    }
    let mut tag_captures = run_query(&queries.tags, root_node, source);
    let edge_captures = queries
        .spur_edges
        .as_ref()
        .map(|query| run_query(query, root_node, source));
    if language_label == "rust" {
        // Assemble all owners before assigning IDs: variants and foreign blocks
        // come from the edge query. Descendants retain these structural owners
        // while foreign blocks are transparent to Rust lexical names.
        if let Some(captures) = &edge_captures {
            tag_captures.extend(captures.iter().cloned().map(|mut capture| {
                capture.pattern_index += queries.tags.pattern_count();
                capture
            }));
        }
    }
    let mut definitions = emit_definitions(
        config,
        builder,
        relative_path,
        file_id,
        file_node,
        source,
        &tag_captures,
    );
    if language_label == "rust" {
        emit_mcp_tools(
            builder,
            relative_path,
            file_id,
            file_node,
            source,
            root_node,
            &definitions,
        );
    }
    if let Some(edge_captures) = edge_captures {
        if language_label != "rust" {
            let edge_definitions = emit_definitions_with_parents(
                config,
                builder,
                relative_path,
                file_id,
                file_node,
                source,
                &edge_captures,
                &definitions,
            );
            definitions.extend(edge_definitions);
        }
        // Phase-one macro evidence uses physical file bytes and coordinates.
        // Notebook cells keep normal definitions/edges until a cell source
        // contract exists; their virtual paths cannot identify physical files.
        if language_label == "rust"
            && builder
                .facts
                .nodes
                .iter()
                .any(|node| node.node_id == file_node && node.kind == NodeKind::File)
        {
            crate::extract::rust_macros::emit_macro_invocations(
                builder,
                relative_path,
                file_node,
                source,
                &definitions,
                &edge_captures,
            )?;
        }
        // Resolve ordinary Rust calls while physical lexical intervals are
        // still available. PendingEdge deliberately remains schema-compatible.
        let edge_captures = if language_label == "rust" {
            edge_captures
                .into_iter()
                .filter(|capture| {
                    if capture.name != "call" {
                        return true;
                    }
                    let Some(target) = crate::extract::rust_bindings::resolve(
                        capture.node,
                        source,
                        &definitions,
                        relative_path,
                    ) else {
                        // Generic captures are a Rust lexical supplement.
                        // Unsupported associated/type paths keep the existing
                        // extractor behavior rather than entering a new path.
                        return !capture
                            .node
                            .child_by_field_name("function")
                            .is_some_and(|callee| callee.kind() == "generic_function");
                    };
                    let Some(callee) = crate::extract::rust_bindings::callee(capture.node) else {
                        return true;
                    };
                    let name = callee
                        .child_by_field_name("name")
                        .or_else(|| callee.child_by_field_name("field"))
                        .unwrap_or(callee);
                    let Ok(name) = name.utf8_text(source.as_bytes()) else {
                        return true;
                    };
                    let owner = definitions
                        .iter()
                        .rev()
                        .find(|d| {
                            d.node.start_byte() <= capture.node.start_byte()
                                && capture.node.end_byte() <= d.node.end_byte()
                        })
                        .map_or(file_node, |d| d.node_id);
                    // Preserve the established singleton path and provenance
                    // for ordinary nonrecursive calls without local homonyms.
                    if callee.kind() == "identifier"
                        && target.is_some_and(|target| target != owner)
                        && definitions
                            .iter()
                            .filter(|d| {
                                d.node.kind() == "function_item"
                                    && d.node.child_by_field_name("name").is_some_and(|n| {
                                        n.utf8_text(source.as_bytes()).ok() == Some(name)
                                    })
                            })
                            .count()
                            == 1
                    {
                        return true;
                    }
                    let edge = PendingEdge {
                        source: owner,
                        target_name: name.to_owned(),
                        import_path: None,
                        relation: RelationKind::Calls,
                        edge_kind: None,
                        origin: CallOrigin::Expression,
                        receiver_text: (callee.kind() == "field_expression")
                            .then(|| callee.child_by_field_name("value"))
                            .flatten()
                            .and_then(|value| value.utf8_text(source.as_bytes()).ok())
                            .map(str::to_owned),
                        scope_text: callee
                            .child_by_field_name("path")
                            .and_then(|p| p.utf8_text(source.as_bytes()).ok())
                            .map(str::to_owned)
                            .or_else(|| {
                                crate::extract::rust_bindings::receiver_scope(capture.node, source)
                            }),
                    };
                    let bind_method = target.map(|_| {
                        if edge.receiver_text.is_none() && edge.scope_text.is_some() {
                            "fqn"
                        } else {
                            "scope_match"
                        }
                    });
                    builder.add_pending_edge_with_bind_method(&edge, target, bind_method);
                    false
                })
                .collect()
        } else {
            edge_captures
        };
        emit_edges(
            config,
            builder,
            file_node,
            source,
            &definitions,
            &edge_captures,
        );
    }
    if let Some(jsx_edges) = queries.jsx_edges.as_ref() {
        let edge_captures = run_query(jsx_edges, root_node, source);
        emit_edges(
            config,
            builder,
            file_node,
            source,
            &definitions,
            &edge_captures,
        );
    }
    if language_label == "rust" {
        emit_rust_dyn_trait_edges(builder, file_node, source, &definitions, root_node);
    }
    Ok(())
}

pub(crate) fn compile_queries(
    config: &LanguageConfig,
    language: Language,
) -> anyhow::Result<CompiledQueries> {
    let language_label = language.label();
    let mut tags = None;
    let mut tags_source = None;
    let mut spur_edges = None;
    let mut jsx_edges = None;
    let mut inline_spur_edges = None;
    let mut spur_notebook_facts = None;
    for (name, source) in config.queries {
        match *name {
            "tags" => {
                tags_source = Some(source);
                tags = Some(Query::new(&config.language, source).with_context(|| {
                    format!("failed to compile tree-sitter query `{name}` for `{language_label}`")
                })?);
            }
            "spur-edges" => {
                let source = if language == Language::Rust {
                    std::borrow::Cow::Owned(format!(
                        "{source}\n{}",
                        crate::extract::rust_bindings::GENERIC_FREE_CALLS
                    ))
                } else {
                    std::borrow::Cow::Borrowed(*source)
                };
                spur_edges = Some(Query::new(&config.language, &source).with_context(|| {
                    format!("failed to compile tree-sitter query `{name}` for `{language_label}`")
                })?);
            }
            "jsx-edges" => {
                jsx_edges = Some(Query::new(&config.language, source).with_context(|| {
                    format!("failed to compile tree-sitter query `{name}` for `{language_label}`")
                })?);
            }
            "inline-spur-edges" => {
                let inline_language = config.inline_language.as_ref().ok_or_else(|| {
                    anyhow!(
                        "query `{name}` declared for `{language_label}` but inline language is not configured"
                    )
                })?;
                inline_spur_edges =
                    Some(Query::new(inline_language, source).with_context(|| {
                        format!(
                        "failed to compile inline tree-sitter query `{name}` for `{language_label}`"
                    )
                    })?);
            }
            "spur-notebook-facts" => {
                spur_notebook_facts =
                    Some(Query::new(&config.language, source).with_context(|| {
                        format!(
                            "failed to compile tree-sitter query `{name}` for `{language_label}`"
                        )
                    })?);
            }
            name => {
                return Err(anyhow!(
                    "unknown tree-sitter query name `{name}` for `{language_label}`"
                ));
            }
        }
    }
    let tags_source =
        tags_source.with_context(|| format!("missing `{language_label}` tags query"))?;
    let symbols_source = symbol_query_policy(language).source(tags_source);
    let symbols = Query::new(&config.language, symbols_source).with_context(|| {
        format!("failed to compile tree-sitter query `symbols` for `{language_label}`")
    })?;
    Ok(CompiledQueries {
        tags: tags.with_context(|| format!("missing `{language_label}` tags query"))?,
        symbols,
        spur_edges,
        jsx_edges,
        inline_spur_edges,
        spur_notebook_facts,
    })
}

fn discover_language_groups(
    root: &Path,
) -> anyhow::Result<(Vec<LanguageFileGroup>, BTreeMap<&'static str, usize>)> {
    let allowed_extensions = all_supported_extensions();
    let files = discover_files(root, &allowed_extensions)?;
    let mut groups: GroupAccumulator = BTreeMap::new();
    let descriptors: &[LanguageDescriptor] = language_registry();
    for path in files {
        let Some(descriptor) = descriptors.iter().find(|d| (d.matcher)(&path)) else {
            continue;
        };
        groups
            .entry(descriptor.label)
            .or_insert_with(|| (descriptor.language, descriptor.factory, Vec::new()))
            .2
            .push(path);
    }

    let file_counts = groups
        .iter()
        .map(|(label, (_, _, files))| (*label, files.len()))
        .collect();

    let language_groups = groups
        .into_iter()
        .map(|(label, (language, factory, files))| LanguageFileGroup {
            label,
            language,
            config: factory(),
            files,
        })
        .collect();

    Ok((language_groups, file_counts))
}

fn language_groups_for_paths(
    root: &Path,
    files: &[PathBuf],
) -> anyhow::Result<Vec<LanguageFileGroup>> {
    let mut groups: GroupAccumulator = BTreeMap::new();
    let descriptors: &[LanguageDescriptor] = language_registry();
    for path in super::normalize_selected_paths(root, files)? {
        let full_path = root.join(path);
        if !full_path.is_file() {
            continue;
        }
        let Some(descriptor) = descriptors.iter().find(|d| (d.matcher)(&full_path)) else {
            continue;
        };
        groups
            .entry(descriptor.label)
            .or_insert_with(|| (descriptor.language, descriptor.factory, Vec::new()))
            .2
            .push(full_path);
    }

    Ok(groups
        .into_iter()
        .map(|(label, (language, factory, files))| LanguageFileGroup {
            label,
            language,
            config: factory(),
            files,
        })
        .collect())
}

pub(crate) fn run_query<'tree>(
    query: &Query,
    root_node: Node<'tree>,
    source: &str,
) -> Vec<CaptureHit<'tree>> {
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root_node, source.as_bytes());
    let mut hits = Vec::new();
    let mut match_index = 0usize;
    let mut string_context_by_node_id = HashMap::new();
    let mut string_context_ancestor_path = Vec::new();
    while let Some(query_match) = matches.next() {
        let current_match_index = match_index;
        match_index += 1;
        for capture in query_match.captures {
            let name = capture_names[capture.index as usize];
            if is_string_literal_context(
                capture.node,
                &mut string_context_by_node_id,
                &mut string_context_ancestor_path,
            ) && !is_explicit_string_content_edge_capture(name, capture.node)
            {
                continue;
            }
            hits.push(CaptureHit {
                name: name.to_owned(),
                node: capture.node,
                pattern_index: query_match.pattern_index,
                match_index: current_match_index,
            });
        }
    }
    hits
}

/// Some grammars expose quote-free import or link targets only as a named
/// content node inside a string literal. Explicit edge captures on those
/// content nodes must survive the general string-context noise suppression.
fn is_explicit_string_content_edge_capture(name: &str, node: Node<'_>) -> bool {
    match node.kind() {
        "interpreted_string_literal_content" | "raw_string_literal_content" => {
            matches!(name, "import.name" | "import.path")
        }
        "string_content" => matches!(name, "import.name" | "import.path" | "link.name"),
        _ => false,
    }
}

fn is_string_literal_context(
    node: Node<'_>,
    by_node_id: &mut HashMap<usize, bool>,
    ancestor_path: &mut Vec<usize>,
) -> bool {
    ancestor_path.clear();
    let mut current = Some(node);
    let is_string_literal_context = loop {
        let Some(node) = current else {
            break false;
        };
        if let Some(&cached) = by_node_id.get(&node.id()) {
            break cached;
        }
        ancestor_path.push(node.id());
        if matches!(
            node.kind(),
            "string_literal"
                | "raw_string_literal"
                | "string_content"
                | "interpreted_string_literal"
                | "raw_string_literal_content"
        ) {
            break true;
        }
        #[cfg(test)]
        STRING_CONTEXT_PARENT_WALKS.with(|walks| walks.set(walks.get() + 1));
        current = node.parent();
    };
    for node_id in ancestor_path.drain(..) {
        by_node_id.insert(node_id, is_string_literal_context);
    }
    is_string_literal_context
}

#[cfg(test)]
std::thread_local! {
    static STRING_CONTEXT_PARENT_WALKS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

fn impl_identity_fqn(fqn: &str) -> String {
    if let Some((prefix, segment)) = fqn.rsplit_once("::") {
        if let Some(stripped) = segment.strip_prefix("impl ") {
            return format!("{prefix}::{stripped}");
        }
    }
    fqn.strip_prefix("impl ").unwrap_or(fqn).to_owned()
}

fn qualified_symbols_by_name_from_maps(
    facts: &GraphFacts,
    nodes_by_id: &HashMap<NodeId, &GraphNode>,
    parent_by_target: &HashMap<NodeId, NodeId>,
) -> HashMap<String, Vec<NodeId>> {
    let mut index: HashMap<String, Vec<NodeId>> = HashMap::new();
    for node in &facts.nodes {
        if matches!(node.kind, NodeKind::File | NodeKind::McpTool) {
            continue;
        }
        let qualified_name = qualified_node_name(node, nodes_by_id, parent_by_target);
        index.entry(qualified_name).or_default().push(node.node_id);
    }
    index
}

fn qualified_name_by_id_from_index(
    index: &HashMap<String, Vec<NodeId>>,
) -> HashMap<NodeId, String> {
    let mut qualified_name_by_id = HashMap::new();
    for (qualified_name, ids) in index {
        for id in ids {
            qualified_name_by_id.insert(*id, qualified_name.clone());
        }
    }
    qualified_name_by_id
}

fn parent_by_target(facts: &GraphFacts) -> HashMap<NodeId, NodeId> {
    let mut parent_by_target = HashMap::new();
    for edge in &facts.edges {
        if edge.relation != RelationKind::Contains {
            continue;
        }
        let Some(target) = edge.target_node_id else {
            continue;
        };
        parent_by_target
            .entry(target)
            .or_insert(edge.source_node_id);
    }
    parent_by_target
}

fn qualified_node_name(
    node: &GraphNode,
    nodes_by_id: &HashMap<NodeId, &GraphNode>,
    parent_by_target: &HashMap<NodeId, NodeId>,
) -> String {
    let mut segments = vec![qualified_node_segment(node)];
    let mut current = node;
    let mut seen = HashSet::new();
    seen.insert(node.node_id);

    while let Some(parent) = parent_by_target
        .get(&current.node_id)
        .and_then(|id| nodes_by_id.get(id).copied())
    {
        if !seen.insert(parent.node_id) || parent.kind == NodeKind::File {
            break;
        }
        if parent.kind != NodeKind::ExternBlock {
            segments.push(qualified_node_segment(parent));
        }
        current = parent;
    }

    segments.reverse();
    segments.join("::")
}

fn qualified_node_segment(node: &GraphNode) -> String {
    match node.kind {
        NodeKind::Impl => format!(
            "impl {}",
            node.label.strip_prefix("impl ").unwrap_or(&node.label)
        ),
        _ => node
            .label
            .strip_prefix("impl ")
            .unwrap_or(&node.label)
            .to_owned(),
    }
}

fn trait_method_candidates(facts: &GraphFacts, target_name: &str) -> Vec<NodeId> {
    let Some((trait_name, method_name)) = target_name.rsplit_once("::") else {
        return Vec::new();
    };
    let nodes_by_id: HashMap<_, _> = facts
        .nodes
        .iter()
        .map(|node| (node.node_id, node))
        .collect();
    let parent_by_target = parent_by_target(facts);

    facts
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method && node.label == method_name)
        .filter_map(|node| {
            let parent = parent_by_target
                .get(&node.node_id)
                .and_then(|id| nodes_by_id.get(id).copied())?;
            (parent.kind == NodeKind::Trait
                && (parent.label == trait_name
                    || qualified_node_name(parent, &nodes_by_id, &parent_by_target) == trait_name))
                .then_some(node.node_id)
        })
        .collect()
}

pub(crate) fn relative_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("`{}` is outside `{}`", path.display(), root.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;

    // Keep this fixture independent of the recognizer: the original AST path is
    // the oracle, including definitions projected from spur-edges captures.
    fn generated_icu_source(elements: &str) -> String {
        format!(
            concat!(
                "\n// This file is generated by scripts/inline-data.py, do not edit it manually //\n\n",
                "#include \"unicode/utypes.h\"\n",
                "#include \"unicode/udata.h\"\n",
                "#include \"unicode/uversion.h\"\n\n",
                "extern \"C\" U_EXPORT const unsigned char U_ICUDATA_ENTRY_POINT [] = {{\n",
                "{}\n}};\n"
            ),
            elements
        )
    }

    fn ast_symbol_oracle(extractor: &mut BytesExtractor, source: &str) -> Vec<ExtractedSymbol> {
        let tree = extractor.parser.parse(source.as_bytes(), None).unwrap();
        let mut captures = run_query(&extractor.queries.symbols, tree.root_node(), source);
        if let Some(spur_edges) = extractor.queries.spur_edges.as_ref() {
            captures.extend(
                run_query(spur_edges, tree.root_node(), source)
                    .into_iter()
                    .map(|mut capture| {
                        capture.pattern_index += extractor.queries.symbols.pattern_count();
                        capture
                    }),
            );
        }
        extracted_symbols(&extractor.config, source, &captures)
    }

    fn record_parser_activity(extractor: &mut BytesExtractor) -> Arc<AtomicBool> {
        let parsed = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&parsed);
        extractor.parser.set_logger(Some(Box::new(move |_, _| {
            observed.store(true, Ordering::Relaxed);
        })));
        parsed
    }

    #[test]
    fn generated_icu_numeric_data_skips_cpp_parser_with_ast_symbol_parity() {
        let all_bytes = (u8::MIN..=u8::MAX)
            .map(|byte| byte.to_string())
            .collect::<Vec<_>>()
            .join(",");
        for elements in [
            "0",
            "255",
            "0,1,9,10,99,100,254,255",
            " \t0 ,\r\n255, \t",
            &all_bytes,
        ] {
            let source = generated_icu_source(elements);
            let mut extractor = BytesExtractor::for_language(Language::Cpp).unwrap();
            let expected = ast_symbol_oracle(&mut extractor, &source);
            assert!(
                expected.is_empty(),
                "AST query contract changed: {elements}"
            );
            let parsed = record_parser_activity(&mut extractor);
            assert_eq!(
                extractor
                    .extract(Path::new("arbitrary.unknown"), source.as_bytes())
                    .unwrap(),
                expected
            );
            assert!(
                !parsed.load(Ordering::Relaxed),
                "parsed generated byte list: {elements}"
            );
        }
    }

    #[test]
    fn generated_icu_near_misses_use_ast_with_exact_symbol_parity() {
        let source = generated_icu_source("0,255");
        let mut near_misses = vec![
            generated_icu_source(""),
            generated_icu_source(",0"),
            generated_icu_source("0,,1"),
            generated_icu_source("0 1"),
            generated_icu_source("256"),
            generated_icu_source("999999999999999999999999999999999999"),
            generated_icu_source("00"),
            generated_icu_source("08"),
            generated_icu_source("0xff"),
            generated_icu_source("0b1"),
            generated_icu_source("1u"),
            generated_icu_source("-1"),
            generated_icu_source("+1"),
            generated_icu_source("1.0"),
            generated_icu_source("1e0"),
            generated_icu_source("1'0"),
            generated_icu_source("1, /* data */ 2"),
            generated_icu_source("1, // data\n2"),
            generated_icu_source("\"1\""),
            generated_icu_source("'1'"),
            generated_icu_source("payload()"),
            generated_icu_source("U_ICUDATA_ENTRY_POINT"),
            generated_icu_source("1\0"),
            generated_icu_source("1\u{a0}"),
            source.replace("[]", "[1]"),
            source.replace("unsigned char", "unsigned int"),
            source.replace("extern \"C\"", "extern \"C++\""),
            source.replace("U_ICUDATA_ENTRY_POINT", "OTHER_DATA"),
            source.replace("unicode/utypes.h", "other.h"),
            source.replace("do not edit it manually", "custom data"),
            source.replace("};", "} ;"),
            source.replace("};", "}"),
            source.trim_start().to_owned(),
        ];
        for suffix in [
            "// trailing comment\n",
            "/* trailing comment */",
            "\"trailing string\"",
            ";",
            "\0",
        ] {
            near_misses.push(format!("{source}{suffix}"));
        }
        near_misses.push(format!("// additional comment\n{source}"));
        for source in near_misses {
            let mut extractor = BytesExtractor::for_language(Language::Cpp).unwrap();
            let expected = ast_symbol_oracle(&mut extractor, &source);
            let parsed = record_parser_activity(&mut extractor);
            assert_eq!(
                extractor
                    .extract(Path::new("stubdata.cpp"), source.as_bytes())
                    .unwrap(),
                expected
            );
            assert!(
                parsed.load(Ordering::Relaxed),
                "near miss skipped AST: {source:?}"
            );
        }
    }

    #[test]
    fn generated_icu_envelope_does_not_hide_other_definitions() {
        let data = generated_icu_source("0,255");
        for (declaration, expected_name) in [
            ("#define VISIBLE_MACRO 1\n", "VISIBLE_MACRO"),
            ("int visible_function() { return 1; }\n", "visible_function"),
            ("namespace visible_namespace {}\n", "visible_namespace"),
            ("enum VisibleEnum { VisibleVariant };\n", "VisibleEnum"),
            ("const int VISIBLE_CONST = 1;\n", "VISIBLE_CONST"),
            ("struct VisibleStruct {};\n", "VisibleStruct"),
            ("using VisibleAlias = int;\n", "VisibleAlias"),
        ] {
            for source in [
                format!("{data}{declaration}"),
                format!("{declaration}{data}"),
            ] {
                let mut extractor = BytesExtractor::for_language(Language::Cpp).unwrap();
                let expected = ast_symbol_oracle(&mut extractor, &source);
                assert!(expected
                    .iter()
                    .any(|symbol| symbol.entity_name == expected_name));
                let parsed = record_parser_activity(&mut extractor);
                assert_eq!(
                    extractor
                        .extract(Path::new("stubdata.cpp"), source.as_bytes())
                        .unwrap(),
                    expected
                );
                assert!(parsed.load(Ordering::Relaxed));
            }
        }
    }

    #[test]
    fn generated_icu_recognition_preserves_utf8_errors_and_other_languages() {
        let source = generated_icu_source("0,255");
        let mut invalid = source.as_bytes().to_vec();
        invalid.push(0xff);
        let mut extractor = BytesExtractor::for_language(Language::Cpp).unwrap();
        let parsed = record_parser_activity(&mut extractor);
        assert!(matches!(
            extractor.extract(Path::new("stubdata.cpp"), &invalid),
            Err(ExtractError::InvalidUtf8(_))
        ));
        assert!(!parsed.load(Ordering::Relaxed));
        for language in [Language::C, Language::Rust, Language::Python] {
            let mut extractor = BytesExtractor::for_language(language).unwrap();
            let expected = ast_symbol_oracle(&mut extractor, &source);
            let parsed = record_parser_activity(&mut extractor);
            assert_eq!(
                extractor
                    .extract(Path::new("stubdata.cpp"), source.as_bytes())
                    .unwrap(),
                expected
            );
            assert!(parsed.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn generated_icu_graph_facts_still_parse() {
        let source = generated_icu_source("0,255");
        let mut extractor = BytesExtractor::for_language(Language::Cpp).unwrap();
        let parsed = record_parser_activity(&mut extractor);
        let mut builder = FactBuilder::new(Path::new("/workspace"));
        extractor
            .extract_graph_facts(
                &mut builder,
                Path::new("/workspace/stubdata.cpp"),
                source.as_bytes(),
            )
            .unwrap();
        assert!(parsed.load(Ordering::Relaxed));
        assert!(builder
            .into_facts()
            .nodes
            .iter()
            .any(|node| node.kind == NodeKind::File && node.label == "stubdata.cpp"));
    }

    fn extracted_definition_set(facts: &GraphFacts) -> BTreeSet<String> {
        facts
            .nodes
            .iter()
            .filter(|node| {
                !matches!(
                    node.kind,
                    NodeKind::File | NodeKind::External | NodeKind::McpTool
                )
            })
            .map(|node| format!("{}:{}", node.kind.discriminator(), node.label))
            .collect()
    }

    fn web_relation_set(facts: &GraphFacts) -> BTreeSet<String> {
        facts
            .edges
            .iter()
            .filter_map(|edge| {
                let relation = match edge.relation {
                    RelationKind::Imports => "imports",
                    RelationKind::Links => "links",
                    _ => return None,
                };
                edge.target_label
                    .as_deref()
                    .map(|target| format!("{relation}:{target}"))
            })
            .collect()
    }

    fn node_with_label<'a>(facts: &'a GraphFacts, label: &str) -> &'a GraphNode {
        facts
            .nodes
            .iter()
            .find(|node| node.label == label)
            .unwrap_or_else(|| panic!("missing node `{label}`"))
    }

    fn file_node<'a>(facts: &'a GraphFacts, label: &str) -> &'a GraphNode {
        facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == label)
            .unwrap_or_else(|| panic!("missing file node `{label}`"))
    }

    fn assert_node_span_matches(
        facts: &GraphFacts,
        node: &GraphNode,
        source: &str,
        expected_source: &str,
    ) {
        let span_id = node.source_span_id.expect("node source span");
        let span = facts
            .spans
            .iter()
            .find(|span| span.span_id == span_id)
            .expect("source span record");
        let start = source.find(expected_source).unwrap_or_else(|| {
            panic!(
                "expected source fragment for `{}` was not found",
                node.label
            )
        });
        let end = start + expected_source.len();
        let line_at = |offset: usize| {
            source.as_bytes()[..offset]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u32
                + 1
        };

        assert_eq!(span.start_byte, start as u32, "{} start byte", node.label);
        assert_eq!(span.end_byte, end as u32, "{} end byte", node.label);
        assert_eq!(span.start_line, line_at(start), "{} start line", node.label);
        assert_eq!(span.end_line, line_at(end), "{} end line", node.label);
    }

    fn assert_graph_edge(
        facts: &GraphFacts,
        source: NodeId,
        target: NodeId,
        relation: RelationKind,
    ) {
        assert!(
            facts.edges.iter().any(|edge| {
                edge.source_node_id == source
                    && edge.target_node_id == Some(target)
                    && edge.relation == relation
            }),
            "missing {relation:?} edge from {source:?} to {target:?}"
        );
    }

    fn assert_relation_source(
        facts: &GraphFacts,
        relation: RelationKind,
        target: &str,
        expected_source: NodeId,
    ) {
        let edge = facts
            .edges
            .iter()
            .find(|edge| edge.relation == relation && edge.target_label.as_deref() == Some(target))
            .unwrap_or_else(|| panic!("missing {relation:?} edge to `{target}`"));
        assert_eq!(edge.source_node_id, expected_source, "edge to `{target}`");
    }

    #[test]
    fn html_extraction_emits_low_noise_symbols_and_relations() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.html"),
            r#"<main id="app">
  <script src="app.js">function embedded() {}</script>
  <link href="theme.css" rel="stylesheet">
  <a href="/docs/start.html">Docs</a>
  <img src="hero.png" class="hero">
  <div class="not-a-symbol">content</div>
  <style>.embedded { color: red; }</style>
</main>
"#,
        )
        .expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        assert_eq!(
            extracted_definition_set(&facts),
            BTreeSet::from(["section:app".to_owned()])
        );
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from([
                "imports:app.js".to_owned(),
                "imports:theme.css".to_owned(),
                "links:/docs/start.html".to_owned(),
                "links:hero.png".to_owned(),
            ])
        );
    }

    #[test]
    fn css_extraction_emits_low_noise_symbols_and_relations() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("site.css"),
            r#"@import "base.css";
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
:root { --brand: #f00; color: red; }
.card > img { background-image: url("../img/card.png"); }
"#,
        )
        .expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        assert_eq!(
            extracted_definition_set(&facts),
            BTreeSet::from([
                "constant:--brand".to_owned(),
                "function:fade".to_owned(),
                "section:.card > img".to_owned(),
                "section::root".to_owned(),
            ])
        );
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from([
                "imports:base.css".to_owned(),
                "links:../img/card.png".to_owned(),
            ])
        );
    }

    #[test]
    fn html_extraction_preserves_spans_containment_and_nearest_owner_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = "<main id=\"app\">\n  <section id=\"panel\"><img src=\"panel.png\"></section>\n  <img src=\"hero.png\">\n</main>\n<img src=\"root.png\">\n";
        fs::write(dir.path().join("index.html"), source).expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        let file = file_node(&facts, "index.html");
        let app = node_with_label(&facts, "app");
        let panel = node_with_label(&facts, "panel");

        assert_node_span_matches(
            &facts,
            app,
            source,
            "<main id=\"app\">\n  <section id=\"panel\"><img src=\"panel.png\"></section>\n  <img src=\"hero.png\">\n</main>",
        );
        assert_node_span_matches(
            &facts,
            panel,
            source,
            "<section id=\"panel\"><img src=\"panel.png\"></section>",
        );
        assert_graph_edge(&facts, file.node_id, app.node_id, RelationKind::Contains);
        assert_graph_edge(&facts, app.node_id, panel.node_id, RelationKind::Contains);
        assert_graph_edge(&facts, app.node_id, panel.node_id, RelationKind::Defines);
        assert!(!facts.edges.iter().any(|edge| {
            edge.source_node_id == file.node_id
                && edge.target_node_id == Some(app.node_id)
                && edge.relation == RelationKind::Defines
        }));
        assert_relation_source(&facts, RelationKind::Links, "panel.png", panel.node_id);
        assert_relation_source(&facts, RelationKind::Links, "hero.png", app.node_id);
        assert_relation_source(&facts, RelationKind::Links, "root.png", file.node_id);
    }

    #[test]
    fn css_extraction_preserves_spans_containment_and_nearest_owner_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = ":root {\n  --brand: #f00;\n  background-image: url(root.png);\n}\n.card { background: url(card.png); }\n@import \"base.css\";\n";
        fs::write(dir.path().join("site.css"), source).expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        let file = file_node(&facts, "site.css");
        let root = node_with_label(&facts, ":root");
        let brand = node_with_label(&facts, "--brand");
        let card = node_with_label(&facts, ".card");

        assert_node_span_matches(
            &facts,
            root,
            source,
            ":root {\n  --brand: #f00;\n  background-image: url(root.png);\n}",
        );
        assert_node_span_matches(&facts, brand, source, "--brand: #f00;");
        assert_graph_edge(&facts, file.node_id, root.node_id, RelationKind::Contains);
        assert_graph_edge(&facts, root.node_id, brand.node_id, RelationKind::Contains);
        assert_graph_edge(&facts, root.node_id, brand.node_id, RelationKind::Defines);
        assert_relation_source(&facts, RelationKind::Links, "root.png", root.node_id);
        assert_relation_source(&facts, RelationKind::Links, "card.png", card.node_id);
        assert_relation_source(&facts, RelationKind::Imports, "base.css", file.node_id);
    }

    #[test]
    fn html_extraction_handles_attribute_order_and_unquoted_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.html"),
            r#"<link href=first.css rel=stylesheet>
<link rel="stylesheet" href="second.css">
<script src=app.js></script>
<a href=/docs/start.html>Docs</a>
<img src=hero.png>
"#,
        )
        .expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        let file = file_node(&facts, "index.html");
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from([
                "imports:app.js".to_owned(),
                "imports:first.css".to_owned(),
                "imports:second.css".to_owned(),
                "links:/docs/start.html".to_owned(),
                "links:hero.png".to_owned(),
            ])
        );
        for (relation, target) in [
            (RelationKind::Imports, "app.js"),
            (RelationKind::Imports, "first.css"),
            (RelationKind::Imports, "second.css"),
            (RelationKind::Links, "/docs/start.html"),
            (RelationKind::Links, "hero.png"),
        ] {
            assert_relation_source(&facts, relation, target, file.node_id);
        }
    }

    #[test]
    fn html_extraction_handles_case_insensitive_names_and_stylesheet_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.html"),
            r#"<DIV ID=app>
  <A HREF=guide>Guide</A>
  <IMG SRC=hero.png />
  <LINK HREF=theme.css REL="alternate StyleSheet" />
</DIV>"#,
        )
        .expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        assert_eq!(
            extracted_definition_set(&facts),
            BTreeSet::from(["section:app".to_owned()])
        );
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from([
                "imports:theme.css".to_owned(),
                "links:guide".to_owned(),
                "links:hero.png".to_owned(),
            ])
        );
    }

    #[test]
    fn html_id_sections_cover_self_closing_script_and_style_elements() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = r#"<img id="hero" />
<script id=loader></script>
<style id='theme'></style>
"#;
        fs::write(dir.path().join("index.html"), source).expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        assert_eq!(
            extracted_definition_set(&facts),
            BTreeSet::from([
                "section:hero".to_owned(),
                "section:loader".to_owned(),
                "section:theme".to_owned(),
            ])
        );
        assert_node_span_matches(
            &facts,
            node_with_label(&facts, "hero"),
            source,
            "<img id=\"hero\" />",
        );
        assert_node_span_matches(
            &facts,
            node_with_label(&facts, "loader"),
            source,
            "<script id=loader></script>",
        );
        assert_node_span_matches(
            &facts,
            node_with_label(&facts, "theme"),
            source,
            "<style id='theme'></style>",
        );
    }

    #[test]
    fn html_media_links_reject_similar_attributes_and_non_stylesheet_links() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.html"),
            r#"<source src=stream.webm>
<audio src="song.mp3"></audio>
<video src=movie.mp4 poster='poster.jpg'></video>
<img data-src=lazy.png>
<a href-lang=en>Language</a>
<link rel=icon href=favicon.ico>
"#,
        )
        .expect("write HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract HTML");
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from([
                "links:movie.mp4".to_owned(),
                "links:poster.jpg".to_owned(),
                "links:song.mp3".to_owned(),
                "links:stream.webm".to_owned(),
            ])
        );
    }

    #[test]
    fn css_extraction_normalizes_urls_and_rejects_noise() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("site.css"),
            r##".hero {
  background: url("../img/hero.png");
  mask: url("#mask");
  empty-quoted: url("");
  empty-bare: url();
  inline: url(data:image/png,AAAA);
}
"##,
        )
        .expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        let hero = node_with_label(&facts, ".hero");
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from(["links:#mask".to_owned(), "links:../img/hero.png".to_owned(),])
        );
        assert_relation_source(&facts, RelationKind::Links, "../img/hero.png", hero.node_id);
        assert_relation_source(&facts, RelationKind::Links, "#mask", hero.node_id);
    }

    #[test]
    fn css_import_url_is_an_import_without_a_duplicate_link() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("site.css"), r#"@import url("theme.css");"#)
            .expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from(["imports:theme.css".to_owned()])
        );
    }

    #[test]
    fn css_url_function_name_is_case_insensitive() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("site.css"),
            ".hero { background: URL(hero.png); }\n@import UrL(\"theme.css\");\n",
        )
        .expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        assert_eq!(
            web_relation_set(&facts),
            BTreeSet::from(["imports:theme.css".to_owned(), "links:hero.png".to_owned(),])
        );
    }

    #[test]
    fn links_remain_unresolved_when_their_labels_collide_with_symbols() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.html"),
            r#"<section id="guide"><a href="guide">Guide</a></section>"#,
        )
        .expect("write HTML fixture");
        fs::write(
            dir.path().join("site.css"),
            "@keyframes fade { from { opacity: 0; } }\n.hero { background: url(fade); }\n",
        )
        .expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract web files");
        for target in ["guide", "fade"] {
            let edge = facts
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Links
                        && edge.target_label.as_deref() == Some(target)
                })
                .unwrap_or_else(|| panic!("missing link to {target}"));
            assert_eq!(
                edge.target_node_id, None,
                "link target {target} must not bind through symbol lookup"
            );
        }
    }

    #[test]
    fn css_custom_property_without_trailing_semicolon_is_a_constant() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = ":root { --brand: #f00 }\n";
        fs::write(dir.path().join("site.css"), source).expect("write CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract CSS");
        let brand = node_with_label(&facts, "--brand");
        assert_eq!(brand.kind, NodeKind::Constant);
        assert_node_span_matches(&facts, brand, source, "--brand: #f00");
    }

    #[test]
    fn malformed_html_retains_file_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("broken.html"),
            "<main id=\"kept\"><div><a href=\"broken.html\">",
        )
        .expect("write malformed HTML fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract malformed HTML");
        file_node(&facts, "broken.html");
    }

    #[test]
    fn malformed_css_retains_file_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("broken.css"),
            ".card { color: red; background: url(\"broken.png\");",
        )
        .expect("write malformed CSS fixture");

        let (facts, _) = build_facts(dir.path(), None).expect("extract malformed CSS");
        file_node(&facts, "broken.css");
    }

    #[test]
    fn run_query_compresses_shared_string_context_ancestor_paths() {
        let source = "fn shared() { let tuple = (alpha, beta, gamma); }\n";
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser.parse(source, None).expect("parse source");
        let query = Query::new(&language, "(identifier) @identifier").expect("compile query");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut captures = Vec::new();
        while let Some(query_match) = matches.next() {
            captures.extend(query_match.captures.iter().map(|capture| capture.node));
        }

        let mut unique_ancestor_ids = HashSet::new();
        let mut uncached_parent_walks = 0usize;
        for capture in &captures {
            let mut current = Some(*capture);
            while let Some(node) = current {
                unique_ancestor_ids.insert(node.id());
                uncached_parent_walks += 1;
                current = node.parent();
            }
        }
        assert!(uncached_parent_walks > unique_ancestor_ids.len());

        STRING_CONTEXT_PARENT_WALKS.with(|walks| walks.set(0));
        let hits = run_query(&query, tree.root_node(), source);
        let cached_parent_walks = STRING_CONTEXT_PARENT_WALKS.with(std::cell::Cell::get);

        assert_eq!(hits.len(), captures.len());
        assert_eq!(cached_parent_walks, unique_ancestor_ids.len());
    }

    #[test]
    fn extern_container_is_not_a_resolution_target() {
        let kind = NodeKind::from_discriminator("extern_block").expect("extern container kind");
        assert!(!is_callable_target_kind(kind));
        assert!(!constructs_target_kinds().contains(&kind));
        assert!(!is_import_resolution_candidate_kind(kind));
        assert!(!is_import_resolution_fallback_candidate_kind(kind));
        eprintln!(
            "G2A_ROLES={}",
            serde_json::json!({
                "kind": kind.discriminator(),
                "callable": is_callable_target_kind(kind),
                "constructor": constructs_target_kinds().contains(&kind),
                "import": is_import_resolution_candidate_kind(kind),
                "fallback": is_import_resolution_fallback_candidate_kind(kind),
            })
        );
    }

    #[test]
    fn import_resolution_fallback_candidate_kind_is_total_for_node_kinds() {
        for (kind, expected) in [
            (NodeKind::Module, false),
            (NodeKind::ExternBlock, false),
            (NodeKind::Function, false),
            (NodeKind::Class, false),
            (NodeKind::Interface, false),
            (NodeKind::Struct, false),
            (NodeKind::Impl, false),
            (NodeKind::Trait, false),
            (NodeKind::Enum, false),
            (NodeKind::EnumVariant, true),
            (NodeKind::File, false),
            (NodeKind::External, false),
            (NodeKind::Method, false),
            (NodeKind::Field, false),
            (NodeKind::Constant, true),
            (NodeKind::TypeAlias, false),
            (NodeKind::Macro, false),
            (NodeKind::Section, false),
            (NodeKind::Commit, false),
            (NodeKind::McpTool, false),
            (NodeKind::Cell, false),
            (NodeKind::Port, false),
            (NodeKind::Resource, false),
        ] {
            assert_eq!(
                is_import_resolution_fallback_candidate_kind(kind),
                expected,
                "{kind:?}"
            );
        }
    }

    fn import_workspace_index(
        rust_crates: &[&str],
        javascript_path_aliases: &[&str],
        javascript_workspace_roots: &[&str],
        python_modules: &[&str],
    ) -> ImportWorkspaceIndex {
        ImportWorkspaceIndex {
            rust_crates: rust_crates
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            javascript_path_aliases: javascript_path_aliases
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            javascript_workspace_roots: javascript_workspace_roots
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            python_modules: python_modules
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn external_import(origin: &str, path: &str, name: &str) -> ImportOriginClassification {
        ImportOriginClassification::External {
            origin: origin.to_owned(),
            path: path.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn classify_import_origin_rust_uses_path_not_collision_label() {
        let workspace_index = import_workspace_index(
            &["spur_core", "workspace_dep"],
            &["std/"],
            &["Path", "Command", "Terminal", "Ordering", "Write"],
            &["std", "Path", "Command", "Terminal", "Ordering", "Write"],
        );

        for import_path in [
            "crate::model::Foo",
            "self::local::Foo",
            "super::parent::Foo",
            "$crate::macros::Foo",
            "workspace_dep::api::Foo",
        ] {
            assert_eq!(
                classify_import_origin(
                    import_path,
                    "crates/spur-core/src/session.rs",
                    &workspace_index
                ),
                ImportOriginClassification::Internal,
                "{import_path} should classify as workspace-internal"
            );
        }

        for (import_path, origin, name) in [
            ("std::path::Path", "std", "Path"),
            ("std::process::Command", "std", "Command"),
            ("ratatui::Terminal", "ratatui", "Terminal"),
            ("std::cmp::Ordering", "std", "Ordering"),
            ("std::io::Write", "std", "Write"),
            ("core::fmt::Debug", "std", "Debug"),
            ("alloc::vec::Vec", "std", "Vec"),
            ("serde_json::Value", "serde_json", "Value"),
        ] {
            assert_eq!(
                classify_import_origin(
                    import_path,
                    "crates/spur-core/src/session.rs",
                    &workspace_index
                ),
                external_import(origin, import_path, name),
                "{import_path} should classify as external by import path"
            );
        }
    }

    #[test]
    fn classify_import_origin_javascript_respects_relative_alias_and_bare_specifiers() {
        let workspace_index = import_workspace_index(
            &["react"],
            &["@/", "@app/*"],
            &["src", "packages/ui"],
            &["react"],
        );

        for import_path in [
            "./button",
            "../utils/theme",
            "/src/app",
            "@/components/Button",
            "@app/components/Button",
            "src/local/module",
            "packages/ui/Button",
        ] {
            assert_eq!(
                classify_import_origin(
                    import_path,
                    "crates/spur-notebook/jute-notebook/src/App.tsx",
                    &workspace_index
                ),
                ImportOriginClassification::Internal,
                "{import_path} should classify as workspace-internal"
            );
        }

        for (import_path, origin, name) in [
            ("react", "react", "react"),
            ("react/jsx-runtime", "react", "jsx-runtime"),
            ("@scope/pkg", "@scope/pkg", "pkg"),
            ("@scope/pkg/subpath", "@scope/pkg", "subpath"),
        ] {
            assert_eq!(
                classify_import_origin(
                    import_path,
                    "crates/spur-notebook/jute-notebook/src/App.tsx",
                    &workspace_index
                ),
                external_import(origin, import_path, name),
                "{import_path} should classify as external"
            );
        }
    }

    #[test]
    fn classify_web_imports_and_resolve_workspace_paths() {
        let workspace_index = ImportWorkspaceIndex::default();
        for (source_file, import_path) in [
            ("web/index.html", "./app.js"),
            ("web/index.htm", "theme.css?v=2"),
            ("web/index.html", "/theme.css"),
            ("web/index.html", "#bootstrap"),
            ("web/index.html", "data:text/javascript;base64,AAAA"),
            ("web/styles/site.css", "../base.css"),
        ] {
            assert_eq!(
                classify_import_origin(import_path, source_file, &workspace_index),
                ImportOriginClassification::Internal,
                "{source_file} import {import_path} should be workspace-local"
            );
        }
        assert_eq!(
            classify_import_origin(
                "https://cdn.example/app.js",
                "web/index.html",
                &workspace_index
            ),
            external_import(
                "https://cdn.example/app.js",
                "https://cdn.example/app.js",
                "https://cdn.example/app.js"
            )
        );

        assert_eq!(
            web_import_target_path("web/pages/index.html", "../app.js?v=2#boot").as_deref(),
            Some("web/app.js")
        );
        assert_eq!(
            web_import_target_path("web/styles/site.css", "/theme.css#screen"),
            None
        );
        assert_eq!(
            web_import_target_path("web/index.html", "https://cdn.example/app.js"),
            None
        );
        assert_eq!(
            web_import_target_path("web/index.html", "../../outside.css"),
            None
        );
    }

    #[test]
    fn classify_import_origin_python_reconciles_bare_imports_with_workspace_modules() {
        let workspace_index = import_workspace_index(
            &["numpy"],
            &["@/"],
            &["pkg"],
            &["pkg", "pkg.widgets", "anywidget", "anywidget._descriptor"],
        );

        for import_path in [
            ".local",
            "..shared",
            "pkg",
            "pkg.widgets",
            "anywidget._descriptor",
        ] {
            assert_eq!(
                classify_import_origin(import_path, "pkg/app.py", &workspace_index),
                ImportOriginClassification::Internal,
                "{import_path} should classify as workspace-internal"
            );
        }

        for (import_path, origin, name) in [
            ("numpy", "numpy", "numpy"),
            ("click.core", "click", "core"),
            ("IPython.display", "IPython", "display"),
        ] {
            assert_eq!(
                classify_import_origin(import_path, "pkg/app.py", &workspace_index),
                external_import(origin, import_path, name),
                "{import_path} should classify as external"
            );
        }
    }

    #[test]
    fn classify_import_origin_does_not_leak_rules_between_language_families() {
        let workspace_index =
            import_workspace_index(&["numpy", "react"], &["crate/"], &["std"], &["std"]);

        assert_eq!(
            classify_import_origin("numpy", "pkg/app.py", &workspace_index),
            external_import("numpy", "numpy", "numpy"),
            "Rust crate names must not make Python imports internal"
        );
        assert_eq!(
            classify_import_origin("react", "web/App.tsx", &workspace_index),
            external_import("react", "react", "react"),
            "Python modules and Rust crates must not make JS bare specifiers internal"
        );
        assert_eq!(
            classify_import_origin("std::path::Path", "web/App.tsx", &workspace_index),
            external_import("std::path::Path", "std::path::Path", "std::path::Path"),
            "Rust path rules must not apply to JS/TS files"
        );
    }

    #[test]
    fn symbol_query_policy_documents_shared_and_dedicated_sources() {
        assert_eq!(
            symbol_query_policy(Language::Rust),
            SymbolQueryPolicy::ReuseTags
        );
        assert_eq!(
            symbol_query_policy(Language::TypeScript),
            SymbolQueryPolicy::ReuseTags
        );
        assert_eq!(
            symbol_query_policy(Language::Tsx),
            SymbolQueryPolicy::ReuseTags
        );
        assert_eq!(
            symbol_query_policy(Language::Cpp),
            SymbolQueryPolicy::ReuseTags
        );
        assert_eq!(
            symbol_query_policy(Language::Python),
            SymbolQueryPolicy::ReuseTags
        );
        assert!(matches!(
            symbol_query_policy(Language::Markdown),
            SymbolQueryPolicy::Dedicated(_)
        ));
        assert_eq!(
            symbol_query_policy(Language::JupyterNotebook),
            SymbolQueryPolicy::ReuseTags
        );
    }

    #[test]
    fn jupyter_notebook_extraction_routes_container_before_parse() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nb.ipynb");
        let notebook = serde_json::json!({
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
        let bytes = serde_json::to_vec(&notebook).expect("serialize notebook");
        let mut builder = FactBuilder::new(dir.path());
        let mut extractor = BytesExtractor::for_language(Language::JupyterNotebook)
            .expect("configure notebook extractor");

        extractor
            .extract_graph_facts(&mut builder, &path, &bytes)
            .expect("extract notebook facts");
        let facts = builder.into_facts();

        let file_node = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "nb.ipynb")
            .expect("notebook file node")
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

        for label in ["load_df", "Analysis"] {
            let node = facts
                .nodes
                .iter()
                .find(|node| node.label == label)
                .unwrap_or_else(|| panic!("missing notebook child `{label}`"));
            assert!(reachable.contains(&node.node_id));
        }
    }

    #[test]
    fn function_singleton_safe_same_file_allows() {
        assert!(function_singleton_safe(
            "crates/spur-graph/src/git_walk.rs",
            "crates/spur-graph/src/git_walk.rs"
        ));
    }

    #[test]
    fn function_singleton_safe_same_crate_allows() {
        assert!(function_singleton_safe(
            "crates/spur-graph/src/git_walk.rs",
            "crates/spur-graph/src/temporal.rs"
        ));
    }

    #[test]
    fn function_singleton_safe_cross_crate_blocks() {
        assert!(!function_singleton_safe(
            "crates/spur-bot/src/foo.rs",
            "crates/spur-graph/src/git_walk.rs"
        ));
    }

    #[test]
    fn function_singleton_safe_non_crate_path() {
        assert!(!function_singleton_safe(
            "xtask/src/main.rs",
            "crates/spur-graph/src/git_walk.rs"
        ));
    }

    #[test]
    fn function_singleton_safe_cross_language_blocks() {
        assert!(!function_singleton_safe(
            "crates/spur-notebook/jute-notebook/x.ts",
            "crates/spur-notebook/rest-table-gateway/y.rs"
        ));
    }

    #[test]
    fn function_singleton_safe_same_language_family_allows() {
        assert!(function_singleton_safe(
            "crates/foo/src/a.ts",
            "crates/foo/web/b.tsx"
        ));
    }

    #[test]
    fn singleton_function_call_respects_crate_safety() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("crates/source/src")).expect("source crate dir");
        std::fs::create_dir_all(dir.path().join("crates/callee/src")).expect("callee crate dir");
        std::fs::write(
            dir.path().join("crates/source/src/lib.rs"),
            r#"
pub fn caller() {
    cross_crate_helper();
    same_crate_helper();
    same_file_helper();
}

pub fn same_file_helper() {}
"#,
        )
        .expect("write source lib");
        std::fs::write(
            dir.path().join("crates/source/src/helpers.rs"),
            "pub fn same_crate_helper() {}\n",
        )
        .expect("write source helper");
        std::fs::write(
            dir.path().join("crates/callee/src/lib.rs"),
            "pub fn cross_crate_helper() {}\n",
        )
        .expect("write callee lib");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let call = |label: &str| {
            facts
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Calls
                        && edge.target_label.as_deref() == Some(label)
                })
                .unwrap_or_else(|| panic!("missing call edge for {label}"))
        };

        let cross_crate = call("cross_crate_helper");
        assert_eq!(cross_crate.target_node_id, None);
        assert_eq!(cross_crate.bind_method.as_deref(), None);

        let same_crate = call("same_crate_helper");
        assert!(same_crate.target_node_id.is_some());
        assert_eq!(same_crate.bind_method.as_deref(), Some("singleton"));

        let same_file = call("same_file_helper");
        assert!(same_file.target_node_id.is_some());
        assert_eq!(same_file.bind_method.as_deref(), Some("singleton"));
    }

    #[test]
    fn rust_method_spelling_cannot_substitute_an_incompatible_receiver() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("crates/foo/src/sub")).expect("foo sub dir");
        std::fs::create_dir_all(dir.path().join("crates/bar/src")).expect("bar crate dir");
        std::fs::write(
            dir.path().join("crates/foo/src/a.rs"),
            r#"
pub struct Widget;

impl Widget {
    pub fn repaint_panel(&self) {}
    pub fn clone(&self) {}
}
"#,
        )
        .expect("write foo a");
        std::fs::write(
            dir.path().join("crates/foo/src/sub/b.rs"),
            r#"
pub fn caller(panel: &Panel, shadow: &Shadow) {
    panel.repaint_panel();
    panel.repaint_from_bar();
    shadow.clone();
}
"#,
        )
        .expect("write foo b");
        std::fs::write(
            dir.path().join("crates/bar/src/lib.rs"),
            r#"
pub struct ExternalPanel;

impl ExternalPanel {
    pub fn repaint_from_bar(&self) {}
}
"#,
        )
        .expect("write bar lib");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let call = |label: &str| {
            facts
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Calls
                        && edge.target_label.as_deref() == Some(label)
                })
                .unwrap_or_else(|| panic!("missing call edge for {label}"))
        };

        let same_crate = call("repaint_panel");
        // Panel is not Widget; crate membership cannot establish its method.
        assert_eq!(same_crate.target_node_id, None);
        assert_eq!(same_crate.bind_method.as_deref(), None);

        let cross_crate = call("repaint_from_bar");
        assert_eq!(cross_crate.target_node_id, None);
        assert_eq!(cross_crate.bind_method.as_deref(), None);

        let std_named = call("clone");
        assert_eq!(std_named.target_node_id, None);
        assert_eq!(std_named.bind_method.as_deref(), None);
    }

    #[test]
    fn method_crate_singleton_does_not_capture_ts_builtin_methods() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("crates/foo/web")).expect("foo web dir");
        std::fs::write(
            dir.path().join("crates/foo/web/grid.ts"),
            r#"
export class GridApi {
    forEach(callback: (value: number) => void) {}
    setCellType(kind: string) {}
}
"#,
        )
        .expect("write grid ts");
        std::fs::write(
            dir.path().join("crates/foo/web/app.ts"),
            r#"
export function caller(values: number[], grid: GridApi) {
    values.forEach((value) => value);
    grid.setCellType("text");
}
"#,
        )
        .expect("write app ts");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let call = |label: &str| {
            facts
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Calls
                        && edge.target_label.as_deref() == Some(label)
                })
                .unwrap_or_else(|| panic!("missing call edge for {label}"))
        };

        let builtin_named = call("forEach");
        assert_eq!(builtin_named.target_node_id, None);
        assert_eq!(builtin_named.bind_method.as_deref(), None);

        let domain_named = call("setCellType");
        assert!(domain_named.target_node_id.is_some());
        assert_eq!(
            domain_named.bind_method.as_deref(),
            Some("method_crate_singleton")
        );
    }

    #[test]
    fn singleton_function_call_blocks_cross_language_same_crate() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("crates/mixed/web")).expect("mixed web dir");
        std::fs::create_dir_all(dir.path().join("crates/mixed/src")).expect("mixed src dir");
        std::fs::write(
            dir.path().join("crates/mixed/web/app.ts"),
            r#"
export function caller() {
    rust_only_helper();
}
"#,
        )
        .expect("write mixed ts");
        std::fs::write(
            dir.path().join("crates/mixed/src/lib.rs"),
            "pub fn rust_only_helper() {}\n",
        )
        .expect("write mixed rust");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let call = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Calls
                    && edge.target_label.as_deref() == Some("rust_only_helper")
            })
            .expect("missing call edge for rust_only_helper");

        assert_eq!(call.target_node_id, None);
        assert_eq!(call.bind_method.as_deref(), None);
    }

    #[test]
    fn singleton_hof_reference_respects_crate_safety() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("crates/source/src")).expect("source crate dir");
        std::fs::create_dir_all(dir.path().join("crates/callee/src")).expect("callee crate dir");
        std::fs::write(
            dir.path().join("crates/source/src/lib.rs"),
            r#"
pub fn caller(items: Vec<i32>) {
    let _ = items.iter().copied().map(cross_crate_mapper).collect::<Vec<_>>();
    let _ = items.iter().copied().map(same_crate_mapper).collect::<Vec<_>>();
    let _ = items.iter().copied().map(same_file_mapper).collect::<Vec<_>>();
}

pub fn same_file_mapper(value: i32) -> i32 { value }
"#,
        )
        .expect("write source lib");
        std::fs::write(
            dir.path().join("crates/source/src/helpers.rs"),
            "pub fn same_crate_mapper(value: i32) -> i32 { value }\n",
        )
        .expect("write source helper");
        std::fs::write(
            dir.path().join("crates/callee/src/lib.rs"),
            "pub fn cross_crate_mapper(value: i32) -> i32 { value }\n",
        )
        .expect("write callee lib");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let reference = |label: &str| {
            facts
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::References
                        && edge.target_label.as_deref() == Some(label)
                })
                .unwrap_or_else(|| panic!("missing reference edge for {label}"))
        };

        let cross_crate = reference("cross_crate_mapper");
        assert_eq!(cross_crate.target_node_id, None);

        let same_crate = reference("same_crate_mapper");
        assert!(same_crate.target_node_id.is_some());

        let same_file = reference("same_file_mapper");
        assert!(same_file.target_node_id.is_some());
    }

    #[test]
    fn sql_style_references_resolve_non_callable_singletons_and_keep_ambiguous_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse("fn placeholder() {}\nstruct Orders;\n", None)
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let file = builder.add_file_node("schema.sql", file_id, root_node);
        let source = builder.add_node(
            "schema.sql",
            "orders_view".to_owned(),
            "orders_view".to_owned(),
            NodeKind::TypeAlias,
            file_id,
            root_node,
        );
        let orders = builder.add_node(
            "schema.sql",
            "orders".to_owned(),
            "orders".to_owned(),
            NodeKind::Struct,
            file_id,
            root_node,
        );
        let public_table = builder.add_node(
            "schema.sql",
            "ambiguous_table".to_owned(),
            "public::ambiguous_table".to_owned(),
            NodeKind::Struct,
            file_id,
            root_node,
        );
        let archive_table = builder.add_node(
            "schema.sql",
            "ambiguous_table".to_owned(),
            "archive::ambiguous_table".to_owned(),
            NodeKind::Struct,
            file_id,
            root_node,
        );
        builder.add_edge(file, Some(source), RelationKind::Contains, None);
        builder.add_edge(file, Some(orders), RelationKind::Contains, None);
        builder.add_edge(file, Some(public_table), RelationKind::Contains, None);
        builder.add_edge(file, Some(archive_table), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "orders".to_owned(),
            import_path: None,
            relation: RelationKind::References,
            edge_kind: Some(GraphEdgeKind::ReferencesHof),
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "ambiguous_table".to_owned(),
            import_path: None,
            relation: RelationKind::References,
            edge_kind: Some(GraphEdgeKind::ReferencesHof),
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let resolved = builder
            .facts
            .edges
            .iter()
            .find(|edge| edge.target_label.as_deref() == Some("orders"))
            .expect("orders reference edge");
        assert_eq!(resolved.relation, RelationKind::References);
        assert_eq!(resolved.source_node_id, source);
        assert_eq!(resolved.target_node_id, Some(orders));

        let unresolved = builder
            .facts
            .edges
            .iter()
            .find(|edge| edge.target_label.as_deref() == Some("ambiguous_table"))
            .expect("ambiguous reference edge");
        assert_eq!(unresolved.relation, RelationKind::References);
        assert_eq!(unresolved.source_node_id, source);
        assert_eq!(unresolved.target_node_id, None);
        assert_eq!(
            builder
                .facts
                .edges
                .iter()
                .filter(|edge| edge.relation == RelationKind::References)
                .count(),
            2
        );
    }

    #[test]
    fn sql_partitioned_alter_is_not_captured_as_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("schema.sql"),
            r#"
CREATE TABLE events (
    id INTEGER,
    created_at TIMESTAMP
);

ALTER TABLE events SET PARTITIONED BY (id);
"#,
        )
        .expect("write sql");

        let (facts, _counts) = build_facts(dir.path(), None).expect("build facts");
        let fields: Vec<_> = facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Field)
            .map(|node| node.label.as_str())
            .collect();

        assert!(fields.contains(&"id"));
        assert!(fields.contains(&"created_at"));
        assert!(!fields.contains(&"PARTITIONED"));
    }

    #[test]
    fn path_crate_extracts_correctly() {
        assert_eq!(
            path_crate("crates/spur-graph/src/git_walk.rs"),
            Some("spur-graph")
        );
        assert_eq!(path_crate("xtask/src/main.rs"), None);
        assert_eq!(path_crate("README.md"), None);
    }

    #[test]
    fn ambiguous_pending_bare_name_edge_remains_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn caller() { flush(); }\nfn flush() {}\nmod inner { fn flush() {} }\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let source = builder.add_node(
            "src/lib.rs",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.add_node(
            "src/lib.rs",
            "flush".to_owned(),
            "flush".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.add_node(
            "src/lib.rs",
            "flush".to_owned(),
            "inner::flush".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "flush".to_owned(),
            import_path: None,
            relation: RelationKind::Calls,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        assert_eq!(builder.facts.edges.len(), 1);
        let edge = &builder.facts.edges[0];
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.target_label.as_deref(), Some("flush"));
    }

    #[test]
    fn bare_call_ignores_non_callable_same_label_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn caller() { helper(); }\nfn helper() {}\nstruct App { helper: Helper }\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let source = builder.add_node(
            "src/lib.rs",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        let function = builder.add_node(
            "src/lib.rs",
            "helper".to_owned(),
            "helper".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.add_node(
            "src/lib.rs",
            "helper".to_owned(),
            "App::helper".to_owned(),
            NodeKind::Field,
            file_id,
            root_node,
        );
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "helper".to_owned(),
            import_path: None,
            relation: RelationKind::Calls,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        assert_eq!(builder.facts.edges.len(), 1);
        let edge = &builder.facts.edges[0];
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, Some(function));
        assert_eq!(edge.target_label.as_deref(), Some("helper"));
        assert_eq!(edge.bind_method.as_deref(), Some("singleton"));
    }

    #[test]
    fn constructs_type_singleton_binds_unique_type() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn caller() { let _ = Widget { value: 1 }; }\nstruct Widget { value: i32 }\nfn Widget() {}\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let file = builder.add_file_node("src/lib.rs", file_id, root_node);
        let source = builder.add_node(
            "src/lib.rs",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        let target = builder.add_node(
            "src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Struct,
            file_id,
            root_node,
        );
        builder.add_node(
            "src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.add_edge(file, Some(source), RelationKind::Contains, None);
        builder.add_edge(file, Some(target), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "Widget".to_owned(),
            import_path: None,
            relation: RelationKind::Constructs,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Constructs
                    && edge.target_label.as_deref() == Some("Widget")
            })
            .expect("constructs edge");
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, Some(target));
        assert_eq!(
            edge.bind_method.as_deref(),
            Some("constructs_type_singleton")
        );
    }

    #[test]
    fn constructs_type_singleton_ambiguous_multi_type_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn caller() { let _ = Widget { value: 1 }; }\nstruct Widget { value: i32 }\nenum Widget { Value }\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let file = builder.add_file_node("src/lib.rs", file_id, root_node);
        let source = builder.add_node(
            "src/lib.rs",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        let first = builder.add_node(
            "src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Struct,
            file_id,
            root_node,
        );
        let second = builder.add_node(
            "src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Enum,
            file_id,
            root_node,
        );
        builder.add_edge(file, Some(source), RelationKind::Contains, None);
        builder.add_edge(file, Some(first), RelationKind::Contains, None);
        builder.add_edge(file, Some(second), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "Widget".to_owned(),
            import_path: None,
            relation: RelationKind::Constructs,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Constructs
                    && edge.target_label.as_deref() == Some("Widget")
            })
            .expect("constructs edge");
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn constructs_type_singleton_blocks_cross_language() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse("fn caller() {}\nstruct Widget;\n", None)
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let ts_file_id = FileId(builder.next_file_id());
        let rs_file_id = FileId(builder.next_file_id());
        let ts_file = builder.add_file_node("crates/mixed/web/app.ts", ts_file_id, root_node);
        let rs_file = builder.add_file_node("crates/mixed/src/lib.rs", rs_file_id, root_node);
        let source = builder.add_node(
            "crates/mixed/web/app.ts",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            ts_file_id,
            root_node,
        );
        let target = builder.add_node(
            "crates/mixed/src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Struct,
            rs_file_id,
            root_node,
        );
        builder.add_edge(ts_file, Some(source), RelationKind::Contains, None);
        builder.add_edge(rs_file, Some(target), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "Widget".to_owned(),
            import_path: None,
            relation: RelationKind::Constructs,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Constructs
                    && edge.target_label.as_deref() == Some("Widget")
            })
            .expect("constructs edge");
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn relational_candidates_excludes_cross_language() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse("class Child(Base): pass\ninterface Base {}\n", None)
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let py_source_file_id = FileId(builder.next_file_id());
        let py_target_file_id = FileId(builder.next_file_id());
        let ts_file_id = FileId(builder.next_file_id());
        builder.add_file_node("pkg/widgets.py", py_source_file_id, root_node);
        builder.add_file_node("pkg/base.py", py_target_file_id, root_node);
        builder.add_file_node("pkg/types.ts", ts_file_id, root_node);
        let source = builder.add_node(
            "pkg/widgets.py",
            "Child".to_owned(),
            "Child".to_owned(),
            NodeKind::Class,
            py_source_file_id,
            root_node,
        );
        let py_base = builder.add_node(
            "pkg/base.py",
            "Base".to_owned(),
            "Base".to_owned(),
            NodeKind::Class,
            py_target_file_id,
            root_node,
        );
        let ts_base = builder.add_node(
            "pkg/types.ts",
            "Base".to_owned(),
            "Base".to_owned(),
            NodeKind::Interface,
            ts_file_id,
            root_node,
        );
        let edge = PendingEdge {
            source,
            target_name: "Base".to_owned(),
            import_path: None,
            relation: RelationKind::Extends,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        };
        let singleton_symbols_by_label = HashMap::<String, NodeId>::new();
        let ambiguous_symbols_by_label = HashMap::<String, usize>::new();
        let files_by_label = HashMap::<String, NodeId>::new();
        let pending_imports_by_source = HashMap::<NodeId, Vec<PendingEdge>>::new();
        let file_by_id = HashMap::from([
            (source, "pkg/widgets.py"),
            (py_base, "pkg/base.py"),
            (ts_base, "pkg/types.ts"),
        ]);
        let node_kind_by_id = HashMap::from([
            (source, NodeKind::Class),
            (py_base, NodeKind::Class),
            (ts_base, NodeKind::Interface),
        ]);
        let enclosing_scope_by_id = HashMap::<NodeId, String>::new();
        let qualified_name_by_id = HashMap::<NodeId, String>::new();
        let indexes = PendingResolutionIndexes {
            singleton_symbols_by_label: &singleton_symbols_by_label,
            ambiguous_symbols_by_label: &ambiguous_symbols_by_label,
            files_by_label: &files_by_label,
            pending_imports_by_source: &pending_imports_by_source,
            file_by_id: &file_by_id,
            node_kind_by_id: &node_kind_by_id,
            enclosing_scope_by_id: &enclosing_scope_by_id,
            qualified_name_by_id: &qualified_name_by_id,
        };

        let candidates = relational_symbol_candidates(
            &builder,
            &edge,
            &indexes,
            relational_target_kinds(edge.relation).expect("relational target kinds"),
        );

        assert_eq!(candidates, vec![py_base]);
    }

    #[test]
    fn relational_language_gate_resolves_in_repo_base_class() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("pkg")).expect("mkdir pkg");
        fs::write(
            root.join("pkg/widget.py"),
            r#"
class Child(Base):
    pass
"#,
        )
        .expect("write widget.py");
        fs::write(
            root.join("pkg/base.py"),
            r#"
class Base:
    pass
"#,
        )
        .expect("write base.py");
        fs::write(
            root.join("pkg/types.ts"),
            r#"
export interface Base {}
"#,
        )
        .expect("write types.ts");

        let facts = build_facts_for_paths(
            root,
            &[
                PathBuf::from("pkg/widget.py"),
                PathBuf::from("pkg/base.py"),
                PathBuf::from("pkg/types.ts"),
            ],
        )
        .expect("build facts");
        let child = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Class && node.label == "Child")
            .expect("Child class");
        let py_base = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Class && node.label == "Base")
            .expect("Python Base class");
        let ts_base = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Interface && node.label == "Base")
            .expect("TypeScript Base interface");

        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.source_node_id == child.node_id
                    && edge.relation == RelationKind::Extends
                    && edge.target_label.as_deref() == Some("Base")
            })
            .expect("Child extends Base edge");

        assert_eq!(edge.target_node_id, Some(py_base.node_id));
        assert_ne!(edge.target_node_id, Some(ts_base.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("relational"));
    }

    #[test]
    fn constructs_type_singleton_behavior_recovers_type_non_type_collision() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn make_widget() {}\nstruct Widget { value: i32 }\nfn Widget() {}\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let source_file_id = FileId(builder.next_file_id());
        let target_file_id = FileId(builder.next_file_id());
        let source_file = builder.add_file_node("crates/app/src/lib.rs", source_file_id, root_node);
        let target_file =
            builder.add_file_node("crates/types/src/lib.rs", target_file_id, root_node);
        let source = builder.add_node(
            "crates/app/src/lib.rs",
            "make_widget".to_owned(),
            "make_widget".to_owned(),
            NodeKind::Function,
            source_file_id,
            root_node,
        );
        let target = builder.add_node(
            "crates/types/src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Struct,
            target_file_id,
            root_node,
        );
        let non_type = builder.add_node(
            "crates/types/src/lib.rs",
            "Widget".to_owned(),
            "Widget".to_owned(),
            NodeKind::Function,
            target_file_id,
            root_node,
        );
        builder.add_edge(source_file, Some(source), RelationKind::Contains, None);
        builder.add_edge(target_file, Some(target), RelationKind::Contains, None);
        builder.add_edge(target_file, Some(non_type), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "Widget".to_owned(),
            import_path: None,
            relation: RelationKind::Constructs,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.source_node_id == source
                    && edge.relation == RelationKind::Constructs
                    && edge.target_label.as_deref() == Some("Widget")
            })
            .expect("Widget constructs edge");
        assert_eq!(edge.target_node_id, Some(target));
        assert_eq!(
            edge.bind_method.as_deref(),
            Some("constructs_type_singleton")
        );
    }

    #[test]
    fn import_ignores_non_importable_same_label_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "use crate::utils::helper;\nfn helper() {}\nstruct App { helper: Helper }\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let source = builder.add_node(
            "src/lib.rs",
            "src/lib.rs".to_owned(),
            "src/lib.rs".to_owned(),
            NodeKind::File,
            file_id,
            root_node,
        );
        let function = builder.add_node(
            "src/utils.rs",
            "helper".to_owned(),
            "helper".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.add_node(
            "src/lib.rs",
            "helper".to_owned(),
            "App::helper".to_owned(),
            NodeKind::Field,
            file_id,
            root_node,
        );
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "helper".to_owned(),
            import_path: None,
            relation: RelationKind::Imports,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        assert_eq!(builder.facts.edges.len(), 1);
        let edge = &builder.facts.edges[0];
        assert_eq!(edge.source_node_id, source);
        assert_eq!(edge.target_node_id, Some(function));
        assert_eq!(edge.target_label.as_deref(), Some("helper"));
    }

    #[test]
    fn import_candidate_hygiene_collapses_impl_shadow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "use crate::model::SessionId;\npub struct SessionId;\nimpl SessionId {}\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let source_file_id = FileId(builder.next_file_id());
        let target_file_id = FileId(builder.next_file_id());
        let source_file = builder.add_file_node("src/lib.rs", source_file_id, root_node);
        let target_file = builder.add_file_node("src/model.rs", target_file_id, root_node);
        let target = builder.add_node(
            "src/model.rs",
            "SessionId".to_owned(),
            "SessionId".to_owned(),
            NodeKind::Struct,
            target_file_id,
            root_node,
        );
        let impl_shadow = builder.add_node(
            "src/model.rs",
            "SessionId".to_owned(),
            "impl SessionId".to_owned(),
            NodeKind::Impl,
            target_file_id,
            root_node,
        );
        builder.add_edge(target_file, Some(target), RelationKind::Contains, None);
        builder.add_edge(target_file, Some(impl_shadow), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source: source_file,
            target_name: "SessionId".to_owned(),
            import_path: None,
            relation: RelationKind::Imports,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("SessionId")
            })
            .expect("SessionId import edge");
        assert_eq!(edge.source_node_id, source_file);
        assert_eq!(edge.target_node_id, Some(target));
    }

    #[test]
    fn import_resolves_unique_type_despite_enum_variant_shadow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod model;
mod shadow;

use crate::model::Foo;

pub fn consume(_foo: Foo) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/model.rs"), "pub struct Foo;\n").expect("write model.rs");
        fs::write(
            root.join("src/shadow.rs"),
            r#"
pub enum SomeEnum {
    Foo,
}
"#,
        )
        .expect("write shadow.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let target = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Struct && node.label == "Foo")
            .expect("Foo struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
            })
            .expect("Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
    }

    #[test]
    fn import_path_disambiguates_same_bare_type_across_modules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod left;
mod right;

use crate::right::Dup;

pub fn consume(_value: Dup) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/left.rs"), "pub struct Dup;\n").expect("write left.rs");
        fs::write(root.join("src/right.rs"), "pub struct Dup;\n").expect("write right.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let right_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/right.rs")
            .and_then(|node| node.file_id)
            .expect("right file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Struct
                    && node.label == "Dup"
                    && node.file_id == Some(right_file_id)
            })
            .expect("right Dup struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Dup")
                    && edge.import_path.as_deref() == Some("crate::right::Dup")
            })
            .expect("Dup import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_single_reexport_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod other;
mod prelude;
mod real;

use crate::prelude::Foo;

pub fn consume(_foo: Foo) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/other.rs"), "pub struct Foo;\n").expect("write other.rs");
        fs::write(root.join("src/prelude.rs"), "pub use crate::real::Foo;\n")
            .expect("write prelude.rs");
        fs::write(root.join("src/real.rs"), "pub struct Foo;\n").expect("write real.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/real.rs")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Struct
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("crate::prelude::Foo")
            })
            .expect("prelude Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_two_hop_reexport_chain_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod first;
mod other;
mod real;
mod second;

use crate::first::Foo;

pub fn consume(_foo: Foo) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/first.rs"), "pub use crate::second::Foo;\n")
            .expect("write first.rs");
        fs::write(root.join("src/other.rs"), "pub struct Foo;\n").expect("write other.rs");
        fs::write(root.join("src/real.rs"), "pub struct Foo;\n").expect("write real.rs");
        fs::write(root.join("src/second.rs"), "pub use crate::real::Foo;\n")
            .expect("write second.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/real.rs")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Struct
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("crate::first::Foo")
            })
            .expect("first Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_rust_glob_reexport_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod other;
mod prelude;
mod real;

use crate::prelude::Foo;

pub fn consume(_foo: Foo) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/other.rs"), "pub struct Foo;\n").expect("write other.rs");
        fs::write(root.join("src/prelude.rs"), "pub use crate::real::*;\n")
            .expect("write prelude.rs");
        fs::write(root.join("src/real.rs"), "pub struct Foo;\n").expect("write real.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/real.rs")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Struct
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("crate::prelude::Foo")
            })
            .expect("prelude Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_typescript_reexport_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/index.ts"),
            r#"
import { Foo } from "./prelude";

export function consume(foo: Foo) {
  return foo;
}
"#,
        )
        .expect("write index.ts");
        fs::write(root.join("src/other.ts"), "export class Foo {}\n").expect("write other.ts");
        fs::write(
            root.join("src/prelude.ts"),
            "export { Foo } from \"./real\";\n",
        )
        .expect("write prelude.ts");
        fs::write(root.join("src/real.ts"), "export class Foo {}\n").expect("write real.ts");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/real.ts")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Class
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo class");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("./prelude")
            })
            .expect("prelude Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_typescript_glob_reexport_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/index.ts"),
            r#"
import { Foo } from "./prelude";

export function consume(foo: Foo) {
  return foo;
}
"#,
        )
        .expect("write index.ts");
        fs::write(root.join("src/other.ts"), "export class Foo {}\n").expect("write other.ts");
        fs::write(root.join("src/prelude.ts"), "export * from \"./real\";\n")
            .expect("write prelude.ts");
        fs::write(root.join("src/real.ts"), "export class Foo {}\n").expect("write real.ts");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "src/real.ts")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Class
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo class");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("./prelude")
            })
            .expect("prelude Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_follows_python_init_reexport_to_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("pkg")).expect("mkdir pkg");
        fs::write(root.join("app.py"), "from pkg import Foo\n").expect("write app.py");
        fs::write(root.join("pkg/__init__.py"), "from .real import Foo\n")
            .expect("write __init__.py");
        fs::write(root.join("pkg/other.py"), "class Foo:\n    pass\n").expect("write other.py");
        fs::write(root.join("pkg/real.py"), "class Foo:\n    pass\n").expect("write real.py");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let real_file_id = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::File && node.label == "pkg/real.py")
            .and_then(|node| node.file_id)
            .expect("real file id");
        let target = facts
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::Class
                    && node.label == "Foo"
                    && node.file_id == Some(real_file_id)
            })
            .expect("real Foo class");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("pkg")
            })
            .expect("pkg Foo import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn import_path_reexport_cycle_stays_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod a;
mod b;
mod left;
mod right;

use crate::a::Foo;

pub fn consume(_foo: Foo) {}
"#,
        )
        .expect("write lib.rs");
        fs::write(root.join("src/a.rs"), "pub use crate::b::Foo;\n").expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub use crate::a::Foo;\n").expect("write b.rs");
        fs::write(root.join("src/left.rs"), "pub struct Foo;\n").expect("write left.rs");
        fs::write(root.join("src/right.rs"), "pub struct Foo;\n").expect("write right.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
                    && edge.import_path.as_deref() == Some("crate::a::Foo")
            })
            .expect("cyclic Foo import edge");

        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn ambiguous_glob_reexport_stays_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse("pub use crate::left::*;\nuse crate::prelude::Foo;\n", None)
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let lib_file_id = FileId(builder.next_file_id());
        let left_file_id = FileId(builder.next_file_id());
        let right_file_id = FileId(builder.next_file_id());
        let prelude_file_id = FileId(builder.next_file_id());
        let source = builder.add_file_node("src/lib.rs", lib_file_id, root_node);
        let left_file = builder.add_file_node("src/left.rs", left_file_id, root_node);
        let right_file = builder.add_file_node("src/right.rs", right_file_id, root_node);
        let prelude_file = builder.add_file_node("src/prelude.rs", prelude_file_id, root_node);
        let left_foo = builder.add_node(
            "src/left.rs",
            "Foo".to_owned(),
            "Foo".to_owned(),
            NodeKind::Struct,
            left_file_id,
            root_node,
        );
        let right_foo = builder.add_node(
            "src/right.rs",
            "Foo".to_owned(),
            "Foo".to_owned(),
            NodeKind::Struct,
            right_file_id,
            root_node,
        );
        builder.add_edge(left_file, Some(left_foo), RelationKind::Contains, None);
        builder.add_edge(right_file, Some(right_foo), RelationKind::Contains, None);
        builder.pending_edges.push(PendingEdge {
            source: prelude_file,
            target_name: "*".to_owned(),
            import_path: Some("crate::left::*".to_owned()),
            relation: RelationKind::Imports,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });
        builder.pending_edges.push(PendingEdge {
            source: prelude_file,
            target_name: "*".to_owned(),
            import_path: Some("crate::right::*".to_owned()),
            relation: RelationKind::Imports,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "Foo".to_owned(),
            import_path: Some("crate::prelude::Foo".to_owned()),
            relation: RelationKind::Imports,
            edge_kind: None,
            origin: CallOrigin::Expression,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        let edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.source_node_id == source
                    && edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Foo")
            })
            .expect("Foo import edge");
        assert_eq!(edge.target_node_id, None);
    }

    #[test]
    fn import_path_empty_workspace_match_falls_back_to_unique_cross_crate_type_unstamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("crates/spur-acp/src")).expect("mkdir spur-acp src");
        fs::create_dir_all(root.join("crates/spur-core/src")).expect("mkdir spur-core src");
        fs::write(root.join("crates/spur-acp/src/lib.rs"), "pub mod types;\n")
            .expect("write spur-acp lib.rs");
        fs::write(
            root.join("crates/spur-acp/src/types.rs"),
            "pub struct SessionId;\n",
        )
        .expect("write spur-acp types.rs");
        fs::write(
            root.join("crates/spur-core/src/lib.rs"),
            r#"
use spur_acp::types::SessionId;

pub fn attach(_session: SessionId) {}
"#,
        )
        .expect("write spur-core lib.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let target = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Struct && node.label == "SessionId")
            .expect("SessionId struct");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("SessionId")
                    && edge.import_path.as_deref() == Some("spur_acp::types::SessionId")
            })
            .expect("SessionId import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn import_path_empty_workspace_match_rejects_fallback_variant_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod shadow;

use std::path::Path;
"#,
        )
        .expect("write lib.rs");
        fs::write(
            root.join("src/shadow.rs"),
            r#"
pub enum Shadow {
    Path,
}
"#,
        )
        .expect("write shadow.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Path")
                    && edge.import_path.as_deref() == Some("std::path::Path")
            })
            .expect("Path import edge");

        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn import_path_fallback_rejects_cross_language_type_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub struct Widget;\n").expect("write lib.rs");
        fs::write(
            root.join("consumer.py"),
            "from external.widget import Widget\n",
        )
        .expect("write consumer.py");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Widget")
                    && edge.import_path.as_deref() == Some("external.widget")
            })
            .expect("Widget import edge");

        assert_eq!(edge.target_node_id, None);
        assert_eq!(edge.bind_method.as_deref(), None);
    }

    #[test]
    fn import_keeps_enum_variant_candidate_when_no_type_like_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod shadow;

use crate::shadow::Variant;
"#,
        )
        .expect("write lib.rs");
        fs::write(
            root.join("src/shadow.rs"),
            r#"
pub enum SomeEnum {
    Variant,
}
"#,
        )
        .expect("write shadow.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let target = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::EnumVariant && node.label == "Variant")
            .expect("Variant enum variant");
        let edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_label.as_deref() == Some("Variant")
            })
            .expect("Variant import edge");

        assert_eq!(edge.target_node_id, Some(target.node_id));
    }

    #[test]
    fn rust_use_list_import_ignores_same_label_field_during_extraction() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
use crate::utils::{helper, Helper};

pub struct App {
    helper: Helper,
}

impl App {
    pub fn run(&self) {
        helper();
    }
}
"#,
        )
        .expect("write lib.rs");
        fs::write(
            root.join("src/utils.rs"),
            r#"
pub struct Helper;

pub fn helper() {}
"#,
        )
        .expect("write utils.rs");

        let facts = crate::build_facts(root, None).expect("extract").0;
        let helper = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Function && node.label == "helper")
            .expect("helper function");

        assert!(
            facts.edges.iter().any(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_node_id == Some(helper.node_id)
                    && edge.target_label.as_deref() == Some("helper")
            }),
            "expected helper import to resolve to the function, not the same-label field"
        );
        assert!(
            !facts.edges.iter().any(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_node_id.is_none()
                    && edge.target_label.as_deref() == Some("helper")
            }),
            "helper import should not also leave an unresolved duplicate edge"
        );

        let artifact = crate::store::build::artifact_from_facts(&facts, root).expect("artifact");
        let helper_artifact = artifact
            .symbols
            .iter()
            .find(|symbol| symbol.symbol_kind == "function" && symbol.entity_name == "helper")
            .expect("helper artifact symbol");
        assert!(
            artifact.edges.iter().any(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.target_stable_symbol_id.as_deref()
                        == Some(helper_artifact.stable_symbol_id.as_str())
                    && edge.target_label.as_deref() == Some("helper")
            }),
            "expected helper import artifact edge to keep the resolved function target"
        );
    }

    #[test]
    fn bare_call_to_cfg_duplicate_free_function_resolves_to_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        let path = root.join("src/lib.rs");
        let source = r#"
#[cfg(unix)]
fn send_control() {}

#[cfg(not(unix))]
fn send_control() {}

pub fn caller() {
    send_control();
}
"#;
        fs::write(&path, source).expect("write lib.rs");

        let facts =
            build_facts_for_paths(root, &[PathBuf::from("src/lib.rs")]).expect("build facts");
        let caller = facts
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Function && node.label == "caller")
            .expect("caller node");
        let duplicate_targets = facts
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Function && node.label == "send_control")
            .map(|node| node.node_id)
            .collect::<Vec<_>>();
        assert_eq!(duplicate_targets.len(), 2);

        let call_edge = facts
            .edges
            .iter()
            .find(|edge| {
                edge.source_node_id == caller.node_id
                    && edge.relation == RelationKind::Calls
                    && edge.target_label.as_deref() == Some("send_control")
            })
            .expect("send_control call edge");

        let target = call_edge
            .target_node_id
            .expect("cfg duplicate call should resolve to one target");
        assert!(duplicate_targets.contains(&target));
    }

    #[test]
    fn macro_body_singleton_pending_edge_is_heuristic_and_stamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language).expect("configure parser");
        let tree = parser
            .parse(
                "fn caller() { json!({ \"x\": helper() }); }\nfn helper() {}\n",
                None,
            )
            .expect("parse source");
        let root_node = tree.root_node();

        let mut builder = FactBuilder::new(dir.path());
        let file_id = FileId(builder.next_file_id());
        let source = builder.add_node(
            "src/lib.rs",
            "caller".to_owned(),
            "caller".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        let target = builder.add_node(
            "src/lib.rs",
            "helper".to_owned(),
            "helper".to_owned(),
            NodeKind::Function,
            file_id,
            root_node,
        );
        builder.pending_edges.push(PendingEdge {
            source,
            target_name: "helper".to_owned(),
            import_path: None,
            relation: RelationKind::Calls,
            edge_kind: None,
            origin: CallOrigin::MacroBody,
            receiver_text: None,
            scope_text: None,
        });

        builder.resolve_pending_edges();

        assert_eq!(builder.facts.edges.len(), 1);
        let edge = &builder.facts.edges[0];
        assert_eq!(edge.target_node_id, Some(target));
        assert_eq!(edge.confidence, Confidence::Heuristic);
        assert_eq!(edge.confidence_score, 0.8);
        assert_eq!(edge.bind_method.as_deref(), Some("macro_body_singleton"));
    }

    #[test]
    fn rust_edges_group_receivers_by_query_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        let path = root.join("src/lib.rs");
        let source = "pub struct Outer;\n\
             pub struct Inner;\n\
             impl Outer {\n\
                 pub fn foo(&self, _handler: fn(&Inner)) {}\n\
             }\n\
             impl Inner {\n\
                 pub fn bar(&self) {}\n\
             }\n\
             pub fn caller(outer: &Outer) {\n\
                 outer.foo({\n\
                     fn nested(inner: &Inner) {\n\
                         inner.bar();\n\
                     }\n\
                     nested\n\
                 });\n\
             }\n";
        fs::write(&path, source).expect("write lib.rs");

        let config = crate::extract::languages::rust_config();
        let queries = compile_queries(&config, Language::Rust).expect("compile queries");
        let mut parser = Parser::new();
        parser
            .set_language(&config.language)
            .expect("configure parser");
        let tree = parser.parse(source.as_bytes(), None).expect("parse source");
        let mut builder = FactBuilder::new(root);

        extract_file_from_tree(
            &mut builder,
            "rust",
            &config,
            &path,
            source,
            tree.root_node(),
            &queries,
        )
        .expect("extract file");

        let caller = builder
            .facts
            .nodes
            .iter()
            .find(|node| node.label == "caller" && node.kind == NodeKind::Function)
            .expect("caller function");
        let outer_edge = builder
            .facts
            .edges
            .iter()
            .find(|edge| {
                edge.source_node_id == caller.node_id && edge.target_label.as_deref() == Some("foo")
            })
            .expect("outer foo edge");

        assert_eq!(outer_edge.receiver_text.as_deref(), Some("outer"));
        assert_ne!(outer_edge.receiver_text.as_deref(), Some("inner"));
        assert!(!builder
            .facts
            .edges
            .iter()
            .any(|edge| edge.source_node_id == caller.node_id
                && edge.target_label.as_deref() == Some("bar")));
    }
}
