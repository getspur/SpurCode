//! Lossless core transport for the authenticated physical Rust envelope.
//! Checksums establish stored integrity; saved source bytes establish syntax.

use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    path::Path,
    sync::Arc,
};

use anyhow::{ensure, Context as _};
use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::{
    arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    current_manifest_version, parquet::GraphArtifactManifest, EXTRACTOR_VERSION, SCHEMA_VERSION,
};
use crate::panic_evidence::{
    MethodCallSourceScope, RustBodyEvidence, RustCallSiteEvidence, RustCallSiteEvidenceSet,
    RustCallSourceBinding, RUST_CALL_SITE_EXTRACTOR_VERSION, RUST_CALL_SITE_SCHEMA_VERSION,
};
use crate::rust_evidence::{referenced_origins, RustEvidenceQueryContext};
use crate::{
    rust_evidence_checksum_blake3_hex, GraphIndexArtifact, RustCallEvidence,
    RustDeclarationEvidenceSet, RustDeclarationFile, RustDeclarationKind, RustDeclarationOwner,
    RustDeclarationScope, RustEvidenceFile, RustEvidenceOrigin, RustEvidenceOriginSource,
    RustEvidenceSet, RUST_DECLARATION_SCHEMA_VERSION, RUST_EVIDENCE_EXTRACTOR_VERSION,
    RUST_EVIDENCE_SCHEMA_VERSION,
};

const TABLES: [&str; 5] = [
    "rust_evidence_origins.parquet",
    "rust_evidence_sources.parquet",
    "rust_evidence_bodies.parquet",
    "rust_evidence_sites.parquet",
    "rust_declarations.parquet",
];

