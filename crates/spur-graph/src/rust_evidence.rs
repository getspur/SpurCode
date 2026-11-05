//! Canonical per-file physical Rust evidence with original snapshot provenance.
//!
//! Stored validation checks structure against the authoritative graph. Only
//! source validation authenticates syntax and completeness. Neither establishes
//! cfg activation, macro expansion, callees, execution or panic completeness.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};

use crate::panic_evidence::{
    collect_rust_call_sites, snapshot_id, MethodCallSourceScope, PanicEvidenceSource, RustBodyKind,
    RustCallSiteEvidenceSet, RUST_CALL_SITE_EXTRACTOR_VERSION, RUST_CALL_SITE_SCHEMA_VERSION,
};
use crate::{
    collect_rust_declarations, compute_graph_content_hash, git_blob_oid, GraphFileManifestEntry,
    GraphIndexArtifact, GraphSymbolArtifact, RelationKind, RustDeclarationAbiSyntax,
    RustDeclarationAvailability, RustDeclarationBody, RustDeclarationEvidenceSet,
    RustDeclarationKind, RustDeclarationOwner, RustDeclarationScope, RustDeclarationSource,
    RustDeclarationText, RUST_DECLARATION_SCHEMA_VERSION,
};

pub const RUST_EVIDENCE_SCHEMA_VERSION: &str = "spur-rust-evidence-v1";
pub const RUST_EVIDENCE_EXTRACTOR_VERSION: &str = "rust-evidence-wrapper-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceOriginSource {
    pub path: String,
    pub stable_file_id: String,
    pub content_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceOrigin {
    pub graph_content_hash: String,
    pub manifest_version: String,
    pub sources: Vec<RustEvidenceOriginSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RustEvidenceUnknownReason {
    InvalidUtf8,
    ParseRecovery,
    UnevaluatedCfg,
    ParseUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RustCallEvidence {
    Complete { evidence: RustCallSiteEvidenceSet },
    Unknown { reason: RustEvidenceUnknownReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceFile {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub byte_length: usize,
    pub origin_graph_content_hash: String,
    pub declarations: RustDeclarationEvidenceSet,
    pub calls: RustCallEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceSet {
    pub schema_version: String,
    pub extractor_version: String,
    pub graph_content_hash: String,
    pub origins: Vec<RustEvidenceOrigin>,
    pub files: Vec<RustEvidenceFile>,
}

/// Capture exactly the eligible physical Rust manifest domain, including zeros.
pub fn collect_rust_evidence(
    graph: &GraphIndexArtifact,
    sources: &[RustDeclarationSource<'_>],
) -> anyhow::Result<RustEvidenceSet> {
    authenticate_sources(graph, sources)?;
    let mut ordered = sources.to_vec();
    ordered.sort_by_key(|s| s.file_path);
    let mut files = Vec::new();
    for source in ordered {
        let manifest = graph
            .file_manifests
            .iter()
            .find(|m| m.path == source.file_path)
            .context("authenticated source missing")?;
        // Declaration collection independently checks intact owners, including
        // recovered syntax, before any call uncertainty is classified.
        let declarations = collect_rust_declarations(graph, &[source])?;
        let calls = collect_calls(graph, source)?;
        files.push(RustEvidenceFile {
            file_path: source.file_path.into(),
            stable_file_id: manifest.stable_file_id.clone(),
            source_oid: manifest.content_oid.clone(),
            byte_length: source.bytes.len(),
            origin_graph_content_hash: graph.graph_content_hash.clone(),
            declarations,
            calls,
        });
    }
    let origins = if files.is_empty() {
        Vec::new()
    } else {
        let mut sources: Vec<_> = graph
            .file_manifests
            .iter()
            .map(|m| RustEvidenceOriginSource {
                path: m.path.clone(),
                stable_file_id: m.stable_file_id.clone(),
                content_oid: m.content_oid.clone(),
            })
            .collect();
        sources.sort_by(|a, b| a.path.cmp(&b.path));
        vec![RustEvidenceOrigin {
            graph_content_hash: graph.graph_content_hash.clone(),
            manifest_version: graph.manifest_version.clone(),
            sources,
        }]
    };
    let set = RustEvidenceSet {
        schema_version: RUST_EVIDENCE_SCHEMA_VERSION.into(),
        extractor_version: RUST_EVIDENCE_EXTRACTOR_VERSION.into(),
        graph_content_hash: graph.graph_content_hash.clone(),
        origins,
        files,
    };
    set.validate_stored(graph)?;
    Ok(set)
}

impl RustEvidenceSet {
    /// Validate stored shape, coverage, identities, bounds and actual graph owners.
    /// Unseen syntax and omissions with consistently changed counts need bytes.
    pub fn validate_stored(&self, graph: &GraphIndexArtifact) -> anyhow::Result<()> {
        let domain = graph_domain(graph)?;
        ensure!(
            self.schema_version == RUST_EVIDENCE_SCHEMA_VERSION
                && self.extractor_version == RUST_EVIDENCE_EXTRACTOR_VERSION,
            "incompatible Rust evidence versions"
        );
        ensure!(
            self.graph_content_hash == graph.graph_content_hash,
            "Rust evidence aggregate hash mismatch"
        );
        ensure!(
            self.files
                .windows(2)
                .all(|w| w[0].file_path < w[1].file_path),
            "noncanonical Rust evidence file order or duplicate source"
        );
        ensure!(
            self.files
                .iter()
                .map(|f| f.file_path.as_str())
                .collect::<BTreeSet<_>>()
                == domain.keys().copied().collect(),
            "Rust evidence physical source domain mismatch"
        );
        ensure!(
            self.origins
                .windows(2)
                .all(|w| (&w[0].manifest_version, &w[0].graph_content_hash)
                    < (&w[1].manifest_version, &w[1].graph_content_hash)),
            "noncanonical or duplicate Rust origin"
        );
        let mut origins = BTreeMap::new();
        for origin in &self.origins {
            ensure!(
                origin.manifest_version == graph.manifest_version,
                "unsupported Rust origin manifest version"
            );
            ensure!(
                origin.sources.windows(2).all(|w| w[0].path < w[1].path),
                "noncanonical origin source order or duplicate path"
            );
            let mut ids = BTreeSet::new();
            for source in &origin.sources {
                ensure!(
                    !source.path.is_empty()
                        && !source.stable_file_id.is_empty()
                        && !source.content_oid.is_empty()
                        && ids.insert(&source.stable_file_id),
                    "invalid origin source identity"
                );
            }
            ensure!(
                origin.graph_content_hash
                    == compute_graph_content_hash(
                        origin.sources.iter().map(|s| (&s.path, &s.content_oid))
                    ),
                "Rust origin hash does not match original inventory"
            );
            ensure!(
                origins
                    .insert(origin.graph_content_hash.as_str(), origin)
                    .is_none(),
                "ambiguous Rust origin hash"
            );
        }
        let mut used = BTreeSet::new();
        let mut declaration_ids = BTreeSet::new();
        let mut call_ids = BTreeSet::new();
        for file in &self.files {
            let manifest = domain
                .get(file.file_path.as_str())
                .context("Rust source missing")?;
            ensure!(
                file.stable_file_id == manifest.stable_file_id
                    && file.source_oid == manifest.content_oid,
                "Rust source identity differs from authoritative graph"
            );
            validate_graph_symbols(graph, &file.file_path, file.byte_length)?;
            let origin = origins
                .get(file.origin_graph_content_hash.as_str())
                .context("Rust source origin missing")?;
            let original = origin
                .sources
                .iter()
                .find(|s| s.path == file.file_path)
                .context("Rust source missing from original inventory")?;
            ensure!(
                original.stable_file_id == file.stable_file_id
                    && original.content_oid == file.source_oid,
                "Rust source differs from original binding"
            );
            used.insert(file.origin_graph_content_hash.as_str());
            validate_declarations(graph, file, &mut declaration_ids)?;
            if let RustCallEvidence::Complete { evidence } = &file.calls {
                validate_calls(graph, file, evidence, &mut call_ids)?;
            }
        }
        ensure!(used.len() == origins.len(), "unreferenced Rust origin");
        Ok(())
    }

    /// Authenticate exact current bytes and recollect under each original
    /// inventory, keeping current authoritative files/symbols and original IDs.
    pub fn validate_sources(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[RustDeclarationSource<'_>],
    ) -> anyhow::Result<()> {
        authenticate_sources(graph, sources)?;
        self.validate_stored(graph)?;
        for file in &self.files {
            let source = *sources
                .iter()
                .find(|s| s.file_path == file.file_path)
                .context("Rust source bytes missing")?;
            ensure!(
                file.byte_length == source.bytes.len(),
                "Rust source byte length mismatch"
            );
            let origin = self
                .origins
                .iter()
                .find(|o| o.graph_content_hash == file.origin_graph_content_hash)
                .context("Rust source origin missing")?;
            let mut view = graph.clone();
            view.macro_invocations = None;
            view.graph_content_hash = origin.graph_content_hash.clone();
            view.manifest_version = origin.manifest_version.clone();
            view.file_manifests = origin
                .sources
                .iter()
                .map(|s| GraphFileManifestEntry {
                    path: s.path.clone(),
                    stable_file_id: s.stable_file_id.clone(),
                    content_oid: s.content_oid.clone(),
                    node_ids: vec![],
                })
                .collect();
            // Current graph owners remain authoritative; do not rewrite any
            // nested header, ID, path or source binding to the aggregate hash.
            file.declarations.validate(&view, &[source])?;
            let calls = collect_calls(&view, source)?;
            ensure!(
                file.calls == calls,
                "Rust call availability/evidence differs from canonical source collection"
            );
            if let RustCallEvidence::Complete { evidence } = &file.calls {
                evidence.validate(
                    &view,
                    &[PanicEvidenceSource {
                        file_path: source.file_path,
                        bytes: source.bytes,
                    }],
                )?;
            }
        }
        Ok(())
    }
}

/// Domain-separated checksum over the entire ordered envelope, including zeros.
pub fn rust_evidence_checksum_blake3_hex(set: &RustEvidenceSet) -> anyhow::Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"spur-rust-evidence-tables-v1\0");
    hash.update(&serde_json::to_vec(set)?);
    Ok(hash.finalize().to_hex().to_string())
}

fn eligible(path: &str) -> anyhow::Result<bool> {
    if path.starts_with("external://")
        || path.contains("#cell:")
        || Path::new(path).extension().is_none_or(|e| e != "rs")
    {
        return Ok(false);
    }
    ensure!(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "source is not a physical Rust path"
    );
    Ok(true)
}

fn graph_domain(
    graph: &GraphIndexArtifact,
) -> anyhow::Result<BTreeMap<&str, &GraphFileManifestEntry>> {
    ensure!(
        graph.manifest_version == crate::store::current_manifest_version(),
        "incompatible graph extractor"
    );
    ensure!(
        graph.graph_content_hash
            == compute_graph_content_hash(
                graph
                    .file_manifests
                    .iter()
                    .map(|m| (&m.path, &m.content_oid))
            ),
        "stale graph content hash"
    );
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut domain = BTreeMap::new();
    for manifest in &graph.file_manifests {
        ensure!(
            !manifest.path.is_empty()
                && !manifest.stable_file_id.is_empty()
                && !manifest.content_oid.is_empty()
                && paths.insert(&manifest.path)
                && ids.insert(&manifest.stable_file_id),
            "duplicate or invalid graph manifest identity"
        );
        if eligible(&manifest.path)? {
            let files: Vec<_> = graph
                .files
                .iter()
                .filter(|f| f.file_path == manifest.path)
                .collect();
            ensure!(
                files.len() == 1
                    && files[0].stable_file_id == manifest.stable_file_id
                    && graph
                        .files
                        .iter()
                        .filter(|f| f.stable_file_id == manifest.stable_file_id)
                        .count()
                        == 1,
                "source is not an authoritative physical graph file"
            );
            domain.insert(manifest.path.as_str(), manifest);
        }
    }
    for file in &graph.files {
        ensure!(
            !eligible(&file.file_path)? || domain.contains_key(file.file_path.as_str()),
            "physical Rust graph file has no manifest"
        );
    }
    Ok(domain)
}

fn authenticate_sources(
    graph: &GraphIndexArtifact,
    sources: &[RustDeclarationSource<'_>],
) -> anyhow::Result<()> {
    let domain = graph_domain(graph)?;
    let mut paths = BTreeSet::new();
    for source in sources {
        ensure!(
            eligible(source.file_path)? && paths.insert(source.file_path),
            "duplicate or unsupported Rust source"
        );
        let manifest = domain
            .get(source.file_path)
            .context("source outside physical Rust graph domain")?;
        ensure!(
            git_blob_oid(source.bytes) == manifest.content_oid,
            "source bytes do not match graph manifest blob OID"
        );
        validate_graph_symbols(graph, source.file_path, source.bytes.len())?;
    }
    ensure!(
        paths == domain.keys().copied().collect(),
        "supplied Rust sources must equal complete physical graph domain"
    );
    Ok(())
}

fn collect_calls(
    graph: &GraphIndexArtifact,
    source: RustDeclarationSource<'_>,
) -> anyhow::Result<RustCallEvidence> {
    if let Some(reason) = call_uncertainty(source.bytes)? {
        return Ok(RustCallEvidence::Unknown { reason });
    }
    Ok(RustCallEvidence::Complete {
        evidence: collect_rust_call_sites(
            graph,
            &[PanicEvidenceSource {
                file_path: source.file_path,
                bytes: source.bytes,
            }],
        )?,
    })
}

fn call_uncertainty(bytes: &[u8]) -> anyhow::Result<Option<RustEvidenceUnknownReason>> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(Some(RustEvidenceUnknownReason::InvalidUtf8));
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let Some(tree) = parser.parse(bytes, None) else {
        return Ok(Some(RustEvidenceUnknownReason::ParseUnavailable));
    };
    if tree.root_node().has_error() {
        return Ok(Some(RustEvidenceUnknownReason::ParseRecovery));
    }
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if matches!(
            node.kind(),
            "macro_definition" | "macro_invocation" | "token_tree"
        ) {
            continue;
        }
        if node.kind() == "attribute" {
            let name = node.named_child(0).map(|n| {
                text[n.byte_range()]
                    .strip_prefix("r#")
                    .unwrap_or(&text[n.byte_range()])
            });
            if matches!(name, Some("cfg" | "cfg_attr")) {
                return Ok(Some(RustEvidenceUnknownReason::UnevaluatedCfg));
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(None)
}

fn range(range: [usize; 2], limit: usize) -> anyhow::Result<()> {
    ensure!(
        range[0] < range[1] && range[1] <= limit,
        "Rust evidence range out of source bounds"
    );
    Ok(())
}
fn contains(outer: [usize; 2], inner: [usize; 2]) -> bool {
    outer[0] <= inner[0] && inner[1] <= outer[1]
}
fn lines(value: [usize; 2], limit: usize) -> anyhow::Result<()> {
    ensure!(
        value[0] > 0 && value[0] <= value[1] && value[1].saturating_sub(1) <= limit,
        "Rust evidence line bounds invalid"
    );
    Ok(())
}
fn validate_graph_symbols(
    graph: &GraphIndexArtifact,
    file_path: &str,
    byte_length: usize,
) -> anyhow::Result<()> {
    for s in graph.symbols.iter().filter(|s| s.file_path == file_path) {
        ensure!(
            !s.stable_symbol_id.is_empty()
                && graph
                    .symbols
                    .iter()
                    .filter(|other| other.stable_symbol_id == s.stable_symbol_id)
                    .count()
                    == 1,
            "ambiguous graph symbol identity"
        );
        range(s.byte_range, byte_length)?;
        lines(s.line_range, byte_length)?;
    }
    Ok(())
}
fn symbol<'a>(
    graph: &'a GraphIndexArtifact,
    file: &RustEvidenceFile,
    id: &str,
) -> anyhow::Result<&'a GraphSymbolArtifact> {
    let found: Vec<_> = graph
        .symbols
        .iter()
        .filter(|s| s.stable_symbol_id == id)
        .collect();
    ensure!(
        found.len() == 1 && found[0].file_path == file.file_path,
        "Rust evidence graph symbol absent, ambiguous or cross-file"
    );
    Ok(found[0])
}
fn owner_id(owner: &RustDeclarationOwner) -> &str {
    match owner {
        RustDeclarationOwner::File { stable_file_id } => stable_file_id,
        RustDeclarationOwner::Symbol { stable_symbol_id } => stable_symbol_id,
    }
}
fn owner(
    graph: &GraphIndexArtifact,
    file: &RustEvidenceFile,
    owner: &RustDeclarationOwner,
    child: [usize; 2],
) -> anyhow::Result<()> {
    match owner {
        RustDeclarationOwner::File { stable_file_id } => ensure!(
            stable_file_id == &file.stable_file_id,
            "Rust declaration file owner mismatch"
        ),
        RustDeclarationOwner::Symbol { stable_symbol_id } => ensure!(
            contains(symbol(graph, file, stable_symbol_id)?.byte_range, child),
            "Rust declaration symbol owner does not contain child"
        ),
    }
    Ok(())
}
fn parent<'a>(graph: &'a GraphIndexArtifact, id: &str) -> anyhow::Result<&'a str> {
    let parents: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Contains && e.target_stable_symbol_id.as_deref() == Some(id)
        })
        .collect();
    ensure!(
        parents.len() == 1,
        "Rust declaration graph parent absent or ambiguous"
    );
    Ok(&parents[0].source_stable_symbol_id)
}
fn binding(file: &RustEvidenceFile, path: &str, id: &str, oid: &str) -> anyhow::Result<()> {
    ensure!(
        path == file.file_path && id == file.stable_file_id && oid == file.source_oid,
        "Rust evidence row source mismatch"
    );
    Ok(())
}
fn metadata(
    file: &RustEvidenceFile,
    full: [usize; 2],
    line: [usize; 2],
    text: &RustDeclarationText,
) -> anyhow::Result<()> {
    range(text.byte_range, file.byte_length)?;
    lines(text.line_range, file.byte_length)?;
    ensure!(
        contains(full, text.byte_range)
            && contains(line, text.line_range)
            && text.text.len() == text.byte_range[1] - text.byte_range[0],
        "Rust declaration metadata bounds/length mismatch"
    );
    Ok(())
}
fn texts(
    file: &RustEvidenceFile,
    full: [usize; 2],
    line: [usize; 2],
    values: &[RustDeclarationText],
) -> anyhow::Result<()> {
    ensure!(
        values
            .windows(2)
            .all(|w| w[0].byte_range[1] <= w[1].byte_range[0]),
        "noncanonical Rust metadata order"
    );
    for value in values {
        metadata(file, full, line, value)?;
    }
    Ok(())
}

fn validate_declarations(
    graph: &GraphIndexArtifact,
    file: &RustEvidenceFile,
    ids: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let set = &file.declarations;
    ensure!(
        set.schema_version == RUST_DECLARATION_SCHEMA_VERSION
            && set.extractor_version == crate::store::EXTRACTOR_VERSION
            && set.graph_content_hash == file.origin_graph_content_hash
            && set.scope == RustDeclarationScope::SuppliedPhysicalRustFiles
            && set.covered_kinds
                == vec![
                    RustDeclarationKind::TupleField,
                    RustDeclarationKind::ForeignFunction,
                    RustDeclarationKind::ExternBlock
                ],
        "Rust declaration header mismatch"
    );
    let [source] = set.sources.as_slice() else {
        anyhow::bail!("Rust declaration requires exactly one source");
    };
    binding(
        file,
        &source.file_path,
        &source.stable_file_id,
        &source.source_oid,
    )?;
    ensure!(
        source.byte_length == file.byte_length,
        "Rust declaration source length mismatch"
    );
    match &source.availability {
        RustDeclarationAvailability::Complete { row_count } => ensure!(
            *row_count == set.rows.len() + set.foreign_functions.len() + set.extern_blocks.len(),
            "Rust declaration row count mismatch"
        ),
        RustDeclarationAvailability::Unknown { reason } => ensure!(
            !reason.is_empty(),
            "Rust declaration unknown reason missing"
        ),
    }
    ensure!(
        set.rows
            .windows(2)
            .all(|w| w[0].full_range < w[1].full_range)
            && set
                .foreign_functions
                .windows(2)
                .all(|w| w[0].full_range < w[1].full_range)
            && set
                .extern_blocks
                .windows(2)
                .all(|w| w[0].full_range < w[1].full_range),
        "noncanonical Rust declaration order"
    );
    let mut ordinals: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &set.rows {
        binding(file, &row.file_path, &row.stable_file_id, &row.source_oid)?;
        let found = declaration_symbol(
            graph,
            file,
            &row.stable_symbol_id,
            row.full_range,
            row.line_range,
            "field",
            ids,
        )?;
        range(row.type_range, file.byte_length)?;
        ensure!(
            contains(row.full_range, row.type_range),
            "tuple type outside declaration"
        );
        owner(graph, file, &row.structural_owner, row.full_range)?;
        ensure!(
            parent(graph, &row.stable_symbol_id)? == owner_id(&row.structural_owner)
                && row.lexical_owner == row.structural_owner,
            "tuple actual graph owner mismatch"
        );
        let owner_symbol = symbol(graph, file, owner_id(&row.structural_owner))?;
        ensure!(
            matches!(owner_symbol.symbol_kind.as_str(), "struct" | "enum_variant"),
            "invalid tuple owner kind"
        );
        let next = ordinals.entry(owner_id(&row.structural_owner)).or_default();
        ensure!(
            row.ordinal == *next && found.entity_name == row.ordinal.to_string(),
            "tuple physical ordinal mismatch"
        );
        *next += 1;
        if let Some(v) = &row.visibility {
            metadata(file, row.full_range, row.line_range, v)?;
        }
        texts(file, row.full_range, row.line_range, &row.attributes)?;
    }
    for row in &set.extern_blocks {
        binding(file, &row.file_path, &row.stable_file_id, &row.source_oid)?;
        let found = declaration_symbol(
            graph,
            file,
            &row.stable_symbol_id,
            row.full_range,
            row.line_range,
            "extern_block",
            ids,
        )?;
        ensure!(
            found.entity_name == row.entity_name && found.qualified_name == row.qualified_name,
            "extern block graph name mismatch"
        );
        owner(graph, file, &row.structural_owner, row.full_range)?;
        ensure!(
            parent(graph, &row.stable_symbol_id)? == owner_id(&row.structural_owner)
                && row.lexical_owner == row.structural_owner,
            "extern block actual graph owner mismatch"
        );
        ensure!(
            row.body == RustDeclarationBody::NonExecutableContainer,
            "extern block must be nonexecutable"
        );
        foreign_metadata(
            file,
            row.full_range,
            row.line_range,
            row.core_range,
            row.core_line_range,
            &row.qualifiers,
            row.visibility.as_ref(),
            &row.attributes,
        )?;
        match &row.abi {
            RustDeclarationAbiSyntax::Omitted => {}
            RustDeclarationAbiSyntax::Explicit { literal } => {
                metadata(file, row.core_range, row.core_line_range, literal)?
            }
            RustDeclarationAbiSyntax::Inherited { .. } => {
                anyhow::bail!("extern block cannot inherit ABI")
            }
        }
    }
    for row in &set.foreign_functions {
        binding(file, &row.file_path, &row.stable_file_id, &row.source_oid)?;
        let found = declaration_symbol(
            graph,
            file,
            &row.stable_symbol_id,
            row.full_range,
            row.line_range,
            "function",
            ids,
        )?;
        ensure!(
            found.entity_name == row.entity_name && found.qualified_name == row.qualified_name,
            "foreign function graph name mismatch"
        );
        let block = set
            .extern_blocks
            .iter()
            .find(|b| b.stable_symbol_id == owner_id(&row.structural_owner))
            .context("foreign function missing extern block")?;
        owner(graph, file, &row.structural_owner, row.full_range)?;
        ensure!(
            parent(graph, &row.stable_symbol_id)? == block.stable_symbol_id
                && row.lexical_owner == block.lexical_owner,
            "foreign function actual graph owner mismatch"
        );
        owner(graph, file, &row.lexical_owner, row.full_range)?;
        ensure!(
            row.body == RustDeclarationBody::AbsentForeignDeclaration,
            "foreign function must have absent body"
        );
        ensure!(
            row.abi
                == RustDeclarationAbiSyntax::Inherited {
                    extern_block_id: block.stable_symbol_id.clone()
                },
            "foreign function ABI inheritance mismatch"
        );
        foreign_metadata(
            file,
            row.full_range,
            row.line_range,
            row.core_range,
            row.core_line_range,
            &row.qualifiers,
            row.visibility.as_ref(),
            &row.attributes,
        )?;
    }
    Ok(())
}

fn declaration_symbol<'a>(
    graph: &'a GraphIndexArtifact,
    file: &RustEvidenceFile,
    id: &str,
    full: [usize; 2],
    line: [usize; 2],
    kind: &str,
    ids: &mut BTreeSet<String>,
) -> anyhow::Result<&'a GraphSymbolArtifact> {
    ensure!(ids.insert(id.into()), "duplicate Rust declaration ID");
    range(full, file.byte_length)?;
    lines(line, file.byte_length)?;
    let found = symbol(graph, file, id)?;
    ensure!(
        found.symbol_kind == kind && found.byte_range == full && found.line_range == line,
        "Rust declaration disagrees with graph symbol kind/range"
    );
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
fn foreign_metadata(
    file: &RustEvidenceFile,
    full: [usize; 2],
    line: [usize; 2],
    core: [usize; 2],
    core_line: [usize; 2],
    qualifiers: &[RustDeclarationText],
    visibility: Option<&RustDeclarationText>,
    attributes: &[RustDeclarationText],
) -> anyhow::Result<()> {
    range(core, file.byte_length)?;
    lines(core_line, file.byte_length)?;
    ensure!(
        contains(full, core) && contains(line, core_line),
        "foreign core outside full declaration"
    );
    texts(file, core, core_line, qualifiers)?;
    texts(file, full, line, attributes)?;
    if let Some(v) = visibility {
        metadata(file, core, core_line, v)?;
    }
    Ok(())
}

