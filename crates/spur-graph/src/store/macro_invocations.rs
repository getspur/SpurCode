//! Core physical-Rust macro evidence tables. These are independent of Lance
//! sidecars and are required, including at zero rows, when evidence is complete.
//! Nested attributes and tagged resolution payloads use JSON columns to retain
//! exact raw text, owner references and local-definition coordinates losslessly.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::Path,
    sync::Arc,
};

use anyhow::{ensure, Context as _};
use arrow_array::{Array, ArrayRef, BooleanArray, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::{
    arrow::{
        arrow_reader::{ArrowPredicateFn, ParquetRecordBatchReaderBuilder, RowFilter},
        ArrowWriter, ProjectionMask,
    },
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};
use serde_json::{Map, Value};

use super::{parquet::GraphArtifactManifest, EXTRACTOR_VERSION, SCHEMA_VERSION};
use crate::{
    GraphIndexArtifact, MacroEvidenceSourceScope, MacroInvocationEvidenceSet,
    MACRO_INVOCATION_SCHEMA_VERSION,
};

const INVOCATIONS: &str = "macro_invocations.parquet";
const SCOPES: &str = "macro_scopes.parquet";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroEvidenceManifest {
    pub complete: bool,
    pub schema_version: String,
    pub extractor_version: String,
    pub source_scope: MacroEvidenceSourceScope,
    /// Dedicated body checksum; graph_content_hash binds source files only.
    pub checksum_blake3: String,
}

fn checksum(set: &MacroInvocationEvidenceSet) -> anyhow::Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"spur-macro-evidence-tables-v1\0");
    hash.update(&serde_json::to_vec(set)?);
    Ok(hash.finalize().to_hex().to_string())
}