#[cfg(test)]
thread_local! {
    // Successful evidence-file byte reads, isolated from concurrent test threads.
    pub(crate) static RUST_EVIDENCE_QUERY_IO: std::cell::Cell<(usize, u64)> = const { std::cell::Cell::new((0, 0)) };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceRowCounts {
    pub origins: usize,
    pub sources: usize,
    pub declarations: usize,
    pub bodies: usize,
    pub sites: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceManifest {
    pub complete: bool,
    pub schema_version: String,
    pub extractor_version: String,
    pub declaration_schema_version: String,
    pub declaration_extractor_version: String,
    pub call_schema_version: String,
    pub call_extractor_version: String,
    pub checksum_blake3: String,
    pub row_counts: RustEvidenceRowCounts,
}

/// Identical presence/version/checksum encoding to the artifact helper. This
/// identity is not source authentication or a full canonical artifact hash.
pub fn rust_evidence_manifest_capability_digest(
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<String> {
    let evidence = manifest.rust_evidence.as_ref();
    let encoded = serde_json::to_vec(&(
        "spur-rust-evidence-capability-v1",
        if evidence.is_some() {
            "present"
        } else {
            "absent"
        },
        evidence.map_or(RUST_EVIDENCE_SCHEMA_VERSION, |e| e.schema_version.as_str()),
        evidence.map_or(RUST_EVIDENCE_EXTRACTOR_VERSION, |e| {
            e.extractor_version.as_str()
        }),
        evidence.map_or(RUST_DECLARATION_SCHEMA_VERSION, |e| {
            e.declaration_schema_version.as_str()
        }),
        evidence.map_or(EXTRACTOR_VERSION, |e| {
            e.declaration_extractor_version.as_str()
        }),
        evidence.map_or(RUST_CALL_SITE_SCHEMA_VERSION, |e| {
            e.call_schema_version.as_str()
        }),
        evidence.map_or(RUST_CALL_SITE_EXTRACTOR_VERSION, |e| {
            e.call_extractor_version.as_str()
        }),
        manifest.manifest_version.as_str(),
        evidence.map(|e| e.checksum_blake3.as_str()),
    ))?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
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

fn fields(table: usize) -> Vec<Field> {
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
        .map(|(name, int, nullable)| {
            Field::new(
                *name,
                if *int {
                    DataType::Int64
                } else {
                    DataType::Utf8
                },
                *nullable,
            )
        })
        .collect()
}

fn metadata(
    evidence: &RustEvidenceManifest,
    graph_hash: &str,
    manifest_version: &str,
    table: &str,
) -> HashMap<String, String> {
    HashMap::from([
        ("graph_schema_version".into(), SCHEMA_VERSION.into()),
        ("graph_extractor_version".into(), EXTRACTOR_VERSION.into()),
        ("manifest_version".into(), manifest_version.into()),
        ("graph_content_hash".into(), graph_hash.into()),
        ("schema_version".into(), evidence.schema_version.clone()),
        (
            "extractor_version".into(),
            evidence.extractor_version.clone(),
        ),
        (
            "declaration_schema_version".into(),
            evidence.declaration_schema_version.clone(),
        ),
        (
            "declaration_extractor_version".into(),
            evidence.declaration_extractor_version.clone(),
        ),
        (
            "call_schema_version".into(),
            evidence.call_schema_version.clone(),
        ),
        (
            "call_extractor_version".into(),
            evidence.call_extractor_version.clone(),
        ),
        ("source_scope".into(), "supplied_physical_rust_files".into()),
        ("checksum_blake3".into(), evidence.checksum_blake3.clone()),
        ("table_name".into(), table.into()),
    ])
}

fn no_extra_tables(dir: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        ensure!(
            !((name.starts_with("rust_evidence_") || name.starts_with("rust_declarations"))
                && name.ends_with(".parquet"))
                || TABLES.contains(&name.as_ref()),
            "extra Rust evidence table {}; rebuild with spur explore sync",
            entry.path().display()
        );
    }
    Ok(())
}

pub(super) fn write(
    artifact: &GraphIndexArtifact,
    dir: &Path,
) -> anyhow::Result<Option<RustEvidenceManifest>> {
    no_extra_tables(dir)?;
    let Some(set) = &artifact.rust_evidence else {
        ensure!(
            TABLES.iter().all(|t| !dir.join(t).exists()),
            "orphan Rust evidence tables in {}; rebuild with spur explore sync",
            dir.display()
        );
        return Ok(None);
    };
    set.validate_stored(artifact)
        .with_context(|| format!("invalid Rust evidence in {}", dir.display()))?;
    let tables = encode(set)?;
    let evidence = RustEvidenceManifest {
        complete: true,
        schema_version: set.schema_version.clone(),
        extractor_version: set.extractor_version.clone(),
        declaration_schema_version: RUST_DECLARATION_SCHEMA_VERSION.into(),
        declaration_extractor_version: EXTRACTOR_VERSION.into(),
        call_schema_version: RUST_CALL_SITE_SCHEMA_VERSION.into(),
        call_extractor_version: RUST_CALL_SITE_EXTRACTOR_VERSION.into(),
        checksum_blake3: rust_evidence_checksum_blake3_hex(set)?,
        row_counts: RustEvidenceRowCounts {
            origins: tables[0].len(),
            sources: tables[1].len(),
            bodies: tables[2].len(),
            sites: tables[3].len(),
            declarations: tables[4].len(),
        },
    };
    for (index, table) in TABLES.iter().enumerate() {
        write_table(
            &dir.join(table),
            &tables[index],
            fields(index),
            metadata(
                &evidence,
                &artifact.graph_content_hash,
                &artifact.manifest_version,
                table,
            ),
        )?;
    }
    Ok(Some(evidence))
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

fn compatible(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    evidence: &RustEvidenceManifest,
) -> anyhow::Result<()> {
    for (name, actual, expected) in [
        (
            "disk schema",
            manifest.schema_version.as_str(),
            SCHEMA_VERSION,
        ),
        (
            "graph extractor",
            manifest.extractor_version.as_str(),
            EXTRACTOR_VERSION,
        ),
        (
            "manifest version",
            manifest.manifest_version.as_str(),
            current_manifest_version().as_str(),
        ),
        (
            "wrapper schema",
            evidence.schema_version.as_str(),
            RUST_EVIDENCE_SCHEMA_VERSION,
        ),
        (
            "wrapper extractor",
            evidence.extractor_version.as_str(),
            RUST_EVIDENCE_EXTRACTOR_VERSION,
        ),
        (
            "declaration schema",
            evidence.declaration_schema_version.as_str(),
            RUST_DECLARATION_SCHEMA_VERSION,
        ),
        (
            "declaration extractor",
            evidence.declaration_extractor_version.as_str(),
            EXTRACTOR_VERSION,
        ),
        (
            "call schema",
            evidence.call_schema_version.as_str(),
            RUST_CALL_SITE_SCHEMA_VERSION,
        ),
        (
            "call extractor",
            evidence.call_extractor_version.as_str(),
            RUST_CALL_SITE_EXTRACTOR_VERSION,
        ),
    ] {
        ensure!(actual==expected,"incompatible Rust evidence in {}: {name}: expected {expected:?}, actual {actual:?}; rebuild with spur explore sync",dir.display());
    }
    ensure!(
        evidence.complete,
        "incomplete Rust evidence in {}; rebuild with spur explore sync",
        dir.display()
    );
    Ok(())
}

pub(super) fn read(
    dir: &Path,
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<Option<RustEvidenceSet>> {
    read_impl(dir, manifest, None)
}

fn read_impl(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    frozen: Option<&[FrozenRustTable]>,
) -> anyhow::Result<Option<RustEvidenceSet>> {
    no_extra_tables(dir)?;
    let Some(evidence) = &manifest.rust_evidence else {
        ensure!(
            TABLES.iter().all(|t| !dir.join(t).exists()),
            "orphan Rust evidence tables in {}; rebuild with spur explore sync",
            dir.display()
        );
        // Older general graph schemas retain their established absence behavior.
        // They never acquire Rust capability or become supported Rust seeds.
        // Schema names are exact identities, including their canonical spelling.
        let known_legacy =
            (1..=12).any(|v| manifest.schema_version == format!("spur-graph-schema-v{v}"));
        ensure!(manifest.schema_version==SCHEMA_VERSION || known_legacy,"incompatible Rust evidence disk schema in {}: expected {SCHEMA_VERSION} or recognized legacy absence, actual {:?}; rebuild with spur explore sync",dir.display(),manifest.schema_version);
        return Ok(None);
    };
    compatible(dir, manifest, evidence)?;
    let counts = &evidence.row_counts;
    let counts = [
        counts.origins,
        counts.sources,
        counts.bodies,
        counts.sites,
        counts.declarations,
    ];
    let mut tables = Vec::new();
    for (index, table) in TABLES.iter().enumerate() {
        tables.push(read_table(
            &dir.join(table),
            fields(index),
            &metadata(
                evidence,
                &manifest.graph_content_hash,
                &manifest.manifest_version,
                table,
            ),
            counts[index],
            frozen.map(|tables| &tables[index]),
        )?);
    }
    let (files, origins) = decode_tables(&tables)?;
    let set = RustEvidenceSet {
        schema_version: evidence.schema_version.clone(),
        extractor_version: evidence.extractor_version.clone(),
        graph_content_hash: manifest.graph_content_hash.clone(),
        files,
        origins,
    };
    ensure!(
        rust_evidence_checksum_blake3_hex(&set)? == evidence.checksum_blake3,
        "Rust evidence checksum mismatch in {}; rebuild with spur explore sync",
        dir.display()
    );
    Ok(Some(set))
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
    row[name]
        .as_str()
        .with_context(|| format!("invalid required Rust string: {name}"))
}

/// All compressed tables belong to one immutable, fully validated snapshot.
/// Full decoded rows exist only during opening and for the selected query.
pub(crate) struct RustEvidenceQueryTables {
    tables: Vec<FrozenRustTable>,
    context: RustEvidenceQueryContext,
    origins: Vec<RustEvidenceOrigin>,
    // Preserve the actual owner/source projection used for stored validation.
    // It has no evidence rows, unrelated edges or temporal/Lance data.
    _owners: GraphIndexArtifact,
}

impl RustEvidenceQueryTables {
    pub(crate) fn open(
        dir: &Path,
        manifest: &GraphArtifactManifest,
    ) -> anyhow::Result<(Option<Self>, Option<RustEvidenceSet>)> {
        let Some(evidence) = &manifest.rust_evidence else {
            // Absence still checks for orphan tables and legacy compatibility.
            return Ok((None, read(dir, manifest)?));
        };
        no_extra_tables(dir)?;
        compatible(dir, manifest, evidence)?;
        let tables = (0..TABLES.len())
            .map(|i| FrozenRustTable::open(dir, manifest, i))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let set =
            read_impl(dir, manifest, Some(&tables))?.context("missing captured Rust evidence")?;
        let client = crate::query_client::ParquetClient::open(dir)?;
        let mut owners = client.rust_evidence_validation_projection(Some(&set))?;
        owners.rust_evidence = Some(set);
        let context = RustEvidenceQueryContext::new(&owners)?;
        let set = owners
            .rust_evidence
            .take()
            .context("missing validated Rust evidence")?;
        let origins = set.origins.clone();
        Ok((
            Some(Self {
                tables,
                context,
                origins,
                _owners: owners,
            }),
            Some(set),
        ))
    }

    pub(crate) fn query(
        &self,
        filter: &crate::RustEvidenceFilter,
    ) -> anyhow::Result<crate::RustEvidenceQueryResult> {
        let mut result = self.context.result(filter)?;
        if matches!(
            result.availability,
            crate::RustEvidenceAvailability::Unavailable
                | crate::RustEvidenceAvailability::UnsupportedSource
        ) {
            return Ok(result);
        }
        let mut rows = vec![Vec::new()];
        for table in self.tables.iter().skip(1) {
            rows.push((table.read)(filter.file_path.as_deref())?);
        }
        // Decode a projection directly into files; never construct a partial
        // RustEvidenceSet with a forged full-artifact checksum or coverage.
        result.files = decode_tables(&rows)?.0;
        result.origins = referenced_origins(&result.files, &self.origins);
        Ok(result)
    }
}

// Erase Parquet's concrete byte-buffer type, as for the macro query tables,
// without adding a dependency. The closure owns immutable compressed bytes.
type FrozenRustRead = dyn Fn(Option<&str>) -> anyhow::Result<Vec<Value>> + Send + Sync;
struct FrozenRustTable {
    read: Box<FrozenRustRead>,
}

impl FrozenRustTable {
    fn open(dir: &Path, manifest: &GraphArtifactManifest, index: usize) -> anyhow::Result<Self> {
        use parquet::file::reader::ChunkReader as _;
        let path = dir.join(TABLES[index]);
        let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
        let bytes = file.get_bytes(0, usize::try_from(file.metadata()?.len())?)?;
        #[cfg(test)]
        RUST_EVIDENCE_QUERY_IO.with(|n| {
            let (reads, size) = n.get();
            n.set((reads + 1, size + bytes.len() as u64));
        });
        let evidence = manifest
            .rust_evidence
            .as_ref()
            .context("missing Rust evidence metadata")?;
        let counts = &evidence.row_counts;
        let count = [
            counts.origins,
            counts.sources,
            counts.bodies,
            counts.sites,
            counts.declarations,
        ][index];
        let metadata = metadata(
            evidence,
            &manifest.graph_content_hash,
            &manifest.manifest_version,
            TABLES[index],
        );
        Ok(Self {
            read: Box::new(move |filter| {
                read_table_input(
                    bytes.clone(),
                    &path,
                    fields(index),
                    &metadata,
                    count,
                    filter,
                )
            }),
        })
    }
}
fn integer(row: &Value, name: &str) -> anyhow::Result<usize> {
    usize::try_from(
        row[name]
            .as_i64()
            .with_context(|| format!("invalid Rust Int64: {name}"))?,
    )
    .with_context(|| format!("negative/out-of-range Rust Int64: {name}"))
}
fn decode_json<T: serde::de::DeserializeOwned>(row: &Value, name: &str) -> anyhow::Result<T> {
    serde_json::from_str(string(row, name)?)
        .with_context(|| format!("invalid strict Rust JSON: {name}"))
}

fn write_table(
    path: &Path,
    rows: &[Value],
    fields: Vec<Field>,
    metadata: HashMap<String, String>,
) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new_with_metadata(fields, metadata));
    let columns = schema
        .fields()
        .iter()
        .map(|field| -> anyhow::Result<ArrayRef> {
            match field.data_type() {
                DataType::Utf8 => {
                    let values = rows
                        .iter()
                        .map(|r| {
                            let value = &r[field.name()];
                            ensure!(
                                (value.is_null() && field.is_nullable()) || value.is_string(),
                                "invalid Rust string column {}",
                                field.name()
                            );
                            Ok(value.as_str())
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    Ok(Arc::new(StringArray::from(values)))
                }
                DataType::Int64 => {
                    let values = rows
                        .iter()
                        .map(|r| {
                            let value = r[field.name()]
                                .as_u64()
                                .context("invalid nonnegative Rust integer")?;
                            Ok(i64::try_from(value).context("Rust integer exceeds Int64")?)
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    Ok(Arc::new(Int64Array::from(values)))
                }
                _ => anyhow::bail!("unsupported Rust column type"),
            }
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
        .set_max_row_group_row_count(Some(super::parquet::PARQUET_ROW_GROUP_SIZE))
        .build();
    let mut writer = ArrowWriter::try_new(
        File::create(path).with_context(|| format!("create {}", path.display()))?,
        schema,
        Some(properties),
    )?;
    writer.write(&batch)?;
    writer.close()?;
    File::open(path)?.sync_all()?;
    Ok(())
}

fn read_table(
    path: &Path,
    fields: Vec<Field>,
    metadata: &HashMap<String, String>,
    count: usize,
    frozen: Option<&FrozenRustTable>,
) -> anyhow::Result<Vec<Value>> {
    if let Some(frozen) = frozen {
        return (frozen.read)(None);
    }
    read_table_input(
        File::open(path).with_context(|| format!("open {}", path.display()))?,
        path,
        fields,
        metadata,
        count,
        None,
    )
}

fn read_table_input(
    input: impl parquet::file::reader::ChunkReader + 'static,
    path: &Path,
    fields: Vec<Field>,
    metadata: &HashMap<String, String>,
    count: usize,
    filter: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(input)
        .with_context(|| format!("invalid Rust Parquet table {}", path.display()))?;
    ensure!(
        reader.schema().fields() == Schema::new(fields).fields(),
        "Rust table column schema mismatch in {}",
        path.display()
    );
    ensure!(
        reader.schema().metadata() == metadata,
        "Rust table metadata mismatch in {}: expected {:?}, actual {:?}",
        path.display(),
        metadata,
        reader.schema().metadata()
    );
    ensure!(
        usize::try_from(reader.metadata().file_metadata().num_rows())? == count,
        "Rust table manifest row count mismatch in {}",
        path.display()
    );
    if let Some(path_filter) = filter {
        use parquet::arrow::{
            arrow_reader::{ArrowPredicateFn, RowFilter},
            ProjectionMask,
        };
        // The shared checksum authenticates decoded rows, not footer statistics.
        // Filter actual path values from the frozen bytes; unauthenticated min/max
        // values must never exclude rows from a validated whole-file projection.
        let projection = ProjectionMask::columns(reader.parquet_schema(), ["file_path"]);
        let wanted = path_filter.to_owned();
        let predicate = ArrowPredicateFn::new(projection, move |batch| {
            let paths = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| {
                    arrow_schema::ArrowError::SchemaError("invalid Rust filter column".into())
                })?;
            Ok(arrow_array::BooleanArray::from_iter(
                paths.iter().map(|p| Some(p == Some(wanted.as_str()))),
            ))
        });
        reader = reader.with_row_filter(RowFilter::new(vec![Box::new(predicate)]));
    }
    let mut rows = Vec::new();
    for batch in reader
        .build()
        .with_context(|| format!("read {}", path.display()))?
    {
        let batch = batch.with_context(|| format!("decode {}", path.display()))?;
        for index in 0..batch.num_rows() {
            let mut row = serde_json::Map::new();
            for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
                let value = if column.is_null(index) {
                    ensure!(
                        field.is_nullable(),
                        "null required Rust field {} in {}",
                        field.name(),
                        path.display()
                    );
                    Value::Null
                } else if let Some(strings) = column.as_any().downcast_ref::<StringArray>() {
                    json!(strings.value(index))
                } else if let Some(ints) = column.as_any().downcast_ref::<Int64Array>() {
                    ensure!(
                        ints.value(index) >= 0,
                        "negative Rust integer {} in {}",
                        field.name(),
                        path.display()
                    );
                    json!(ints.value(index))
                } else {
                    anyhow::bail!("wrong Rust field type in {}", path.display());
                };
                row.insert(field.name().clone(), value);
            }
            rows.push(Value::Object(row));
        }
    }
    ensure!(
        filter.is_some() || rows.len() == count,
        "decoded Rust table count mismatch in {}",
        path.display()
    );
    Ok(rows)
}