fn validate_calls(
    graph: &GraphIndexArtifact,
    file: &RustEvidenceFile,
    set: &RustCallSiteEvidenceSet,
    ids: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    ensure!(
        set.schema_version == RUST_CALL_SITE_SCHEMA_VERSION
            && set.extractor_version == RUST_CALL_SITE_EXTRACTOR_VERSION
            && set.graph_content_hash == file.origin_graph_content_hash
            && set.source_scope == MethodCallSourceScope::SuppliedPhysicalRustFiles,
        "Rust call header mismatch"
    );
    let [source] = set.sources.as_slice() else {
        anyhow::bail!("Rust calls require exactly one source");
    };
    binding(
        file,
        &source.file_path,
        &source.stable_file_id,
        &source.source_oid,
    )?;
    ensure!(
        source.byte_length == file.byte_length
            && source.body_count == set.bodies.len()
            && source.site_count == set.sites.len(),
        "Rust call source length/count mismatch"
    );
    ensure!(
        set.bodies
            .windows(2)
            .all(|w| (w[0].byte_range, &w[0].body_id) < (w[1].byte_range, &w[1].body_id))
            && set
                .sites
                .windows(2)
                .all(|w| w[0].byte_range < w[1].byte_range),
        "noncanonical Rust body/site order"
    );
    for body in &set.bodies {
        ensure!(
            body.file_path == file.file_path && body.source_oid == file.source_oid,
            "Rust body source mismatch"
        );
        range(body.byte_range, file.byte_length)?;
        ensure!(
            ids.insert(body.body_id.clone())
                && body.body_id
                    == snapshot_id(
                        "body",
                        &file.file_path,
                        &file.source_oid,
                        body.byte_range,
                        &format!("{:?}", body.kind)
                    ),
            "duplicate or invalid Rust body ID"
        );
        if let Some(id) = &body.definition_stable_symbol_id {
            let found = symbol(graph, file, id)?;
            ensure!(
                matches!(
                    body.kind,
                    RustBodyKind::Function
                        | RustBodyKind::AsyncFunction
                        | RustBodyKind::NestedFunction
                        | RustBodyKind::NestedAsyncFunction
                ) && matches!(found.symbol_kind.as_str(), "function" | "method")
                    && contains(found.byte_range, body.byte_range)
                    && found.byte_range[0] < body.byte_range[0]
                    && found.byte_range[1] == body.byte_range[1]
                    && !file
                        .declarations
                        .foreign_functions
                        .iter()
                        .any(|f| f.stable_symbol_id == *id),
                "Rust body definition is not a local executable function"
            );
        }
        // Foreign signatures can contain independent constant-expression
        // contexts. Preserve those R3 bodies; the foreign function itself
        // cannot be a definition-bound executable body.
    }
    for site in &set.sites {
        ensure!(
            site.file_path == file.file_path && site.source_oid == file.source_oid,
            "Rust site source mismatch"
        );
        range(site.byte_range, file.byte_length)?;
        ensure!(
            ids.insert(site.site_id.clone())
                && site.site_id
                    == snapshot_id(
                        "call",
                        &file.file_path,
                        &file.source_oid,
                        site.byte_range,
                        ""
                    )
                && site.call_text.len() == site.byte_range[1] - site.byte_range[0],
            "duplicate or invalid Rust call ID/text length"
        );
        let body = set
            .bodies
            .iter()
            .find(|b| b.body_id == site.body_id)
            .context("Rust call body missing")?;
        ensure!(
            contains(body.byte_range, site.byte_range),
            "Rust call outside body range"
        );
    }
    Ok(())
}
