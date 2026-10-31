//! Direct macro syntax evidence, independent of runtime call edges and expansion.
//!
//! A globally meaningful identity is `(set.graph_content_hash, row.site_id)`.
//! The site digest binds source bytes and coordinates, not enumeration order or
//! the graph hash; binding the set to a snapshot therefore has no circular hash.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};

use crate::git_blob_oid;

pub const MACRO_INVOCATION_SCHEMA_VERSION: &str = "spur-macro-invocation-v1";
const SITE_ID_VERSION: &[u8] = b"spur-macro-invocation-site-v1";

/// Coverage is limited to physical Rust file owners, excluding notebook cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroEvidenceSourceScope {
    PhysicalRustFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroScopeKind {
    File,
    Module,
    Function,
    Const,
    Static,
    Type,
    Impl,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroScopeRef {
    pub stable_symbol_id: String,
    pub kind: MacroScopeKind,
}

/// A supplied syntactic scope, including synthetic file scopes. No compiler
/// resolution is implied by a stable ID or by validating this finite reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroScopeEvidence {
    pub scope: MacroScopeRef,
    pub file_path: String,
    pub source_oid: String,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroAttributeEvidence {
    pub raw_text: String,
    pub owner: MacroScopeRef,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroTestContext {
    /// Absence of a test marker does not establish production execution.
    Unspecified,
    TestAttributeCandidate,
    /// Includes cfg/cfg_attr evidence; no build condition has been evaluated.
    Conditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroExecutionPhase {
    Unknown,
    /// Function body syntax alone does not establish runtime execution.
    FunctionBodySyntax,
    ItemSyntax,
    ConstContextSyntax,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroDefinitionLocation {
    pub file_path: String,
    pub source_oid: String,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MacroResolutionEvidence {
    /// Includes aliases whose targets are not established.
    Unresolved,
    /// Even an explicit std/core path is only a candidate identity.
    ExplicitPathCandidate,
    /// Records a lexical candidate, not full Rust macro namespace resolution.
    LocalDefinitionCandidate { definition: MacroDefinitionLocation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroSinkFamily {
    Panic,
    Assertion,
    DebugAssertion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroInvocationEvidence {
    pub site_id: String,
    pub file_path: String,
    /// Git blob OID of the exact bytes, including dirty worktree bytes.
    pub source_oid: String,
    /// Exact enclosing function ID, or None for a non-function scope.
    pub caller_stable_symbol_id: Option<String>,
    pub enclosing: MacroScopeRef,
    pub raw_macro_path: String,
    pub normalized_macro_path: String,
    /// Half-open UTF-8 byte coordinates, excluding any following semicolon.
    pub byte_range: [usize; 2],
    /// One-based inclusive lines of the bytes in byte_range.
    pub line_range: [usize; 2],
    /// Raw attributes on this scope or enclosing scopes, never evaluated cfg.
    pub attributes: Vec<MacroAttributeEvidence>,
    pub test_context: MacroTestContext,
    pub debug_assertions_candidate: bool,
    pub execution_phase: MacroExecutionPhase,
    pub resolution: MacroResolutionEvidence,
    pub sink_family_candidate: Option<MacroSinkFamily>,
}

/// Downstream optional coverage uses None for legacy/unavailable evidence and
/// Some with empty rows for extraction that captured zero direct sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroInvocationEvidenceSet {
    /// Version of this evidence schema, independent of the graph schema.
    pub schema_version: String,
    pub extractor_version: String,
    /// Source graph snapshot hash, supplied independently of this set's hash.
    pub graph_content_hash: String,
    pub source_scope: MacroEvidenceSourceScope,
    pub rows: Vec<MacroInvocationEvidence>,
    pub scopes: Vec<MacroScopeEvidence>,
}

/// Exact, conjunctive filters; no implicit limit or macro-name resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroInvocationFilter {
    pub file_path: Option<String>,
    pub caller_stable_symbol_id: Option<String>,
    /// Optional expected snapshot identity, requiring file_path.
    pub source_oid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacroEvidenceAvailability {
    Complete,
    Partial,
    Unavailable,
    UnsupportedSource,
}

/// A source domain from one snapshot. Overlays retain separate base/delta
/// bindings, removing replaced sources from the base domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroEvidenceSnapshot {
    pub graph_content_hash: String,
    pub schema_version: String,
    pub extractor_version: String,
    pub source_oids: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroInvocationQueryResult {
    pub availability: MacroEvidenceAvailability,
    pub source_scope: Option<MacroEvidenceSourceScope>,
    pub snapshots: Vec<MacroEvidenceSnapshot>,
    pub rows: Vec<MacroInvocationEvidence>,
    pub scopes: Vec<MacroScopeEvidence>,
}

/// Exact bytes supplied by extraction or artifact validation, not HEAD guesses.
#[derive(Debug, Clone, Copy)]
pub struct MacroEvidenceSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}

/// Canonical site identity helper for extraction, artifacts, storage and queries.
/// Pass `git_blob_oid(exact_bytes)`, half-open bytes and inclusive one-based lines.
/// Every field (including fixed-width big-endian coordinates) is length-prefixed
/// with a big-endian u64. The complete BLAKE3 digest is retained without truncation.
pub fn macro_invocation_site_id(
    file_path: &str,
    source_oid: &str,
    byte_range: [usize; 2],
    line_range: [usize; 2],
) -> String {
    let mut hash = blake3::Hasher::new();
    let mut field = |bytes: &[u8]| {
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    };
    for bytes in [SITE_ID_VERSION, file_path.as_bytes(), source_oid.as_bytes()] {
        field(bytes);
    }
    for coordinate in byte_range.into_iter().chain(line_range) {
        field(&(coordinate as u64).to_be_bytes());
    }
    hash.finalize().to_hex().to_string()
}

impl MacroInvocationEvidenceSet {
    /// Validate persisted provenance without reading a later working tree.
    /// Exact UTF-8 boundaries, attribute text and byte/line agreement require
    /// `validate` with the original source bytes; this checks stored references.
    pub(crate) fn validate_stored(
        &self,
        artifact: &crate::GraphIndexArtifact,
    ) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == MACRO_INVOCATION_SCHEMA_VERSION,
            "unsupported macro evidence schema"
        );
        ensure!(
            self.extractor_version == crate::store::EXTRACTOR_VERSION,
            "incompatible macro extractor version"
        );
        ensure!(
            !self.graph_content_hash.is_empty()
                && self.graph_content_hash == artifact.graph_content_hash,
            "macro evidence snapshot mismatch"
        );
        let mut sources = BTreeMap::new();
        for file in &artifact.file_manifests {
            ensure!(
                sources
                    .insert(file.path.as_str(), file.content_oid.as_str())
                    .is_none(),
                "duplicate artifact source path"
            );
        }
        let location =
            |path: &str, oid: &str, bytes: [usize; 2], lines: [usize; 2]| -> anyhow::Result<()> {
                ensure!(
                    std::path::Path::new(path)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs")),
                    "macro evidence requires physical Rust files"
                );
                ensure!(
                    oid.len() == 40
                        && oid.bytes().all(|b| b.is_ascii_hexdigit())
                        && sources.get(path).copied() == Some(oid),
                    "macro source does not match artifact source manifest"
                );
                ensure!(
                    bytes[0] < bytes[1] && lines[0] > 0 && lines[0] <= lines[1],
                    "invalid persisted macro range"
                );
                Ok(())
            };
        let mut scopes = BTreeMap::new();
        for scope in &self.scopes {
            ensure!(
                !scope.scope.stable_symbol_id.trim().is_empty()
                    && scopes
                        .insert(scope.scope.stable_symbol_id.as_str(), scope)
                        .is_none(),
                "empty or duplicate persisted scope ID"
            );
            location(
                &scope.file_path,
                &scope.source_oid,
                scope.byte_range,
                scope.line_range,
            )?;
        }
        let mut sites = BTreeSet::new();
        for row in &self.rows {
            location(
                &row.file_path,
                &row.source_oid,
                row.byte_range,
                row.line_range,
            )?;
            ensure!(
                row.site_id
                    == macro_invocation_site_id(
                        &row.file_path,
                        &row.source_oid,
                        row.byte_range,
                        row.line_range
                    )
                    && sites.insert(&row.site_id),
                "invalid or duplicate macro site identity"
            );
            ensure!(
                !row.raw_macro_path.trim().is_empty()
                    && !row.normalized_macro_path.trim().is_empty(),
                "empty macro path"
            );
            let scope_for = |reference: &MacroScopeRef| -> anyhow::Result<&MacroScopeEvidence> {
                let scope = scopes
                    .get(reference.stable_symbol_id.as_str())
                    .copied()
                    .context("unknown macro scope reference")?;
                ensure!(
                    scope.scope == *reference
                        && scope.file_path == row.file_path
                        && scope.source_oid == row.source_oid,
                    "macro scope provenance mismatch"
                );
                ensure!(
                    contains(scope.byte_range, row.byte_range)
                        && contains(scope.line_range, row.line_range),
                    "macro site outside enclosing scope"
                );
                Ok(scope)
            };
            let enclosing = scope_for(&row.enclosing)?;
            ensure!(
                row.execution_phase != MacroExecutionPhase::FunctionBodySyntax
                    || row.enclosing.kind == MacroScopeKind::Function,
                "function body syntax requires a function enclosing scope"
            );
            match (row.enclosing.kind, row.caller_stable_symbol_id.as_deref()) {
                (MacroScopeKind::Function, Some(caller)) => {
                    ensure!(
                        caller == row.enclosing.stable_symbol_id,
                        "macro caller differs from enclosing function"
                    );
                    ensure!(
                        artifact.symbols.iter().any(|s| s.stable_symbol_id == caller
                            && s.file_path == row.file_path
                            && matches!(s.symbol_kind.as_str(), "function" | "method")
                            && s.byte_range == enclosing.byte_range
                            && s.line_range == enclosing.line_range),
                        "macro caller is not an actual graph function symbol"
                    );
                }
                (MacroScopeKind::Function, None) => {
                    anyhow::bail!("enclosing function requires its caller ID")
                }
                (_, Some(_)) => anyhow::bail!("non-function scope cannot invent a caller"),
                (_, None) => {}
            }
            for attribute in &row.attributes {
                scope_for(&attribute.owner)?;
                location(
                    &row.file_path,
                    &row.source_oid,
                    attribute.byte_range,
                    attribute.line_range,
                )?;
                ensure!(!attribute.raw_text.is_empty(), "empty macro attribute");
            }
            if let MacroResolutionEvidence::LocalDefinitionCandidate { definition } =
                &row.resolution
            {
                location(
                    &definition.file_path,
                    &definition.source_oid,
                    definition.byte_range,
                    definition.line_range,
                )?;
            }
        }
        Ok(())
    }

    /// Validate finite provenance, identity, coordinates and lexical references.
    /// The caller supplies source bytes and exact scope locations from the same
    /// snapshot. This does not validate parsing, name resolution, build conditions,
    /// macro expansion or execution. Extractor compatibility belongs to the
    /// enclosing artifact; this layer requires a nonempty extractor version.
    pub fn validate(
        &self,
        expected_graph_content_hash: &str,
        sources: &[MacroEvidenceSource<'_>],
        scopes: &[MacroScopeEvidence],
    ) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == MACRO_INVOCATION_SCHEMA_VERSION,
            "unsupported macro evidence schema"
        );
        ensure!(
            !self.extractor_version.trim().is_empty(),
            "empty macro extractor version"
        );
        ensure!(
            !expected_graph_content_hash.trim().is_empty()
                && self.graph_content_hash == expected_graph_content_hash,
            "macro evidence snapshot mismatch"
        );
        let mut source_map = BTreeMap::new();
        for source in sources {
            ensure!(!source.file_path.trim().is_empty(), "empty source path");
            ensure!(
                source_map.insert(source.file_path, source.bytes).is_none(),
                "duplicate source path"
            );
        }
        let mut authoritative_scopes = BTreeMap::new();
        for scope in scopes {
            ensure!(
                !scope.scope.stable_symbol_id.trim().is_empty(),
                "empty scope ID"
            );
            ensure!(
                authoritative_scopes
                    .insert(scope.scope.stable_symbol_id.as_str(), scope)
                    .is_none(),
                "duplicate scope ID"
            );
        }
        let mut scope_map = BTreeMap::new();
        for scope in &self.scopes {
            ensure!(
                scope_map
                    .insert(scope.scope.stable_symbol_id.as_str(), scope)
                    .is_none(),
                "duplicate persisted scope ID"
            );
            ensure!(
                authoritative_scopes
                    .get(scope.scope.stable_symbol_id.as_str())
                    .copied()
                    == Some(scope),
                "persisted macro scope differs from authoritative scope"
            );
            validate_location(
                &scope.file_path,
                &scope.source_oid,
                scope.byte_range,
                scope.line_range,
                &source_map,
            )?;
        }
        let mut identities = BTreeSet::new();
        for row in &self.rows {
            ensure!(
                !row.raw_macro_path.trim().is_empty()
                    && !row.normalized_macro_path.trim().is_empty(),
                "empty macro path"
            );
            validate_location(
                &row.file_path,
                &row.source_oid,
                row.byte_range,
                row.line_range,
                &source_map,
            )?;
            ensure!(
                row.site_id
                    == macro_invocation_site_id(
                        &row.file_path,
                        &row.source_oid,
                        row.byte_range,
                        row.line_range
                    ),
                "macro site identity mismatch"
            );
            ensure!(
                identities.insert(&row.site_id),
                "duplicate macro site identity"
            );
            let enclosing = validate_scope(&row.enclosing, row, &source_map, &scope_map)?;
            ensure!(
                contains(enclosing.byte_range, row.byte_range),
                "macro site outside enclosing scope"
            );
            ensure!(
                row.execution_phase != MacroExecutionPhase::FunctionBodySyntax
                    || row.enclosing.kind == MacroScopeKind::Function,
                "function body syntax requires a function enclosing scope"
            );
            match (row.enclosing.kind, row.caller_stable_symbol_id.as_deref()) {
                (MacroScopeKind::Function, Some(caller)) => ensure!(
                    caller == row.enclosing.stable_symbol_id,
                    "macro caller differs from enclosing function"
                ),
                (MacroScopeKind::Function, None) => {
                    anyhow::bail!("enclosing function requires its caller ID")
                }
                (_, None) => {}
                (_, Some(_)) => anyhow::bail!("non-function scope cannot invent a caller"),
            }
            for attribute in &row.attributes {
                let owner = validate_scope(&attribute.owner, row, &source_map, &scope_map)?;
                ensure!(
                    contains(owner.byte_range, row.byte_range),
                    "attribute owner does not enclose macro site"
                );
                validate_location(
                    &row.file_path,
                    &row.source_oid,
                    attribute.byte_range,
                    attribute.line_range,
                    &source_map,
                )?;
                let bytes = source_map[row.file_path.as_str()];
                ensure!(
                    attribute.raw_text.as_bytes()
                        == &bytes[attribute.byte_range[0]..attribute.byte_range[1]],
                    "raw macro attribute does not match source"
                );
            }
            if let MacroResolutionEvidence::LocalDefinitionCandidate { definition } =
                &row.resolution
            {
                validate_location(
                    &definition.file_path,
                    &definition.source_oid,
                    definition.byte_range,
                    definition.line_range,
                    &source_map,
                )?;
            }
        }
        Ok(())
    }
}

fn contains(outer: [usize; 2], inner: [usize; 2]) -> bool {
    outer[0] <= inner[0] && inner[1] <= outer[1]
}

fn validate_scope<'a>(
    reference: &MacroScopeRef,
    row: &MacroInvocationEvidence,
    sources: &BTreeMap<&str, &[u8]>,
    scopes: &BTreeMap<&str, &'a MacroScopeEvidence>,
) -> anyhow::Result<&'a MacroScopeEvidence> {
    let scope = scopes
        .get(reference.stable_symbol_id.as_str())
        .context("unknown macro scope reference")?;
    ensure!(
        scope.scope == *reference
            && scope.file_path == row.file_path
            && scope.source_oid == row.source_oid,
        "macro scope provenance mismatch"
    );
    validate_location(
        &scope.file_path,
        &scope.source_oid,
        scope.byte_range,
        scope.line_range,
        sources,
    )?;
    Ok(scope)
}

fn validate_location(
    path: &str,
    oid: &str,
    bytes: [usize; 2],
    lines: [usize; 2],
    sources: &BTreeMap<&str, &[u8]>,
) -> anyhow::Result<()> {
    ensure!(
        !path.trim().is_empty() && !oid.trim().is_empty(),
        "empty macro source provenance"
    );
    let source = sources.get(path).context("missing macro source bytes")?;
    ensure!(oid == git_blob_oid(source), "macro source OID mismatch");
    ensure!(
        bytes[0] < bytes[1] && bytes[1] <= source.len(),
        "invalid macro byte range"
    );
    ensure!(
        lines[0] > 0 && lines[0] <= lines[1],
        "invalid macro line range"
    );
    let text = std::str::from_utf8(source).context("macro source is not UTF-8")?;
    ensure!(
        text.is_char_boundary(bytes[0]) && text.is_char_boundary(bytes[1]),
        "macro range splits UTF-8 character"
    );
    let line_at = |offset| {
        1 + source[..offset]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
    };
    ensure!(
        lines == [line_at(bytes[0]), line_at(bytes[1] - 1)],
        "macro byte and line coordinates disagree"
    );
    Ok(())
}

pub(crate) fn referenced_scope_ids(rows: &[MacroInvocationEvidence]) -> BTreeSet<String> {
    rows.iter()
        .flat_map(|row| {
            std::iter::once(row.enclosing.stable_symbol_id.clone()).chain(
                row.attributes
                    .iter()
                    .map(|a| a.owner.stable_symbol_id.clone()),
            )
        })
        .collect()
}

impl MacroInvocationFilter {
    pub(crate) fn validate_shape(&self) -> anyhow::Result<()> {
        ensure!(
            self.source_oid.is_none() || self.file_path.is_some(),
            "source_oid requires file_path"
        );
        Ok(())
    }

    pub(crate) fn matches(&self, row: &MacroInvocationEvidence) -> bool {
        self.file_path.as_ref().is_none_or(|p| *p == row.file_path)
            && self
                .caller_stable_symbol_id
                .as_ref()
                .is_none_or(|c| row.caller_stable_symbol_id.as_ref() == Some(c))
    }

    pub(crate) fn validate_source(&self, sources: &BTreeMap<String, String>) -> anyhow::Result<()> {
        self.validate_shape()?;
        if let (Some(oid), Some(path)) = (&self.source_oid, &self.file_path) {
            ensure!(
                sources.get(path) == Some(oid),
                "macro query source identity mismatch"
            );
        }
        Ok(())
    }
}

impl MacroInvocationQueryResult {
    pub(crate) fn unavailable() -> Self {
        Self {
            availability: MacroEvidenceAvailability::Unavailable,
            source_scope: None,
            snapshots: Vec::new(),
            rows: Vec::new(),
            scopes: Vec::new(),
        }
    }

    pub(crate) fn sort(&mut self) {
        self.rows.sort_by(|a, b| {
            (&a.file_path, a.byte_range, &a.site_id).cmp(&(&b.file_path, b.byte_range, &b.site_id))
        });
        self.scopes.sort_by(|a, b| {
            (&a.file_path, a.byte_range, &a.scope.stable_symbol_id).cmp(&(
                &b.file_path,
                b.byte_range,
                &b.scope.stable_symbol_id,
            ))
        });
    }

    pub(crate) fn retain_referenced_scopes(&mut self) {
        let ids = referenced_scope_ids(&self.rows);
        self.scopes
            .retain(|s| ids.contains(&s.scope.stable_symbol_id));
    }
}
