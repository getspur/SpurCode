//! Pure compatibility codecs for physical Rust evidence. No disk IO.
use super::{canonical, integrity, layout, read_rows, semantic, supported, write_rows};
use crate::panic_evidence::{
    MethodCallSourceScope, RustBodyEvidence, RustCallSiteEvidence, RustCallSiteEvidenceSet,
    RustCallSourceBinding, RUST_CALL_SITE_EXTRACTOR_VERSION, RUST_CALL_SITE_SCHEMA_VERSION,
};
use crate::{evidence::*, *};
use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const PROVIDER_ID: &str = "spur.rust.physical";
pub struct RustEvidenceProvider;
const TABLES: [&str; 5] = [
    "rust_evidence_origins.parquet",
    "rust_evidence_sources.parquet",
    "rust_evidence_bodies.parquet",
    "rust_evidence_sites.parquet",
    "rust_declarations.parquet",
];

fn layouts() -> Vec<EvidenceTable> {
    vec![
        layout(
            "rust_evidence_origins",
            fields(0),
            &["manifest_version", "origin_graph_content_hash"],
            &["manifest_version", "origin_graph_content_hash"],
            true,
        ),
        layout(
            "rust_evidence_sources",
            fields(1),
            &["file_path"],
            &["file_path"],
            false,
        ),
        layout(
            "rust_evidence_bodies",
            fields(2),
            &["file_path", "body_id"],
            &["file_path", "byte_range_start", "byte_range_end", "body_id"],
            false,
        ),
        layout(
            "rust_evidence_sites",
            fields(3),
            &["file_path", "site_id"],
            &["file_path", "byte_range_start", "byte_range_end"],
            false,
        ),
        layout(
            "rust_declarations",
            fields(4),
            &["file_path", "kind", "stable_symbol_id"],
            &["file_path", "kind", "full_range_start", "full_range_end"],
            false,
        ),
    ]
}
fn components() -> BTreeMap<String, String> {
    [
        (
            "declaration_schema_version",
            RUST_DECLARATION_SCHEMA_VERSION,
        ),
        (
            "declaration_extractor_version",
            crate::store::EXTRACTOR_VERSION,
        ),
        ("call_schema_version", RUST_CALL_SITE_SCHEMA_VERSION),
        ("call_extractor_version", RUST_CALL_SITE_EXTRACTOR_VERSION),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}
fn origins(set: &RustEvidenceSet) -> Vec<EvidenceOrigin> {
    set.origins
        .iter()
        .map(|o| EvidenceOrigin {
            key: EvidenceOriginKey {
                manifest_version: o.manifest_version.clone(),
                graph_content_hash: o.graph_content_hash.clone(),
            },
            sources: o
                .sources
                .iter()
                .map(|s| EvidenceSourceKey {
                    file_path: s.path.clone(),
                    stable_file_id: s.stable_file_id.clone(),
                    source_oid: s.content_oid.clone(),
                })
                .collect(),
        })
        .collect()
}
fn domain(set: &RustEvidenceSet) -> anyhow::Result<EvidenceDomain> {
    Ok(EvidenceDomain::Known {
        sources: set
            .files
            .iter()
            .map(|f| {
                let origin = set
                    .origins
                    .iter()
                    .find(|o| o.graph_content_hash == f.origin_graph_content_hash)
                    .context("missing Rust origin")?;
                let declaration_unknown =
                    f.declarations.sources.iter().any(|s| {
                        matches!(s.availability, RustDeclarationAvailability::Unknown { .. })
                    });
                let coverage =
                    if declaration_unknown || matches!(f.calls, RustCallEvidence::Unknown { .. }) {
                        let reason = match &f.calls {
                            RustCallEvidence::Unknown { reason } => Some(reason),
                            _ => None,
                        };
                        EvidenceCoverage::Unknown {
                            code: serde_json::to_string(&(
                                f.declarations
                                    .sources
                                    .iter()
                                    .map(|s| &s.availability)
                                    .collect::<Vec<_>>(),
                                reason,
                            ))?,
                        }
                    } else {
                        EvidenceCoverage::Complete
                    };
                Ok(EvidenceSourceCoverage {
                    source: EvidenceSourceKey {
                        file_path: f.file_path.clone(),
                        stable_file_id: f.stable_file_id.clone(),
                        source_oid: f.source_oid.clone(),
                    },
                    byte_length: Some(u64::try_from(f.byte_length)?),
                    origin: Some(EvidenceOriginKey {
                        manifest_version: origin.manifest_version.clone(),
                        graph_content_hash: origin.graph_content_hash.clone(),
                    }),
                    coverage,
                })
            })
            .collect::<anyhow::Result<_>>()?,
    })
}
fn extension(set: &RustEvidenceSet, manifest_version: &str) -> anyhow::Result<EvidenceExtension> {
    let components = components();
    let descriptor = EvidenceDescriptor {
        provider_id: PROVIDER_ID.into(),
        schema_version: set.schema_version.clone(),
        extractor_version: set.extractor_version.clone(),
        component_versions: components.clone(),
        presence: EvidencePresence::Captured,
        metadata: BTreeMap::from([
            ("graph_content_hash".into(), set.graph_content_hash.clone()),
            ("manifest_version".into(), manifest_version.into()),
        ]),
    };
    let rows = encode(set)?;
    let checksum = rust_evidence_checksum_blake3_hex(set)?;
    let mut tables = layouts();
    for (i, t) in tables.iter_mut().enumerate() {
        t.metadata = BTreeMap::from([
            (
                "graph_schema_version".into(),
                crate::store::SCHEMA_VERSION.into(),
            ),
            (
                "graph_extractor_version".into(),
                crate::store::EXTRACTOR_VERSION.into(),
            ),
            ("manifest_version".into(), manifest_version.into()),
            ("graph_content_hash".into(), set.graph_content_hash.clone()),
            ("schema_version".into(), set.schema_version.clone()),
            ("extractor_version".into(), set.extractor_version.clone()),
            ("source_scope".into(), "supplied_physical_rust_files".into()),
            ("checksum_blake3".into(), checksum.clone()),
            ("table_name".into(), TABLES[i].into()),
        ]);
        t.metadata.extend(components.clone());
        write_rows(t, &rows[i])?;
    }
    Ok(canonical(&EvidenceExtension {
        descriptor,
        origins: origins(set),
        domain: domain(set)?,
        tables,
    })?)
}
fn decode_projection(extension: &EvidenceExtension) -> Result<RustEvidenceSet, EvidenceError> {
    supported(extension, PROVIDER_ID, RUST_EVIDENCE_SCHEMA_VERSION)?;
    (|| -> anyhow::Result<_> {
        ensure!(
            extension.descriptor.presence == EvidencePresence::Captured,
            "Rust evidence is absent"
        );
        ensure!(
            extension.descriptor.extractor_version == RUST_EVIDENCE_EXTRACTOR_VERSION
                && extension.descriptor.component_versions == components(),
            "incompatible Rust evidence components"
        );
        // A validated query projection may include unavailable coverage with
        // no captured length or origin. Canonical stored extensions require
        // those fields; decode their rows here without imposing stored-set
        // rules on query coverage. Full decode still checks integrity and
        // exact canonical reconstruction against the supplied graph below.
        let tables = read_rows(extension, &layouts())?;
        let (files, origins) = decode_tables(&tables)?;
        Ok(RustEvidenceSet {
            schema_version: extension.descriptor.schema_version.clone(),
            extractor_version: extension.descriptor.extractor_version.clone(),
            graph_content_hash: extension
                .descriptor
                .metadata
                .get("graph_content_hash")
                .context("missing graph hash")?
                .clone(),
            files,
            origins,
        })
    })()
    .map_err(|e| semantic(PROVIDER_ID, e))
}

pub fn encode_rust_evidence(
    graph: &GraphIndexArtifact,
    set: &RustEvidenceSet,
) -> Result<EvidenceExtension, EvidenceError> {
    set.validate_stored(graph)
        .map_err(|e| semantic(PROVIDER_ID, e))?;
    let ext = extension(set, &graph.manifest_version).map_err(|e| semantic(PROVIDER_ID, e))?;
    integrity(graph, &ext)?;
    Ok(ext)
}
pub fn decode_rust_evidence(
    graph: &GraphIndexArtifact,
    ext: &EvidenceExtension,
) -> Result<RustEvidenceSet, EvidenceError> {
    integrity(graph, ext)?;
    let set = decode_projection(ext)?;
    set.validate_stored(graph)
        .map_err(|e| semantic(PROVIDER_ID, e))?;
    let expected =
        extension(&set, &graph.manifest_version).map_err(|e| semantic(PROVIDER_ID, e))?;
    if expected != canonical(ext)? {
        return Err(semantic(
            PROVIDER_ID,
            "Rust scalar, header, metadata, origin or coverage projection mismatch",
        ));
    }
    Ok(set)
}
impl EvidenceProvider for RustEvidenceProvider {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn supports_path(&self, p: &str) -> bool {
        super::physical_path(p)
            && std::path::Path::new(p)
                .extension()
                .is_some_and(|e| e == "rs")
    }
    fn source_domain(
        &self,
        graph: &GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError> {
        super::source_domain(graph, |p| self.supports_path(p)).map_err(|e| semantic(PROVIDER_ID, e))
    }
    fn validate_stored(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        integrity(graph, ext)?;
        supported(ext, PROVIDER_ID, RUST_EVIDENCE_SCHEMA_VERSION)?;
        if ext.descriptor.presence == EvidencePresence::Absent {
            if ext.descriptor.extractor_version != RUST_EVIDENCE_EXTRACTOR_VERSION
                || ext.descriptor.component_versions != components()
            {
                return Err(semantic(PROVIDER_ID, "incompatible absent Rust descriptor"));
            }
            self.source_domain(graph)?;
            return Ok(EvidenceValidation {
                status: EvidenceSemanticStatus::Validated,
                domain: EvidenceDomain::Unknown,
            });
        }
        decode_rust_evidence(graph, ext)?;
        Ok(EvidenceValidation {
            status: EvidenceSemanticStatus::Validated,
            domain: ext.domain.clone(),
        })
    }
    fn validate_sources(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
        sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError> {
        let set = decode_rust_evidence(graph, ext)?;
        let sources = sources
            .iter()
            .map(|s| RustDeclarationSource {
                file_path: s.file_path,
                bytes: s.bytes,
            })
            .collect::<Vec<_>>();
        set.validate_sources(graph, &sources)
            .map_err(|e| semantic(PROVIDER_ID, e))
    }
}

pub fn query_rust_evidence<C: GraphQueryClient + ?Sized>(
    client: &C,
    filter: &RustEvidenceFilter,
) -> anyhow::Result<RustEvidenceQueryResult> {
    filter.validate_shape()?;
    let result = client.evidence(&EvidenceFilter {
        provider_id: PROVIDER_ID.into(),
        file_path: filter.file_path.clone(),
        source_oid: filter.source_oid.clone(),
    })?;
    let mut output = RustEvidenceQueryResult {
        availability: super::availability(&result.availability),
        source_domain: None,
        files: vec![],
        origins: vec![],
    };
    if result.semantic_status != EvidenceSemanticStatus::Validated {
        output.availability = RustEvidenceAvailability::Unavailable;
        return Ok(output);
    }
    if let EvidenceDomain::Known { sources } = &result.domain {
        output.source_domain = Some(
            sources
                .iter()
                .filter(|s| {
                    filter
                        .file_path
                        .as_ref()
                        .is_none_or(|p| p == &s.source.file_path)
                })
                .map(|s| RustEvidenceQuerySource {
                    file_path: s.source.file_path.clone(),
                    stable_file_id: s.source.stable_file_id.clone(),
                    source_oid: s.source.source_oid.clone(),
                    availability: match s.coverage {
                        EvidenceCoverage::Complete => RustEvidenceAvailability::Complete,
                        EvidenceCoverage::Unknown { .. } => RustEvidenceAvailability::Partial,
                        EvidenceCoverage::Unavailable => RustEvidenceAvailability::Unavailable,
                    },
                })
                .collect(),
        );
    }
    let Some(ext) = super::query_extension(&result) else {
        return Ok(output);
    };
    if ext.descriptor.presence == EvidencePresence::Absent {
        output.availability = RustEvidenceAvailability::Unavailable;
        return Ok(output);
    }
    let set = decode_projection(&ext)?;
    output.files = set
        .files
        .into_iter()
        .filter(|f| filter.file_path.as_ref().is_none_or(|p| p == &f.file_path))
        .collect();
    output.origins = crate::rust_evidence::referenced_origins(&output.files, &set.origins);
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationHeader {
    schema_version: String,
    extractor_version: String,
    graph_content_hash: String,
    scope: RustDeclarationScope,
    covered_kinds: Vec<RustDeclarationKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallHeader {
    schema_version: String,
    extractor_version: String,
    graph_content_hash: String,
    source_scope: MethodCallSourceScope,
}

fn fields(table: usize) -> Vec<EvidenceColumn> {
    let columns: &[(&str, bool, bool)] = match table {
        0 => &[
            ("origin_graph_content_hash", false, false),
            ("manifest_version", false, false),
            ("sources_json", false, false),
        ],
        1 => &[
            ("file_path", false, false),
            ("stable_file_id", false, false),
            ("source_oid", false, false),
            ("byte_length", true, false),
            ("origin_graph_content_hash", false, false),
            ("declaration_header_json", false, false),
            ("declaration_source_json", false, false),
            ("call_availability", false, false),
            ("call_unknown_reason", false, true),
            ("call_header_json", false, true),
            ("call_source_json", false, true),
        ],
        2 => &[
            ("file_path", false, false),
            ("source_oid", false, false),
            ("origin_graph_content_hash", false, false),
            ("body_id", false, false),
            ("byte_range_start", true, false),
            ("byte_range_end", true, false),
            ("kind", false, false),
            ("definition_stable_symbol_id", false, true),
        ],
        3 => &[
            ("file_path", false, false),
            ("source_oid", false, false),
            ("origin_graph_content_hash", false, false),
            ("site_id", false, false),
            ("body_id", false, false),
            ("byte_range_start", true, false),
            ("byte_range_end", true, false),
            ("call_text", false, false),
        ],
        4 => &[
            ("file_path", false, false),
            ("stable_file_id", false, false),
            ("source_oid", false, false),
            ("origin_graph_content_hash", false, false),
            ("stable_symbol_id", false, false),
            ("kind", false, false),
            ("structural_owner_json", false, false),
            ("lexical_owner_json", false, false),
            ("full_range_start", true, false),
            ("full_range_end", true, false),
            ("line_start", true, false),
            ("line_end", true, false),
            ("payload_json", false, false),
        ],
        _ => unreachable!(),
    };
    columns
        .iter()
        .map(|(name, int, nullable)| EvidenceColumn {
            name: (*name).into(),
            data_type: if *int {
                EvidenceColumnType::Int64
            } else {
                EvidenceColumnType::Utf8
            },
            nullable: *nullable,
        })
        .collect()
}
fn encode(set: &RustEvidenceSet) -> anyhow::Result<[Vec<Value>; 5]> {
    let mut tables: [Vec<Value>; 5] = Default::default();
    for origin in &set.origins {
        tables[0].push(json!({"origin_graph_content_hash":origin.graph_content_hash,"manifest_version":origin.manifest_version,"sources_json":serde_json::to_string(&origin.sources)?}));
    }
    for file in &set.files {
        let declarations = &file.declarations;
        let declaration_header = DeclarationHeader {
            schema_version: declarations.schema_version.clone(),
            extractor_version: declarations.extractor_version.clone(),
            graph_content_hash: declarations.graph_content_hash.clone(),
            scope: declarations.scope,
            covered_kinds: declarations.covered_kinds.clone(),
        };
        let (availability, reason, call_header, call_source) = match &file.calls {
            RustCallEvidence::Complete { evidence } => {
                let header = CallHeader {
                    schema_version: evidence.schema_version.clone(),
                    extractor_version: evidence.extractor_version.clone(),
                    graph_content_hash: evidence.graph_content_hash.clone(),
                    source_scope: evidence.source_scope.clone(),
                };
                (
                    "complete",
                    None,
                    Some(serde_json::to_string(&header)?),
                    Some(serde_json::to_string(&evidence.sources[0])?),
                )
            }
            RustCallEvidence::Unknown { reason } => {
                ("unknown", Some(serde_json::to_value(reason)?), None, None)
            }
        };
        tables[1].push(json!({"file_path":file.file_path,"stable_file_id":file.stable_file_id,"source_oid":file.source_oid,"byte_length":file.byte_length,"origin_graph_content_hash":file.origin_graph_content_hash,"declaration_header_json":serde_json::to_string(&declaration_header)?,"declaration_source_json":serde_json::to_string(&declarations.sources[0])?,"call_availability":availability,"call_unknown_reason":reason,"call_header_json":call_header,"call_source_json":call_source}));
        if let RustCallEvidence::Complete { evidence } = &file.calls {
            for body in &evidence.bodies {
                tables[2].push(json!({"file_path":body.file_path,"source_oid":body.source_oid,"origin_graph_content_hash":file.origin_graph_content_hash,"body_id":body.body_id,"byte_range_start":body.byte_range[0],"byte_range_end":body.byte_range[1],"kind":body.kind,"definition_stable_symbol_id":body.definition_stable_symbol_id}));
            }
            for site in &evidence.sites {
                tables[3].push(json!({"file_path":site.file_path,"source_oid":site.source_oid,"origin_graph_content_hash":file.origin_graph_content_hash,"site_id":site.site_id,"body_id":site.body_id,"byte_range_start":site.byte_range[0],"byte_range_end":site.byte_range[1],"call_text":site.call_text}));
            }
        }
        for (kind, rows) in [
            ("tuple_field", serde_json::to_value(&declarations.rows)?),
            (
                "foreign_function",
                serde_json::to_value(&declarations.foreign_functions)?,
            ),
            (
                "extern_block",
                serde_json::to_value(&declarations.extern_blocks)?,
            ),
        ] {
            for payload in rows.as_array().context("invalid Rust declaration rows")? {
                let mut row = serde_json::Map::new();
                for name in [
                    "file_path",
                    "stable_file_id",
                    "source_oid",
                    "stable_symbol_id",
                ] {
                    row.insert(name.into(), payload[name].clone());
                }
                row.insert(
                    "origin_graph_content_hash".into(),
                    json!(file.origin_graph_content_hash),
                );
                row.insert("kind".into(), json!(kind));
                for name in ["structural_owner", "lexical_owner"] {
                    row.insert(
                        format!("{name}_json"),
                        json!(serde_json::to_string(&payload[name])?),
                    );
                }
                for (name, from, index) in [
                    ("full_range_start", "full_range", 0),
                    ("full_range_end", "full_range", 1),
                    ("line_start", "line_range", 0),
                    ("line_end", "line_range", 1),
                ] {
                    row.insert(name.into(), payload[from][index].clone());
                }
                row.insert(
                    "payload_json".into(),
                    json!(serde_json::to_string(payload)?),
                );
                tables[4].push(Value::Object(row));
            }
        }
    }
    Ok(tables)
}
fn decode_tables(
    tables: &[Vec<Value>],
) -> anyhow::Result<(Vec<RustEvidenceFile>, Vec<RustEvidenceOrigin>)> {
    let mut decoded_files = Vec::new();
    let mut origins = Vec::new();
    for row in &tables[0] {
        origins.push(RustEvidenceOrigin {
            graph_content_hash: string(row, "origin_graph_content_hash")?.into(),
            manifest_version: string(row, "manifest_version")?.into(),
            sources: decode_json::<Vec<RustEvidenceOriginSource>>(row, "sources_json")?,
        });
    }
    let mut files = BTreeMap::new();
    for row in &tables[1] {
        let header: DeclarationHeader = decode_json(row, "declaration_header_json")?;
        let source: RustDeclarationFile = decode_json(row, "declaration_source_json")?;
        let declarations = RustDeclarationEvidenceSet {
            schema_version: header.schema_version,
            extractor_version: header.extractor_version,
            graph_content_hash: header.graph_content_hash,
            scope: header.scope,
            covered_kinds: header.covered_kinds,
            sources: vec![source],
            rows: Vec::new(),
            foreign_functions: Vec::new(),
            extern_blocks: Vec::new(),
        };
        let calls = match string(row, "call_availability")? {
            "complete" => {
                ensure!(
                    row["call_unknown_reason"].is_null(),
                    "complete Rust calls have an unknown reason"
                );
                let header: CallHeader = decode_json(row, "call_header_json")?;
                let source: RustCallSourceBinding = decode_json(row, "call_source_json")?;
                RustCallEvidence::Complete {
                    evidence: RustCallSiteEvidenceSet {
                        schema_version: header.schema_version,
                        extractor_version: header.extractor_version,
                        graph_content_hash: header.graph_content_hash,
                        source_scope: header.source_scope,
                        sources: vec![source],
                        bodies: Vec::new(),
                        sites: Vec::new(),
                    },
                }
            }
            "unknown" => {
                ensure!(
                    row["call_header_json"].is_null() && row["call_source_json"].is_null(),
                    "unknown Rust calls have a complete payload"
                );
                RustCallEvidence::Unknown {
                    reason: serde_json::from_value(row["call_unknown_reason"].clone())?,
                }
            }
            value => anyhow::bail!("invalid Rust call availability {value:?}"),
        };
        let file = RustEvidenceFile {
            file_path: string(row, "file_path")?.into(),
            stable_file_id: string(row, "stable_file_id")?.into(),
            source_oid: string(row, "source_oid")?.into(),
            byte_length: integer(row, "byte_length")?,
            origin_graph_content_hash: string(row, "origin_graph_content_hash")?.into(),
            declarations,
            calls,
        };
        ensure!(
            files
                .insert(file.file_path.clone(), decoded_files.len())
                .is_none(),
            "duplicate Rust source row"
        );
        decoded_files.push(file);
    }
    for (index, rows) in tables.iter().enumerate().skip(2) {
        for row in rows {
            let file = &mut decoded_files[*files
                .get(string(row, "file_path")?)
                .context("Rust row has no source")?];
            ensure!(
                row["source_oid"] == file.source_oid
                    && row["origin_graph_content_hash"] == file.origin_graph_content_hash,
                "Rust source/origin projection mismatch in {}",
                TABLES[index]
            );
            if index == 4 {
                let text = string(row, "payload_json")?;
                // Decode typed rows directly: Value-first parsing would erase
                // duplicate JSON keys before strict Serde could reject them.
                let payload = match string(row, "kind")? {
                    "tuple_field" => {
                        let value = serde_json::from_str(text)?;
                        let payload = serde_json::to_value(&value)?;
                        file.declarations.rows.push(value);
                        payload
                    }
                    "foreign_function" => {
                        let value = serde_json::from_str(text)?;
                        let payload = serde_json::to_value(&value)?;
                        file.declarations.foreign_functions.push(value);
                        payload
                    }
                    "extern_block" => {
                        let value = serde_json::from_str(text)?;
                        let payload = serde_json::to_value(&value)?;
                        file.declarations.extern_blocks.push(value);
                        payload
                    }
                    kind => anyhow::bail!("invalid Rust declaration kind {kind:?}"),
                };
                for name in [
                    "file_path",
                    "stable_file_id",
                    "source_oid",
                    "stable_symbol_id",
                ] {
                    ensure!(
                        row[name] == payload[name],
                        "Rust declaration scalar/payload mismatch: {name}"
                    );
                }
                for name in ["structural_owner", "lexical_owner"] {
                    let owner: RustDeclarationOwner = decode_json(row, &format!("{name}_json"))?;
                    ensure!(
                        serde_json::to_value(owner)? == payload[name],
                        "Rust declaration owner/payload mismatch: {name}"
                    );
                }
                for (name, from, offset) in [
                    ("full_range_start", "full_range", 0),
                    ("full_range_end", "full_range", 1),
                    ("line_start", "line_range", 0),
                    ("line_end", "line_range", 1),
                ] {
                    ensure!(
                        row[name] == payload[from][offset],
                        "Rust declaration range/payload mismatch: {name}"
                    );
                }
            } else {
                let RustCallEvidence::Complete { evidence } = &mut file.calls else {
                    anyhow::bail!("Rust body/site belongs to Unknown calls");
                };
                let mut payload = row.as_object().context("invalid Rust row")?.clone();
                payload.remove("origin_graph_content_hash");
                let start = payload
                    .remove("byte_range_start")
                    .context("missing range start")?;
                let end = payload
                    .remove("byte_range_end")
                    .context("missing range end")?;
                payload.insert("byte_range".into(), json!([start, end]));
                if index == 2 {
                    evidence
                        .bodies
                        .push(serde_json::from_value::<RustBodyEvidence>(Value::Object(
                            payload,
                        ))?);
                } else {
                    evidence
                        .sites
                        .push(serde_json::from_value::<RustCallSiteEvidence>(
                            Value::Object(payload),
                        )?);
                }
            }
        }
    }
    Ok((decoded_files, origins))
}
fn string<'a>(row: &'a Value, name: &str) -> anyhow::Result<&'a str> {
    row[name].as_str().context("invalid string column")
}
fn integer(row: &Value, name: &str) -> anyhow::Result<usize> {
    usize::try_from(row[name].as_i64().context("invalid integer column")?)
        .context("negative or overflowing coordinate")
}
fn decode_json<T: serde::de::DeserializeOwned>(row: &Value, name: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(string(row, name)?)?)
}
