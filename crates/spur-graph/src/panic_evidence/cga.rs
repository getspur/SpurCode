use super::{
    CaptureBuildIdentity, CaptureInputBytes, MethodCallSiteEvidenceSet, PanicEvidenceSource,
};
use crate::GraphIndexArtifact;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub fn to_byte_range(&self, _bytes: &[u8]) -> anyhow::Result<[usize; 2]> {
        Ok([0, 0])
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
    pub edges: BTreeMap<u32, Vec<CgaEdge>>,
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
    Line1ScalarColumn0EndExclusiveNeedsRealExportCheck,
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
    pub fn validate(&self, _request: CgaImportRequest<'_>) -> anyhow::Result<()> {
        Ok(())
    }
}

pub fn import_cga(request: CgaImportRequest<'_>) -> anyhow::Result<CompilerEvidenceSet> {
    Ok(CompilerEvidenceSet {
        schema_version: String::new(),
        build_id: String::new(),
        identity: request.identity.clone(),
        raw_graph: CgaGraph {
            roots: vec![],
            nodes: vec![],
            edges: BTreeMap::new(),
        },
        observations: vec![],
        capabilities: CgaCapabilities {
            receiver_type: CapabilityStatus::Unavailable,
            assert_reason: CapabilityStatus::Unavailable,
            macro_expansion: CapabilityStatus::Unavailable,
        },
        coordinate_convention:
            CgaCoordinateConvention::Line1ScalarColumn0EndExclusiveNeedsRealExportCheck,
    })
}
