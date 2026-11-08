use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{anyhow, Context as _};
use serde::{Deserialize, Serialize};

use crate::{EdgeId, EvidenceId, FileId, NodeId, RunId, SpanId};

pub const CODE_FILE_URI_PREFIX: &str = "graph://file/";
pub const CODE_SYMBOL_URI_PREFIX: &str = "graph://symbol/";
pub const GRAPH_INDEX_VERSION_TEMPORAL: &str = "4";

pub type SourceRange = [usize; 2];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeMentionKind {
    File,
    Symbol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeMentionPayload {
    pub authoritative: CodeMentionAuthoritative,
    pub extraction_hints: CodeMentionExtractionHints,
    pub display_meta: CodeMentionDisplayMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeMentionAuthoritative {
    pub display: String,
    pub uri: String,
    pub kind: CodeMentionKind,
    pub file_path: String,
    pub validation: CodeMentionValidationSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeMentionValidationSpec {
    FileExists {
        path: String,
    },
    SymbolRange {
        path: String,
        line_range: SourceRange,
        byte_range: SourceRange,
        entity_name: String,
        anchor_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeMentionExtractionHints {
    pub line_range: Option<SourceRange>,
    pub byte_range: Option<SourceRange>,
    pub symbol_kind: Option<String>,
    pub entity_name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub qualified_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeMentionDisplayMeta {
    pub enclosing_scope: Option<String>,
    pub graph_index_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphIndexArtifact {
    pub header: GraphIndexHeader,
    #[serde(default)]
    pub manifest_version: String,
    #[serde(default)]
    pub graph_content_hash: String,
    #[serde(default)]
    pub file_manifests: Vec<GraphFileManifestEntry>,
    pub files: Vec<GraphFileArtifact>,
    #[serde(skip)]
    pub file_node_ids: Vec<NodeId>,
    pub symbols: Vec<GraphSymbolArtifact>,
    #[serde(skip)]
    pub symbol_node_ids: Vec<NodeId>,
    #[serde(default)]
    pub edges: Vec<GraphEdgeArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macro_invocations: Option<crate::MacroInvocationEvidenceSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_evidence: Option<crate::RustEvidenceSet>,
    #[serde(default)]
    pub tombstones: Vec<GraphTombstoneEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<CommitArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbol_snapshots: Vec<SymbolSnapshotArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_edges: Vec<TemporalEdgeArtifact>,
}

impl GraphIndexArtifact {
    /// Construct an unchecked draft with no captured evidence or completeness claim.
    pub fn empty() -> Self {
        Self {
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: crate::store::current_manifest_version(),
            graph_content_hash: crate::compute_graph_content_hash(
                std::iter::empty::<(&str, &str)>(),
            ),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: Vec::new(),
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            macro_invocations: None,
            rust_evidence: None,
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    /// Preparation guard; current typed fields are already legacy-representable.
    /// The later artifact cutover supplies the legacy adapter check.
    pub(crate) fn validate_legacy_evidence_export(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Return an owned view of the current typed field without validating it.
    pub fn rust_evidence_set(&self) -> anyhow::Result<Option<crate::RustEvidenceSet>> {
        Ok(self.rust_evidence.clone())
    }

    /// Replace the typed lane exactly, preserving absence and captured-empty values.
    pub fn set_rust_evidence(&mut self, set: Option<crate::RustEvidenceSet>) -> anyhow::Result<()> {
        self.rust_evidence = set;
        Ok(())
    }

    /// Return an owned view of the current typed field without validating it.
    pub fn macro_evidence_set(&self) -> anyhow::Result<Option<crate::MacroInvocationEvidenceSet>> {
        Ok(self.macro_invocations.clone())
    }

    /// Replace the typed lane exactly, preserving absence and captured-empty values.
    pub fn set_macro_evidence(
        &mut self,
        set: Option<crate::MacroInvocationEvidenceSet>,
    ) -> anyhow::Result<()> {
        self.macro_invocations = set;
        Ok(())
    }

    /// Authenticate captured Rust evidence against the graph and exact bytes.
    /// Legacy absence is unavailable, including on an otherwise empty graph.
    pub fn validate_rust_evidence_sources(
        &self,
        sources: &[crate::RustDeclarationSource<'_>],
    ) -> anyhow::Result<()> {
        let set = self.rust_evidence_set()?.ok_or_else(|| {
            anyhow::anyhow!("Rust evidence unavailable; rebuild with Rust evidence capture enabled")
        })?;
        set.validate_sources(self, sources)
    }

    /// Check optional direct macro evidence against exact bytes, authoritative
    /// lexical scopes and this graph's physical files and function symbols.
    /// Evidence-only scope IDs never establish graph symbol membership.
    pub fn validate_macro_invocations(
        &self,
        sources: &[crate::MacroEvidenceSource<'_>],
        scopes: &[crate::MacroScopeEvidence],
    ) -> anyhow::Result<()> {
        let Some(set) = self.macro_evidence_set()? else {
            return Ok(());
        };
        set.validate(&self.graph_content_hash, sources, scopes)?;
        for scope in &set.scopes {
            anyhow::ensure!(
                self.file_manifests.iter().any(
                    |file| file.path == scope.file_path && file.content_oid == scope.source_oid
                ),
                "macro scope does not match artifact source manifest"
            );
        }
        for row in &set.rows {
            anyhow::ensure!(
                self.file_manifests
                    .iter()
                    .any(|file| file.path == row.file_path && file.content_oid == row.source_oid),
                "macro site does not match artifact source manifest"
            );
            if let Some(caller) = &row.caller_stable_symbol_id {
                let enclosing = set
                    .scopes
                    .iter()
                    .find(|scope| scope.scope == row.enclosing)
                    .context("missing persisted caller scope")?;
                anyhow::ensure!(
                    self.symbols
                        .iter()
                        .any(|symbol| symbol.stable_symbol_id == *caller
                            && symbol.file_path == row.file_path
                            && matches!(symbol.symbol_kind.as_str(), "function" | "method")
                            && symbol.byte_range == enclosing.byte_range
                            && symbol.line_range == enclosing.line_range),
                    "macro caller is not an actual graph function symbol"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphIndexHeader {
    pub graph_index_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash_blake3: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphFileArtifact {
    pub stable_file_id: String,
    pub file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphFileManifestEntry {
    pub stable_file_id: String,
    pub path: String,
    pub content_oid: String,
    pub node_ids: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphTombstoneEntry {
    pub path: String,
    pub stable_file_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphIndexPointer {
    pub schema: String,
    pub graph_content_hash: String,
    pub manifest_version: String,
    pub source_kind: SourceKind,
    pub indexed_commit_oid: Option<String>,
    pub canonical_artifact_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Git,
    Fs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSymbolArtifact {
    pub stable_symbol_id: String,
    pub file_path: String,
    pub byte_range: SourceRange,
    pub line_range: SourceRange,
    pub entity_name: String,
    #[serde(default)]
    pub qualified_name: String,
    pub symbol_kind: String,
    pub anchor_hash: String,
    pub enclosing_scope: Option<String>,
}

// Symbol artifacts already persist byte/line ranges, so edge artifacts only carry
// stable symbol identifiers and relation metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdgeArtifact {
    pub source_stable_symbol_id: String,
    pub target_stable_symbol_id: Option<String>,
    pub target_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_path: Option<String>,
    /// Receiver expression text captured at the call site, when syntax exposes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_text: Option<String>,
    /// Scope evidence for the call, from syntactic qualification or an inferred receiver type.
    /// This is evidence for syntax-first resolution, not compiler-grade semantic certainty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_text: Option<String>,
    pub relation: RelationKind,
    pub confidence: Confidence,
    pub confidence_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<ChangeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<GraphEdgeKind>,
    /// Provenance for resolved edge targets.
    ///
    /// Current resolver stamps include `fqn`, `scope_match`, `singleton`,
    /// `bare_qualified_singleton`, `macro_body_singleton`, `relational`,
    /// `constructs_type_singleton`, `method_crate_singleton`, `import_path`,
    /// `import_licensed`, `external`, `address_module_scope`, and
    /// `address_singleton`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub node_id: NodeId,
    pub stable_key: String,
    pub label: String,
    pub kind: NodeKind,
    pub file_id: Option<FileId>,
    pub source_span_id: Option<SpanId>,
    pub first_seen_run_id: RunId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub edge_id: EdgeId,
    pub source_node_id: NodeId,
    pub target_node_id: Option<NodeId>,
    pub relation: RelationKind,
    pub target_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_path: Option<String>,
    /// Receiver expression text captured at the call site, when syntax exposes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_text: Option<String>,
    /// Scope evidence for the call, from syntactic qualification or an inferred receiver type.
    /// This is evidence for syntax-first resolution, not compiler-grade semantic certainty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_text: Option<String>,
    pub confidence: Confidence,
    pub confidence_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<GraphEdgeKind>,
    /// Provenance for resolved edge targets. See `GraphEdgeArtifact::bind_method`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_method: Option<String>,
    pub evidence_id: EvidenceId,
    pub directed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<ChangeKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub span_id: SpanId,
    pub file_id: FileId,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Module,
    ExternBlock,
    Function,
    Class,
    Interface,
    Struct,
    Impl,
    Trait,
    Enum,
    EnumVariant,
    File,
    External,
    Method,
    Field,
    Constant,
    TypeAlias,
    Macro,
    Section,
    Commit,
    McpTool,
    Cell,
    Port,
    Resource,
}

impl NodeKind {
    pub fn discriminator(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::External => "external",
            Self::Module => "module",
            Self::ExternBlock => "extern_block",
            Self::Function => "function",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::EnumVariant => "enum_variant",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Field => "field",
            Self::Constant => "constant",
            Self::TypeAlias => "type_alias",
            Self::Macro => "macro",
            Self::Section => "section",
            Self::Commit => "commit",
            Self::McpTool => "mcp_tool",
            Self::Cell => "cell",
            Self::Port => "port",
            Self::Resource => "resource",
        }
    }

    pub fn from_discriminator(discriminator: &str) -> Option<Self> {
        let deserializer =
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(discriminator);
        Self::deserialize(deserializer).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Imports,
    Calls,
    Constructs,
    Contains,
    Implements,
    Defines,
    References,
    Extends,
    Links,
    Touches,
    Produces,
    Consumes,
    Binds,
    Emits,
}

/// Cardinality of a predicate: how many objects one subject may relate to (and back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationCardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

/// Tier-0 algebra of a predicate. Inverses are virtual reverse-edge labels, not
/// distinct `RelationKind` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationMetadata {
    pub inverse_label: Option<&'static str>,
    pub cardinality: RelationCardinality,
    pub transitive: bool,
}

impl RelationKind {
    pub fn metadata(self) -> RelationMetadata {
        use RelationCardinality::*;

        match self {
            Self::Imports => RelationMetadata {
                inverse_label: Some("imported_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Calls => RelationMetadata {
                inverse_label: Some("called_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Constructs => RelationMetadata {
                inverse_label: Some("constructed_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Contains => RelationMetadata {
                inverse_label: Some("contained_by"),
                cardinality: OneToMany,
                transitive: true,
            },
            Self::Implements => RelationMetadata {
                inverse_label: Some("implemented_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Defines => RelationMetadata {
                inverse_label: Some("defined_in"),
                cardinality: OneToMany,
                transitive: false,
            },
            Self::References => RelationMetadata {
                inverse_label: Some("referenced_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Extends => RelationMetadata {
                inverse_label: Some("extended_by"),
                cardinality: ManyToMany,
                transitive: true,
            },
            Self::Links => RelationMetadata {
                inverse_label: Some("linked_from"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Touches => RelationMetadata {
                inverse_label: Some("touched_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Produces => RelationMetadata {
                inverse_label: Some("produced_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Consumes => RelationMetadata {
                inverse_label: Some("consumed_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Binds => RelationMetadata {
                inverse_label: Some("bound_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
            Self::Emits => RelationMetadata {
                inverse_label: Some("emitted_by"),
                cardinality: ManyToMany,
                transitive: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    Calls,
    CallsDyn,
    ReferencesHof,
    ReferencesOther,
    ReferencesAddress,
}

pub fn graph_edge_kind_or_default(
    relation: RelationKind,
    edge_kind: Option<GraphEdgeKind>,
) -> GraphEdgeKind {
    edge_kind.unwrap_or(match relation {
        RelationKind::Calls | RelationKind::Constructs => GraphEdgeKind::Calls,
        RelationKind::References => GraphEdgeKind::ReferencesOther,
        _ => GraphEdgeKind::ReferencesOther,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    SyntaxExact,
    Heuristic,
    Unknown,
}

pub type StableSymbolId = String;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GitPath(Vec<u8>);

impl GitPath {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn to_path_buf(&self) -> PathBuf {
        pathbuf_from_git_bytes(&self.0)
    }

    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    pub fn display(&self) -> GitPathDisplay<'_> {
        GitPathDisplay(&self.0)
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.to_path_buf().ends_with(child)
    }
}

impl AsRef<[u8]> for GitPath {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&str> for GitPath {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

impl From<String> for GitPath {
    fn from(value: String) -> Self {
        Self(value.into_bytes())
    }
}

impl From<Vec<u8>> for GitPath {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<&[u8]> for GitPath {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

impl<const N: usize> From<&[u8; N]> for GitPath {
    fn from(value: &[u8; N]) -> Self {
        Self(value.to_vec())
    }
}

impl From<&Path> for GitPath {
    fn from(value: &Path) -> Self {
        Self(git_bytes_from_path(value))
    }
}

impl From<PathBuf> for GitPath {
    fn from(value: PathBuf) -> Self {
        Self::from(value.as_path())
    }
}

impl From<&PathBuf> for GitPath {
    fn from(value: &PathBuf) -> Self {
        Self::from(value.as_path())
    }
}

impl PartialEq<PathBuf> for GitPath {
    fn eq(&self, other: &PathBuf) -> bool {
        self.0 == git_bytes_from_path(other.as_path())
    }
}

impl PartialEq<GitPath> for PathBuf {
    fn eq(&self, other: &GitPath) -> bool {
        git_bytes_from_path(self.as_path()) == other.0
    }
}

impl Serialize for GitPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Ok(path) = std::str::from_utf8(&self.0) {
            serializer.serialize_str(path)
        } else {
            use serde::ser::SerializeStruct as _;

            let mut state = serializer.serialize_struct("GitPath", 2)?;
            state.serialize_field("encoding", "base64")?;
            state.serialize_field("bytes", &base64_encode(&self.0))?;
            state.end()
        }
    }
}

impl<'de> Deserialize<'de> for GitPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(GitPathVisitor)
    }
}

struct GitPathVisitor;

impl<'de> serde::de::Visitor<'de> for GitPathVisitor {
    type Value = GitPath;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a UTF-8 path string or a base64-encoded Git path object")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(GitPath::from(value))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(GitPath::from(value))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut encoding = None::<String>;
        let mut bytes = None::<String>;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "encoding" => {
                    if encoding.is_some() {
                        return Err(serde::de::Error::duplicate_field("encoding"));
                    }
                    encoding = Some(map.next_value()?);
                }
                "bytes" => {
                    if bytes.is_some() {
                        return Err(serde::de::Error::duplicate_field("bytes"));
                    }
                    bytes = Some(map.next_value()?);
                }
                other => {
                    return Err(serde::de::Error::unknown_field(
                        other,
                        &["encoding", "bytes"],
                    ));
                }
            }
        }

        match encoding.as_deref() {
            Some("base64") => {}
            Some(other) => {
                return Err(serde::de::Error::custom(format!(
                    "unsupported GitPath encoding `{other}`"
                )));
            }
            None => return Err(serde::de::Error::missing_field("encoding")),
        }

        let bytes = bytes.ok_or_else(|| serde::de::Error::missing_field("bytes"))?;
        base64_decode(&bytes)
            .map(GitPath)
            .map_err(serde::de::Error::custom)
    }
}

pub struct GitPathDisplay<'a>(&'a [u8]);

impl fmt::Display for GitPathDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", String::from_utf8_lossy(self.0))
    }
}

#[cfg(unix)]
fn git_bytes_from_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn git_bytes_from_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn pathbuf_from_git_bytes(path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt as _;

    PathBuf::from(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
fn pathbuf_from_git_bytes(path: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(path).into_owned())
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        out.push(BASE64_ALPHABET[(first >> 2) as usize] as char);
        out.push(BASE64_ALPHABET[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(
                BASE64_ALPHABET[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char,
            );
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[(third & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(encoded: &str) -> Result<Vec<u8>, String> {
    let bytes = encoded.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("base64 length must be a multiple of 4".to_owned());
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk[0] == b'=' || chunk[1] == b'=' {
            return Err("base64 padding cannot appear in the first two positions".to_owned());
        }
        let padding = match (chunk[2] == b'=', chunk[3] == b'=') {
            (false, false) => 0,
            (false, true) => 1,
            (true, true) => 2,
            (true, false) => return Err("base64 padding must be contiguous".to_owned()),
        };

        let v0 = base64_value(chunk[0])?;
        let v1 = base64_value(chunk[1])?;
        let v2 = if padding == 2 {
            0
        } else {
            base64_value(chunk[2])?
        };
        let v3 = if padding >= 1 {
            0
        } else {
            base64_value(chunk[3])?
        };

        out.push((v0 << 2) | (v1 >> 4));
        if padding < 2 {
            out.push((v1 << 4) | (v2 >> 2));
        }
        if padding == 0 {
            out.push((v2 << 6) | v3);
        }
    }

    Ok(out)
}

fn base64_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        other => Err(format!("invalid base64 byte 0x{other:02x}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    RenamedFrom(RenamePrev),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenamePrev {
    File(GitPath),
    Symbol(SnapshotKey),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotKey {
    pub stable_symbol_id: StableSymbolId,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolSnapshotArtifact {
    pub key: SnapshotKey,
    pub file_path: GitPath,
    pub entity_name: String,
    pub symbol_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosing_scope: Option<String>,
    pub byte_range: SourceRange,
    pub line_range: SourceRange,
    pub anchor_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitArtifact {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_time: i64,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub author_email: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WalkStrategy {
    #[default]
    Reachable,
    FirstParent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitIndexArtifact {
    pub schema_version: u32,
    pub commits: Vec<CommitArtifact>,
    pub refs: BTreeMap<String, String>,
    pub indexed_at: String,
    #[serde(default)]
    pub walk_strategy: WalkStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "endpoint", rename_all = "snake_case")]
pub enum EdgeEndpoint {
    File { path: GitPath },
    Symbol { stable_symbol_id: StableSymbolId },
    Snapshot { key: SnapshotKey },
    Commit { sha: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalEdgeArtifact {
    pub source: EdgeEndpoint,
    pub target: EdgeEndpoint,
    pub relation: RelationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<ChangeKind>,
}

pub fn load_artifact(path: &Path) -> anyhow::Result<GraphIndexArtifact> {
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "failed to inspect graph index artifact `{}`",
            path.display()
        )
    })?;

    if metadata.is_dir() {
        return crate::store::parquet::read_artifact_parquet(path);
    }

    Err(anyhow!(
        "expected a Parquet artifact directory at `{}`",
        path.display()
    ))
}

pub fn load_artifact_slim(path: &Path) -> anyhow::Result<GraphIndexArtifact> {
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "failed to inspect graph index artifact `{}`",
            path.display()
        )
    })?;

    if metadata.is_dir() {
        return crate::store::parquet::read_artifact_parquet_slim(path);
    }

    Err(anyhow!(
        "expected a Parquet artifact directory at `{}`",
        path.display()
    ))
}

pub(crate) fn rehash_legacy_snapshot_ids(artifact: &mut GraphIndexArtifact) {
    let mut rewritten_keys = HashMap::new();
    let legacy_fqns = legacy_snapshot_fqns(&artifact.symbol_snapshots);
    for (snapshot, fqn) in artifact.symbol_snapshots.iter_mut().zip(legacy_fqns) {
        let old_key = snapshot.key.clone();
        snapshot.key.stable_symbol_id = canonical_snapshot_stable_symbol_id(snapshot, &fqn);
        rewritten_keys.insert(old_key, snapshot.key.clone());
    }

    for edge in &mut artifact.temporal_edges {
        rewrite_snapshot_endpoint(&mut edge.source, &rewritten_keys);
        rewrite_snapshot_endpoint(&mut edge.target, &rewritten_keys);
        if let Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(previous))) = &mut edge.change_kind {
            if let Some(rewritten) = rewritten_keys.get(previous) {
                *previous = rewritten.clone();
            }
        }
    }
}

fn canonical_snapshot_stable_symbol_id(snapshot: &SymbolSnapshotArtifact, fqn: &str) -> String {
    crate::identity::stable_symbol_id_for_discriminator(
        snapshot.file_path.to_string_lossy().as_ref(),
        fqn,
        &snapshot.symbol_kind,
        snapshot.byte_range[0] as u64,
    )
}

fn rewrite_snapshot_endpoint(
    endpoint: &mut EdgeEndpoint,
    rewritten_keys: &HashMap<SnapshotKey, SnapshotKey>,
) {
    if let EdgeEndpoint::Snapshot { key } = endpoint {
        if let Some(rewritten) = rewritten_keys.get(key) {
            *key = rewritten.clone();
        }
    }
}

fn legacy_snapshot_fqns(snapshots: &[SymbolSnapshotArtifact]) -> Vec<String> {
    let mut memo = vec![None; snapshots.len()];
    (0..snapshots.len())
        .map(|index| legacy_snapshot_fqn(index, snapshots, &mut memo))
        .collect()
}

fn legacy_snapshot_fqn(
    index: usize,
    snapshots: &[SymbolSnapshotArtifact],
    memo: &mut [Option<String>],
) -> String {
    if let Some(fqn) = &memo[index] {
        return fqn.clone();
    }

    let segment = legacy_snapshot_identity_segment(&snapshots[index]);
    let fqn = if let Some(parent) = nearest_legacy_snapshot_parent(index, snapshots) {
        format!(
            "{}::{segment}",
            legacy_snapshot_scope_fqn(parent, snapshots, memo)
        )
    } else if let Some(scope) = snapshots[index].enclosing_scope.as_deref() {
        format!("{scope}::{segment}")
    } else {
        segment
    };
    memo[index] = Some(fqn.clone());
    fqn
}

fn nearest_legacy_snapshot_parent(
    index: usize,
    snapshots: &[SymbolSnapshotArtifact],
) -> Option<usize> {
    let child = &snapshots[index];
    snapshots
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != index
                && candidate.key.commit == child.key.commit
                && candidate.file_path == child.file_path
                && candidate.byte_range[0] <= child.byte_range[0]
                && candidate.byte_range[1] >= child.byte_range[1]
                && candidate.byte_range != child.byte_range
        })
        .min_by_key(|(_, candidate)| {
            candidate.byte_range[1].saturating_sub(candidate.byte_range[0])
        })
        .map(|(candidate_index, _)| candidate_index)
}

fn legacy_snapshot_scope_fqn(
    index: usize,
    snapshots: &[SymbolSnapshotArtifact],
    memo: &mut [Option<String>],
) -> String {
    let scope_segment = legacy_snapshot_scope_segment(&snapshots[index]);
    if let Some(parent) = nearest_legacy_snapshot_parent(index, snapshots) {
        format!(
            "{}::{scope_segment}",
            legacy_snapshot_scope_fqn(parent, snapshots, memo)
        )
    } else if snapshots[index].symbol_kind == "impl" {
        scope_segment
    } else {
        legacy_snapshot_fqn(index, snapshots, memo)
    }
}

fn legacy_snapshot_identity_segment(snapshot: &SymbolSnapshotArtifact) -> String {
    snapshot
        .entity_name
        .strip_prefix("impl ")
        .unwrap_or(&snapshot.entity_name)
        .to_owned()
}

fn legacy_snapshot_scope_segment(snapshot: &SymbolSnapshotArtifact) -> String {
    match snapshot.symbol_kind.as_str() {
        "impl" => format!(
            "impl {}",
            snapshot
                .entity_name
                .strip_prefix("impl ")
                .unwrap_or(&snapshot.entity_name)
        ),
        _ => snapshot
            .entity_name
            .strip_prefix("impl ")
            .unwrap_or(&snapshot.entity_name)
            .to_owned(),
    }
}

pub fn file_id_from_uri(uri: &str) -> String {
    uri.strip_prefix(CODE_FILE_URI_PREFIX)
        .unwrap_or(uri)
        .to_owned()
}

pub fn symbol_id_from_uri(uri: &str) -> String {
    uri.strip_prefix(CODE_SYMBOL_URI_PREFIX)
        .unwrap_or(uri)
        .to_owned()
}

#[cfg(test)]
mod change_kind_tests {
    use super::*;

    #[test]
    fn change_kind_round_trips_json() {
        let added = ChangeKind::Added;
        let s = serde_json::to_string(&added).unwrap();
        assert_eq!(s, "\"added\"");
        let back: ChangeKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ChangeKind::Added);

        let renamed = ChangeKind::RenamedFrom(RenamePrev::File("src/old.rs".into()));
        let s = serde_json::to_string(&renamed).unwrap();
        let back: ChangeKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, renamed);
    }

    #[test]
    fn node_kind_has_commit_variant() {
        let k = NodeKind::Commit;
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"commit\"");
    }

    #[test]
    fn relation_kind_has_touches_variant() {
        let r = RelationKind::Touches;
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, "\"touches\"");
    }

    #[test]
    fn node_kind_resource_round_trips() {
        let kind: NodeKind = serde_json::from_str("\"resource\"").expect("resource node kind");
        assert_eq!(kind, NodeKind::Resource);
        assert_eq!(serde_json::to_string(&kind).unwrap(), "\"resource\"");
        assert_eq!(kind.discriminator(), "resource");
    }

    #[test]
    fn node_kind_from_discriminator_agrees_with_discriminators() {
        for kind in [
            NodeKind::Module,
            NodeKind::ExternBlock,
            NodeKind::Function,
            NodeKind::Class,
            NodeKind::Interface,
            NodeKind::Struct,
            NodeKind::Impl,
            NodeKind::Trait,
            NodeKind::Enum,
            NodeKind::EnumVariant,
            NodeKind::File,
            NodeKind::External,
            NodeKind::Method,
            NodeKind::Field,
            NodeKind::Constant,
            NodeKind::TypeAlias,
            NodeKind::Macro,
            NodeKind::Section,
            NodeKind::Commit,
            NodeKind::McpTool,
            NodeKind::Cell,
            NodeKind::Port,
            NodeKind::Resource,
        ] {
            assert_eq!(
                NodeKind::from_discriminator(kind.discriminator()),
                Some(kind)
            );
        }

        assert_eq!(NodeKind::from_discriminator("future_kind"), None);
    }

    #[test]
    fn graph_edge_kind_references_address_round_trips() {
        let kind: GraphEdgeKind =
            serde_json::from_str("\"references_address\"").expect("references_address edge kind");
        assert_eq!(kind, GraphEdgeKind::ReferencesAddress);
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            "\"references_address\""
        );
    }

    #[test]
    fn notebook_node_and_relation_kinds_roundtrip() {
        use serde_json::json;

        for (kind, disc) in [(NodeKind::Cell, "cell"), (NodeKind::Port, "port")] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(disc));
            assert_eq!(kind.discriminator(), disc);
            let back: NodeKind = serde_json::from_value(json!(disc)).unwrap();
            assert_eq!(back, kind);
        }

        for (rel, disc, inverse) in [
            (RelationKind::Produces, "produces", "produced_by"),
            (RelationKind::Consumes, "consumes", "consumed_by"),
            (RelationKind::Binds, "binds", "bound_by"),
            (RelationKind::Emits, "emits", "emitted_by"),
        ] {
            assert_eq!(serde_json::to_value(rel).unwrap(), json!(disc));
            let back: RelationKind = serde_json::from_value(json!(disc)).unwrap();
            assert_eq!(back, rel);
            let metadata = rel.metadata();
            assert_eq!(metadata.inverse_label, Some(inverse));
            assert_eq!(metadata.cardinality, RelationCardinality::ManyToMany);
            assert!(!metadata.transitive);
        }
    }

    #[test]
    fn relation_metadata_declares_algebra() {
        use RelationKind::*;

        assert_eq!(Calls.metadata().inverse_label, Some("called_by"));
        assert!(Contains.metadata().transitive);
        assert_eq!(Contains.metadata().inverse_label, Some("contained_by"));
        assert_eq!(
            Implements.metadata().cardinality,
            RelationCardinality::ManyToMany
        );
        assert_eq!(Imports.metadata().inverse_label, Some("imported_by"));
        assert!(Extends.metadata().transitive);
        assert_eq!(Produces.metadata().inverse_label, Some("produced_by"));
        assert_eq!(Consumes.metadata().inverse_label, Some("consumed_by"));
        assert_eq!(Binds.metadata().inverse_label, Some("bound_by"));
        assert_eq!(Emits.metadata().inverse_label, Some("emitted_by"));

        for relation in [
            Imports, Calls, Constructs, Contains, Implements, Defines, References, Extends, Links,
            Touches, Produces, Consumes, Binds, Emits,
        ] {
            let _ = relation.metadata();
        }
    }

    #[test]
    fn graph_edge_artifact_change_kind_is_optional() {
        let json = r#"{
            "source_stable_symbol_id":"a",
            "target_stable_symbol_id":"b",
            "target_label":null,
            "relation":"calls",
            "confidence":"syntax_exact",
            "confidence_score":1.0
        }"#;
        let e: GraphEdgeArtifact = serde_json::from_str(json).unwrap();
        assert!(e.change_kind.is_none());
    }
}

#[cfg(test)]
mod temporal_artifact_tests {
    use super::*;

    #[test]
    fn symbol_snapshot_round_trips() {
        let s = SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: "graph://symbol/foo".to_owned(),
                commit: "abc123".to_owned(),
            },
            file_path: "src/lib.rs".into(),
            entity_name: "foo".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 42],
            line_range: [1, 5],
            anchor_hash: "deadbeef".to_owned(),
            tokens: vec!["foo".to_owned()],
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: SymbolSnapshotArtifact = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn edge_endpoint_serializes_tagged() {
        let e = EdgeEndpoint::Snapshot {
            key: SnapshotKey {
                stable_symbol_id: "x".into(),
                commit: "y".into(),
            },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"endpoint\":\"snapshot\""));
        let back: EdgeEndpoint = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn graph_index_artifact_temporal_fields_default_empty() {
        let json = r#"{
            "header":{"graph_index_version":"1"},
            "files":[],
            "symbols":[]
        }"#;
        let a: GraphIndexArtifact = serde_json::from_str(json).unwrap();
        assert!(a.commits.is_empty());
        assert!(a.symbol_snapshots.is_empty());
        assert!(a.temporal_edges.is_empty());
    }

    #[test]
    fn temporal_graph_index_version_is_v4() {
        assert_eq!(GRAPH_INDEX_VERSION_TEMPORAL, "4");
    }
}