pub(super) fn write(
    artifact: &GraphIndexArtifact,
    dir: &Path,
) -> anyhow::Result<Option<MacroEvidenceManifest>> {
    let Some(set) = &artifact.macro_invocations else {
        return Ok(None);
    };
    set.validate_stored(artifact)?;
    let manifest = MacroEvidenceManifest {
        complete: true,
        schema_version: set.schema_version.clone(),
        extractor_version: set.extractor_version.clone(),
        source_scope: set.source_scope,
        checksum_blake3: checksum(set)?,
    };
    let metadata = metadata(
        &manifest,
        &artifact.graph_content_hash,
        &artifact.manifest_version,
        SCHEMA_VERSION,
    );
    let invocations = set
        .rows
        .iter()
        .map(|row| {
            let mut value = serde_json::to_value(row)?;
            let object = value.as_object_mut().context("invalid invocation object")?;
            let scope = object
                .remove("enclosing")
                .context("missing enclosing scope")?;
            object.insert(
                "enclosing_scope_id".into(),
                scope["stable_symbol_id"].clone(),
            );
            object.insert("enclosing_scope_kind".into(), scope["kind"].clone());
            flatten_ranges(object)?;
            for name in ["attributes", "resolution"] {
                let nested = object.remove(name).context("missing nested macro field")?;
                object.insert(name.into(), Value::String(serde_json::to_string(&nested)?));
            }
            Ok(value)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let scopes = set
        .scopes
        .iter()
        .map(|scope| {
            let mut value = serde_json::to_value(scope)?;
            let object = value.as_object_mut().context("invalid scope object")?;
            let scope = object.remove("scope").context("missing scope")?;
            object.insert("scope_id".into(), scope["stable_symbol_id"].clone());
            object.insert("kind".into(), scope["kind"].clone());
            flatten_ranges(object)?;
            Ok(value)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    write_table(
        &dir.join(INVOCATIONS),
        &invocations,
        fields(false),
        &metadata,
    )?;
    write_table(&dir.join(SCOPES), &scopes, fields(true), &metadata)?;
    Ok(Some(manifest))
}

pub(super) fn read(
    dir: &Path,
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<Option<MacroInvocationEvidenceSet>> {
    read_impl(dir, manifest, None)
}

fn read_impl(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    frozen: Option<&MacroQueryTables>,
) -> anyhow::Result<Option<MacroInvocationEvidenceSet>> {
    let Some(evidence) = &manifest.macro_evidence else {
        ensure!(
            manifest.row_counts.macro_invocations.is_none()
                && manifest.row_counts.macro_scopes.is_none(),
            "macro counts without evidence metadata"
        );
        ensure!(
            !dir.join(INVOCATIONS).exists() && !dir.join(SCOPES).exists(),
            "macro tables without evidence metadata"
        );
        return Ok(None);
    };
    ensure!(evidence.complete, "incomplete macro evidence");
    ensure!(
        (manifest.schema_version == SCHEMA_VERSION
            || (manifest.schema_version == "spur-graph-schema-v12"
                && manifest.rust_evidence.is_none()))
            && evidence.schema_version == MACRO_INVOCATION_SCHEMA_VERSION,
        "incompatible macro graph/evidence schema"
    );
    ensure!(
        manifest.extractor_version == EXTRACTOR_VERSION
            && evidence.extractor_version == manifest.extractor_version,
        "incompatible macro extractor metadata"
    );
    let expected = metadata(
        evidence,
        &manifest.graph_content_hash,
        &manifest.manifest_version,
        &manifest.schema_version,
    );
    let rows = read_table(
        &dir.join(INVOCATIONS),
        fields(false),
        &expected,
        manifest
            .row_counts
            .macro_invocations
            .context("missing invocation count")?,
        frozen.map(|t| &t.invocations),
        &[],
    )?;
    let rows = decode_invocations(rows)?;
    let scopes = read_table(
        &dir.join(SCOPES),
        fields(true),
        &expected,
        manifest
            .row_counts
            .macro_scopes
            .context("missing scope count")?,
        frozen.map(|t| &t.scopes),
        &[],
    )?;
    let scopes = decode_scopes(scopes)?;
    let set = MacroInvocationEvidenceSet {
        schema_version: evidence.schema_version.clone(),
        extractor_version: evidence.extractor_version.clone(),
        graph_content_hash: manifest.graph_content_hash.clone(),
        source_scope: evidence.source_scope,
        rows,
        scopes,
    };
    ensure!(
        checksum(&set)? == evidence.checksum_blake3,
        "macro evidence checksum mismatch"
    );
    Ok(Some(set))
}

fn metadata(
    evidence: &MacroEvidenceManifest,
    hash: &str,
    version: &str,
    graph_schema_version: &str,
) -> HashMap<String, String> {
    HashMap::from([
        ("schema_version".into(), evidence.schema_version.clone()),
        ("graph_schema_version".into(), graph_schema_version.into()),
        (
            "extractor_version".into(),
            evidence.extractor_version.clone(),
        ),
        ("graph_content_hash".into(), hash.into()),
        ("manifest_version".into(), version.into()),
        ("source_scope".into(), "physical_rust_files".into()),
        ("checksum_blake3".into(), evidence.checksum_blake3.clone()),
    ])
}

fn flatten_ranges(object: &mut Map<String, Value>) -> anyhow::Result<()> {
    for (name, start, end) in [
        ("byte_range", "byte_range_start", "byte_range_end"),
        ("line_range", "line_start", "line_end"),
    ] {
        let range = object.remove(name).context("missing macro range")?;
        object.insert(start.into(), range[0].clone());
        object.insert(end.into(), range[1].clone());
    }
    Ok(())
}

fn unflatten_ranges(object: &mut Map<String, Value>) -> anyhow::Result<()> {
    for (name, start, end) in [
        ("byte_range", "byte_range_start", "byte_range_end"),
        ("line_range", "line_start", "line_end"),
    ] {
        let start = object.remove(start).context("missing macro range start")?;
        let end = object.remove(end).context("missing macro range end")?;
        object.insert(name.into(), Value::Array(vec![start, end]));
    }
    Ok(())
}

fn fields(scopes: bool) -> Vec<Field> {
    let mut fields = vec![
        Field::new("file_path", DataType::Utf8, false),
        Field::new("source_oid", DataType::Utf8, false),
    ];
    for name in [
        "byte_range_start",
        "byte_range_end",
        "line_start",
        "line_end",
    ] {
        fields.push(Field::new(name, DataType::Int64, false));
    }
    let names: &[&str] = if scopes {
        &["scope_id", "kind"]
    } else {
        &[
            "site_id",
            "enclosing_scope_id",
            "enclosing_scope_kind",
            "raw_macro_path",
            "normalized_macro_path",
            "attributes",
            "test_context",
            "execution_phase",
            "resolution",
        ]
    };
    fields.extend(
        names
            .iter()
            .map(|name| Field::new(*name, DataType::Utf8, false)),
    );
    if !scopes {
        fields.extend([
            Field::new("caller_stable_symbol_id", DataType::Utf8, true),
            Field::new("sink_family_candidate", DataType::Utf8, true),
            Field::new("debug_assertions_candidate", DataType::Boolean, false),
        ]);
    }
    fields
}

fn write_table(
    path: &Path,
    rows: &[Value],
    fields: Vec<Field>,
    metadata: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new_with_metadata(fields, metadata.clone()));
    let columns = schema
        .fields()
        .iter()
        .map(|field| -> anyhow::Result<ArrayRef> {
            let values: Vec<_> = rows.iter().map(|r| &r[field.name()]).collect();
            Ok(match field.data_type() {
                DataType::Utf8 => Arc::new(StringArray::from(
                    values.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
                )),
                DataType::Int64 => Arc::new(Int64Array::from(
                    values
                        .iter()
                        .map(|v| v.as_i64().context("macro coordinate exceeds Int64"))
                        .collect::<anyhow::Result<Vec<_>>>()?,
                )),
                DataType::Boolean => Arc::new(BooleanArray::from(
                    values
                        .iter()
                        .map(|v| v.as_bool().context("invalid macro Boolean"))
                        .collect::<anyhow::Result<Vec<_>>>()?,
                )),
                _ => anyhow::bail!("unsupported macro column type"),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
        .set_max_row_group_row_count(Some(super::parquet::PARQUET_ROW_GROUP_SIZE))
        .build();
    let mut writer = ArrowWriter::try_new(
        File::create(path).with_context(|| format!("create {}", path.display()))?,
        schema,
        Some(props),
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
    frozen: Option<&FrozenMacroTable>,
    filters: &[(String, HashSet<String>)],
) -> anyhow::Result<Vec<Value>> {
    if let Some(frozen) = frozen {
        return (frozen.read)(filters);
    }
    read_table_input(
        File::open(path).with_context(|| format!("open {}", path.display()))?,
        path,
        fields,
        metadata,
        count,
        filters,
    )
}

fn read_table_input(
    input: impl parquet::file::reader::ChunkReader + 'static,
    path: &Path,
    fields: Vec<Field>,
    metadata: &HashMap<String, String>,
    count: usize,
    filters: &[(String, HashSet<String>)],
) -> anyhow::Result<Vec<Value>> {
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(input)?;
    ensure!(
        reader.schema().fields() == Schema::new(fields).fields(),
        "macro table column schema mismatch: {}",
        path.display()
    );
    for (key, value) in metadata {
        ensure!(
            reader.schema().metadata().get(key) == Some(value),
            "macro table metadata mismatch: {key}"
        );
    }
    ensure!(
        usize::try_from(reader.metadata().file_metadata().num_rows())? == count,
        "macro table manifest row count mismatch"
    );
    if !filters.is_empty() {
        // Skip row groups only when exact string statistics rule them out.
        let groups = (0..reader.metadata().num_row_groups())
            .filter(|&group| {
                filters.iter().all(|(name, values)| {
                    let column = reader
                        .schema()
                        .index_of(name)
                        .expect("validated macro column");
                    let stats = reader
                        .metadata()
                        .row_group(group)
                        .column(column)
                        .statistics();
                    !values.is_empty()
                        && stats.is_none_or(|s| {
                            if !s.min_is_exact() || !s.max_is_exact() {
                                return true;
                            }
                            match (s.min_bytes_opt(), s.max_bytes_opt()) {
                                (Some(min), Some(max)) => values
                                    .iter()
                                    .any(|v| min <= v.as_bytes() && v.as_bytes() <= max),
                                _ => true,
                            }
                        })
                })
            })
            .collect();
        let mut predicates: Vec<Box<dyn parquet::arrow::arrow_reader::ArrowPredicate>> = Vec::new();
        for (name, values) in filters {
            let values = values.clone();
            let projection = ProjectionMask::columns(reader.parquet_schema(), [name.as_str()]);
            predicates.push(Box::new(ArrowPredicateFn::new(projection, move |batch| {
                let strings = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        arrow_schema::ArrowError::SchemaError("invalid macro filter column".into())
                    })?;
                Ok(BooleanArray::from_iter(
                    strings
                        .iter()
                        .map(|v| Some(v.is_some_and(|v| values.contains(v)))),
                ))
            })));
        }
        reader = reader
            .with_row_groups(groups)
            .with_row_filter(RowFilter::new(predicates));
    }
    let mut rows = Vec::new();
    for batch in reader.build()? {
        let batch = batch?;
        for row in 0..batch.num_rows() {
            let mut object = Map::new();
            for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
                let value = if column.is_null(row) {
                    ensure!(field.is_nullable(), "null required macro column");
                    Value::Null
                } else if let Some(strings) = column.as_any().downcast_ref::<StringArray>() {
                    Value::from(strings.value(row))
                } else if let Some(ints) = column.as_any().downcast_ref::<Int64Array>() {
                    ensure!(ints.value(row) >= 0, "negative macro coordinate");
                    Value::from(ints.value(row))
                } else if let Some(bools) = column.as_any().downcast_ref::<BooleanArray>() {
                    Value::from(bools.value(row))
                } else {
                    anyhow::bail!("unsupported macro column type")
                };
                object.insert(field.name().clone(), value);
            }
            rows.push(Value::Object(object));
        }
    }
    ensure!(
        !filters.is_empty() || rows.len() == count,
        "decoded macro row count mismatch"
    );
    Ok(rows)
}

fn decode_invocations(values: Vec<Value>) -> anyhow::Result<Vec<crate::MacroInvocationEvidence>> {
    values
        .into_iter()
        .map(|mut value| {
            let object = value.as_object_mut().context("invalid invocation object")?;
            let id = object
                .remove("enclosing_scope_id")
                .context("missing enclosing scope ID")?;
            let kind = object
                .remove("enclosing_scope_kind")
                .context("missing enclosing scope kind")?;
            object.insert(
                "enclosing".into(),
                serde_json::json!({"stable_symbol_id":id,"kind":kind}),
            );
            unflatten_ranges(object)?;
            for name in ["attributes", "resolution"] {
                let nested = object.remove(name).context("missing nested macro field")?;
                object.insert(
                    name.into(),
                    serde_json::from_str(nested.as_str().context("invalid macro JSON column")?)?,
                );
            }
            Ok(serde_json::from_value(value)?)
        })
        .collect::<anyhow::Result<Vec<_>>>()
}

fn decode_scopes(values: Vec<Value>) -> anyhow::Result<Vec<crate::MacroScopeEvidence>> {
    values
        .into_iter()
        .map(|mut value| {
            let object = value.as_object_mut().context("invalid scope object")?;
            let id = object.remove("scope_id").context("missing scope ID")?;
            let kind = object.remove("kind").context("missing scope kind")?;
            object.insert(
                "scope".into(),
                serde_json::json!({"stable_symbol_id":id,"kind":kind}),
            );
            unflatten_ranges(object)?;
            Ok(serde_json::from_value(value)?)
        })
        .collect::<anyhow::Result<Vec<_>>>()
}

/// Immutable compressed table snapshot. Opening decodes the entire evidence set
/// once to verify the shared checksum. Queries prune row groups and decode only
/// selected invocations and their referenced scope records from these bytes.
/// Subsequent disk mutation cannot invalidate this validated context.
pub(crate) struct MacroQueryTables {
    invocations: FrozenMacroTable,
    scopes: FrozenMacroTable,
}

impl MacroQueryTables {
    pub(crate) fn open(
        dir: &Path,
        manifest: &GraphArtifactManifest,
    ) -> anyhow::Result<(Option<Self>, Option<MacroInvocationEvidenceSet>)> {
        if manifest.macro_evidence.is_none() {
            return Ok((None, read(dir, manifest)?));
        }
        let tables = Self {
            invocations: FrozenMacroTable::open(&dir.join(INVOCATIONS), false, manifest)?,
            scopes: FrozenMacroTable::open(&dir.join(SCOPES), true, manifest)?,
        };
        let set = read_impl(dir, manifest, Some(&tables))?;
        Ok((Some(tables), set))
    }

    pub(crate) fn query(
        &self,
        manifest: &GraphArtifactManifest,
        filter: &crate::MacroInvocationFilter,
    ) -> anyhow::Result<(
        Vec<crate::MacroInvocationEvidence>,
        Vec<crate::MacroScopeEvidence>,
    )> {
        let evidence = manifest
            .macro_evidence
            .as_ref()
            .context("missing macro metadata")?;
        let metadata = metadata(
            evidence,
            &manifest.graph_content_hash,
            &manifest.manifest_version,
            &manifest.schema_version,
        );
        let mut filters = Vec::new();
        for (name, value) in [
            ("file_path", &filter.file_path),
            ("caller_stable_symbol_id", &filter.caller_stable_symbol_id),
        ] {
            if let Some(value) = value {
                filters.push((name.to_owned(), HashSet::from([value.clone()])));
            }
        }
        let rows = decode_invocations(read_table(
            Path::new(INVOCATIONS),
            fields(false),
            &metadata,
            manifest
                .row_counts
                .macro_invocations
                .context("missing invocation count")?,
            Some(&self.invocations),
            &filters,
        )?)?;
        let ids = crate::macro_invocations::referenced_scope_ids(&rows);
        let scopes = if ids.is_empty() {
            Vec::new()
        } else {
            decode_scopes(read_table(
                Path::new(SCOPES),
                fields(true),
                &metadata,
                manifest
                    .row_counts
                    .macro_scopes
                    .context("missing scope count")?,
                Some(&self.scopes),
                &[("scope_id".to_owned(), ids.into_iter().collect())],
            )?)?
        };
        Ok((rows, scopes))
    }
}

// Erase the concrete byte-buffer type behind the table reader so this module
// uses Parquet's ChunkReader contract without adding a production dependency.
type MacroTableFilters = [(String, HashSet<String>)];
type FrozenTableRead = dyn Fn(&MacroTableFilters) -> anyhow::Result<Vec<Value>> + Send + Sync;
struct FrozenMacroTable {
    read: Box<FrozenTableRead>,
}
impl FrozenMacroTable {
    fn open(path: &Path, scopes: bool, manifest: &GraphArtifactManifest) -> anyhow::Result<Self> {
        use parquet::file::reader::ChunkReader as _;
        let file = File::open(path)?;
        let bytes = file.get_bytes(0, usize::try_from(file.metadata()?.len())?)?;
        let path = path.to_owned();
        let evidence = manifest
            .macro_evidence
            .as_ref()
            .context("missing macro metadata")?;
        let metadata = metadata(
            evidence,
            &manifest.graph_content_hash,
            &manifest.manifest_version,
            &manifest.schema_version,
        );
        let count = if scopes {
            manifest.row_counts.macro_scopes
        } else {
            manifest.row_counts.macro_invocations
        }
        .context("missing macro row count")?;
        Ok(Self {
            read: Box::new(move |filters| {
                read_table_input(
                    bytes.clone(),
                    &path,
                    fields(scopes),
                    &metadata,
                    count,
                    filters,
                )
            }),
        })
    }
}
