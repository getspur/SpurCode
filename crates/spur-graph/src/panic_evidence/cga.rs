//! Pinned CGA wire format, checked independently from the graph v11 schema.
//! Canonical names are opaque compiler instance strings. Associations are
//! spatial observations, never receiver typing, dispatch proofs or safety.
use super::{
    build_identity::normal_relative_path, sha256_bytes, CaptureBuildIdentity, CaptureInputBytes,
    CaptureInputKind, MethodCallLexicalPhase, MethodCallSiteEvidence, MethodCallSiteEvidenceSet,
    PanicEvidenceSource,
};
use crate::GraphIndexArtifact;
use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tree_sitter::Parser;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaFilePosition {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaSpan {
    pub start: CgaFilePosition,
    pub end: CgaFilePosition,
}

impl CgaSpan {
    /// Pinned compiler convention: one-based line, one-based Unicode scalar
    /// column, exclusive end. Upstream uses RangeInclusive only as a serde
    /// container for rustc span_to_location_info endpoints (col + 1, span.hi).
    /// Verified by the frozen real-export UTF-8 witness; P3 must also revalidate
    /// its held-out exports. LF-only physical UTF-8 is the
    /// supported bounded domain; rustc BOM/CRLF normalization is not guessed.
    pub fn to_byte_range(&self, bytes: &[u8]) -> anyhow::Result<[usize; 2]> {
        let text = std::str::from_utf8(bytes).context("compiler source is not UTF-8")?;
        ensure!(
            !text.contains('\r') && !text.starts_with('\u{feff}'),
            "source normalization requires a verified coordinate convention"
        );
        fn offset(text: &str, pos: &CgaFilePosition) -> anyhow::Result<usize> {
            ensure!(pos.line > 0, "compiler lines are one-based");
            ensure!(pos.column > 0, "compiler columns are one-based");
            let mut base = 0;
            for (line_index, line) in text.split('\n').enumerate() {
                if line_index == pos.line as usize - 1 {
                    let column = pos.column as usize - 1;
                    let at = match line.char_indices().nth(column) {
                        Some((index, _)) => index,
                        None => {
                            ensure!(
                                column == line.chars().count(),
                                "compiler column exceeds source line"
                            );
                            line.len()
                        }
                    };
                    return Ok(base + at);
                }
                base += line.len() + 1;
            }
            anyhow::bail!("compiler line exceeds source file")
        }
        let start = offset(text, &self.start)?;
        let end = offset(text, &self.end)?;
        ensure!(start <= end, "reversed compiler span");
        Ok([start, end])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaLocation {
    pub path: String,
    pub span: CgaSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaNode {
    pub name: String,
    pub location: CgaLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgaEdgeKind {
    Called(u32),
    VTableCalled(u32),
    FnPtrCast(u32),
    BuiltinAssert,
    AsyncPanic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaEdge {
    pub kind: CgaEdgeKind,
    pub location: CgaLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaGraph {
    pub roots: Vec<u32>,
    pub nodes: Vec<CgaNode>,
    #[serde(deserialize_with = "deserialize_edges")]
    pub edges: BTreeMap<u32, Vec<CgaEdge>>,
}

// Serde's ordinary map accepts duplicate keys by overwriting. That would drop
// exported observations before reference validation, so reject them explicitly.
fn deserialize_edges<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<u32, Vec<CgaEdge>>, D::Error> {
    struct Edges;
    impl<'de> serde::de::Visitor<'de> for Edges {
        type Value = BTreeMap<u32, Vec<CgaEdge>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique canonical u32 edge-source keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut edges = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, Vec<CgaEdge>>()? {
                let id = key.parse::<u32>().map_err(serde::de::Error::custom)?;
                if key != id.to_string() || edges.insert(id, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate/noncanonical CGA edge-source key",
                    ));
                }
            }
            Ok(edges)
        }
    }
    deserializer.deserialize_map(Edges)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmappedReason {
    UnknownLocation,
    SourceNotSupplied,
    NoSiteAtSpan,
    CallerNotBound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocationAssociation {
    Exact { site_id: String },
    UniqueContaining { site_id: String },
    Ambiguous { site_ids: Vec<String> },
    Unmapped { reason: UnmappedReason },
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertedCgaLocation {
    pub file_path: Option<String>,
    pub byte_range: Option<[usize; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerObservation {
    pub from_node: u32,
    pub edge_index: usize,
    pub canonical_caller: String,
    pub canonical_callee: Option<String>,
    pub kind: CgaEdgeKind,
    pub raw_location: CgaLocation,
    pub converted_location: ConvertedCgaLocation,
    pub association: LocationAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgaCapabilities {
    pub receiver_type: CapabilityStatus,
    pub assert_reason: CapabilityStatus,
    pub macro_expansion: CapabilityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CgaCoordinateConvention {
    Line1ScalarColumn1EndExclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerEvidenceSet {
    pub schema_version: String,
    pub build_id: String,
    pub identity: CaptureBuildIdentity,
    pub raw_graph: CgaGraph,
    pub observations: Vec<CompilerObservation>,
    pub capabilities: CgaCapabilities,
    pub coordinate_convention: CgaCoordinateConvention,
}

pub struct CgaImportRequest<'a> {
    pub identity: &'a CaptureBuildIdentity,
    pub expected_build_id: &'a str,
    pub raw_export: &'a [u8],
    pub inputs: &'a [CaptureInputBytes<'a>],
    pub graph: &'a GraphIndexArtifact,
    pub sources: &'a [PanicEvidenceSource<'a>],
    pub method_sites: &'a MethodCallSiteEvidenceSet,
}

impl CompilerEvidenceSet {
    /// Reconstruct all observations from checksum-bound raw bytes and validated
    /// P1 syntax; serialized associations alone do not establish provenance.
    pub fn validate(&self, request: CgaImportRequest<'_>) -> anyhow::Result<()> {
        ensure!(
            self == &import_cga(request)?,
            "compiler evidence differs from canonical import"
        );
        Ok(())
    }
}

pub fn import_cga(request: CgaImportRequest<'_>) -> anyhow::Result<CompilerEvidenceSet> {
    let identity = request.identity;
    let build_id = identity.build_id()?;
    ensure!(
        build_id == request.expected_build_id,
        "expected build identity mismatch before association"
    );
    ensure!(
        sha256_bytes(request.raw_export) == identity.raw_export_sha256,
        "raw export checksum mismatch before parsing"
    );
    identity.validate_inputs(request.inputs)?;
    ensure!(
        identity.graph_content_hash == request.graph.graph_content_hash,
        "compiler capture/graph snapshot mismatch"
    );
    request
        .method_sites
        .validate(request.graph, request.sources)?;
    for source in request.sources {
        ensure!(
            identity.inputs.iter().any(|i| i.path == source.file_path
                && i.kind == CaptureInputKind::Source
                && i.sha256 == sha256_bytes(source.bytes)),
            "method-site source missing or changed in compiler capture inventory"
        );
    }
    let raw_graph: CgaGraph =
        serde_json::from_slice(request.raw_export).context("invalid pinned CGA export format")?;
    let node_exists = |id: u32| (id as usize) < raw_graph.nodes.len();
    let mut roots = BTreeSet::new();
    for root in &raw_graph.roots {
        ensure!(
            node_exists(*root) && roots.insert(*root),
            "dangling/duplicate CGA root"
        );
    }
    let mut node_locations = Vec::new();
    for node in &raw_graph.nodes {
        ensure!(
            !node.name.trim().is_empty(),
            "missing canonical CGA instance name"
        );
        node_locations.push(convert_location(&node.location, &request)?);
    }
    // Validate every reference and local span before constructing any result.
    for (from, edges) in &raw_graph.edges {
        ensure!(node_exists(*from), "dangling CGA edge source");
        for edge in edges {
            if let Some(target) = edge_target(edge.kind) {
                ensure!(node_exists(target), "dangling CGA edge target");
            }
            convert_location(&edge.location, &request)?;
        }
    }
    let callables = lexical_callables(&request)?;
    let caller_owners: Vec<_> = node_locations
        .iter()
        .map(|(location, _)| resolve_caller(location, &callables))
        .collect();
    let mut observations = Vec::new();
    for (from, edges) in &raw_graph.edges {
        for (edge_index, edge) in edges.iter().enumerate() {
            let (converted_location, state) = convert_location(&edge.location, &request)?;
            let association = associate(
                &converted_location,
                state,
                caller_owners[*from as usize],
                &request,
            );
            observations.push(CompilerObservation {
                from_node: *from,
                edge_index,
                canonical_caller: raw_graph.nodes[*from as usize].name.clone(),
                canonical_callee: edge_target(edge.kind)
                    .map(|id| raw_graph.nodes[id as usize].name.clone()),
                kind: edge.kind,
                raw_location: edge.location.clone(),
                converted_location,
                association,
            });
        }
    }
    Ok(CompilerEvidenceSet {
        schema_version: "panic-cga-evidence-v1".into(),
        build_id,
        identity: identity.canonical()?,
        raw_graph,
        observations,
        capabilities: CgaCapabilities {
            receiver_type: CapabilityStatus::Unavailable,
            assert_reason: CapabilityStatus::Unavailable,
            macro_expansion: CapabilityStatus::Unavailable,
        },
        coordinate_convention: CgaCoordinateConvention::Line1ScalarColumn1EndExclusive,
    })
}

fn edge_target(kind: CgaEdgeKind) -> Option<u32> {
    match kind {
        CgaEdgeKind::Called(id) | CgaEdgeKind::VTableCalled(id) | CgaEdgeKind::FnPtrCast(id) => {
            Some(id)
        }
        CgaEdgeKind::BuiltinAssert | CgaEdgeKind::AsyncPanic => None,
    }
}

#[derive(Clone, Copy)]
enum LocationState {
    Local,
    Unmapped,
    Unknown,
    External,
}

fn convert_location(
    location: &CgaLocation,
    request: &CgaImportRequest<'_>,
) -> anyhow::Result<(ConvertedCgaLocation, LocationState)> {
    let empty = || ConvertedCgaLocation {
        file_path: None,
        byte_range: None,
    };
    if location.path.is_empty() || location.path == "{unknown}" {
        return Ok((empty(), LocationState::Unknown));
    }
    let path = Path::new(&location.path);
    ensure!(!location.path.contains(['\0', '\\']), "invalid CGA path");
    let relative = if path.is_absolute() {
        match path.strip_prefix(&request.identity.capture_root) {
            Ok(local) => local.to_str().context("non UTF-8 CGA path")?,
            Err(_) => return Ok((empty(), LocationState::External)),
        }
    } else {
        &location.path
    };
    ensure!(
        normal_relative_path(relative),
        "noncanonical local CGA path"
    );
    ensure!(
        location.span.start.line > 0
            && location.span.start.column > 0
            && location.span.end.column > 0
            && (location.span.start.line, location.span.start.column)
                <= (location.span.end.line, location.span.end.column),
        "malformed local CGA span"
    );
    let Some(input) = request
        .identity
        .inputs
        .iter()
        .find(|i| i.path == relative && i.kind == CaptureInputKind::Source)
    else {
        return Ok((
            ConvertedCgaLocation {
                file_path: Some(relative.into()),
                byte_range: None,
            },
            LocationState::Unmapped,
        ));
    };
    let bytes = request
        .inputs
        .iter()
        .find(|i| i.path == input.path)
        .context("missing inventoried compiler source")?
        .bytes;
    let range = location
        .span
        .to_byte_range(bytes)
        .with_context(|| format!("invalid local CGA span in {relative}"))?;
    Ok((
        ConvertedCgaLocation {
            file_path: Some(relative.into()),
            byte_range: Some(range),
        },
        LocationState::Local,
    ))
}

fn associate(
    location: &ConvertedCgaLocation,
    state: LocationState,
    caller: Option<&LexicalCallable>,
    request: &CgaImportRequest<'_>,
) -> LocationAssociation {
    let unmapped = |reason| LocationAssociation::Unmapped { reason };
    match state {
        LocationState::External => return LocationAssociation::External,
        LocationState::Unknown => return unmapped(UnmappedReason::UnknownLocation),
        LocationState::Unmapped => return unmapped(UnmappedReason::SourceNotSupplied),
        LocationState::Local => {}
    }
    let (Some(path), Some(range)) = (&location.file_path, location.byte_range) else {
        return unmapped(UnmappedReason::SourceNotSupplied);
    };
    if !request.sources.iter().any(|s| s.file_path == path) {
        return unmapped(UnmappedReason::SourceNotSupplied);
    }
    if range[0] == range[1] {
        return unmapped(UnmappedReason::NoSiteAtSpan);
    }
    let candidates: Vec<_> = request
        .method_sites
        .sites
        .iter()
        .filter(|site| site.file_path == *path && contains(site.byte_range, range))
        .collect();
    if candidates.is_empty() {
        return unmapped(UnmappedReason::NoSiteAtSpan);
    }
    let bound: Vec<_> = candidates
        .into_iter()
        .filter(|site| caller_owns(site, caller))
        .collect();
    if bound.is_empty() {
        return unmapped(UnmappedReason::CallerNotBound);
    }
    let exact: Vec<_> = bound
        .iter()
        .filter(|s| s.byte_range == range)
        .copied()
        .collect();
    if let [site] = exact.as_slice() {
        return LocationAssociation::Exact {
            site_id: site.site_id.clone(),
        };
    }
    let remaining = if exact.is_empty() { &bound } else { &exact };
    if let [site] = remaining.as_slice() {
        return LocationAssociation::UniqueContaining {
            site_id: site.site_id.clone(),
        };
    }
    let mut site_ids: Vec<_> = remaining.iter().map(|s| s.site_id.clone()).collect();
    site_ids.sort();
    LocationAssociation::Ambiguous { site_ids }
}

fn contains(outer: [usize; 2], inner: [usize; 2]) -> bool {
    outer[0] <= inner[0] && inner[1] <= outer[1]
}

struct LexicalCallable {
    file_path: String,
    syntax_kind: String,
    byte_range: [usize; 2],
    header_range: [usize; 2],
    body_range: [usize; 2],
}

/// Resolve definitions from the same validated physical bytes as P1, without
/// changing P1's collector. Nested callable bodies are never part of an
/// ancestor's definition header. Opaque macro token trees stay opaque.
fn lexical_callables(request: &CgaImportRequest<'_>) -> anyhow::Result<Vec<LexicalCallable>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let mut callables = Vec::new();
    for source in request.sources {
        let tree = parser
            .parse(source.bytes, None)
            .context("caller definition parse unavailable")?;
        let mut pending = vec![tree.root_node()];
        while let Some(node) = pending.pop() {
            if matches!(
                node.kind(),
                "macro_definition" | "macro_invocation" | "token_tree"
            ) {
                continue;
            }
            if matches!(node.kind(), "function_item" | "closure_expression")
                && !node.has_error()
                && !node.is_missing()
            {
                if let Some(body) = node.child_by_field_name("body") {
                    callables.push(LexicalCallable {
                        file_path: source.file_path.into(),
                        syntax_kind: node.kind().into(),
                        byte_range: [node.start_byte(), node.end_byte()],
                        header_range: [node.start_byte(), body.start_byte()],
                        body_range: [body.start_byte(), body.end_byte()],
                    });
                }
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
    }
    Ok(callables)
}

/// Reuse the importer's definition-header rules for the new body sidecar.
/// The result is spatial provenance, never compiler instance/type equivalence.
pub(super) fn cga_body_locations(
    graph: &CgaGraph,
    request: &CgaImportRequest<'_>,
) -> anyhow::Result<Vec<Option<ConvertedCgaLocation>>> {
    let callables = lexical_callables(request)?;
    graph
        .nodes
        .iter()
        .map(|node| {
            let (location, _) = convert_location(&node.location, request)?;
            Ok(
                resolve_caller(&location, &callables).map(|owner| ConvertedCgaLocation {
                    file_path: Some(owner.file_path.clone()),
                    byte_range: Some(owner.body_range),
                }),
            )
        })
        .collect()
}

fn resolve_caller<'a>(
    location: &ConvertedCgaLocation,
    callables: &'a [LexicalCallable],
) -> Option<&'a LexicalCallable> {
    let (Some(path), Some(range)) = (&location.file_path, location.byte_range) else {
        return None;
    };
    if range[0] == range[1] {
        return None;
    }
    // rustc def_span may cover a signature or the full callable. A partial
    // location must lie in the definition header, never an arbitrary body
    // token. More than one candidate is insufficient evidence of identity.
    let mut candidates = callables.iter().filter(|owner| {
        owner.file_path == *path
            && (owner.byte_range == range || contains(owner.header_range, range))
    });
    let candidate = candidates.next()?;
    candidates.next().is_none().then_some(candidate)
}

fn caller_owns(site: &MethodCallSiteEvidence, caller: Option<&LexicalCallable>) -> bool {
    let Some(caller) = caller else {
        return false;
    };
    if !matches!(
        site.lexical_phase,
        MethodCallLexicalPhase::FunctionBodySyntax | MethodCallLexicalPhase::ClosureBodySyntax
    ) || caller.file_path != site.file_path
    {
        return false;
    }
    site.lexical_owners
        .iter()
        .find(|o| {
            matches!(
                o.syntax_kind.as_str(),
                "function_item" | "closure_expression"
            )
        })
        .is_some_and(|owner| {
            owner.byte_range == caller.byte_range && owner.syntax_kind == caller.syntax_kind
        })
}
