use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::ScopedJoinHandle;

use anyhow::{anyhow, bail, Context as _};
use arrow_array::{
    Array as _, BooleanArray, Float32Array, Int32Array, Int64Array, ListArray, RecordBatch,
    StringArray,
};
use arrow_schema::ArrowError;
use base64::Engine as _;
use parquet::arrow::arrow_reader::{
    ArrowPredicateFn, ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
    RowFilter, RowSelection, RowSelector,
};
use parquet::arrow::ProjectionMask;
use parquet::basic::{Compression, ZstdLevel};
use parquet::data_type::{ByteArray, ByteArrayType, FloatType, Int32Type, Int64Type};
use parquet::file::metadata::{PageIndexPolicy, ParquetMetaData};
use parquet::file::page_index::column_index::ColumnIndexMetaData;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::writer::{SerializedColumnWriter, SerializedFileWriter};
use parquet::schema::parser::parse_message_type;
use parquet::schema::types::{ColumnPath, SchemaDescriptor};

use super::{ShardIndexEntry, TemporalShardSink};
use crate::store::build::{build_worktree_delta_artifact, EXTRACTOR_VERSION, SCHEMA_VERSION};
use crate::{
    ChangeKind, CommitArtifact, Confidence, EdgeEndpoint, GitPath, GraphEdgeArtifact,
    GraphEdgeKind, GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader,
    GraphSymbolArtifact, GraphTombstoneEntry, NodeId, RelationKind, RenamePrev, SnapshotKey,
    SymbolSnapshotArtifact, TemporalEdgeArtifact,
};

pub const PARQUET_ROW_GROUP_SIZE: usize = 16_384;
const ENCLOSING_SCOPE_DICTIONARY: bool = true;
const EDGES_BY_DST_PRESENT: bool = true;

#[derive(Debug, Default)]
pub(crate) struct ParquetMetadataCache {
    metadata: Mutex<HashMap<PathBuf, ArrowReaderMetadata>>,
}

impl ParquetMetadataCache {
    pub(crate) fn get(&self, path: &Path) -> anyhow::Result<ArrowReaderMetadata> {
        let metadata = self
            .metadata
            .lock()
            .map_err(|_| anyhow!("Parquet metadata cache lock poisoned"))?;
        if let Some(cached) = metadata.get(path) {
            return Ok(cached.clone());
        }
        drop(metadata);

        let file =
            File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
        let loaded = ArrowReaderMetadata::load(
            &file,
            ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional),
        )
        .with_context(|| format!("failed to read `{}`", path.display()))?;
        let mut metadata = self
            .metadata
            .lock()
            .map_err(|_| anyhow!("Parquet metadata cache lock poisoned"))?;
        Ok(metadata.entry(path.to_path_buf()).or_insert(loaded).clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StringPruningPredicate {
    AnyValue { column: String, values: Vec<String> },
    All(Vec<Self>),
    Any(Vec<Self>),
}

impl StringPruningPredicate {
    pub(crate) fn eq(column: impl Into<String>, value: impl Into<String>) -> Self {
        Self::AnyValue {
            column: column.into(),
            values: vec![value.into()],
        }
    }

    pub(crate) fn any_value(
        column: impl Into<String>,
        values: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::AnyValue {
            column: column.into(),
            values: values.into_iter().collect(),
        }
    }

    pub(crate) fn all(predicates: impl IntoIterator<Item = Self>) -> Self {
        Self::All(predicates.into_iter().collect())
    }

    pub(crate) fn any(predicates: impl IntoIterator<Item = Self>) -> Self {
        Self::Any(predicates.into_iter().collect())
    }

    fn may_match_row_group(
        &self,
        builder: &ParquetRecordBatchReaderBuilder<File>,
        row_group_idx: usize,
    ) -> anyhow::Result<bool> {
        match self {
            Self::AnyValue { column, values } => {
                leaf_may_match_row_group(builder, row_group_idx, column, values)
            }
            Self::All(predicates) => {
                for predicate in predicates {
                    if !predicate.may_match_row_group(builder, row_group_idx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Any(predicates) => {
                for predicate in predicates {
                    if predicate.may_match_row_group(builder, row_group_idx)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    fn leaf(&self) -> Option<(&str, &[String])> {
        match self {
            Self::AnyValue { column, values } => Some((column, values)),
            Self::All(predicates) | Self::Any(predicates) if predicates.len() == 1 => {
                predicates[0].leaf()
            }
            Self::All(_) | Self::Any(_) => None,
        }
    }
}

pub(crate) fn read_projected_batches<const N: usize>(
    path: &Path,
    metadata_cache: &ParquetMetadataCache,
    columns: [&str; N],
) -> anyhow::Result<Vec<RecordBatch>> {
    let metadata = metadata_cache.get(path)?;
    let file = File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata);
    let projection = ProjectionMask::columns(builder.parquet_schema(), columns);
    builder
        .with_batch_size(PARQUET_ROW_GROUP_SIZE)
        .with_projection(projection)
        .build()
        .with_context(|| format!("failed to build Arrow reader for `{}`", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to decode `{}`", path.display()))
}

pub(crate) fn read_filtered_projected_batches<const N: usize>(
    path: &Path,
    metadata_cache: &ParquetMetadataCache,
    columns: [&str; N],
    pruning_predicate: &StringPruningPredicate,
    row_filter: impl FnOnce(&SchemaDescriptor) -> RowFilter,
) -> anyhow::Result<Vec<RecordBatch>> {
    let metadata = metadata_cache.get(path)?;
    let file = File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata);
    let projection = ProjectionMask::columns(builder.parquet_schema(), columns);
    let row_filter = row_filter(builder.parquet_schema());
    let row_groups = candidate_row_groups(&builder, pruning_predicate)?;
    let row_selection = page_row_selection(builder.metadata(), &row_groups, pruning_predicate);
    let builder = builder
        .with_batch_size(PARQUET_ROW_GROUP_SIZE)
        .with_projection(projection)
        .with_row_groups(row_groups)
        .with_row_filter(row_filter);
    let builder = match row_selection {
        Some(selection) => builder.with_row_selection(selection),
        None => builder,
    };
    builder
        .build()
        .with_context(|| format!("failed to build Arrow reader for `{}`", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to decode `{}`", path.display()))
}

fn candidate_row_groups(
    builder: &ParquetRecordBatchReaderBuilder<File>,
    predicate: &StringPruningPredicate,
) -> anyhow::Result<Vec<usize>> {
    let mut row_groups = Vec::new();
    for row_group_idx in 0..builder.metadata().num_row_groups() {
        if predicate.may_match_row_group(builder, row_group_idx)? {
            row_groups.push(row_group_idx);
        }
    }
    Ok(row_groups)
}

fn leaf_may_match_row_group(
    builder: &ParquetRecordBatchReaderBuilder<File>,
    row_group_idx: usize,
    column: &str,
    values: &[String],
) -> anyhow::Result<bool> {
    if values.is_empty() {
        return Ok(false);
    }
    let Some(column_idx) = parquet_column_index(builder.parquet_schema(), column) else {
        return Ok(true);
    };
    let column_metadata = builder
        .metadata()
        .row_group(row_group_idx)
        .column(column_idx);
    if let Some(statistics) = column_metadata.statistics() {
        if statistics.min_is_exact()
            && statistics.max_is_exact()
            && !values_may_match_bounds(
                values,
                statistics.min_bytes_opt(),
                statistics.max_bytes_opt(),
            )
        {
            return Ok(false);
        }
    }
    if let Some(bloom_filter) = builder
        .get_row_group_column_bloom_filter(row_group_idx, column_idx)
        .with_context(|| {
            format!("failed to read Bloom filter for row group {row_group_idx}, column `{column}`")
        })?
    {
        return Ok(values
            .iter()
            .any(|value| bloom_filter.check(value.as_str())));
    }
    Ok(true)
}

fn parquet_column_index(schema: &SchemaDescriptor, column: &str) -> Option<usize> {
    schema
        .columns()
        .iter()
        .position(|descriptor| descriptor.name() == column)
}

fn values_may_match_bounds(values: &[String], min: Option<&[u8]>, max: Option<&[u8]>) -> bool {
    let (Some(min), Some(max)) = (min, max) else {
        return true;
    };
    values.iter().any(|value| {
        let value = value.as_bytes();
        min <= value && value <= max
    })
}

fn page_row_selection(
    metadata: &ParquetMetaData,
    row_groups: &[usize],
    predicate: &StringPruningPredicate,
) -> Option<RowSelection> {
    if row_groups.is_empty() {
        return None;
    }
    let (column, values) = predicate.leaf()?;
    let column_idx = parquet_column_index(metadata.file_metadata().schema_descr(), column)?;
    let column_indexes = metadata.column_index()?;
    let offset_indexes = metadata.offset_index()?;
    let mut selectors = Vec::new();

    for &row_group_idx in row_groups {
        let row_count = usize::try_from(metadata.row_group(row_group_idx).num_rows()).ok()?;
        let column_index = column_indexes.get(row_group_idx)?.get(column_idx)?;
        let offset_index = offset_indexes.get(row_group_idx)?.get(column_idx)?;
        let page_locations = offset_index.page_locations();
        let byte_array_index = match column_index {
            ColumnIndexMetaData::BYTE_ARRAY(index)
            | ColumnIndexMetaData::FIXED_LEN_BYTE_ARRAY(index) => index,
            _ => {
                selectors.push(RowSelector::select(row_count));
                continue;
            }
        };
        if page_locations.is_empty()
            || usize::try_from(byte_array_index.num_pages()).ok()? != page_locations.len()
        {
            selectors.push(RowSelector::select(row_count));
            continue;
        }

        let mut row_group_selectors = Vec::with_capacity(page_locations.len());
        let mut valid = true;
        for (page_idx, location) in page_locations.iter().enumerate() {
            let Ok(start) = usize::try_from(location.first_row_index) else {
                valid = false;
                break;
            };
            let end = match page_locations.get(page_idx + 1) {
                Some(next) => match usize::try_from(next.first_row_index) {
                    Ok(end) => end,
                    Err(_) => {
                        valid = false;
                        break;
                    }
                },
                None => row_count,
            };
            if end < start || end > row_count {
                valid = false;
                break;
            }
            let page_matches = values_may_match_bounds(
                values,
                byte_array_index.min_value(page_idx),
                byte_array_index.max_value(page_idx),
            );
            row_group_selectors.push(if page_matches {
                RowSelector::select(end - start)
            } else {
                RowSelector::skip(end - start)
            });
        }
        if valid {
            selectors.extend(row_group_selectors);
        } else {
            selectors.push(RowSelector::select(row_count));
        }
    }

    Some(RowSelection::from(selectors))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    pub emit_edges_by_dst: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            emit_edges_by_dst: EDGES_BY_DST_PRESENT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalArtifactTable {
    TemporalEdges,
    SymbolSnapshots,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphArtifactManifest {
    pub graph_index_version: String,
    pub schema_version: String,
    pub manifest_version: String,
    pub graph_content_hash: String,
    pub indexed_commit_oid: Option<String>,
    pub extractor_version: String,
    pub complete: bool,
    pub row_counts: GraphArtifactRowCounts,
    #[serde(default)]
    pub macro_evidence: Option<super::macro_invocations::MacroEvidenceManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_evidence: Option<super::rust_evidence::RustEvidenceManifest>,
    #[serde(default)]
    pub sidecar_complete: bool,
    #[serde(default)]
    pub sidecar_row_counts: GraphArtifactSidecarRowCounts,
    pub parquet_writer: GraphArtifactParquetWriter,
    pub edges_by_dst_present: bool,
    #[serde(default)]
    pub temporal_shards: Vec<ShardIndexEntry>,
}

impl GraphArtifactManifest {
    /// Construct an unchecked, incomplete draft using the existing writer defaults.
    pub fn empty() -> Self {
        Self {
            graph_index_version: crate::schema::GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            schema_version: SCHEMA_VERSION.to_owned(),
            manifest_version: super::current_manifest_version(),
            graph_content_hash: crate::compute_graph_content_hash(
                std::iter::empty::<(&str, &str)>(),
            ),
            indexed_commit_oid: None,
            extractor_version: EXTRACTOR_VERSION.to_owned(),
            complete: false,
            row_counts: GraphArtifactRowCounts::empty(),
            macro_evidence: None,
            rust_evidence: None,
            sidecar_complete: false,
            sidecar_row_counts: GraphArtifactSidecarRowCounts::default(),
            parquet_writer: GraphArtifactParquetWriter {
                compression: "zstd-3".to_owned(),
                row_group_size: PARQUET_ROW_GROUP_SIZE,
            },
            edges_by_dst_present: false,
            temporal_shards: Vec::new(),
        }
    }

    /// Return an owned view of the current metadata without validating it.
    pub fn rust_evidence_metadata(
        &self,
    ) -> anyhow::Result<Option<super::rust_evidence::RustEvidenceManifest>> {
        Ok(self.rust_evidence.clone())
    }

    /// Replace the existing optional metadata without introducing validation.
    pub fn set_rust_evidence_metadata(
        &mut self,
        value: Option<super::rust_evidence::RustEvidenceManifest>,
    ) -> anyhow::Result<()> {
        self.rust_evidence = value;
        Ok(())
    }

    /// Return an owned view of the current metadata without validating it.
    pub fn macro_evidence_metadata(
        &self,
    ) -> anyhow::Result<Option<super::macro_invocations::MacroEvidenceManifest>> {
        Ok(self.macro_evidence.clone())
    }

    /// Replace the existing optional metadata without introducing validation.
    pub fn set_macro_evidence_metadata(
        &mut self,
        value: Option<super::macro_invocations::MacroEvidenceManifest>,
    ) -> anyhow::Result<()> {
        self.macro_evidence = value;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphArtifactRowCounts {
    pub nodes: usize,
    pub edges: usize,
    pub edges_by_dst: Option<usize>,
    pub edges_unresolved: usize,
    pub files: usize,
    pub file_manifests: usize,
    pub tombstones: usize,
    #[serde(default)]
    pub macro_invocations: Option<usize>,
    #[serde(default)]
    pub macro_scopes: Option<usize>,
    #[serde(default)]
    pub commits: usize,
    #[serde(default)]
    pub symbol_snapshots: usize,
    #[serde(default)]
    pub temporal_edges: usize,
    #[serde(default)]
    pub diagnostics: usize,
}

impl GraphArtifactRowCounts {
    pub fn empty() -> Self {
        Self {
            nodes: 0,
            edges: 0,
            edges_by_dst: None,
            edges_unresolved: 0,
            files: 0,
            file_manifests: 0,
            tombstones: 0,
            macro_invocations: None,
            macro_scopes: None,
            commits: 0,
            symbol_snapshots: 0,
            temporal_edges: 0,
            diagnostics: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphArtifactSidecarRowCounts {
    #[serde(default)]
    pub section_bodies: usize,
    #[serde(default)]
    pub code_symbols: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphArtifactSidecarStatus {
    pub complete: bool,
    pub row_counts: GraphArtifactSidecarRowCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphArtifactParquetWriter {
    pub compression: String,
    pub row_group_size: usize,
}

#[derive(Debug, Clone)]
struct NodeRow {
    node_id: NodeId,
    symbol: GraphSymbolArtifact,
}

#[derive(Debug, Clone)]
struct FileRow {
    node_id: NodeId,
    file: GraphFileArtifact,
}

#[derive(Debug, Clone)]
struct ResolvedEdgeRow {
    edge: GraphEdgeArtifact,
    src_id: NodeId,
    dst_id: NodeId,
}

#[derive(Debug, Clone)]
struct UnresolvedEdgeRow {
    edge: GraphEdgeArtifact,
    src_id: NodeId,
}

#[derive(Debug, Clone)]
struct CommitRow {
    sha: String,
    parents: Vec<String>,
    author_time: i64,
    author_name: String,
    author_email: String,
    summary: String,
}

#[derive(Debug, Clone)]
struct SymbolSnapshotRow {
    key_stable_symbol_id: String,
    key_commit: String,
    file_path_b64: String,
    entity_name: String,
    symbol_kind: String,
    enclosing_scope: Option<String>,
    byte_range_start: i64,
    byte_range_end: i64,
    line_range_start: i64,
    line_range_end: i64,
    anchor_hash: String,
    tokens: Vec<String>,
}

#[derive(Debug, Clone)]
struct TemporalEdgeRow {
    source_kind: String,
    source_path_b64: Option<String>,
    source_stable_symbol_id: Option<String>,
    source_commit: Option<String>,
    target_kind: String,
    target_path_b64: Option<String>,
    target_stable_symbol_id: Option<String>,
    target_commit: Option<String>,
    relation: String,
    parent: Option<String>,
    change_kind: Option<String>,
    rename_prev_path_b64: Option<String>,
    rename_prev_stable_symbol_id: Option<String>,
    rename_prev_commit: Option<String>,
}

const SYMBOL_SNAPSHOT_COLUMNS: [&str; 12] = [
    "key_stable_symbol_id",
    "key_commit",
    "file_path_b64",
    "entity_name",
    "symbol_kind",
    "enclosing_scope",
    "byte_range_start",
    "byte_range_end",
    "line_range_start",
    "line_range_end",
    "anchor_hash",
    "tokens",
];

const TEMPORAL_EDGE_COLUMNS: [&str; 14] = [
    "source_kind",
    "source_path_b64",
    "source_stable_symbol_id",
    "source_commit",
    "target_kind",
    "target_path_b64",
    "target_stable_symbol_id",
    "target_commit",
    "relation",
    "parent",
    "change_kind",
    "rename_prev_path_b64",
    "rename_prev_stable_symbol_id",
    "rename_prev_commit",
];

const TEMPORAL_EDGE_SYMBOL_ID_COLUMNS: [&str; 3] = [
    "source_stable_symbol_id",
    "target_stable_symbol_id",
    "rename_prev_stable_symbol_id",
];

enum ColumnData {
    RequiredString(Vec<String>),
    OptionalString(Vec<Option<String>>),
    RequiredI64(Vec<i64>),
    RequiredI32(Vec<i32>),
    RequiredF32(Vec<f32>),
    RequiredListI64(Vec<Vec<i64>>),
    RequiredListString(Vec<Vec<String>>),
}

impl ColumnData {
    fn len(&self) -> usize {
        match self {
            Self::RequiredString(values) => values.len(),
            Self::OptionalString(values) => values.len(),
            Self::RequiredI64(values) => values.len(),
            Self::RequiredI32(values) => values.len(),
            Self::RequiredF32(values) => values.len(),
            Self::RequiredListI64(values) => values.len(),
            Self::RequiredListString(values) => values.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaSummary {
    pub changed_files: usize,
    pub new_symbols: usize,
    pub tombstoned_symbols: usize,
}

pub fn write_worktree_delta(
    prev: &GraphIndexArtifact,
    worktree_root: &Path,
    out_dir: &Path,
) -> anyhow::Result<DeltaSummary> {
    let delta = build_worktree_delta_artifact(prev, worktree_root)?;
    let artifact = &delta.artifact;
    artifact.validate_legacy_evidence_export()?;

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create `{}`", out_dir.display()))?;

    let mut files = file_rows(artifact)?;
    let mut nodes = node_rows(artifact)?;
    files.sort_by(|a, b| a.file.file_path.cmp(&b.file.file_path));
    nodes.sort_by(|a, b| {
        a.symbol
            .file_path
            .cmp(&b.symbol.file_path)
            .then(a.symbol.stable_symbol_id.cmp(&b.symbol.stable_symbol_id))
    });

    let endpoint_ids = endpoint_id_map(&files, &nodes)?;
    let (mut resolved_edges, mut unresolved_edges) = edge_rows(artifact, &endpoint_ids)?;
    resolved_edges.sort_by(|a, b| {
        a.src_id
            .get()
            .cmp(&b.src_id.get())
            .then(a.dst_id.get().cmp(&b.dst_id.get()))
    });
    unresolved_edges.sort_by_key(|edge| edge.src_id.get());

    let mut file_manifests = artifact.file_manifests.clone();
    file_manifests.sort_by(|a, b| a.path.cmp(&b.path));
    let mut tombstones = artifact.tombstones.clone();
    tombstones.sort_by(|a, b| a.path.cmp(&b.path));

    let nodes_path = out_dir.join("nodes.parquet");
    let edges_path = out_dir.join("edges.parquet");
    let unresolved_edges_path = out_dir.join("edges_unresolved.parquet");
    let files_path = out_dir.join("files.parquet");
    let file_manifests_path = out_dir.join("file_manifests.parquet");
    let tombstones_path = out_dir.join("tombstones.parquet");

    std::thread::scope(|scope| {
        let nodes_handle = scope.spawn(|| write_nodes(&nodes_path, &nodes));
        let edges_handle = scope.spawn(|| write_edges(&edges_path, &resolved_edges));
        let unresolved_edges_handle =
            scope.spawn(|| write_unresolved_edges(&unresolved_edges_path, &unresolved_edges));
        let files_handle = scope.spawn(|| write_files(&files_path, &files));
        let file_manifests_handle =
            scope.spawn(|| write_file_manifests(&file_manifests_path, &file_manifests));
        let tombstones_handle = scope.spawn(|| write_tombstones(&tombstones_path, &tombstones));

        join_scoped(nodes_handle, "write delta nodes.parquet")?;
        join_scoped(edges_handle, "write delta edges.parquet")?;
        join_scoped(
            unresolved_edges_handle,
            "write delta edges_unresolved.parquet",
        )?;
        join_scoped(files_handle, "write delta files.parquet")?;
        join_scoped(file_manifests_handle, "write delta file_manifests.parquet")?;
        join_scoped(tombstones_handle, "write delta tombstones.parquet")?;
        Ok::<_, anyhow::Error>(())
    })?;

    let rust_evidence = super::rust_evidence::write(artifact, out_dir)?;
    let macro_evidence = super::macro_invocations::write(artifact, out_dir)?;
    let manifest = GraphArtifactManifest {
        graph_index_version: artifact.header.graph_index_version.clone(),
        schema_version: SCHEMA_VERSION.to_owned(),
        manifest_version: artifact.manifest_version.clone(),
        graph_content_hash: artifact.graph_content_hash.clone(),
        indexed_commit_oid: None,
        extractor_version: EXTRACTOR_VERSION.to_owned(),
        complete: true,
        macro_evidence,
        rust_evidence,
        row_counts: GraphArtifactRowCounts {
            nodes: nodes.len(),
            edges: resolved_edges.len(),
            edges_by_dst: None,
            edges_unresolved: unresolved_edges.len(),
            files: files.len(),
            file_manifests: file_manifests.len(),
            tombstones: tombstones.len(),
            macro_invocations: artifact
                .macro_invocations
                .as_ref()
                .map(|set| set.rows.len()),
            macro_scopes: artifact
                .macro_invocations
                .as_ref()
                .map(|set| set.scopes.len()),
            commits: 0,
            symbol_snapshots: 0,
            temporal_edges: 0,
            diagnostics: 0,
        },
        sidecar_complete: false,
        sidecar_row_counts: GraphArtifactSidecarRowCounts::default(),
        parquet_writer: GraphArtifactParquetWriter {
            compression: "zstd-3".to_owned(),
            row_group_size: PARQUET_ROW_GROUP_SIZE,
        },
        edges_by_dst_present: false,
        temporal_shards: Vec::new(),
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("failed to encode Parquet manifest")?;
    write_manifest(out_dir, &manifest_json)?;
    fsync_dir(out_dir)?;

    Ok(DeltaSummary {
        changed_files: delta.changed_files,
        new_symbols: delta.new_symbols,
        tombstoned_symbols: delta.tombstoned_symbols,
    })
}

pub fn write_artifact_parquet(
    artifact: &GraphIndexArtifact,
    out_dir: &Path,
    options: WriteOptions,
    temporal_shards: Vec<ShardIndexEntry>,
) -> anyhow::Result<PathBuf> {
    artifact.validate_legacy_evidence_export()?;
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create `{}`", out_dir.display()))?;

    let mut files = file_rows(artifact)?;
    let mut nodes = node_rows(artifact)?;
    files.sort_by(|a, b| a.file.file_path.cmp(&b.file.file_path));
    nodes.sort_by(|a, b| {
        a.symbol
            .file_path
            .cmp(&b.symbol.file_path)
            .then(a.symbol.stable_symbol_id.cmp(&b.symbol.stable_symbol_id))
    });

    let endpoint_ids = endpoint_id_map(&files, &nodes)?;
    let (mut resolved_edges, mut unresolved_edges) = edge_rows(artifact, &endpoint_ids)?;
    resolved_edges.sort_by(|a, b| {
        a.src_id
            .get()
            .cmp(&b.src_id.get())
            .then(a.dst_id.get().cmp(&b.dst_id.get()))
    });
    unresolved_edges.sort_by_key(|edge| edge.src_id.get());

    let mut file_manifests = artifact.file_manifests.clone();
    file_manifests.sort_by(|a, b| a.path.cmp(&b.path));
    let mut tombstones = artifact.tombstones.clone();
    tombstones.sort_by(|a, b| a.path.cmp(&b.path));

    let mut edges_by_dst = None;
    if options.emit_edges_by_dst {
        let mut by_dst = resolved_edges.clone();
        by_dst.sort_by(|a, b| {
            a.dst_id
                .get()
                .cmp(&b.dst_id.get())
                .then(a.src_id.get().cmp(&b.src_id.get()))
        });
        edges_by_dst = Some(by_dst);
    }

    let fallback_temporal_shards =
        !artifact.symbol_snapshots.is_empty() || !artifact.temporal_edges.is_empty();
    let temporal_shards = if fallback_temporal_shards {
        synthesize_single_temporal_shard(artifact)
    } else {
        temporal_shards
    };

    let nodes_path = out_dir.join("nodes.parquet");
    let edges_path = out_dir.join("edges.parquet");
    let edges_by_dst_path = out_dir.join("edges_by_dst.parquet");
    let unresolved_edges_path = out_dir.join("edges_unresolved.parquet");
    let files_path = out_dir.join("files.parquet");
    let file_manifests_path = out_dir.join("file_manifests.parquet");
    let tombstones_path = out_dir.join("tombstones.parquet");
    let commits_path = out_dir.join("commits.parquet");
    let symbol_snapshots_dir = out_dir.join("symbol_snapshots");
    let temporal_edges_dir = out_dir.join("temporal_edges");
    let symbol_snapshots_path = symbol_snapshots_dir.join("00000.parquet");
    let temporal_edges_path = temporal_edges_dir.join("00000.parquet");
    let diagnostics_path = out_dir.join("diagnostics.parquet");

    std::thread::scope(|scope| {
        let nodes_handle = scope.spawn(|| write_nodes(&nodes_path, &nodes));
        let edges_handle = scope.spawn(|| write_edges(&edges_path, &resolved_edges));
        let unresolved_edges_handle =
            scope.spawn(|| write_unresolved_edges(&unresolved_edges_path, &unresolved_edges));
        let files_handle = scope.spawn(|| write_files(&files_path, &files));
        let file_manifests_handle =
            scope.spawn(|| write_file_manifests(&file_manifests_path, &file_manifests));
        let tombstones_handle = scope.spawn(|| write_tombstones(&tombstones_path, &tombstones));
        let commits_handle = (!artifact.commits.is_empty())
            .then(|| scope.spawn(|| write_commits(&commits_path, &artifact.commits, &options)));
        if fallback_temporal_shards && !artifact.symbol_snapshots.is_empty() {
            fs::create_dir_all(&symbol_snapshots_dir).with_context(|| {
                format!("failed to create `{}`", symbol_snapshots_dir.display())
            })?;
        }
        if fallback_temporal_shards && !artifact.temporal_edges.is_empty() {
            fs::create_dir_all(&temporal_edges_dir)
                .with_context(|| format!("failed to create `{}`", temporal_edges_dir.display()))?;
        }
        let symbol_snapshots_handle = (fallback_temporal_shards
            && !artifact.symbol_snapshots.is_empty())
        .then(|| {
            scope.spawn(|| {
                write_symbol_snapshots(&symbol_snapshots_path, &artifact.symbol_snapshots, &options)
            })
        });
        let temporal_edges_handle =
            (fallback_temporal_shards && !artifact.temporal_edges.is_empty()).then(|| {
                scope.spawn(|| {
                    write_temporal_edges(&temporal_edges_path, &artifact.temporal_edges, &options)
                })
            });
        let diagnostics_handle = (!artifact.diagnostics.is_empty())
            .then(|| scope.spawn(|| write_diagnostics(&diagnostics_path, &artifact.diagnostics)));
        let edges_by_dst_handle = edges_by_dst
            .as_ref()
            .map(|rows| scope.spawn(|| write_edges(&edges_by_dst_path, rows)));

        join_scoped(nodes_handle, "write nodes.parquet")?;
        join_scoped(edges_handle, "write edges.parquet")?;
        join_scoped(unresolved_edges_handle, "write edges_unresolved.parquet")?;
        join_scoped(files_handle, "write files.parquet")?;
        join_scoped(file_manifests_handle, "write file_manifests.parquet")?;
        join_scoped(tombstones_handle, "write tombstones.parquet")?;
        if let Some(handle) = commits_handle {
            join_scoped(handle, "write commits.parquet")?;
        }
        if let Some(handle) = symbol_snapshots_handle {
            join_scoped(handle, "write symbol_snapshots.parquet")?;
        }
        if let Some(handle) = temporal_edges_handle {
            join_scoped(handle, "write temporal_edges.parquet")?;
        }
        if let Some(handle) = diagnostics_handle {
            join_scoped(handle, "write diagnostics.parquet")?;
        }
        if let Some(handle) = edges_by_dst_handle {
            join_scoped(handle, "write edges_by_dst.parquet")?;
        }
        Ok::<_, anyhow::Error>(())
    })?;

    let rust_evidence = super::rust_evidence::write(artifact, out_dir)?;
    let macro_evidence = super::macro_invocations::write(artifact, out_dir)?;
    let manifest = GraphArtifactManifest {
        graph_index_version: artifact.header.graph_index_version.clone(),
        schema_version: SCHEMA_VERSION.to_owned(),
        manifest_version: artifact.manifest_version.clone(),
        graph_content_hash: artifact.graph_content_hash.clone(),
        indexed_commit_oid: None,
        extractor_version: EXTRACTOR_VERSION.to_owned(),
        complete: true,
        macro_evidence,
        rust_evidence,
        row_counts: GraphArtifactRowCounts {
            nodes: nodes.len(),
            edges: resolved_edges.len(),
            edges_by_dst: options.emit_edges_by_dst.then_some(resolved_edges.len()),
            edges_unresolved: unresolved_edges.len(),
            files: files.len(),
            file_manifests: file_manifests.len(),
            tombstones: tombstones.len(),
            macro_invocations: artifact
                .macro_invocations
                .as_ref()
                .map(|set| set.rows.len()),
            macro_scopes: artifact
                .macro_invocations
                .as_ref()
                .map(|set| set.scopes.len()),
            commits: artifact.commits.len(),
            symbol_snapshots: temporal_shards
                .iter()
                .map(|entry| entry.row_count_snapshots)
                .sum(),
            temporal_edges: temporal_shards
                .iter()
                .map(|entry| entry.row_count_edges)
                .sum(),
            diagnostics: artifact.diagnostics.len(),
        },
        sidecar_complete: false,
        sidecar_row_counts: GraphArtifactSidecarRowCounts::default(),
        parquet_writer: GraphArtifactParquetWriter {
            compression: "zstd-3".to_owned(),
            row_group_size: PARQUET_ROW_GROUP_SIZE,
        },
        edges_by_dst_present: options.emit_edges_by_dst,
        temporal_shards,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("failed to encode Parquet manifest")?;
    write_manifest(out_dir, &manifest_json)?;
    fsync_dir(out_dir)?;

    Ok(out_dir.to_path_buf())
}

pub fn read_artifact_header_parquet(dir: &Path) -> anyhow::Result<GraphArtifactManifest> {
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read `{}`", manifest_path.display()))?;
    let manifest: GraphArtifactManifest = serde_json::from_str(&content)
        .with_context(|| format!("invalid Parquet manifest `{}`", manifest_path.display()))?;
    Ok(manifest)
}

pub fn stamp_sidecar_status(dir: &Path, status: GraphArtifactSidecarStatus) -> anyhow::Result<()> {
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read `{}`", manifest_path.display()))?;
    let mut manifest: GraphArtifactManifest = serde_json::from_str(&content)
        .with_context(|| format!("invalid Parquet manifest `{}`", manifest_path.display()))?;
    manifest.sidecar_complete = status.complete;
    manifest.sidecar_row_counts = status.row_counts;

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .context("failed to encode Parquet manifest with sidecar status")?;
    write_manifest(dir, &manifest_json)?;
    fsync_dir(dir)?;
    Ok(())
}

fn synthesize_single_temporal_shard(artifact: &GraphIndexArtifact) -> Vec<ShardIndexEntry> {
    if artifact.symbol_snapshots.is_empty() && artifact.temporal_edges.is_empty() {
        return Vec::new();
    }

    let (commit_time_min, commit_time_max) = artifact
        .commits
        .iter()
        .map(|commit| commit.author_time)
        .fold(None, |range: Option<(i64, i64)>, author_time| {
            Some(match range {
                Some((min, max)) => (min.min(author_time), max.max(author_time)),
                None => (author_time, author_time),
            })
        })
        .unwrap_or((0, 0));

    vec![ShardIndexEntry {
        shard_idx: 0,
        commit_time_min,
        commit_time_max,
        row_count_edges: artifact.temporal_edges.len(),
        row_count_snapshots: artifact.symbol_snapshots.len(),
    }]
}

pub fn read_artifact_parquet(dir: &Path) -> anyhow::Result<GraphIndexArtifact> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let legacy_snapshot_ids = manifest.schema_version == "spur-graph-schema-v5"
        || manifest.graph_index_version != crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
    let nodes_path = dir.join("nodes.parquet");
    let edges_path = dir.join("edges.parquet");
    let unresolved_edges_path = dir.join("edges_unresolved.parquet");
    let files_path = dir.join("files.parquet");
    let file_manifests_path = dir.join("file_manifests.parquet");
    let tombstones_path = dir.join("tombstones.parquet");
    let commits_path = dir.join("commits.parquet");
    let diagnostics_path = dir.join("diagnostics.parquet");
    let row_counts = manifest.row_counts.clone();

    let (files, file_node_ids) = read_files(&files_path, row_counts.files)?;
    let file_manifests = read_file_manifests(&file_manifests_path, row_counts.file_manifests)?;
    let tombstones = read_tombstones(&tombstones_path, row_counts.tombstones)?;

    let (
        (symbols, symbol_node_ids),
        mut edges,
        unresolved_edges,
        commits,
        symbol_snapshots,
        temporal_edges,
        diagnostics,
    ) = std::thread::scope(|scope| {
        let nodes = scope.spawn(|| read_nodes(&nodes_path, row_counts.nodes));
        let edges = scope.spawn(|| read_edges(&edges_path, row_counts.edges));
        let unresolved_edges = scope
            .spawn(|| read_unresolved_edges(&unresolved_edges_path, row_counts.edges_unresolved));
        let commits = scope.spawn(|| read_commits(&commits_path, row_counts.commits));
        let symbol_snapshots = scope.spawn(|| read_symbol_snapshots(dir, &manifest));
        let temporal_edges = scope.spawn(|| read_temporal_edges(dir, &manifest));
        let diagnostics =
            scope.spawn(|| read_diagnostics(&diagnostics_path, row_counts.diagnostics));

        Ok::<_, anyhow::Error>((
            join_scoped(nodes, "read nodes.parquet")?,
            join_scoped(edges, "read edges.parquet")?,
            join_scoped(unresolved_edges, "read edges_unresolved.parquet")?,
            join_scoped(commits, "read commits.parquet")?,
            join_scoped(symbol_snapshots, "read symbol_snapshots.parquet")?,
            join_scoped(temporal_edges, "read temporal_edges.parquet")?,
            join_scoped(diagnostics, "read diagnostics.parquet")?,
        ))
    })?;

    edges.extend(unresolved_edges);

    let rust_evidence = super::rust_evidence::read(dir, &manifest)?;
    let macro_invocations = super::macro_invocations::read(dir, &manifest)?;

    let mut artifact = GraphIndexArtifact {
        rust_evidence,
        macro_invocations,
        header: GraphIndexHeader {
            graph_index_version: manifest.graph_index_version,
            content_hash_blake3: None,
        },
        manifest_version: manifest.manifest_version,
        graph_content_hash: manifest.graph_content_hash,
        file_manifests,
        files,
        file_node_ids,
        symbols,
        symbol_node_ids,
        edges,
        tombstones,
        diagnostics,
        commits,
        symbol_snapshots,
        temporal_edges,
    };
    if legacy_snapshot_ids {
        crate::schema::rehash_legacy_snapshot_ids(&mut artifact);
    }
    if let Some(set) = &artifact.macro_invocations {
        set.validate_stored(&artifact)?;
    }
    if let Some(set) = &artifact.rust_evidence {
        set.validate_stored(&artifact)?;
    }
    Ok(artifact)
}

pub fn read_artifact_parquet_slim(dir: &Path) -> anyhow::Result<GraphIndexArtifact> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let legacy_snapshot_ids = manifest.schema_version == "spur-graph-schema-v5"
        || manifest.graph_index_version != crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
    let nodes_path = dir.join("nodes.parquet");
    let files_path = dir.join("files.parquet");
    let tombstones_path = dir.join("tombstones.parquet");
    let row_counts = manifest.row_counts.clone();

    let ((symbols, symbol_node_ids), (files, file_node_ids), tombstones) =
        std::thread::scope(|scope| {
            let nodes = scope.spawn(|| read_nodes(&nodes_path, row_counts.nodes));
            let files = scope.spawn(|| read_files(&files_path, row_counts.files));
            let tombstones =
                scope.spawn(|| read_tombstones(&tombstones_path, row_counts.tombstones));

            Ok::<_, anyhow::Error>((
                join_scoped(nodes, "read nodes.parquet")?,
                join_scoped(files, "read files.parquet")?,
                join_scoped(tombstones, "read tombstones.parquet")?,
            ))
        })?;

    let rust_evidence = super::rust_evidence::read(dir, &manifest)?;
    let macro_invocations = super::macro_invocations::read(dir, &manifest)?;
    let file_manifests = if macro_invocations.is_some() || rust_evidence.is_some() {
        read_file_manifests(
            &dir.join("file_manifests.parquet"),
            row_counts.file_manifests,
        )?
    } else {
        Vec::new()
    };
    // Stored declaration ownership uses Contains edges. Retain this projection
    // through validation, then clear it with the general slim manifests.
    let edges = if rust_evidence.is_some() {
        read_edges(&dir.join("edges.parquet"), row_counts.edges)?
    } else {
        Vec::new()
    };
    let mut artifact = GraphIndexArtifact {
        rust_evidence,
        macro_invocations,
        header: GraphIndexHeader {
            graph_index_version: manifest.graph_index_version,
            content_hash_blake3: None,
        },
        manifest_version: manifest.manifest_version,
        graph_content_hash: manifest.graph_content_hash,
        file_manifests,
        files,
        file_node_ids,
        symbols,
        symbol_node_ids,
        edges,
        tombstones,
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    };
    if legacy_snapshot_ids {
        crate::schema::rehash_legacy_snapshot_ids(&mut artifact);
    }
    if let Some(set) = &artifact.macro_invocations {
        set.validate_stored(&artifact)?;
    }
    if let Some(set) = &artifact.rust_evidence {
        set.validate_stored(&artifact)?;
    }
    artifact.file_manifests.clear();
    artifact.edges.clear();
    Ok(artifact)
}

/// Read only current resolved and unresolved edges for the adjacency index.
pub(crate) fn read_current_query_edges_parquet(
    dir: &Path,
) -> anyhow::Result<Vec<GraphEdgeArtifact>> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let edges_path = dir.join("edges.parquet");
    let unresolved_edges_path = dir.join("edges_unresolved.parquet");
    let row_counts = manifest.row_counts;

    let (mut edges, unresolved_edges) = std::thread::scope(|scope| {
        let edges = scope.spawn(|| read_edges(&edges_path, row_counts.edges));
        let unresolved_edges = scope
            .spawn(|| read_unresolved_edges(&unresolved_edges_path, row_counts.edges_unresolved));

        Ok::<_, anyhow::Error>((
            join_scoped(edges, "read hot-query edges.parquet")?,
            join_scoped(unresolved_edges, "read hot-query edges_unresolved.parquet")?,
        ))
    })?;
    edges.extend(unresolved_edges);
    Ok(edges)
}

pub fn load_temporal_artifact_parquet(dir: &Path) -> anyhow::Result<GraphIndexArtifact> {
    read_temporal_artifact_parquet(dir)
}

pub(crate) fn load_temporal_artifact_metadata_parquet(
    dir: &Path,
) -> anyhow::Result<GraphIndexArtifact> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let legacy_snapshot_ids = manifest.schema_version == "spur-graph-schema-v5"
        || manifest.graph_index_version != crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
    let commits_path = dir.join("commits.parquet");
    let diagnostics_path = dir.join("diagnostics.parquet");
    let row_counts = manifest.row_counts.clone();

    let (commits, diagnostics) = std::thread::scope(|scope| {
        let commits = scope.spawn(|| read_commits(&commits_path, row_counts.commits));
        let diagnostics =
            scope.spawn(|| read_diagnostics(&diagnostics_path, row_counts.diagnostics));

        Ok::<_, anyhow::Error>((
            join_scoped(commits, "read commits.parquet")?,
            join_scoped(diagnostics, "read diagnostics.parquet")?,
        ))
    })?;

    let mut artifact = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: manifest.graph_index_version,
            content_hash_blake3: None,
        },
        manifest_version: manifest.manifest_version,
        graph_content_hash: manifest.graph_content_hash,
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics,
        commits,
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    };
    if legacy_snapshot_ids {
        crate::schema::rehash_legacy_snapshot_ids(&mut artifact);
    }
    Ok(artifact)
}

pub(crate) fn stream_temporal_artifact_parquet_into_sink(
    dir: &Path,
    sink: &mut TemporalShardSink,
) -> anyhow::Result<()> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    ensure_temporal_shards_layout(dir, &manifest)?;

    let commits = read_commits(&dir.join("commits.parquet"), manifest.row_counts.commits)?;
    let commits_by_sha: HashMap<_, _> = commits
        .iter()
        .map(|commit| (commit.sha.as_str(), commit))
        .collect();

    for shard in &manifest.temporal_shards {
        let mut rows_by_commit: BTreeMap<String, TemporalRowsForCommit> = BTreeMap::new();

        let temporal_edges_path = temporal_shard_path(dir, "temporal_edges", shard.shard_idx);
        for edge in read_temporal_edges_file(&temporal_edges_path, shard.row_count_edges)? {
            let commit_sha = temporal_edge_commit(&edge)
                .with_context(|| {
                    format!(
                        "temporal edge in shard {} has no commit endpoint",
                        shard.shard_idx
                    )
                })?
                .to_owned();
            rows_by_commit
                .entry(commit_sha)
                .or_default()
                .edges
                .push(edge);
        }

        let symbol_snapshots_path = temporal_shard_path(dir, "symbol_snapshots", shard.shard_idx);
        for snapshot in
            read_symbol_snapshots_file(&symbol_snapshots_path, shard.row_count_snapshots)?
        {
            rows_by_commit
                .entry(snapshot.key.commit.clone())
                .or_default()
                .snapshots
                .push(snapshot);
        }

        for (commit_sha, mut rows) in rows_by_commit {
            let commit = commits_by_sha.get(commit_sha.as_str()).with_context(|| {
                format!("temporal shard row references unknown commit `{commit_sha}`")
            })?;
            sink.append_commit(commit, &mut rows.edges, &mut rows.snapshots)?;
        }
    }

    Ok(())
}

#[derive(Default)]
struct TemporalRowsForCommit {
    edges: Vec<TemporalEdgeArtifact>,
    snapshots: Vec<SymbolSnapshotArtifact>,
}

fn temporal_edge_commit(edge: &TemporalEdgeArtifact) -> Option<&str> {
    match &edge.target {
        EdgeEndpoint::Snapshot { key } => Some(&key.commit),
        EdgeEndpoint::Commit { sha } => Some(sha),
        EdgeEndpoint::File { .. } | EdgeEndpoint::Symbol { .. } => match &edge.source {
            EdgeEndpoint::Snapshot { key } => Some(&key.commit),
            EdgeEndpoint::Commit { sha } => Some(sha),
            EdgeEndpoint::File { .. } | EdgeEndpoint::Symbol { .. } => None,
        },
    }
}

pub(crate) fn read_temporal_artifact_parquet(dir: &Path) -> anyhow::Result<GraphIndexArtifact> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let legacy_snapshot_ids = manifest.schema_version == "spur-graph-schema-v5"
        || manifest.graph_index_version != crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
    let commits_path = dir.join("commits.parquet");
    let diagnostics_path = dir.join("diagnostics.parquet");
    let row_counts = manifest.row_counts.clone();

    let (commits, symbol_snapshots, temporal_edges, diagnostics) = std::thread::scope(|scope| {
        let commits = scope.spawn(|| read_commits(&commits_path, row_counts.commits));
        let symbol_snapshots = scope.spawn(|| read_symbol_snapshots(dir, &manifest));
        let temporal_edges = scope.spawn(|| read_temporal_edges(dir, &manifest));
        let diagnostics =
            scope.spawn(|| read_diagnostics(&diagnostics_path, row_counts.diagnostics));

        Ok::<_, anyhow::Error>((
            join_scoped(commits, "read commits.parquet")?,
            join_scoped(symbol_snapshots, "read symbol_snapshots.parquet")?,
            join_scoped(temporal_edges, "read temporal_edges.parquet")?,
            join_scoped(diagnostics, "read diagnostics.parquet")?,
        ))
    })?;

    let mut artifact = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: manifest.graph_index_version,
            content_hash_blake3: None,
        },
        manifest_version: manifest.manifest_version,
        graph_content_hash: manifest.graph_content_hash,
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics,
        commits,
        symbol_snapshots,
        temporal_edges,
    };
    if legacy_snapshot_ids {
        crate::schema::rehash_legacy_snapshot_ids(&mut artifact);
    }
    Ok(artifact)
}

pub fn read_temporal_artifact_parquet_for_symbol_history(
    dir: &Path,
    symbol_id: &str,
) -> anyhow::Result<GraphIndexArtifact> {
    read_temporal_artifact_parquet_for_symbol_history_with_cache(
        dir,
        symbol_id,
        &ParquetMetadataCache::default(),
    )
}

pub(crate) fn read_temporal_artifact_parquet_for_symbol_history_with_cache(
    dir: &Path,
    symbol_id: &str,
    metadata_cache: &ParquetMetadataCache,
) -> anyhow::Result<GraphIndexArtifact> {
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    let legacy_snapshot_ids = manifest.schema_version == "spur-graph-schema-v5"
        || manifest.graph_index_version != crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;

    let mut chain_ids = HashSet::from([symbol_id.to_owned()]);
    let mut frontier = chain_ids.clone();
    while !frontier.is_empty() {
        let edges =
            read_temporal_edges_for_symbols_with_cache(dir, &manifest, &frontier, metadata_cache)?;
        let mut next_frontier = HashSet::new();
        for edge in edges {
            let Some(rename_ids) = rename_symbol_ids(&edge) else {
                continue;
            };
            if !rename_ids.iter().any(|id| frontier.contains(*id)) {
                continue;
            }
            for id in rename_ids {
                let id = id.to_owned();
                if chain_ids.insert(id.clone()) {
                    next_frontier.insert(id);
                }
            }
        }
        frontier = next_frontier;
    }

    let commits_path = dir.join("commits.parquet");
    let commits = read_commits(&commits_path, manifest.row_counts.commits)?;
    let symbol_snapshots =
        read_symbol_snapshots_for_symbols_with_cache(dir, &manifest, &chain_ids, metadata_cache)?;
    let temporal_edges =
        read_temporal_edges_for_symbols_with_cache(dir, &manifest, &chain_ids, metadata_cache)?;

    let mut artifact = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: manifest.graph_index_version,
            content_hash_blake3: None,
        },
        manifest_version: manifest.manifest_version,
        graph_content_hash: manifest.graph_content_hash,
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits,
        symbol_snapshots,
        temporal_edges,
    };
    if legacy_snapshot_ids {
        crate::schema::rehash_legacy_snapshot_ids(&mut artifact);
    }
    Ok(artifact)
}

fn join_scoped<T>(
    handle: ScopedJoinHandle<'_, anyhow::Result<T>>,
    label: &str,
) -> anyhow::Result<T> {
    handle
        .join()
        .map_err(|_| anyhow!("Parquet worker thread panicked during {label}"))?
}

fn file_rows(artifact: &GraphIndexArtifact) -> anyhow::Result<Vec<FileRow>> {
    if artifact.file_node_ids.len() != artifact.files.len() {
        bail!(
            "artifact has {} files but {} file node ids",
            artifact.files.len(),
            artifact.file_node_ids.len()
        );
    }
    artifact
        .files
        .iter()
        .cloned()
        .zip(artifact.file_node_ids.iter().copied())
        .map(|(file, node_id)| Ok(FileRow { node_id, file }))
        .collect()
}

fn node_rows(artifact: &GraphIndexArtifact) -> anyhow::Result<Vec<NodeRow>> {
    if artifact.symbol_node_ids.len() != artifact.symbols.len() {
        bail!(
            "artifact has {} symbols but {} symbol node ids",
            artifact.symbols.len(),
            artifact.symbol_node_ids.len()
        );
    }
    artifact
        .symbols
        .iter()
        .cloned()
        .zip(artifact.symbol_node_ids.iter().copied())
        .map(|(symbol, node_id)| Ok(NodeRow { node_id, symbol }))
        .collect()
}

fn endpoint_id_map(
    files: &[FileRow],
    nodes: &[NodeRow],
) -> anyhow::Result<HashMap<String, NodeId>> {
    let mut endpoint_ids: HashMap<String, NodeId> = HashMap::new();
    let mut endpoint_metadata: HashMap<String, (String, String, String, [usize; 2])> =
        HashMap::new();
    for row in files {
        if let Some(existing) = endpoint_ids.insert(row.file.stable_file_id.clone(), row.node_id) {
            let prev = endpoint_metadata
                .get(&row.file.stable_file_id)
                .cloned()
                .unwrap_or_default();
            bail!(
                "stable endpoint id `{stable_id}` maps to both NodeId({prev_id}) (file path={prev_path}) and NodeId({new_id}) (file path={new_path})",
                stable_id = row.file.stable_file_id,
                prev_id = existing.get(),
                prev_path = prev.2,
                new_id = row.node_id.get(),
                new_path = row.file.file_path,
            );
        }
        endpoint_metadata.insert(
            row.file.stable_file_id.clone(),
            (
                "file".into(),
                row.file.file_path.clone(),
                row.file.file_path.clone(),
                [0, 0],
            ),
        );
    }
    for row in nodes {
        if let Some(existing) =
            endpoint_ids.insert(row.symbol.stable_symbol_id.clone(), row.node_id)
        {
            let prev = endpoint_metadata
                .get(&row.symbol.stable_symbol_id)
                .cloned()
                .unwrap_or_default();
            bail!(
                "stable endpoint id `{stable_id}` maps to both NodeId({prev_id}) (kind={prev_kind} qn={prev_qn:?} path={prev_path} range={prev_range:?}) and NodeId({new_id}) (kind={new_kind} qn={new_qn:?} path={new_path} range={new_range:?})",
                stable_id = row.symbol.stable_symbol_id,
                prev_id = existing.get(),
                prev_kind = prev.0,
                prev_qn = prev.1,
                prev_path = prev.2,
                prev_range = prev.3,
                new_id = row.node_id.get(),
                new_kind = row.symbol.symbol_kind,
                new_qn = row.symbol.qualified_name,
                new_path = row.symbol.file_path,
                new_range = row.symbol.byte_range,
            );
        }
        endpoint_metadata.insert(
            row.symbol.stable_symbol_id.clone(),
            (
                row.symbol.symbol_kind.clone(),
                row.symbol.qualified_name.clone(),
                row.symbol.file_path.clone(),
                row.symbol.byte_range,
            ),
        );
    }
    Ok(endpoint_ids)
}

fn edge_rows(
    artifact: &GraphIndexArtifact,
    endpoint_ids: &HashMap<String, NodeId>,
) -> anyhow::Result<(Vec<ResolvedEdgeRow>, Vec<UnresolvedEdgeRow>)> {
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    let mut dropped_stale = 0usize;
    for edge in &artifact.edges {
        let src_id = *endpoint_ids
            .get(&edge.source_stable_symbol_id)
            .ok_or_else(|| anyhow!("missing source endpoint `{}`", edge.source_stable_symbol_id))?;
        match edge.target_stable_symbol_id.as_deref() {
            Some(target_stable_id) => {
                if let Some(&dst_id) = endpoint_ids.get(target_stable_id) {
                    resolved.push(ResolvedEdgeRow {
                        edge: edge.clone(),
                        src_id,
                        dst_id,
                    });
                } else {
                    dropped_stale += 1;
                    tracing::warn!(
                        stable_symbol_id = %target_stable_id,
                        source_stable_symbol_id = %edge.source_stable_symbol_id,
                        "spur-graph: edge target endpoint missing in current artifact; routing to unresolved (likely stale phantom edge after byte-range shift)"
                    );
                    unresolved.push(UnresolvedEdgeRow {
                        edge: edge.clone(),
                        src_id,
                    });
                }
            }
            None => unresolved.push(UnresolvedEdgeRow {
                edge: edge.clone(),
                src_id,
            }),
        }
    }
    if dropped_stale > 0 {
        tracing::info!(
            dropped_stale,
            "spur-graph: dropped {} edge(s) with stale target id; see warn-level logs for details",
            dropped_stale
        );
    }
    Ok((resolved, unresolved))
}

#[cfg(test)]
mod edge_rows_test {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn edge_rows_routes_stale_target_to_unresolved() {
        let edge = test_edge("sym-a", Some("sym-stale-target"));
        let artifact = test_artifact(edge.clone());
        let endpoint_ids = HashMap::from([("sym-a".to_owned(), NodeId(1))]);

        let (resolved, unresolved) =
            edge_rows(&artifact, &endpoint_ids).expect("stale target routes to unresolved");

        assert!(resolved.is_empty());
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].edge, edge);
        assert_eq!(unresolved[0].src_id, NodeId(1));
    }

    #[test]
    fn edge_rows_still_bails_on_missing_source() {
        let edge = test_edge("sym-missing-source", Some("sym-a"));
        let artifact = test_artifact(edge);
        let endpoint_ids = HashMap::from([("sym-a".to_owned(), NodeId(1))]);

        let err =
            edge_rows(&artifact, &endpoint_ids).expect_err("missing source endpoint remains fatal");

        assert!(
            err.to_string()
                .contains("missing source endpoint `sym-missing-source`"),
            "error should mention missing source endpoint: {err:#}"
        );
    }

    fn test_artifact(edge: GraphEdgeArtifact) -> GraphIndexArtifact {
        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: crate::schema::GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test-manifest".to_owned(),
            graph_content_hash: "test-content-hash".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: vec![GraphSymbolArtifact {
                stable_symbol_id: "sym-a".to_owned(),
                file_path: "src/a.rs".to_owned(),
                byte_range: [0, 10],
                line_range: [1, 1],
                entity_name: "a".to_owned(),
                qualified_name: "crate::a".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-a".to_owned(),
                enclosing_scope: None,
            }],
            symbol_node_ids: vec![NodeId(1)],
            edges: vec![edge],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn test_edge(source: &str, target: Option<&str>) -> GraphEdgeArtifact {
        GraphEdgeArtifact {
            source_stable_symbol_id: source.to_owned(),
            target_stable_symbol_id: target.map(str::to_string),
            target_label: target.map(|_| "target".to_owned()),
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation: RelationKind::Calls,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::Calls),
            bind_method: None,
        }
    }
}

fn write_nodes(path: &Path, rows: &[NodeRow]) -> anyhow::Result<()> {
    write_table(
        path,
        r"
        message schema {
          required binary stable_symbol_id (STRING);
          required int64 node_id;
          required binary file_path (STRING);
          required int64 byte_range_start;
          required int64 byte_range_end;
          required int32 line_start;
          required int32 line_end;
          required binary entity_name (STRING);
          required binary qualified_name (STRING);
          required binary symbol_kind (STRING);
          required binary anchor_hash (STRING);
          optional binary enclosing_scope (STRING);
        }
        ",
        &[
            "file_path",
            "entity_name",
            "qualified_name",
            "symbol_kind",
            if ENCLOSING_SCOPE_DICTIONARY {
                "enclosing_scope"
            } else {
                ""
            },
        ],
        &[
            "stable_symbol_id",
            "file_path",
            "entity_name",
            "qualified_name",
        ],
        vec![
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| node_id_to_i64(row.node_id))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.file_path.clone())
                    .collect(),
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| usize_to_i64(row.symbol.byte_range[0], "byte_range_start"))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| usize_to_i64(row.symbol.byte_range[1], "byte_range_end"))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredI32(
                rows.iter()
                    .map(|row| usize_to_i32(row.symbol.line_range[0], "line_start"))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredI32(
                rows.iter()
                    .map(|row| usize_to_i32(row.symbol.line_range[1], "line_end"))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.entity_name.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.qualified_name.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.symbol_kind.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.symbol.anchor_hash.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.symbol.enclosing_scope.clone())
                    .collect(),
            ),
        ],
    )
}

fn write_edges(path: &Path, rows: &[ResolvedEdgeRow]) -> anyhow::Result<()> {
    write_table(
        path,
        r"
        message schema {
          required binary source_stable_id (STRING);
          required binary target_stable_id (STRING);
          required int64 src_id;
          required int64 dst_id;
          optional binary target_label (STRING);
          required binary relation (STRING);
          required binary confidence (STRING);
          required float confidence_score;
          optional binary edge_kind (STRING);
          optional binary bind_method (STRING);
          optional binary import_path (STRING);
          optional binary receiver_text (STRING);
          optional binary scope_text (STRING);
        }
        ",
        &[
            "source_stable_id",
            "target_stable_id",
            "target_label",
            "relation",
            "confidence",
            "edge_kind",
            "bind_method",
            "import_path",
            "receiver_text",
            "scope_text",
        ],
        &["source_stable_id", "target_stable_id"],
        vec![
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.edge.source_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| {
                        row.edge
                            .target_stable_symbol_id
                            .clone()
                            .expect("resolved edge has target")
                    })
                    .collect(),
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| node_id_to_i64(row.src_id))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| node_id_to_i64(row.dst_id))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.target_label.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| relation_to_str(row.edge.relation).to_owned())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| confidence_to_str(row.edge.confidence).to_owned())
                    .collect(),
            ),
            ColumnData::RequiredF32(rows.iter().map(|row| row.edge.confidence_score).collect()),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.edge_kind.map(edge_kind_to_str).map(str::to_string))
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.bind_method.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.import_path.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.receiver_text.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter().map(|row| row.edge.scope_text.clone()).collect(),
            ),
        ],
    )
}

fn write_unresolved_edges(path: &Path, rows: &[UnresolvedEdgeRow]) -> anyhow::Result<()> {
    write_table(
        path,
        r"
        message schema {
          required binary source_stable_id (STRING);
          required int64 src_id;
          optional binary target_label (STRING);
          required binary relation (STRING);
          required binary confidence (STRING);
          required float confidence_score;
          optional binary edge_kind (STRING);
          optional binary bind_method (STRING);
          optional binary import_path (STRING);
          optional binary receiver_text (STRING);
          optional binary scope_text (STRING);
        }
        ",
        &[
            "source_stable_id",
            "target_label",
            "relation",
            "confidence",
            "edge_kind",
            "bind_method",
            "import_path",
            "receiver_text",
            "scope_text",
        ],
        &["source_stable_id", "target_label"],
        vec![
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.edge.source_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| node_id_to_i64(row.src_id))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.target_label.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| relation_to_str(row.edge.relation).to_owned())
                    .collect(),
            ),
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| confidence_to_str(row.edge.confidence).to_owned())
                    .collect(),
            ),
            ColumnData::RequiredF32(rows.iter().map(|row| row.edge.confidence_score).collect()),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.edge_kind.map(edge_kind_to_str).map(str::to_string))
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.bind_method.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.import_path.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.edge.receiver_text.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter().map(|row| row.edge.scope_text.clone()).collect(),
            ),
        ],
    )
}

fn write_files(path: &Path, rows: &[FileRow]) -> anyhow::Result<()> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.file.file_path.cmp(&b.file.file_path));

    write_table(
        path,
        r"
        message schema {
          required binary stable_file_id (STRING);
          required int64 node_id;
          required binary file_path (STRING);
        }
        ",
        &["file_path"],
        &["file_path"],
        vec![
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.file.stable_file_id.clone())
                    .collect(),
            ),
            ColumnData::RequiredI64(
                rows.iter()
                    .map(|row| node_id_to_i64(row.node_id))
                    .collect::<anyhow::Result<_>>()?,
            ),
            ColumnData::RequiredString(rows.iter().map(|row| row.file.file_path.clone()).collect()),
        ],
    )
}

fn write_file_manifests(path: &Path, rows: &[GraphFileManifestEntry]) -> anyhow::Result<()> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.path.cmp(&b.path));

    write_table(
        path,
        r"
        message schema {
          required binary stable_file_id (STRING);
          required binary path (STRING);
          required binary content_oid (STRING);
          required group node_ids (LIST) {
            repeated group list {
              required int64 element;
            }
          }
        }
        ",
        &["path"],
        &["stable_file_id", "path"],
        vec![
            ColumnData::RequiredString(rows.iter().map(|row| row.stable_file_id.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.path.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.content_oid.clone()).collect()),
            ColumnData::RequiredListI64(
                rows.iter()
                    .map(|row| {
                        row.node_ids
                            .iter()
                            .copied()
                            .map(node_id_to_i64)
                            .collect::<anyhow::Result<Vec<_>>>()
                    })
                    .collect::<anyhow::Result<_>>()?,
            ),
        ],
    )
}

fn write_tombstones(path: &Path, rows: &[GraphTombstoneEntry]) -> anyhow::Result<()> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.path.cmp(&b.path));

    write_table(
        path,
        r"
        message schema {
          required binary path (STRING);
          required binary stable_file_id (STRING);
        }
        ",
        &["path"],
        &["stable_file_id", "path"],
        vec![
            ColumnData::RequiredString(rows.iter().map(|row| row.path.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.stable_file_id.clone()).collect()),
        ],
    )
}

fn write_commits(
    path: &Path,
    rows: &[CommitArtifact],
    _options: &WriteOptions,
) -> anyhow::Result<usize> {
    let rows: Vec<CommitRow> = rows
        .iter()
        .map(|row| CommitRow {
            sha: row.sha.clone(),
            parents: row.parents.clone(),
            author_time: row.author_time,
            author_name: row.author_name.clone(),
            author_email: row.author_email.clone(),
            summary: row.summary.clone(),
        })
        .collect();

    write_table(
        path,
        r"
        message schema {
          required binary sha (STRING);
          optional group parents (LIST) {
            repeated group list {
              required binary element (STRING);
            }
          }
          required int64 author_time;
          required binary summary (STRING);
          optional binary author_name (STRING);
          optional binary author_email (STRING);
        }
        ",
        &["sha", "summary"],
        &["sha"],
        vec![
            ColumnData::RequiredString(rows.iter().map(|row| row.sha.clone()).collect()),
            ColumnData::RequiredListString(rows.iter().map(|row| row.parents.clone()).collect()),
            ColumnData::RequiredI64(rows.iter().map(|row| row.author_time).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.summary.clone()).collect()),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| Some(row.author_name.clone()))
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| Some(row.author_email.clone()))
                    .collect(),
            ),
        ],
    )?;
    Ok(rows.len())
}

pub(crate) fn write_symbol_snapshots(
    path: &Path,
    rows: &[SymbolSnapshotArtifact],
    _options: &WriteOptions,
) -> anyhow::Result<usize> {
    let rows = rows
        .iter()
        .map(|row| {
            Ok(SymbolSnapshotRow {
                key_stable_symbol_id: row.key.stable_symbol_id.clone(),
                key_commit: row.key.commit.clone(),
                file_path_b64: git_path_to_b64(&row.file_path),
                entity_name: row.entity_name.clone(),
                symbol_kind: row.symbol_kind.clone(),
                enclosing_scope: row.enclosing_scope.clone(),
                byte_range_start: usize_to_i64(row.byte_range[0], "byte_range_start")?,
                byte_range_end: usize_to_i64(row.byte_range[1], "byte_range_end")?,
                line_range_start: usize_to_i64(row.line_range[0], "line_range_start")?,
                line_range_end: usize_to_i64(row.line_range[1], "line_range_end")?,
                anchor_hash: row.anchor_hash.clone(),
                tokens: row.tokens.clone(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    write_table(
        path,
        r"
        message schema {
          required binary key_stable_symbol_id (STRING);
          required binary key_commit (STRING);
          required binary file_path_b64 (STRING);
          required binary entity_name (STRING);
          required binary symbol_kind (STRING);
          optional binary enclosing_scope (STRING);
          required int64 byte_range_start;
          required int64 byte_range_end;
          required int64 line_range_start;
          required int64 line_range_end;
          required binary anchor_hash (STRING);
          optional group tokens (LIST) {
            repeated group list {
              required binary element (STRING);
            }
          }
        }
        ",
        &[
            "key_stable_symbol_id",
            "key_commit",
            "file_path_b64",
            "entity_name",
            "symbol_kind",
            if ENCLOSING_SCOPE_DICTIONARY {
                "enclosing_scope"
            } else {
                ""
            },
            "anchor_hash",
        ],
        &["key_stable_symbol_id"],
        vec![
            ColumnData::RequiredString(
                rows.iter()
                    .map(|row| row.key_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::RequiredString(rows.iter().map(|row| row.key_commit.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.file_path_b64.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.entity_name.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.symbol_kind.clone()).collect()),
            ColumnData::OptionalString(
                rows.iter().map(|row| row.enclosing_scope.clone()).collect(),
            ),
            ColumnData::RequiredI64(rows.iter().map(|row| row.byte_range_start).collect()),
            ColumnData::RequiredI64(rows.iter().map(|row| row.byte_range_end).collect()),
            ColumnData::RequiredI64(rows.iter().map(|row| row.line_range_start).collect()),
            ColumnData::RequiredI64(rows.iter().map(|row| row.line_range_end).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.anchor_hash.clone()).collect()),
            ColumnData::RequiredListString(rows.iter().map(|row| row.tokens.clone()).collect()),
        ],
    )?;
    Ok(rows.len())
}

pub(crate) fn write_temporal_edges(
    path: &Path,
    rows: &[TemporalEdgeArtifact],
    _options: &WriteOptions,
) -> anyhow::Result<usize> {
    let rows: Vec<TemporalEdgeRow> = rows
        .iter()
        .map(|row| {
            let (source_path_b64, source_stable_symbol_id, source_commit) =
                endpoint_columns(&row.source);
            let (target_path_b64, target_stable_symbol_id, target_commit) =
                endpoint_columns(&row.target);
            let (rename_prev_path_b64, rename_prev_stable_symbol_id, rename_prev_commit) =
                change_kind_rename_prev_columns(row.change_kind.as_ref());
            TemporalEdgeRow {
                source_kind: endpoint_kind_to_str(&row.source).to_owned(),
                source_path_b64,
                source_stable_symbol_id,
                source_commit,
                target_kind: endpoint_kind_to_str(&row.target).to_owned(),
                target_path_b64,
                target_stable_symbol_id,
                target_commit,
                relation: relation_to_str(row.relation).to_owned(),
                parent: row.parent.clone(),
                change_kind: row
                    .change_kind
                    .as_ref()
                    .map(change_kind_to_str)
                    .map(str::to_string),
                rename_prev_path_b64,
                rename_prev_stable_symbol_id,
                rename_prev_commit,
            }
        })
        .collect();

    write_table(
        path,
        r"
        message schema {
          required binary source_kind (STRING);
          optional binary source_path_b64 (STRING);
          optional binary source_stable_symbol_id (STRING);
          optional binary source_commit (STRING);
          required binary target_kind (STRING);
          optional binary target_path_b64 (STRING);
          optional binary target_stable_symbol_id (STRING);
          optional binary target_commit (STRING);
          required binary relation (STRING);
          optional binary parent (STRING);
          optional binary change_kind (STRING);
          optional binary rename_prev_path_b64 (STRING);
          optional binary rename_prev_stable_symbol_id (STRING);
          optional binary rename_prev_commit (STRING);
        }
        ",
        &["source_kind", "target_kind", "relation", "change_kind"],
        &["target_stable_symbol_id"],
        vec![
            ColumnData::RequiredString(rows.iter().map(|row| row.source_kind.clone()).collect()),
            ColumnData::OptionalString(
                rows.iter().map(|row| row.source_path_b64.clone()).collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.source_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(rows.iter().map(|row| row.source_commit.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.target_kind.clone()).collect()),
            ColumnData::OptionalString(
                rows.iter().map(|row| row.target_path_b64.clone()).collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.target_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(rows.iter().map(|row| row.target_commit.clone()).collect()),
            ColumnData::RequiredString(rows.iter().map(|row| row.relation.clone()).collect()),
            ColumnData::OptionalString(rows.iter().map(|row| row.parent.clone()).collect()),
            ColumnData::OptionalString(rows.iter().map(|row| row.change_kind.clone()).collect()),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.rename_prev_path_b64.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.rename_prev_stable_symbol_id.clone())
                    .collect(),
            ),
            ColumnData::OptionalString(
                rows.iter()
                    .map(|row| row.rename_prev_commit.clone())
                    .collect(),
            ),
        ],
    )?;
    Ok(rows.len())
}

fn write_diagnostics(path: &Path, rows: &[String]) -> anyhow::Result<usize> {
    write_table(
        path,
        r"
        message schema {
          required binary message (STRING);
        }
        ",
        &["message"],
        &[],
        vec![ColumnData::RequiredString(rows.to_vec())],
    )?;
    Ok(rows.len())
}

fn write_table(
    path: &Path,
    schema: &str,
    dictionary_columns: &[&str],
    bloom_filter_columns: &[&str],
    columns: Vec<ColumnData>,
) -> anyhow::Result<()> {
    let row_count = columns.first().map(ColumnData::len).unwrap_or(0);
    for column in &columns {
        if column.len() != row_count {
            bail!("column length mismatch while writing `{}`", path.display());
        }
    }

    let schema = Arc::new(parse_message_type(schema).context("failed to parse Parquet schema")?);
    let file =
        File::create(path).with_context(|| format!("failed to create `{}`", path.display()))?;
    let props = Arc::new(writer_properties(
        dictionary_columns,
        bloom_filter_columns,
        row_count,
    )?);
    let mut writer =
        SerializedFileWriter::new(file, schema, props).context("failed to create writer")?;

    for start in (0..row_count).step_by(PARQUET_ROW_GROUP_SIZE) {
        let end = (start + PARQUET_ROW_GROUP_SIZE).min(row_count);
        let mut row_group = writer
            .next_row_group()
            .context("failed to start Parquet row group")?;
        for column in &columns {
            let mut column_writer = row_group
                .next_column()
                .context("failed to create Parquet column writer")?
                .ok_or_else(|| anyhow!("Parquet schema has fewer columns than data"))?;
            write_column(column, start, end, &mut column_writer)?;
            column_writer
                .close()
                .context("failed to close Parquet column writer")?;
        }
        if row_group
            .next_column()
            .context("failed to inspect Parquet column writer")?
            .is_some()
        {
            bail!(
                "Parquet schema has more columns than data for `{}`",
                path.display()
            );
        }
        row_group.close().context("failed to close row group")?;
    }
    writer.close().context("failed to close Parquet writer")?;
    sync_file(path)?;
    Ok(())
}

fn write_manifest(dir: &Path, manifest_json: &str) -> anyhow::Result<()> {
    let manifest_path = dir.join("manifest.json");
    let tmp_path = dir.join("manifest.json.tmp");
    let mut file = File::create(&tmp_path)
        .with_context(|| format!("failed to create `{}`", tmp_path.display()))?;
    file.write_all(manifest_json.as_bytes())
        .with_context(|| format!("failed to write `{}`", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to fsync `{}`", tmp_path.display()))?;
    drop(file);
    fs::rename(&tmp_path, &manifest_path).with_context(|| {
        format!(
            "failed to atomically rename `{}` to `{}`",
            tmp_path.display(),
            manifest_path.display()
        )
    })?;
    Ok(())
}

fn sync_file(path: &Path) -> anyhow::Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open `{}` for fsync", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to fsync `{}`", path.display()))
}

fn fsync_dir(path: &Path) -> anyhow::Result<()> {
    File::open(path)
        .with_context(|| format!("failed to open directory `{}` for fsync", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to fsync directory `{}`", path.display()))
}

fn writer_properties(
    dictionary_columns: &[&str],
    bloom_filter_columns: &[&str],
    row_count: usize,
) -> anyhow::Result<WriterProperties> {
    let mut builder = WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            ZstdLevel::try_new(3).context("invalid zstd compression level")?,
        ))
        .set_max_row_group_row_count(Some(PARQUET_ROW_GROUP_SIZE))
        .set_dictionary_enabled(false)
        .set_statistics_enabled(EnabledStatistics::Page);

    for column in dictionary_columns
        .iter()
        .copied()
        .filter(|column| !column.is_empty())
    {
        builder = builder.set_column_dictionary_enabled(ColumnPath::from(column), true);
    }
    let bloom_ndv = u64::try_from(row_count.clamp(1, PARQUET_ROW_GROUP_SIZE))
        .expect("row-group row count fits u64");
    for column in bloom_filter_columns
        .iter()
        .copied()
        .filter(|column| !column.is_empty())
    {
        builder = builder
            .set_column_bloom_filter_enabled(ColumnPath::from(column), true)
            .set_column_bloom_filter_fpp(ColumnPath::from(column), 0.01)
            .set_column_bloom_filter_ndv(ColumnPath::from(column), bloom_ndv);
    }
    Ok(builder.build())
}

fn write_column(
    column: &ColumnData,
    start: usize,
    end: usize,
    writer: &mut SerializedColumnWriter<'_>,
) -> anyhow::Result<()> {
    match column {
        ColumnData::RequiredString(values) => {
            let values = values[start..end]
                .iter()
                .map(|value| ByteArray::from(value.as_bytes().to_vec()))
                .collect::<Vec<_>>();
            writer
                .typed::<ByteArrayType>()
                .write_batch(&values, None, None)?;
        }
        ColumnData::OptionalString(values) => {
            let mut encoded = Vec::new();
            let mut def_levels = Vec::with_capacity(end - start);
            for value in &values[start..end] {
                if let Some(value) = value {
                    encoded.push(ByteArray::from(value.as_bytes().to_vec()));
                    def_levels.push(1);
                } else {
                    def_levels.push(0);
                }
            }
            writer
                .typed::<ByteArrayType>()
                .write_batch(&encoded, Some(&def_levels), None)?;
        }
        ColumnData::RequiredI64(values) => {
            writer
                .typed::<Int64Type>()
                .write_batch(&values[start..end], None, None)?;
        }
        ColumnData::RequiredI32(values) => {
            writer
                .typed::<Int32Type>()
                .write_batch(&values[start..end], None, None)?;
        }
        ColumnData::RequiredF32(values) => {
            writer
                .typed::<FloatType>()
                .write_batch(&values[start..end], None, None)?;
        }
        ColumnData::RequiredListI64(values) => {
            let mut encoded = Vec::new();
            let mut def_levels = Vec::new();
            let mut rep_levels = Vec::new();
            for list in &values[start..end] {
                if list.is_empty() {
                    def_levels.push(0);
                    rep_levels.push(0);
                } else {
                    for (index, value) in list.iter().enumerate() {
                        encoded.push(*value);
                        def_levels.push(1);
                        rep_levels.push(i16::from(index != 0));
                    }
                }
            }
            writer.typed::<Int64Type>().write_batch(
                &encoded,
                Some(&def_levels),
                Some(&rep_levels),
            )?;
        }
        ColumnData::RequiredListString(values) => {
            let mut encoded = Vec::new();
            let mut def_levels = Vec::new();
            let mut rep_levels = Vec::new();
            for list in &values[start..end] {
                if list.is_empty() {
                    def_levels.push(1);
                    rep_levels.push(0);
                } else {
                    for (index, value) in list.iter().enumerate() {
                        encoded.push(ByteArray::from(value.as_bytes().to_vec()));
                        def_levels.push(2);
                        rep_levels.push(i16::from(index != 0));
                    }
                }
            }
            writer.typed::<ByteArrayType>().write_batch(
                &encoded,
                Some(&def_levels),
                Some(&rep_levels),
            )?;
        }
    }
    Ok(())
}

fn read_nodes(
    path: &Path,
    row_count: usize,
) -> anyhow::Result<(Vec<GraphSymbolArtifact>, Vec<NodeId>)> {
    let mut symbols = Vec::with_capacity(row_count);
    let mut node_ids = Vec::with_capacity(symbols.capacity());
    for batch in read_record_batches(path)? {
        let stable_symbol_id = string_array(&batch, 0, "stable_symbol_id")?;
        let node_id = i64_array(&batch, 1, "node_id")?;
        let file_path = string_array(&batch, 2, "file_path")?;
        let byte_range_start = i64_array(&batch, 3, "byte_range_start")?;
        let byte_range_end = i64_array(&batch, 4, "byte_range_end")?;
        let line_start = i32_array(&batch, 5, "line_start")?;
        let line_end = i32_array(&batch, 6, "line_end")?;
        let entity_name = string_array(&batch, 7, "entity_name")?;
        let qualified_name = string_array(&batch, 8, "qualified_name")?;
        let symbol_kind = string_array(&batch, 9, "symbol_kind")?;
        let anchor_hash = string_array(&batch, 10, "anchor_hash")?;
        let enclosing_scope = string_array(&batch, 11, "enclosing_scope")?;

        for row in 0..batch.num_rows() {
            symbols.push(GraphSymbolArtifact {
                stable_symbol_id: required_string_value(stable_symbol_id, row, "stable_symbol_id")?,
                file_path: required_string_value(file_path, row, "file_path")?,
                byte_range: [
                    i64_to_usize(byte_range_start.value(row), "byte_range_start")?,
                    i64_to_usize(byte_range_end.value(row), "byte_range_end")?,
                ],
                line_range: [
                    i32_to_usize(line_start.value(row), "line_start")?,
                    i32_to_usize(line_end.value(row), "line_end")?,
                ],
                entity_name: required_string_value(entity_name, row, "entity_name")?,
                qualified_name: required_string_value(qualified_name, row, "qualified_name")?,
                symbol_kind: required_string_value(symbol_kind, row, "symbol_kind")?,
                anchor_hash: required_string_value(anchor_hash, row, "anchor_hash")?,
                enclosing_scope: optional_string_value(enclosing_scope, row),
            });
            node_ids.push(i64_to_node_id(node_id.value(row), "node_id")?);
        }
    }
    Ok((symbols, node_ids))
}

fn read_edges(path: &Path, row_count: usize) -> anyhow::Result<Vec<GraphEdgeArtifact>> {
    let mut edges = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let source_stable_id = string_array(&batch, 0, "source_stable_id")?;
        let target_stable_id = string_array(&batch, 1, "target_stable_id")?;
        let target_label = string_array(&batch, 4, "target_label")?;
        let relation = string_array(&batch, 5, "relation")?;
        let confidence = string_array(&batch, 6, "confidence")?;
        let confidence_score = f32_array(&batch, 7, "confidence_score")?;
        let edge_kind = string_array(&batch, 8, "edge_kind")?;
        let bind_method = optional_string_array_by_name(&batch, "bind_method")?;
        let import_path = optional_string_array_by_name(&batch, "import_path")?;
        let receiver_text = optional_string_array_by_name(&batch, "receiver_text")?;
        let scope_text = optional_string_array_by_name(&batch, "scope_text")?;

        for row in 0..batch.num_rows() {
            edges.push(GraphEdgeArtifact {
                source_stable_symbol_id: required_string_value(
                    source_stable_id,
                    row,
                    "source_stable_id",
                )?,
                target_stable_symbol_id: Some(required_string_value(
                    target_stable_id,
                    row,
                    "target_stable_id",
                )?),
                target_label: optional_string_value(target_label, row),
                import_path: import_path.and_then(|values| optional_string_value(values, row)),
                receiver_text: receiver_text.and_then(|values| optional_string_value(values, row)),
                scope_text: scope_text.and_then(|values| optional_string_value(values, row)),
                relation: relation_from_str(&required_string_value(relation, row, "relation")?)?,
                confidence: confidence_from_str(&required_string_value(
                    confidence,
                    row,
                    "confidence",
                )?)?,
                confidence_score: confidence_score.value(row),
                change_kind: None,
                edge_kind: optional_string_value(edge_kind, row)
                    .as_deref()
                    .map(edge_kind_from_str)
                    .transpose()?,
                bind_method: bind_method.and_then(|values| optional_string_value(values, row)),
            });
        }
    }
    Ok(edges)
}

fn read_unresolved_edges(path: &Path, row_count: usize) -> anyhow::Result<Vec<GraphEdgeArtifact>> {
    let mut edges = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let source_stable_id = string_array(&batch, 0, "source_stable_id")?;
        let target_label = string_array(&batch, 2, "target_label")?;
        let relation = string_array(&batch, 3, "relation")?;
        let confidence = string_array(&batch, 4, "confidence")?;
        let confidence_score = f32_array(&batch, 5, "confidence_score")?;
        let edge_kind = string_array(&batch, 6, "edge_kind")?;
        let bind_method = optional_string_array_by_name(&batch, "bind_method")?;
        let import_path = optional_string_array_by_name(&batch, "import_path")?;
        let receiver_text = optional_string_array_by_name(&batch, "receiver_text")?;
        let scope_text = optional_string_array_by_name(&batch, "scope_text")?;

        for row in 0..batch.num_rows() {
            edges.push(GraphEdgeArtifact {
                source_stable_symbol_id: required_string_value(
                    source_stable_id,
                    row,
                    "source_stable_id",
                )?,
                target_stable_symbol_id: None,
                target_label: optional_string_value(target_label, row),
                import_path: import_path.and_then(|values| optional_string_value(values, row)),
                receiver_text: receiver_text.and_then(|values| optional_string_value(values, row)),
                scope_text: scope_text.and_then(|values| optional_string_value(values, row)),
                relation: relation_from_str(&required_string_value(relation, row, "relation")?)?,
                confidence: confidence_from_str(&required_string_value(
                    confidence,
                    row,
                    "confidence",
                )?)?,
                confidence_score: confidence_score.value(row),
                change_kind: None,
                edge_kind: optional_string_value(edge_kind, row)
                    .as_deref()
                    .map(edge_kind_from_str)
                    .transpose()?,
                bind_method: bind_method.and_then(|values| optional_string_value(values, row)),
            });
        }
    }
    Ok(edges)
}

fn read_files(
    path: &Path,
    row_count: usize,
) -> anyhow::Result<(Vec<GraphFileArtifact>, Vec<NodeId>)> {
    let mut files = Vec::with_capacity(row_count);
    let mut node_ids = Vec::with_capacity(files.capacity());
    for batch in read_record_batches(path)? {
        let stable_file_id = string_array(&batch, 0, "stable_file_id")?;
        let node_id = i64_array(&batch, 1, "node_id")?;
        let file_path = string_array(&batch, 2, "file_path")?;
        for row in 0..batch.num_rows() {
            files.push(GraphFileArtifact {
                stable_file_id: required_string_value(stable_file_id, row, "stable_file_id")?,
                file_path: required_string_value(file_path, row, "file_path")?,
            });
            node_ids.push(i64_to_node_id(node_id.value(row), "node_id")?);
        }
    }
    Ok((files, node_ids))
}

fn read_file_manifests(
    path: &Path,
    row_count: usize,
) -> anyhow::Result<Vec<GraphFileManifestEntry>> {
    let mut manifests = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let stable_file_id = string_array(&batch, 0, "stable_file_id")?;
        let path_col = string_array(&batch, 1, "path")?;
        let content_oid = string_array(&batch, 2, "content_oid")?;
        let node_ids = list_array(&batch, 3, "node_ids")?;
        for row in 0..batch.num_rows() {
            manifests.push(GraphFileManifestEntry {
                stable_file_id: required_string_value(stable_file_id, row, "stable_file_id")?,
                path: required_string_value(path_col, row, "path")?,
                content_oid: required_string_value(content_oid, row, "content_oid")?,
                node_ids: required_node_id_list_value(node_ids, row, "node_ids")?,
            });
        }
    }
    Ok(manifests)
}

fn read_tombstones(path: &Path, row_count: usize) -> anyhow::Result<Vec<GraphTombstoneEntry>> {
    if row_count == 0 {
        return Ok(Vec::new());
    }
    let mut tombstones = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let path_col = string_array(&batch, 0, "path")?;
        let stable_file_id = string_array(&batch, 1, "stable_file_id")?;
        for row in 0..batch.num_rows() {
            tombstones.push(GraphTombstoneEntry {
                path: required_string_value(path_col, row, "path")?,
                stable_file_id: required_string_value(stable_file_id, row, "stable_file_id")?,
            });
        }
    }
    Ok(tombstones)
}

fn read_commits(path: &Path, row_count: usize) -> anyhow::Result<Vec<CommitArtifact>> {
    if row_count == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut commits = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let sha = string_array(&batch, 0, "sha")?;
        let parents = list_array(&batch, 1, "parents")?;
        let author_time = i64_array(&batch, 2, "author_time")?;
        let summary = string_array_by_name(&batch, "summary")?;
        let author_name = optional_string_array_by_name(&batch, "author_name")?;
        let author_email = optional_string_array_by_name(&batch, "author_email")?;
        for row in 0..batch.num_rows() {
            commits.push(CommitArtifact {
                sha: required_string_value(sha, row, "sha")?,
                parents: required_string_list_value(parents, row, "parents")?,
                author_time: author_time.value(row),
                author_name: author_name
                    .and_then(|values| optional_string_value(values, row))
                    .unwrap_or_default(),
                author_email: author_email
                    .and_then(|values| optional_string_value(values, row))
                    .unwrap_or_default(),
                summary: required_string_value(summary, row, "summary")?,
            });
        }
    }
    Ok(commits)
}

fn read_symbol_snapshots(
    dir: &Path,
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    ensure_temporal_shards_layout(dir, manifest)?;
    if manifest.temporal_shards.is_empty() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::with_capacity(manifest.row_counts.symbol_snapshots);
    for shard in &manifest.temporal_shards {
        let path = temporal_shard_path(dir, "symbol_snapshots", shard.shard_idx);
        snapshots.extend(read_symbol_snapshots_file(
            &path,
            shard.row_count_snapshots,
        )?);
    }
    Ok(snapshots)
}

pub fn read_symbol_snapshots_for_symbols(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    ids: &HashSet<String>,
) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    read_symbol_snapshots_for_symbols_with_cache(
        dir,
        manifest,
        ids,
        &ParquetMetadataCache::default(),
    )
}

fn read_symbol_snapshots_for_symbols_with_cache(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    ids: &HashSet<String>,
    metadata_cache: &ParquetMetadataCache,
) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    ensure_temporal_shards_layout(dir, manifest)?;
    if ids.is_empty() || manifest.temporal_shards.is_empty() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();
    for shard in &manifest.temporal_shards {
        let path = temporal_shard_path(dir, "symbol_snapshots", shard.shard_idx);
        snapshots.extend(read_symbol_snapshots_file_for_symbols(
            &path,
            shard.row_count_snapshots,
            ids,
            metadata_cache,
        )?);
    }
    Ok(snapshots)
}

fn read_symbol_snapshots_file_for_symbols(
    path: &Path,
    row_count: usize,
    ids: &HashSet<String>,
    metadata_cache: &ParquetMetadataCache,
) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    if row_count == 0 || ids.is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::new();
    let pruning = StringPruningPredicate::any_value("key_stable_symbol_id", ids.iter().cloned());
    for batch in read_filtered_projected_batches(
        path,
        metadata_cache,
        SYMBOL_SNAPSHOT_COLUMNS,
        &pruning,
        |schema| string_in_row_filter(schema, "key_stable_symbol_id", ids.clone()),
    )? {
        snapshots.extend(symbol_snapshots_from_batch(&batch)?);
    }
    Ok(snapshots)
}

fn read_symbol_snapshots_file(
    path: &Path,
    row_count: usize,
) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    if row_count == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let key_stable_symbol_id = string_array(&batch, 0, "key_stable_symbol_id")?;
        let key_commit = string_array(&batch, 1, "key_commit")?;
        let file_path_b64 = string_array(&batch, 2, "file_path_b64")?;
        let entity_name = string_array(&batch, 3, "entity_name")?;
        let symbol_kind = string_array(&batch, 4, "symbol_kind")?;
        let enclosing_scope = string_array(&batch, 5, "enclosing_scope")?;
        let byte_range_start = i64_array(&batch, 6, "byte_range_start")?;
        let byte_range_end = i64_array(&batch, 7, "byte_range_end")?;
        let line_range_start = i64_array(&batch, 8, "line_range_start")?;
        let line_range_end = i64_array(&batch, 9, "line_range_end")?;
        let anchor_hash = string_array(&batch, 10, "anchor_hash")?;
        let tokens = list_array(&batch, 11, "tokens")?;
        for row in 0..batch.num_rows() {
            let file_path_b64 = required_string_value(file_path_b64, row, "file_path_b64")?;
            let file_path = git_path_from_b64(&file_path_b64)
                .with_context(|| format!("row {row}: invalid file_path_b64"))?;
            snapshots.push(SymbolSnapshotArtifact {
                key: SnapshotKey {
                    stable_symbol_id: required_string_value(
                        key_stable_symbol_id,
                        row,
                        "key_stable_symbol_id",
                    )?,
                    commit: required_string_value(key_commit, row, "key_commit")?,
                },
                file_path,
                entity_name: required_string_value(entity_name, row, "entity_name")?,
                symbol_kind: required_string_value(symbol_kind, row, "symbol_kind")?,
                enclosing_scope: optional_string_value(enclosing_scope, row),
                byte_range: [
                    i64_to_usize(byte_range_start.value(row), "byte_range_start")?,
                    i64_to_usize(byte_range_end.value(row), "byte_range_end")?,
                ],
                line_range: [
                    i64_to_usize(line_range_start.value(row), "line_range_start")?,
                    i64_to_usize(line_range_end.value(row), "line_range_end")?,
                ],
                anchor_hash: required_string_value(anchor_hash, row, "anchor_hash")?,
                tokens: required_string_list_value(tokens, row, "tokens")?,
            });
        }
    }
    Ok(snapshots)
}

fn read_temporal_edges(
    dir: &Path,
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    ensure_temporal_shards_layout(dir, manifest)?;
    if manifest.temporal_shards.is_empty() {
        return Ok(Vec::new());
    }

    let mut edges = Vec::with_capacity(manifest.row_counts.temporal_edges);
    for shard in &manifest.temporal_shards {
        let path = temporal_shard_path(dir, "temporal_edges", shard.shard_idx);
        edges.extend(read_temporal_edges_file(&path, shard.row_count_edges)?);
    }
    Ok(edges)
}

pub fn read_temporal_edges_for_symbols(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    ids: &HashSet<String>,
) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    read_temporal_edges_for_symbols_with_cache(dir, manifest, ids, &ParquetMetadataCache::default())
}

fn read_temporal_edges_for_symbols_with_cache(
    dir: &Path,
    manifest: &GraphArtifactManifest,
    ids: &HashSet<String>,
    metadata_cache: &ParquetMetadataCache,
) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    ensure_temporal_shards_layout(dir, manifest)?;
    if ids.is_empty() || manifest.temporal_shards.is_empty() {
        return Ok(Vec::new());
    }

    let mut edges = Vec::new();
    for shard in &manifest.temporal_shards {
        let path = temporal_shard_path(dir, "temporal_edges", shard.shard_idx);
        edges.extend(read_temporal_edges_file_for_symbols(
            &path,
            shard.row_count_edges,
            ids,
            metadata_cache,
        )?);
    }
    Ok(edges)
}

fn read_temporal_edges_file_for_symbols(
    path: &Path,
    row_count: usize,
    ids: &HashSet<String>,
    metadata_cache: &ParquetMetadataCache,
) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    if row_count == 0 || ids.is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let mut edges = Vec::new();
    let pruning = StringPruningPredicate::any(
        TEMPORAL_EDGE_SYMBOL_ID_COLUMNS
            .iter()
            .map(|column| StringPruningPredicate::any_value(*column, ids.iter().cloned())),
    );
    for batch in read_filtered_projected_batches(
        path,
        metadata_cache,
        TEMPORAL_EDGE_COLUMNS,
        &pruning,
        |schema| string_in_any_row_filter(schema, TEMPORAL_EDGE_SYMBOL_ID_COLUMNS, ids.clone()),
    )? {
        edges.extend(temporal_edges_from_batch(&batch)?);
    }
    Ok(edges)
}

fn read_temporal_edges_file(
    path: &Path,
    row_count: usize,
) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    if row_count == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut edges = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let source_kind = string_array(&batch, 0, "source_kind")?;
        let source_path_b64 = string_array(&batch, 1, "source_path_b64")?;
        let source_stable_symbol_id = string_array(&batch, 2, "source_stable_symbol_id")?;
        let source_commit = string_array(&batch, 3, "source_commit")?;
        let target_kind = string_array(&batch, 4, "target_kind")?;
        let target_path_b64 = string_array(&batch, 5, "target_path_b64")?;
        let target_stable_symbol_id = string_array(&batch, 6, "target_stable_symbol_id")?;
        let target_commit = string_array(&batch, 7, "target_commit")?;
        let relation = string_array(&batch, 8, "relation")?;
        let parent = string_array(&batch, 9, "parent")?;
        let change_kind = string_array(&batch, 10, "change_kind")?;
        let rename_prev_path_b64 = string_array(&batch, 11, "rename_prev_path_b64")?;
        let rename_prev_stable_symbol_id =
            string_array(&batch, 12, "rename_prev_stable_symbol_id")?;
        let rename_prev_commit = string_array(&batch, 13, "rename_prev_commit")?;

        for row in 0..batch.num_rows() {
            let source_kind = required_string_value(source_kind, row, "source_kind")?;
            let target_kind = required_string_value(target_kind, row, "target_kind")?;
            let relation_value = required_string_value(relation, row, "relation")?;
            edges.push(TemporalEdgeArtifact {
                source: endpoint_from_columns(
                    row,
                    "source",
                    &source_kind,
                    optional_string_value(source_path_b64, row),
                    optional_string_value(source_stable_symbol_id, row),
                    optional_string_value(source_commit, row),
                )?,
                target: endpoint_from_columns(
                    row,
                    "target",
                    &target_kind,
                    optional_string_value(target_path_b64, row),
                    optional_string_value(target_stable_symbol_id, row),
                    optional_string_value(target_commit, row),
                )?,
                relation: relation_from_str(&relation_value)
                    .with_context(|| format!("row {row}: invalid relation `{relation_value}`"))?,
                parent: optional_string_value(parent, row),
                change_kind: change_kind_from_columns(
                    row,
                    optional_string_value(change_kind, row),
                    optional_string_value(rename_prev_path_b64, row),
                    optional_string_value(rename_prev_stable_symbol_id, row),
                    optional_string_value(rename_prev_commit, row),
                )?,
            });
        }
    }
    Ok(edges)
}

fn symbol_snapshots_from_batch(batch: &RecordBatch) -> anyhow::Result<Vec<SymbolSnapshotArtifact>> {
    let key_stable_symbol_id = string_array(batch, 0, "key_stable_symbol_id")?;
    let key_commit = string_array(batch, 1, "key_commit")?;
    let file_path_b64 = string_array(batch, 2, "file_path_b64")?;
    let entity_name = string_array(batch, 3, "entity_name")?;
    let symbol_kind = string_array(batch, 4, "symbol_kind")?;
    let enclosing_scope = string_array(batch, 5, "enclosing_scope")?;
    let byte_range_start = i64_array(batch, 6, "byte_range_start")?;
    let byte_range_end = i64_array(batch, 7, "byte_range_end")?;
    let line_range_start = i64_array(batch, 8, "line_range_start")?;
    let line_range_end = i64_array(batch, 9, "line_range_end")?;
    let anchor_hash = string_array(batch, 10, "anchor_hash")?;
    let tokens = list_array(batch, 11, "tokens")?;

    let mut snapshots = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let file_path_b64 = required_string_value(file_path_b64, row, "file_path_b64")?;
        let file_path = git_path_from_b64(&file_path_b64)
            .with_context(|| format!("row {row}: invalid file_path_b64"))?;
        snapshots.push(SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: required_string_value(
                    key_stable_symbol_id,
                    row,
                    "key_stable_symbol_id",
                )?,
                commit: required_string_value(key_commit, row, "key_commit")?,
            },
            file_path,
            entity_name: required_string_value(entity_name, row, "entity_name")?,
            symbol_kind: required_string_value(symbol_kind, row, "symbol_kind")?,
            enclosing_scope: optional_string_value(enclosing_scope, row),
            byte_range: [
                i64_to_usize(byte_range_start.value(row), "byte_range_start")?,
                i64_to_usize(byte_range_end.value(row), "byte_range_end")?,
            ],
            line_range: [
                i64_to_usize(line_range_start.value(row), "line_range_start")?,
                i64_to_usize(line_range_end.value(row), "line_range_end")?,
            ],
            anchor_hash: required_string_value(anchor_hash, row, "anchor_hash")?,
            tokens: required_string_list_value(tokens, row, "tokens")?,
        });
    }
    Ok(snapshots)
}

fn temporal_edges_from_batch(batch: &RecordBatch) -> anyhow::Result<Vec<TemporalEdgeArtifact>> {
    let source_kind = string_array(batch, 0, "source_kind")?;
    let source_path_b64 = string_array(batch, 1, "source_path_b64")?;
    let source_stable_symbol_id = string_array(batch, 2, "source_stable_symbol_id")?;
    let source_commit = string_array(batch, 3, "source_commit")?;
    let target_kind = string_array(batch, 4, "target_kind")?;
    let target_path_b64 = string_array(batch, 5, "target_path_b64")?;
    let target_stable_symbol_id = string_array(batch, 6, "target_stable_symbol_id")?;
    let target_commit = string_array(batch, 7, "target_commit")?;
    let relation = string_array(batch, 8, "relation")?;
    let parent = string_array(batch, 9, "parent")?;
    let change_kind = string_array(batch, 10, "change_kind")?;
    let rename_prev_path_b64 = string_array(batch, 11, "rename_prev_path_b64")?;
    let rename_prev_stable_symbol_id = string_array(batch, 12, "rename_prev_stable_symbol_id")?;
    let rename_prev_commit = string_array(batch, 13, "rename_prev_commit")?;

    let mut edges = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let source_kind = required_string_value(source_kind, row, "source_kind")?;
        let target_kind = required_string_value(target_kind, row, "target_kind")?;
        let relation_value = required_string_value(relation, row, "relation")?;
        edges.push(TemporalEdgeArtifact {
            source: endpoint_from_columns(
                row,
                "source",
                &source_kind,
                optional_string_value(source_path_b64, row),
                optional_string_value(source_stable_symbol_id, row),
                optional_string_value(source_commit, row),
            )?,
            target: endpoint_from_columns(
                row,
                "target",
                &target_kind,
                optional_string_value(target_path_b64, row),
                optional_string_value(target_stable_symbol_id, row),
                optional_string_value(target_commit, row),
            )?,
            relation: relation_from_str(&relation_value)
                .with_context(|| format!("row {row}: invalid relation `{relation_value}`"))?,
            parent: optional_string_value(parent, row),
            change_kind: change_kind_from_columns(
                row,
                optional_string_value(change_kind, row),
                optional_string_value(rename_prev_path_b64, row),
                optional_string_value(rename_prev_stable_symbol_id, row),
                optional_string_value(rename_prev_commit, row),
            )?,
        });
    }
    Ok(edges)
}

fn rename_symbol_ids(edge: &TemporalEdgeArtifact) -> Option<Vec<&str>> {
    let Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(previous))) = &edge.change_kind else {
        return None;
    };

    let mut ids = Vec::with_capacity(3);
    if let Some(id) = endpoint_stable_symbol_id(&edge.source) {
        ids.push(id);
    }
    if let Some(id) = endpoint_stable_symbol_id(&edge.target) {
        ids.push(id);
    }
    ids.push(&previous.stable_symbol_id);
    ids.sort_unstable();
    ids.dedup();
    Some(ids)
}

fn endpoint_stable_symbol_id(endpoint: &EdgeEndpoint) -> Option<&str> {
    match endpoint {
        EdgeEndpoint::Symbol { stable_symbol_id } => Some(stable_symbol_id),
        EdgeEndpoint::Snapshot { key } => Some(&key.stable_symbol_id),
        EdgeEndpoint::File { .. } | EdgeEndpoint::Commit { .. } => None,
    }
}

fn read_diagnostics(path: &Path, row_count: usize) -> anyhow::Result<Vec<String>> {
    if row_count == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut diagnostics = Vec::with_capacity(row_count);
    for batch in read_record_batches(path)? {
        let message = string_array(&batch, 0, "message")?;
        for row in 0..batch.num_rows() {
            diagnostics.push(required_string_value(message, row, "message")?);
        }
    }
    Ok(diagnostics)
}

pub fn stream_temporal_artifact_parquet<F>(dir: &Path, mut callback: F) -> anyhow::Result<()>
where
    F: FnMut(TemporalArtifactTable, RecordBatch) -> anyhow::Result<()>,
{
    let manifest = read_artifact_header_parquet(dir)?;
    if !manifest.complete {
        bail!(
            "refusing to load incomplete Parquet artifact `{}`",
            dir.display()
        );
    }
    ensure_temporal_shards_layout(dir, &manifest)?;

    for shard in &manifest.temporal_shards {
        let temporal_edges_path = temporal_shard_path(dir, "temporal_edges", shard.shard_idx);
        if shard.row_count_edges > 0 {
            stream_record_batches(&temporal_edges_path, |batch| {
                callback(TemporalArtifactTable::TemporalEdges, batch)
            })?;
        }

        let symbol_snapshots_path = temporal_shard_path(dir, "symbol_snapshots", shard.shard_idx);
        if shard.row_count_snapshots > 0 {
            stream_record_batches(&symbol_snapshots_path, |batch| {
                callback(TemporalArtifactTable::SymbolSnapshots, batch)
            })?;
        }
    }

    Ok(())
}

fn ensure_temporal_shards_layout(
    dir: &Path,
    manifest: &GraphArtifactManifest,
) -> anyhow::Result<()> {
    if manifest.temporal_shards.is_empty() && dir.join("temporal_edges.parquet").exists() {
        bail!(
            "legacy temporal_edges.parquet exists in `{}` but manifest has no temporal_shards",
            dir.display()
        );
    }
    Ok(())
}

fn temporal_shard_path(dir: &Path, table: &str, shard_idx: u32) -> PathBuf {
    dir.join(table).join(format!("{shard_idx:05}.parquet"))
}

fn read_record_batches(path: &Path) -> anyhow::Result<Vec<RecordBatch>> {
    let mut batches = Vec::new();
    stream_record_batches(path, |batch| {
        batches.push(batch);
        Ok(())
    })?;
    Ok(batches)
}

fn stream_record_batches<F>(path: &Path, mut callback: F) -> anyhow::Result<()>
where
    F: FnMut(RecordBatch) -> anyhow::Result<()>,
{
    let file = File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    for batch in builder
        .with_batch_size(PARQUET_ROW_GROUP_SIZE)
        .build()
        .with_context(|| format!("failed to build Arrow reader for `{}`", path.display()))?
    {
        callback(batch.with_context(|| format!("failed to decode `{}`", path.display()))?)?;
    }
    Ok(())
}

fn string_array<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> anyhow::Result<&'a StringArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow!("expected string column `{name}`"))
}

fn string_array_by_name<'a>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a StringArray, ArrowError> {
    let index = batch.schema().index_of(name)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| ArrowError::CastError(format!("expected string column `{name}`")))
}

fn string_in_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    expected: HashSet<String>,
) -> RowFilter {
    let projection = ProjectionMask::columns(parquet_schema, [column]);
    let column = column.to_owned();
    let predicate = move |batch: RecordBatch| -> Result<BooleanArray, ArrowError> {
        let values = string_array_by_name(&batch, &column)?;
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            keep.push(!values.is_null(row) && expected.contains(values.value(row)));
        }
        Ok(BooleanArray::from(keep))
    };
    RowFilter::new(vec![Box::new(ArrowPredicateFn::new(projection, predicate))])
}

fn string_in_any_row_filter<const N: usize>(
    parquet_schema: &SchemaDescriptor,
    columns: [&str; N],
    expected: HashSet<String>,
) -> RowFilter {
    let projection = ProjectionMask::columns(parquet_schema, columns);
    let columns = columns.map(str::to_owned);
    let predicate = move |batch: RecordBatch| -> Result<BooleanArray, ArrowError> {
        let values = columns
            .iter()
            .map(|column| string_array_by_name(&batch, column))
            .collect::<Result<Vec<_>, _>>()?;
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            keep.push(
                values
                    .iter()
                    .any(|array| !array.is_null(row) && expected.contains(array.value(row))),
            );
        }
        Ok(BooleanArray::from(keep))
    };
    RowFilter::new(vec![Box::new(ArrowPredicateFn::new(projection, predicate))])
}

fn optional_string_array_by_name<'a>(
    batch: &'a RecordBatch,
    name: &str,
) -> anyhow::Result<Option<&'a StringArray>> {
    let schema = batch.schema();
    let Ok(index) = schema.index_of(name) else {
        return Ok(None);
    };
    string_array(batch, index, name).map(Some)
}

fn i64_array<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> anyhow::Result<&'a Int64Array> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| anyhow!("expected int64 column `{name}`"))
}

fn i32_array<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> anyhow::Result<&'a Int32Array> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int32Array>()
        .ok_or_else(|| anyhow!("expected int32 column `{name}`"))
}

fn f32_array<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> anyhow::Result<&'a Float32Array> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| anyhow!("expected float32 column `{name}`"))
}

fn list_array<'a>(
    batch: &'a RecordBatch,
    index: usize,
    name: &str,
) -> anyhow::Result<&'a ListArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| anyhow!("expected list column `{name}`"))
}

fn required_string_value(values: &StringArray, index: usize, name: &str) -> anyhow::Result<String> {
    if values.is_null(index) {
        bail!("missing required string column `{name}`");
    }
    Ok(values.value(index).to_owned())
}

fn optional_string_value(values: &StringArray, index: usize) -> Option<String> {
    (!values.is_null(index)).then(|| values.value(index).to_owned())
}

fn required_node_id_list_value(
    values: &ListArray,
    index: usize,
    name: &str,
) -> anyhow::Result<Vec<NodeId>> {
    if values.is_null(index) {
        return Ok(Vec::new());
    }
    let item_values = values.value(index);
    let node_ids = item_values
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| anyhow!("expected int64 elements in list column `{name}`"))?;
    (0..node_ids.len())
        .map(|item_index| i64_to_node_id(node_ids.value(item_index), name))
        .collect()
}

fn required_string_list_value(
    values: &ListArray,
    index: usize,
    name: &str,
) -> anyhow::Result<Vec<String>> {
    if values.is_null(index) {
        return Ok(Vec::new());
    }
    let item_values = values.value(index);
    let strings = item_values
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow!("expected string elements in list column `{name}`"))?;
    let mut out = Vec::with_capacity(strings.len());
    for item_index in 0..strings.len() {
        if strings.is_null(item_index) {
            bail!("missing string element {item_index} in list column `{name}`");
        }
        out.push(strings.value(item_index).to_owned());
    }
    Ok(out)
}

fn node_id_to_i64(node_id: NodeId) -> anyhow::Result<i64> {
    i64::try_from(node_id.get()).context("NodeId does not fit in Parquet Int64")
}

fn i64_to_node_id(value: i64, column: &str) -> anyhow::Result<NodeId> {
    Ok(NodeId(u64::try_from(value).with_context(|| {
        format!("negative NodeId in `{column}`")
    })?))
}

fn usize_to_i64(value: usize, column: &str) -> anyhow::Result<i64> {
    i64::try_from(value).with_context(|| format!("`{column}` does not fit in Int64"))
}

fn usize_to_i32(value: usize, column: &str) -> anyhow::Result<i32> {
    i32::try_from(value).with_context(|| format!("`{column}` does not fit in Int32"))
}

fn i64_to_usize(value: i64, column: &str) -> anyhow::Result<usize> {
    usize::try_from(value).with_context(|| format!("negative value in `{column}`"))
}

fn i32_to_usize(value: i32, column: &str) -> anyhow::Result<usize> {
    usize::try_from(value).with_context(|| format!("negative value in `{column}`"))
}

fn relation_to_str(relation: RelationKind) -> &'static str {
    match relation {
        RelationKind::Imports => "imports",
        RelationKind::Calls => "calls",
        RelationKind::Constructs => "constructs",
        RelationKind::Contains => "contains",
        RelationKind::Implements => "implements",
        RelationKind::Defines => "defines",
        RelationKind::References => "references",
        RelationKind::Extends => "extends",
        RelationKind::Links => "links",
        RelationKind::Touches => "touches",
        RelationKind::Produces => "produces",
        RelationKind::Consumes => "consumes",
        RelationKind::Binds => "binds",
        RelationKind::Emits => "emits",
    }
}

pub(crate) fn relation_from_str(value: &str) -> anyhow::Result<RelationKind> {
    match value {
        "imports" => Ok(RelationKind::Imports),
        "calls" => Ok(RelationKind::Calls),
        "constructs" => Ok(RelationKind::Constructs),
        "contains" => Ok(RelationKind::Contains),
        "implements" => Ok(RelationKind::Implements),
        "defines" => Ok(RelationKind::Defines),
        "references" => Ok(RelationKind::References),
        "extends" => Ok(RelationKind::Extends),
        "links" => Ok(RelationKind::Links),
        "touches" => Ok(RelationKind::Touches),
        "produces" => Ok(RelationKind::Produces),
        "consumes" => Ok(RelationKind::Consumes),
        "binds" => Ok(RelationKind::Binds),
        "emits" => Ok(RelationKind::Emits),
        _ => bail!("unknown relation `{value}`"),
    }
}

fn confidence_to_str(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::SyntaxExact => "syntax_exact",
        Confidence::Heuristic => "heuristic",
        Confidence::Unknown => "unknown",
    }
}

pub(crate) fn confidence_from_str(value: &str) -> anyhow::Result<Confidence> {
    match value {
        "syntax_exact" => Ok(Confidence::SyntaxExact),
        "heuristic" => Ok(Confidence::Heuristic),
        "unknown" => Ok(Confidence::Unknown),
        _ => bail!("unknown confidence `{value}`"),
    }
}

fn edge_kind_to_str(edge_kind: GraphEdgeKind) -> &'static str {
    match edge_kind {
        GraphEdgeKind::Calls => "calls",
        GraphEdgeKind::CallsDyn => "calls_dyn",
        GraphEdgeKind::ReferencesHof => "references_hof",
        GraphEdgeKind::ReferencesOther => "references_other",
        GraphEdgeKind::ReferencesAddress => "references_address",
    }
}

pub(crate) fn edge_kind_from_str(value: &str) -> anyhow::Result<GraphEdgeKind> {
    match value {
        "calls" => Ok(GraphEdgeKind::Calls),
        "calls_dyn" => Ok(GraphEdgeKind::CallsDyn),
        "references_hof" => Ok(GraphEdgeKind::ReferencesHof),
        "references_other" => Ok(GraphEdgeKind::ReferencesOther),
        "references_address" => Ok(GraphEdgeKind::ReferencesAddress),
        _ => bail!("unknown edge kind `{value}`"),
    }
}

type EndpointColumns = (Option<String>, Option<String>, Option<String>);

fn endpoint_columns(endpoint: &EdgeEndpoint) -> EndpointColumns {
    match endpoint {
        EdgeEndpoint::File { path } => (Some(git_path_to_b64(path)), None, None),
        EdgeEndpoint::Symbol { stable_symbol_id } => (None, Some(stable_symbol_id.clone()), None),
        EdgeEndpoint::Snapshot { key } => (
            None,
            Some(key.stable_symbol_id.clone()),
            Some(key.commit.clone()),
        ),
        EdgeEndpoint::Commit { sha } => (None, None, Some(sha.clone())),
    }
}

fn change_kind_rename_prev_columns(change_kind: Option<&ChangeKind>) -> EndpointColumns {
    match change_kind {
        Some(ChangeKind::RenamedFrom(RenamePrev::File(path))) => {
            (Some(git_path_to_b64(path)), None, None)
        }
        Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(key))) => (
            None,
            Some(key.stable_symbol_id.clone()),
            Some(key.commit.clone()),
        ),
        Some(ChangeKind::Added | ChangeKind::Modified | ChangeKind::Deleted) | None => {
            (None, None, None)
        }
    }
}

fn endpoint_from_columns(
    row: usize,
    prefix: &str,
    kind: &str,
    path_b64: Option<String>,
    stable_symbol_id: Option<String>,
    commit: Option<String>,
) -> anyhow::Result<EdgeEndpoint> {
    match endpoint_kind_from_str(kind)
        .with_context(|| format!("row {row}: invalid {prefix}_kind `{kind}`"))?
    {
        EdgeEndpointKind::File => {
            let context = format!("{prefix}_kind=file");
            require_column(
                row,
                path_b64.is_some(),
                &context,
                &format!("{prefix}_path_b64"),
            )?;
            reject_column(
                row,
                stable_symbol_id.is_some(),
                &context,
                &format!("{prefix}_stable_symbol_id"),
            )?;
            reject_column(row, commit.is_some(), &context, &format!("{prefix}_commit"))?;
            let path_b64 = path_b64.expect("validated path is present");
            Ok(EdgeEndpoint::File {
                path: git_path_from_b64(&path_b64)
                    .with_context(|| format!("row {row}: invalid {prefix}_path_b64"))?,
            })
        }
        EdgeEndpointKind::Symbol => {
            let context = format!("{prefix}_kind=symbol");
            reject_column(
                row,
                path_b64.is_some(),
                &context,
                &format!("{prefix}_path_b64"),
            )?;
            require_column(
                row,
                stable_symbol_id.is_some(),
                &context,
                &format!("{prefix}_stable_symbol_id"),
            )?;
            reject_column(row, commit.is_some(), &context, &format!("{prefix}_commit"))?;
            Ok(EdgeEndpoint::Symbol {
                stable_symbol_id: stable_symbol_id.expect("validated stable symbol id is present"),
            })
        }
        EdgeEndpointKind::Snapshot => {
            let context = format!("{prefix}_kind=snapshot");
            reject_column(
                row,
                path_b64.is_some(),
                &context,
                &format!("{prefix}_path_b64"),
            )?;
            require_column(
                row,
                stable_symbol_id.is_some(),
                &context,
                &format!("{prefix}_stable_symbol_id"),
            )?;
            require_column(row, commit.is_some(), &context, &format!("{prefix}_commit"))?;
            Ok(EdgeEndpoint::Snapshot {
                key: SnapshotKey {
                    stable_symbol_id: stable_symbol_id
                        .expect("validated stable symbol id is present"),
                    commit: commit.expect("validated commit is present"),
                },
            })
        }
        EdgeEndpointKind::Commit => {
            let context = format!("{prefix}_kind=commit");
            reject_column(
                row,
                path_b64.is_some(),
                &context,
                &format!("{prefix}_path_b64"),
            )?;
            reject_column(
                row,
                stable_symbol_id.is_some(),
                &context,
                &format!("{prefix}_stable_symbol_id"),
            )?;
            require_column(row, commit.is_some(), &context, &format!("{prefix}_commit"))?;
            Ok(EdgeEndpoint::Commit {
                sha: commit.expect("validated commit is present"),
            })
        }
    }
}

fn change_kind_from_columns(
    row: usize,
    change_kind: Option<String>,
    rename_prev_path_b64: Option<String>,
    rename_prev_stable_symbol_id: Option<String>,
    rename_prev_commit: Option<String>,
) -> anyhow::Result<Option<ChangeKind>> {
    let Some(change_kind) = change_kind else {
        reject_column(
            row,
            rename_prev_path_b64.is_some(),
            "change_kind absent",
            "rename_prev_path_b64",
        )?;
        reject_column(
            row,
            rename_prev_stable_symbol_id.is_some(),
            "change_kind absent",
            "rename_prev_stable_symbol_id",
        )?;
        reject_column(
            row,
            rename_prev_commit.is_some(),
            "change_kind absent",
            "rename_prev_commit",
        )?;
        return Ok(None);
    };

    Ok(Some(
        match change_kind_base_from_str(&change_kind)
            .with_context(|| format!("row {row}: invalid change_kind `{change_kind}`"))?
        {
            ChangeKindBase::Added => {
                reject_rename_prev_columns(
                    row,
                    rename_prev_path_b64.as_ref(),
                    rename_prev_stable_symbol_id.as_ref(),
                    rename_prev_commit.as_ref(),
                    &change_kind,
                )?;
                ChangeKind::Added
            }
            ChangeKindBase::Modified => {
                reject_rename_prev_columns(
                    row,
                    rename_prev_path_b64.as_ref(),
                    rename_prev_stable_symbol_id.as_ref(),
                    rename_prev_commit.as_ref(),
                    &change_kind,
                )?;
                ChangeKind::Modified
            }
            ChangeKindBase::Deleted => {
                reject_rename_prev_columns(
                    row,
                    rename_prev_path_b64.as_ref(),
                    rename_prev_stable_symbol_id.as_ref(),
                    rename_prev_commit.as_ref(),
                    &change_kind,
                )?;
                ChangeKind::Deleted
            }
            ChangeKindBase::RenamedFromFile => {
                let context = format!("change_kind `{change_kind}`");
                require_column(
                    row,
                    rename_prev_path_b64.is_some(),
                    &context,
                    "rename_prev_path_b64",
                )?;
                reject_column(
                    row,
                    rename_prev_stable_symbol_id.is_some(),
                    &context,
                    "rename_prev_stable_symbol_id",
                )?;
                reject_column(
                    row,
                    rename_prev_commit.is_some(),
                    &context,
                    "rename_prev_commit",
                )?;
                let path_b64 = rename_prev_path_b64.expect("validated path is present");
                ChangeKind::RenamedFrom(RenamePrev::File(
                    git_path_from_b64(&path_b64)
                        .with_context(|| format!("row {row}: invalid rename_prev_path_b64"))?,
                ))
            }
            ChangeKindBase::RenamedFromSymbol => {
                let context = format!("change_kind `{change_kind}`");
                reject_column(
                    row,
                    rename_prev_path_b64.is_some(),
                    &context,
                    "rename_prev_path_b64",
                )?;
                require_column(
                    row,
                    rename_prev_stable_symbol_id.is_some(),
                    &context,
                    "rename_prev_stable_symbol_id",
                )?;
                require_column(
                    row,
                    rename_prev_commit.is_some(),
                    &context,
                    "rename_prev_commit",
                )?;
                ChangeKind::RenamedFrom(RenamePrev::Symbol(SnapshotKey {
                    stable_symbol_id: rename_prev_stable_symbol_id
                        .expect("validated stable symbol id is present"),
                    commit: rename_prev_commit.expect("validated commit is present"),
                }))
            }
        },
    ))
}

fn reject_rename_prev_columns(
    row: usize,
    rename_prev_path_b64: Option<&String>,
    rename_prev_stable_symbol_id: Option<&String>,
    rename_prev_commit: Option<&String>,
    change_kind: &str,
) -> anyhow::Result<()> {
    if rename_prev_path_b64.is_some()
        || rename_prev_stable_symbol_id.is_some()
        || rename_prev_commit.is_some()
    {
        bail!("row {row}: change_kind `{change_kind}` must not include rename_prev columns");
    }
    Ok(())
}

fn require_column(row: usize, present: bool, context: &str, column: &str) -> anyhow::Result<()> {
    if !present {
        bail!("row {row}: {context} requires `{column}`");
    }
    Ok(())
}

fn reject_column(row: usize, present: bool, context: &str, column: &str) -> anyhow::Result<()> {
    if present {
        bail!("row {row}: {context} must not include `{column}`");
    }
    Ok(())
}

#[allow(dead_code)]
fn git_path_to_b64(path: &GitPath) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(path.as_bytes())
}

#[allow(dead_code)]
fn git_path_from_b64(value: &str) -> anyhow::Result<GitPath> {
    let bytes = match b64_decode_lenient(value) {
        Ok(bytes) => bytes,
        Err(err) => bail!("invalid GitPath base64 `{value}`: {err}"),
    };
    Ok(GitPath::from_bytes(bytes))
}

fn b64_decode_lenient(value: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let standard = value.replace('-', "+").replace('_', "/");
    let padding = (4 - standard.len() % 4) % 4;
    let mut padded = String::with_capacity(standard.len() + padding);
    padded.push_str(&standard);
    padded.extend(std::iter::repeat_n('=', padding));
    base64::engine::general_purpose::STANDARD.decode(padded)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeEndpointKind {
    File,
    Symbol,
    Snapshot,
    Commit,
}

#[allow(dead_code)]
fn endpoint_kind_to_str(endpoint: &EdgeEndpoint) -> &'static str {
    match endpoint {
        EdgeEndpoint::File { .. } => "file",
        EdgeEndpoint::Symbol { .. } => "symbol",
        EdgeEndpoint::Snapshot { .. } => "snapshot",
        EdgeEndpoint::Commit { .. } => "commit",
    }
}

#[allow(dead_code)]
fn endpoint_kind_from_str(value: &str) -> anyhow::Result<EdgeEndpointKind> {
    match value {
        "file" => Ok(EdgeEndpointKind::File),
        "symbol" => Ok(EdgeEndpointKind::Symbol),
        "snapshot" => Ok(EdgeEndpointKind::Snapshot),
        "commit" => Ok(EdgeEndpointKind::Commit),
        _ => bail!("unknown endpoint kind `{value}`"),
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKindBase {
    Added,
    Modified,
    Deleted,
    RenamedFromFile,
    RenamedFromSymbol,
}

#[allow(dead_code)]
fn change_kind_to_str(change: &ChangeKind) -> &'static str {
    match change {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::RenamedFrom(RenamePrev::File(_)) => "renamed_from_file",
        ChangeKind::RenamedFrom(RenamePrev::Symbol(_)) => "renamed_from_symbol",
    }
}

#[allow(dead_code)]
fn change_kind_base_from_str(value: &str) -> anyhow::Result<ChangeKindBase> {
    match value {
        "added" => Ok(ChangeKindBase::Added),
        "modified" => Ok(ChangeKindBase::Modified),
        "deleted" => Ok(ChangeKindBase::Deleted),
        "renamed_from_file" => Ok(ChangeKindBase::RenamedFromFile),
        "renamed_from_symbol" => Ok(ChangeKindBase::RenamedFromSymbol),
        _ => bail!("unknown change kind `{value}`"),
    }
}

#[cfg(test)]
mod helpers_test {
    use super::*;
    use crate::{ChangeKind, EdgeEndpoint, GitPath, RenamePrev, SnapshotKey};

    #[test]
    fn git_path_b64_round_trips_path_bytes() {
        let path = GitPath::from_bytes(b"src/non-utf8-\xff.rs".to_vec());

        let encoded = git_path_to_b64(&path);
        let decoded = git_path_from_b64(&encoded).expect("decode git path");

        assert_eq!(decoded, path);
    }

    #[test]
    fn edge_kind_codec_round_trips_references_address() {
        let kind = edge_kind_from_str("references_address").expect("decode references_address");
        assert_eq!(edge_kind_to_str(kind), "references_address");
    }

    #[test]
    fn endpoint_kind_round_trips_all_discriminators() {
        let snapshot_key = SnapshotKey {
            stable_symbol_id: "graph://symbol/sample".to_owned(),
            commit: "abc123".to_owned(),
        };
        let endpoints = [
            (
                EdgeEndpoint::File {
                    path: GitPath::from_bytes(b"src/lib.rs".to_vec()),
                },
                EdgeEndpointKind::File,
            ),
            (
                EdgeEndpoint::Symbol {
                    stable_symbol_id: "graph://symbol/sample".to_owned(),
                },
                EdgeEndpointKind::Symbol,
            ),
            (
                EdgeEndpoint::Snapshot { key: snapshot_key },
                EdgeEndpointKind::Snapshot,
            ),
            (
                EdgeEndpoint::Commit {
                    sha: "abc123".to_owned(),
                },
                EdgeEndpointKind::Commit,
            ),
        ];

        for (endpoint, expected) in endpoints {
            let encoded = endpoint_kind_to_str(&endpoint);
            let decoded = endpoint_kind_from_str(encoded).expect("decode endpoint kind");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn change_kind_round_trips_simple_base_discriminators() {
        let changes = [
            (ChangeKind::Added, ChangeKindBase::Added),
            (ChangeKind::Modified, ChangeKindBase::Modified),
            (ChangeKind::Deleted, ChangeKindBase::Deleted),
        ];

        for (change, expected) in changes {
            let encoded = change_kind_to_str(&change);
            let decoded = change_kind_base_from_str(encoded).expect("decode change kind");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn change_kind_round_trips_renamed_from_file_base() {
        let change = ChangeKind::RenamedFrom(RenamePrev::File(GitPath::from_bytes(
            b"src/old.rs".to_vec(),
        )));

        let encoded = change_kind_to_str(&change);
        let decoded = change_kind_base_from_str(encoded).expect("decode change kind");

        assert_eq!(decoded, ChangeKindBase::RenamedFromFile);
    }

    #[test]
    fn change_kind_round_trips_renamed_from_symbol_base() {
        let change = ChangeKind::RenamedFrom(RenamePrev::Symbol(SnapshotKey {
            stable_symbol_id: "graph://symbol/old".to_owned(),
            commit: "abc123".to_owned(),
        }));

        let encoded = change_kind_to_str(&change);
        let decoded = change_kind_base_from_str(encoded).expect("decode change kind");

        assert_eq!(decoded, ChangeKindBase::RenamedFromSymbol);
    }
}

#[cfg(test)]
mod parquet_temporal_test {
    use std::collections::BTreeMap;
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::store::build::artifact_from_facts;
    use crate::{
        build_facts, ChangeKind, CommitArtifact, EdgeEndpoint, RenamePrev, SnapshotKey,
        SymbolSnapshotArtifact, TemporalEdgeArtifact,
    };
    use parquet::file::reader::{FileReader as _, SerializedFileReader};

    #[test]
    fn write_worktree_delta_writes_only_changed_rows_without_touching_base_artifact(
    ) -> anyhow::Result<()> {
        let repo = tempfile::tempdir()?;
        let root = repo.path();
        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("src/a.rs"),
            "pub fn alpha() {}\npub fn alpha_helper() {}\n",
        )?;
        fs::write(root.join("src/b.rs"), "pub fn beta() {}\n")?;

        let prev = artifact_from_facts(&build_facts(root, None)?.0, root)?;

        let artifacts = tempfile::tempdir()?;
        let base_dir = artifacts.path().join("base");
        write_artifact_parquet(&prev, &base_dir, WriteOptions::default(), Vec::new())?;
        let base_before = snapshot_directory_bytes(&base_dir)?;

        fs::write(root.join("src/a.rs"), "pub fn alpha_changed() {}\n")?;

        let delta_dir = artifacts.path().join("delta");
        let summary = write_worktree_delta(&prev, root, &delta_dir)?;
        let manifest = read_artifact_header_parquet(&delta_dir)?;
        let delta = read_artifact_parquet(&delta_dir)?;

        assert_eq!(summary.changed_files, 1);
        assert_eq!(manifest.row_counts.file_manifests, 1);
        assert_eq!(manifest.row_counts.files, 1);
        assert_eq!(manifest.row_counts.nodes, delta.symbols.len());
        assert_eq!(delta.file_manifests.len(), 1);
        assert_eq!(delta.file_manifests[0].path, "src/a.rs");
        assert!(delta
            .symbols
            .iter()
            .all(|symbol| symbol.file_path == "src/a.rs"));
        assert!(delta
            .symbols
            .iter()
            .any(|symbol| symbol.entity_name == "alpha_changed"));
        assert!(!delta
            .symbols
            .iter()
            .any(|symbol| symbol.entity_name == "beta"));
        assert_eq!(summary.new_symbols, delta.symbols.len());
        assert_eq!(summary.tombstoned_symbols, delta.tombstones.len());
        assert!(delta
            .tombstones
            .iter()
            .any(|tombstone| tombstone.path == "src/a.rs"));

        assert_eq!(snapshot_directory_bytes(&base_dir)?, base_before);
        Ok(())
    }

    #[test]
    fn write_worktree_delta_sizes_bloom_filters_to_row_count() -> anyhow::Result<()> {
        let repo = tempfile::tempdir()?;
        let root = repo.path();
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n")?;

        let prev = artifact_from_facts(&build_facts(root, None)?.0, root)?;
        fs::write(root.join("src/a.rs"), "pub fn alpha_changed() {}\n")?;

        let artifacts = tempfile::tempdir()?;
        let delta_dir = artifacts.path().join("delta");
        write_worktree_delta(&prev, root, &delta_dir)?;

        let nodes_len = fs::metadata(delta_dir.join("nodes.parquet"))?.len();
        assert!(
            nodes_len < 200_000,
            "delta nodes.parquet was {nodes_len} bytes; bloom NDV should follow row count so a one-file delta stays payload-sized"
        );
        Ok(())
    }

    #[test]
    fn commits_round_trip() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let rows = vec![
            CommitArtifact {
                sha: "c1".to_owned(),
                parents: Vec::new(),
                author_time: 1_700_000_001,
                author_name: "Ada Lovelace".to_owned(),
                author_email: "ada@example.test".to_owned(),
                summary: "initial import".to_owned(),
            },
            CommitArtifact {
                sha: "c2".to_owned(),
                parents: vec!["c1".to_owned(), "merge-parent".to_owned()],
                author_time: 1_700_000_123,
                author_name: "Grace Hopper".to_owned(),
                author_email: "grace@example.test".to_owned(),
                summary: "merge branch".to_owned(),
            },
        ];

        let path = tempdir.path().join("commits.parquet");
        let written = write_commits(&path, &rows, &WriteOptions::default())?;
        let decoded = read_commits(&path, rows.len())?;

        assert_eq!(written, rows.len());
        assert_eq!(decoded, rows);
        Ok(())
    }

    #[test]
    fn commits_without_author_columns_load_with_empty_author_identity() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let path = tempdir.path().join("commits.parquet");

        write_table(
            &path,
            r"
            message schema {
              required binary sha (STRING);
              optional group parents (LIST) {
                repeated group list {
                  required binary element (STRING);
                }
              }
              required int64 author_time;
              required binary summary (STRING);
            }
            ",
            &["sha", "summary"],
            &["sha"],
            vec![
                ColumnData::RequiredString(vec!["legacy-c1".to_owned()]),
                ColumnData::RequiredListString(vec![vec!["legacy-parent".to_owned()]]),
                ColumnData::RequiredI64(vec![1_700_000_321]),
                ColumnData::RequiredString(vec!["legacy commit".to_owned()]),
            ],
        )?;

        let decoded = read_commits(&path, 1)?;

        assert_eq!(
            decoded,
            vec![CommitArtifact {
                sha: "legacy-c1".to_owned(),
                parents: vec!["legacy-parent".to_owned()],
                author_time: 1_700_000_321,
                author_name: String::new(),
                author_email: String::new(),
                summary: "legacy commit".to_owned(),
            }]
        );
        Ok(())
    }

    #[test]
    fn symbol_snapshots_round_trip() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let rows = vec![
            SymbolSnapshotArtifact {
                key: snapshot_key("sym-a", "c1"),
                file_path: GitPath::from_bytes(b"src/lib.rs".to_vec()),
                entity_name: "alpha".to_owned(),
                symbol_kind: "function".to_owned(),
                enclosing_scope: Some("mod root".to_owned()),
                byte_range: [10, 42],
                line_range: [2, 5],
                anchor_hash: "anchor-a".to_owned(),
                tokens: vec!["alpha".to_owned(), "body".to_owned()],
            },
            SymbolSnapshotArtifact {
                key: snapshot_key("sym-b", "c2"),
                file_path: GitPath::from_bytes(b"src/non-utf8-\xff.rs".to_vec()),
                entity_name: "Beta".to_owned(),
                symbol_kind: "struct".to_owned(),
                enclosing_scope: None,
                byte_range: [100, 180],
                line_range: [9, 17],
                anchor_hash: "anchor-b".to_owned(),
                tokens: Vec::new(),
            },
        ];

        let path = tempdir.path().join("symbol_snapshots.parquet");
        let written = write_symbol_snapshots(&path, &rows, &WriteOptions::default())?;
        let decoded = read_symbol_snapshots_file(&path, rows.len())?;

        assert_eq!(written, rows.len());
        assert_eq!(decoded, rows);
        Ok(())
    }

    #[test]
    fn temporal_edges_round_trip() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let old_symbol = snapshot_key("sym-old", "c1");
        let new_symbol = snapshot_key("sym-new", "c2");
        let rows = vec![
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Commit {
                    sha: "c3".to_owned(),
                },
                target: EdgeEndpoint::File {
                    path: GitPath::from_bytes(b"src/new-name.rs".to_vec()),
                },
                relation: RelationKind::Touches,
                parent: Some("c2".to_owned()),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::File(
                    GitPath::from_bytes(b"src/old-name.rs".to_vec()),
                ))),
            },
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot {
                    key: old_symbol.clone(),
                },
                target: EdgeEndpoint::Snapshot {
                    key: new_symbol.clone(),
                },
                relation: RelationKind::Touches,
                parent: Some("c1".to_owned()),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(old_symbol))),
            },
        ];

        let path = tempdir.path().join("temporal_edges.parquet");
        let written = write_temporal_edges(&path, &rows, &WriteOptions::default())?;
        let decoded = read_temporal_edges_file(&path, rows.len())?;

        assert_eq!(written, rows.len());
        assert_eq!(decoded, rows);
        Ok(())
    }

    #[test]
    fn temporal_collections_round_trip() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("temporal-collections-round-trip");
        let old_symbol = snapshot_key("sym-old", "c1");
        let new_symbol = snapshot_key("sym-new", "c2");

        artifact.commits = vec![
            CommitArtifact {
                sha: "c1".to_owned(),
                parents: Vec::new(),
                author_time: 1_700_000_001,
                author_name: "Ada Lovelace".to_owned(),
                author_email: "ada@example.test".to_owned(),
                summary: "initial import".to_owned(),
            },
            CommitArtifact {
                sha: "c2".to_owned(),
                parents: vec!["c1".to_owned()],
                author_time: 1_700_000_123,
                author_name: "Grace Hopper".to_owned(),
                author_email: "grace@example.test".to_owned(),
                summary: "rename alpha to beta".to_owned(),
            },
        ];
        artifact.symbol_snapshots = vec![
            SymbolSnapshotArtifact {
                key: new_symbol.clone(),
                file_path: GitPath::from_bytes(b"src/new.rs".to_vec()),
                entity_name: "beta".to_owned(),
                symbol_kind: "function".to_owned(),
                enclosing_scope: Some("mod root".to_owned()),
                byte_range: [100, 150],
                line_range: [9, 14],
                anchor_hash: "anchor-new".to_owned(),
                tokens: vec!["beta".to_owned(), "body".to_owned()],
            },
            SymbolSnapshotArtifact {
                key: old_symbol.clone(),
                file_path: GitPath::from_bytes(b"src/old.rs".to_vec()),
                entity_name: "alpha".to_owned(),
                symbol_kind: "function".to_owned(),
                enclosing_scope: Some("mod root".to_owned()),
                byte_range: [10, 42],
                line_range: [2, 5],
                anchor_hash: "anchor-old".to_owned(),
                tokens: vec!["alpha".to_owned(), "body".to_owned()],
            },
        ];
        artifact.temporal_edges = vec![
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot {
                    key: old_symbol.clone(),
                },
                target: EdgeEndpoint::Snapshot {
                    key: new_symbol.clone(),
                },
                relation: RelationKind::Touches,
                parent: Some("c1".to_owned()),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(old_symbol))),
            },
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Commit {
                    sha: "c2".to_owned(),
                },
                target: EdgeEndpoint::Snapshot {
                    key: new_symbol.clone(),
                },
                relation: RelationKind::Touches,
                parent: Some("c1".to_owned()),
                change_kind: Some(ChangeKind::Added),
            },
        ];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        let manifest = read_artifact_header_parquet(&dir)?;
        let decoded = read_artifact_parquet(&dir)?;

        assert_eq!(manifest.row_counts.commits, artifact.commits.len());
        assert_eq!(
            manifest.row_counts.symbol_snapshots,
            artifact.symbol_snapshots.len()
        );
        assert_eq!(
            manifest.row_counts.temporal_edges,
            artifact.temporal_edges.len()
        );
        assert_eq!(decoded.commits, artifact.commits);
        assert_eq!(decoded.symbol_snapshots, artifact.symbol_snapshots);
        assert_eq!(decoded.temporal_edges, artifact.temporal_edges);
        Ok(())
    }

    #[test]
    fn symbol_history_reader_filters_temporal_shards_and_expands_renames() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("symbol-history-filtered-temporal-shards");
        let old_symbol = snapshot_key("sym-old", "c1");
        let mid_symbol = snapshot_key("sym-mid", "c2");
        let new_symbol = snapshot_key("sym-new", "c3");
        let other_old_symbol = snapshot_key("sym-other-old", "c1");
        let other_new_symbol = snapshot_key("sym-other-new", "c3");

        artifact.commits = vec![
            CommitArtifact {
                sha: "c1".to_owned(),
                parents: Vec::new(),
                author_time: 1,
                author_name: String::new(),
                author_email: String::new(),
                summary: "initial".to_owned(),
            },
            CommitArtifact {
                sha: "c2".to_owned(),
                parents: vec!["c1".to_owned()],
                author_time: 2,
                author_name: String::new(),
                author_email: String::new(),
                summary: "first rename".to_owned(),
            },
            CommitArtifact {
                sha: "c3".to_owned(),
                parents: vec!["c2".to_owned()],
                author_time: 3,
                author_name: String::new(),
                author_email: String::new(),
                summary: "second rename".to_owned(),
            },
        ];

        let old_snapshot = SymbolSnapshotArtifact {
            key: old_symbol.clone(),
            file_path: GitPath::from_bytes(b"src/old.rs".to_vec()),
            entity_name: "alpha".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 2],
            anchor_hash: "anchor-old".to_owned(),
            tokens: Vec::new(),
        };
        let mid_snapshot = SymbolSnapshotArtifact {
            key: mid_symbol.clone(),
            file_path: GitPath::from_bytes(b"src/mid.rs".to_vec()),
            entity_name: "beta".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [10, 20],
            line_range: [3, 4],
            anchor_hash: "anchor-mid".to_owned(),
            tokens: Vec::new(),
        };
        let new_snapshot = SymbolSnapshotArtifact {
            key: new_symbol.clone(),
            file_path: GitPath::from_bytes(b"src/new.rs".to_vec()),
            entity_name: "gamma".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [20, 30],
            line_range: [5, 6],
            anchor_hash: "anchor-new".to_owned(),
            tokens: Vec::new(),
        };
        let other_old_snapshot = SymbolSnapshotArtifact {
            key: other_old_symbol.clone(),
            file_path: GitPath::from_bytes(b"src/other.rs".to_vec()),
            entity_name: "other_old".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [30, 40],
            line_range: [7, 8],
            anchor_hash: "anchor-other-old".to_owned(),
            tokens: Vec::new(),
        };
        let other_new_snapshot = SymbolSnapshotArtifact {
            key: other_new_symbol.clone(),
            file_path: GitPath::from_bytes(b"src/other_new.rs".to_vec()),
            entity_name: "other_new".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [40, 50],
            line_range: [9, 10],
            anchor_hash: "anchor-other-new".to_owned(),
            tokens: Vec::new(),
        };
        artifact.symbol_snapshots = vec![
            old_snapshot.clone(),
            mid_snapshot.clone(),
            new_snapshot.clone(),
            other_old_snapshot,
            other_new_snapshot,
        ];

        let rename_old_to_mid = TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot {
                key: old_symbol.clone(),
            },
            target: EdgeEndpoint::Snapshot {
                key: mid_symbol.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c1".to_owned()),
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(
                old_symbol.clone(),
            ))),
        };
        let rename_mid_to_new = TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot {
                key: mid_symbol.clone(),
            },
            target: EdgeEndpoint::Snapshot {
                key: new_symbol.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c2".to_owned()),
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(
                mid_symbol.clone(),
            ))),
        };
        let touch_new = TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c3".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: new_symbol.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c2".to_owned()),
            change_kind: Some(ChangeKind::Modified),
        };
        let rename_other = TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot {
                key: other_old_symbol.clone(),
            },
            target: EdgeEndpoint::Snapshot {
                key: other_new_symbol.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c1".to_owned()),
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(
                other_old_symbol.clone(),
            ))),
        };
        let touch_other = TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c3".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: other_new_symbol,
            },
            relation: RelationKind::Touches,
            parent: Some("c2".to_owned()),
            change_kind: Some(ChangeKind::Added),
        };
        artifact.temporal_edges = vec![
            rename_old_to_mid.clone(),
            rename_mid_to_new.clone(),
            touch_new.clone(),
            rename_other,
            touch_other,
        ];
        artifact.diagnostics = vec!["diagnostic excluded from reduced history".to_owned()];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        let manifest = read_artifact_header_parquet(&dir)?;

        let mid_snapshots = read_symbol_snapshots_for_symbols(
            &dir,
            &manifest,
            &std::collections::HashSet::from(["sym-mid".to_owned()]),
        )?;
        assert_eq!(mid_snapshots, vec![mid_snapshot.clone()]);

        let old_edges = read_temporal_edges_for_symbols(
            &dir,
            &manifest,
            &std::collections::HashSet::from(["sym-old".to_owned()]),
        )?;
        assert_eq!(old_edges, vec![rename_old_to_mid.clone()]);

        let history = read_temporal_artifact_parquet_for_symbol_history(&dir, "sym-new")?;
        assert_eq!(history.commits, artifact.commits);
        assert_eq!(
            history.symbol_snapshots,
            vec![old_snapshot, mid_snapshot, new_snapshot]
        );
        assert_eq!(
            history.temporal_edges,
            vec![rename_old_to_mid, rename_mid_to_new, touch_new]
        );
        assert!(history.diagnostics.is_empty());
        assert!(history.files.is_empty());
        assert!(history.symbols.is_empty());
        assert!(history.edges.is_empty());
        Ok(())
    }

    #[test]
    fn fallback_temporal_write_emits_single_shard_layout() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("fallback-temporal-shard-layout");
        artifact.commits = vec![
            CommitArtifact {
                sha: "c1".to_owned(),
                parents: Vec::new(),
                author_time: 30,
                author_name: String::new(),
                author_email: String::new(),
                summary: "newer".to_owned(),
            },
            CommitArtifact {
                sha: "c2".to_owned(),
                parents: vec!["c1".to_owned()],
                author_time: 10,
                author_name: String::new(),
                author_email: String::new(),
                summary: "older clock".to_owned(),
            },
        ];
        let snapshot = SymbolSnapshotArtifact {
            key: snapshot_key("sym-a", "c2"),
            file_path: GitPath::from_bytes(b"src/lib.rs".to_vec()),
            entity_name: "alpha".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 12],
            line_range: [1, 2],
            anchor_hash: "anchor-a".to_owned(),
            tokens: vec!["alpha".to_owned()],
        };
        artifact.symbol_snapshots = vec![snapshot.clone()];
        artifact.temporal_edges = vec![TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c2".to_owned(),
            },
            target: EdgeEndpoint::Snapshot { key: snapshot.key },
            relation: RelationKind::Touches,
            parent: Some("c1".to_owned()),
            change_kind: Some(ChangeKind::Added),
        }];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        let manifest = read_artifact_header_parquet(&dir)?;
        let decoded = read_temporal_artifact_parquet(&dir)?;

        assert!(dir.join("symbol_snapshots/00000.parquet").is_file());
        assert!(dir.join("temporal_edges/00000.parquet").is_file());
        assert!(!dir.join("symbol_snapshots.parquet").exists());
        assert!(!dir.join("temporal_edges.parquet").exists());
        assert_eq!(manifest.temporal_shards.len(), 1);
        assert_eq!(manifest.temporal_shards[0].shard_idx, 0);
        assert_eq!(manifest.temporal_shards[0].commit_time_min, 10);
        assert_eq!(manifest.temporal_shards[0].commit_time_max, 30);
        assert_eq!(manifest.temporal_shards[0].row_count_edges, 1);
        assert_eq!(manifest.temporal_shards[0].row_count_snapshots, 1);
        assert_eq!(decoded.symbol_snapshots, artifact.symbol_snapshots);
        assert_eq!(decoded.temporal_edges, artifact.temporal_edges);
        Ok(())
    }

    #[test]
    fn temporal_reader_chains_manifest_shards_in_order() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("temporal-reader-chains-shards");
        artifact.commits = vec![
            CommitArtifact {
                sha: "c1".to_owned(),
                parents: Vec::new(),
                author_time: 1,
                author_name: String::new(),
                author_email: String::new(),
                summary: "first".to_owned(),
            },
            CommitArtifact {
                sha: "c2".to_owned(),
                parents: vec!["c1".to_owned()],
                author_time: 2,
                author_name: String::new(),
                author_email: String::new(),
                summary: "second".to_owned(),
            },
        ];

        let shard0_snapshot = SymbolSnapshotArtifact {
            key: snapshot_key("sym-a", "c1"),
            file_path: GitPath::from_bytes(b"src/a.rs".to_vec()),
            entity_name: "alpha".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 1],
            line_range: [1, 1],
            anchor_hash: "anchor-a".to_owned(),
            tokens: Vec::new(),
        };
        let shard1_snapshot = SymbolSnapshotArtifact {
            key: snapshot_key("sym-b", "c2"),
            file_path: GitPath::from_bytes(b"src/b.rs".to_vec()),
            entity_name: "beta".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 1],
            line_range: [1, 1],
            anchor_hash: "anchor-b".to_owned(),
            tokens: Vec::new(),
        };
        let shard0_edge = TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c1".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: shard0_snapshot.key.clone(),
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::Added),
        };
        let shard1_edge = TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c2".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: shard1_snapshot.key.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c1".to_owned()),
            change_kind: Some(ChangeKind::Added),
        };

        fs::create_dir_all(tempdir.path().join("symbol_snapshots"))?;
        fs::create_dir_all(tempdir.path().join("temporal_edges"))?;
        write_symbol_snapshots(
            &tempdir.path().join("symbol_snapshots/00000.parquet"),
            &[shard0_snapshot.clone()],
            &WriteOptions::default(),
        )?;
        write_symbol_snapshots(
            &tempdir.path().join("symbol_snapshots/00001.parquet"),
            &[shard1_snapshot.clone()],
            &WriteOptions::default(),
        )?;
        write_temporal_edges(
            &tempdir.path().join("temporal_edges/00000.parquet"),
            &[shard0_edge.clone()],
            &WriteOptions::default(),
        )?;
        write_temporal_edges(
            &tempdir.path().join("temporal_edges/00001.parquet"),
            &[shard1_edge.clone()],
            &WriteOptions::default(),
        )?;

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            vec![
                ShardIndexEntry {
                    shard_idx: 0,
                    commit_time_min: 1,
                    commit_time_max: 1,
                    row_count_edges: 1,
                    row_count_snapshots: 1,
                },
                ShardIndexEntry {
                    shard_idx: 1,
                    commit_time_min: 2,
                    commit_time_max: 2,
                    row_count_edges: 1,
                    row_count_snapshots: 1,
                },
            ],
        )?;

        let decoded = read_temporal_artifact_parquet(&dir)?;
        let mut streamed_rows = Vec::new();
        stream_temporal_artifact_parquet(&dir, |_table, batch| {
            streamed_rows.push(batch.num_rows());
            Ok(())
        })?;

        assert_eq!(
            decoded.symbol_snapshots,
            vec![shard0_snapshot, shard1_snapshot]
        );
        assert_eq!(decoded.temporal_edges, vec![shard0_edge, shard1_edge]);
        assert_eq!(streamed_rows, vec![1, 1, 1, 1]);
        Ok(())
    }

    #[test]
    fn temporal_reader_rejects_legacy_edges_without_manifest_shards() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let artifact = empty_artifact("legacy-temporal-edge-defensive-error");
        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        fs::write(dir.join("temporal_edges.parquet"), b"legacy")?;

        let err = read_temporal_artifact_parquet(&dir)
            .expect_err("legacy temporal_edges.parquet must be rejected");

        assert!(
            err.to_string().contains("temporal_edges.parquet"),
            "error should mention legacy temporal_edges.parquet: {err:#}"
        );
        Ok(())
    }

    #[test]
    fn parquet_slim_reader_skips_heavy_tables_but_loads_files_and_symbols() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("slim-reader-skips-heavy-tables");
        let snapshot = snapshot_key("sym-a", "c1");

        artifact.files = vec![GraphFileArtifact {
            stable_file_id: "file-a".to_owned(),
            file_path: "src/lib.rs".to_owned(),
        }];
        artifact.file_node_ids = vec![NodeId(1)];
        artifact.file_manifests = vec![GraphFileManifestEntry {
            stable_file_id: "file-a".to_owned(),
            path: "src/lib.rs".to_owned(),
            content_oid: "content-a".to_owned(),
            node_ids: vec![NodeId(2)],
        }];
        artifact.symbols = vec![GraphSymbolArtifact {
            stable_symbol_id: "sym-a".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            byte_range: [0, 12],
            line_range: [1, 2],
            entity_name: "alpha".to_owned(),
            qualified_name: "crate::alpha".to_owned(),
            symbol_kind: "function".to_owned(),
            anchor_hash: "anchor-a".to_owned(),
            enclosing_scope: None,
        }];
        artifact.symbol_node_ids = vec![NodeId(2)];
        artifact.edges = vec![GraphEdgeArtifact {
            source_stable_symbol_id: "sym-a".to_owned(),
            target_stable_symbol_id: Some("sym-a".to_owned()),
            target_label: Some("alpha".to_owned()),
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation: RelationKind::Calls,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::Calls),
            bind_method: None,
        }];
        artifact.tombstones = vec![GraphTombstoneEntry {
            path: "src/deleted.rs".to_owned(),
            stable_file_id: "file-deleted".to_owned(),
        }];
        artifact.diagnostics = vec!["diagnostic skipped by slim reader".to_owned()];
        artifact.commits = vec![CommitArtifact {
            sha: "c1".to_owned(),
            parents: Vec::new(),
            author_time: 1_700_000_001,
            author_name: String::new(),
            author_email: String::new(),
            summary: "initial import".to_owned(),
        }];
        artifact.symbol_snapshots = vec![SymbolSnapshotArtifact {
            key: snapshot.clone(),
            file_path: GitPath::from_bytes(b"src/lib.rs".to_vec()),
            entity_name: "alpha".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 12],
            line_range: [1, 2],
            anchor_hash: "anchor-a".to_owned(),
            tokens: vec!["alpha".to_owned()],
        }];
        artifact.temporal_edges = vec![TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c1".to_owned(),
            },
            target: EdgeEndpoint::Snapshot { key: snapshot },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::Added),
        }];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        for table in [
            "edges.parquet",
            "edges_unresolved.parquet",
            "edges_by_dst.parquet",
            "file_manifests.parquet",
            "commits.parquet",
            "diagnostics.parquet",
        ] {
            fs::remove_file(dir.join(table))?;
        }
        fs::remove_dir_all(dir.join("symbol_snapshots"))?;
        fs::remove_dir_all(dir.join("temporal_edges"))?;

        let decoded = read_artifact_parquet_slim(&dir)?;

        assert_eq!(decoded.files, artifact.files);
        assert_eq!(decoded.file_node_ids, artifact.file_node_ids);
        assert_eq!(decoded.symbols, artifact.symbols);
        assert_eq!(decoded.symbol_node_ids, artifact.symbol_node_ids);
        assert_eq!(decoded.tombstones, artifact.tombstones);
        assert!(decoded.file_manifests.is_empty());
        assert!(decoded.edges.is_empty());
        assert!(decoded.diagnostics.is_empty());
        assert!(decoded.commits.is_empty());
        assert!(decoded.symbol_snapshots.is_empty());
        assert!(decoded.temporal_edges.is_empty());
        Ok(())
    }

    #[test]
    fn parquet_writer_emits_node_stats_bloom_filter_and_clusters_paths() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("node-stats-bloom-cluster-paths");
        artifact.symbols = vec![
            GraphSymbolArtifact {
                stable_symbol_id: "sym-z".to_owned(),
                file_path: "src/a.rs".to_owned(),
                byte_range: [0, 8],
                line_range: [1, 2],
                entity_name: "zeta".to_owned(),
                qualified_name: "crate::zeta".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-z".to_owned(),
                enclosing_scope: None,
            },
            GraphSymbolArtifact {
                stable_symbol_id: "sym-a".to_owned(),
                file_path: "src/z.rs".to_owned(),
                byte_range: [10, 18],
                line_range: [3, 4],
                entity_name: "alpha".to_owned(),
                qualified_name: "crate::alpha".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-a".to_owned(),
                enclosing_scope: None,
            },
            GraphSymbolArtifact {
                stable_symbol_id: "sym-m".to_owned(),
                file_path: "src/m.rs".to_owned(),
                byte_range: [20, 28],
                line_range: [5, 6],
                entity_name: "middle".to_owned(),
                qualified_name: "crate::middle".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-m".to_owned(),
                enclosing_scope: None,
            },
        ];
        artifact.symbol_node_ids = vec![NodeId(1), NodeId(2), NodeId(3)];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        let reader = SerializedFileReader::new(File::open(dir.join("nodes.parquet"))?)?;
        let metadata = reader.metadata();
        let file_path_column = metadata.row_group(0).column(2);
        let stats = file_path_column
            .statistics()
            .expect("file_path should have row-group statistics");

        assert!(stats.min_bytes_opt().is_some());
        assert!(stats.max_bytes_opt().is_some());
        assert!(file_path_column.bloom_filter_offset().is_some());

        let decoded = read_artifact_parquet(&dir)?;
        let file_paths = decoded
            .symbols
            .iter()
            .map(|symbol| symbol.file_path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(file_paths, vec!["src/a.rs", "src/m.rs", "src/z.rs"]);
        Ok(())
    }

    #[test]
    fn parquet_bloom_filter_size_is_row_group_local() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let one_group_path = tempdir.path().join("one-group.parquet");
        let two_group_path = tempdir.path().join("two-groups.parquet");
        let write_keys = |path: &Path, row_count: usize| {
            write_table(
                path,
                r"
                message schema {
                  required binary key (STRING);
                }
                ",
                &[],
                &["key"],
                vec![ColumnData::RequiredString(
                    (0..row_count).map(|row| format!("key-{row:08}")).collect(),
                )],
            )
        };
        write_keys(&one_group_path, PARQUET_ROW_GROUP_SIZE)?;
        write_keys(&two_group_path, PARQUET_ROW_GROUP_SIZE * 2)?;

        let one_group = SerializedFileReader::new(File::open(one_group_path)?)?;
        let two_groups = SerializedFileReader::new(File::open(two_group_path)?)?;
        let one_group_bloom_bytes = one_group
            .metadata()
            .row_group(0)
            .column(0)
            .bloom_filter_length()
            .expect("one-group table should write a Bloom filter");
        let two_group_bloom_bytes = two_groups
            .metadata()
            .row_group(0)
            .column(0)
            .bloom_filter_length()
            .expect("two-group table should write a Bloom filter");

        assert_eq!(
            two_group_bloom_bytes, one_group_bloom_bytes,
            "each row group's Bloom filter should be sized from that group's maximum NDV"
        );
        Ok(())
    }

    #[test]
    fn diagnostics_round_trip() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut artifact = empty_artifact("diagnostics-round-trip");
        artifact.diagnostics = vec![
            "parse_failed path=src/lib.rs sha=abc123".to_owned(),
            "tree_sitter_error path=src/main.rs line=42".to_owned(),
            "ambiguous_rename stable_symbol_id=sym-main".to_owned(),
        ];

        let dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;
        let manifest = read_artifact_header_parquet(&dir)?;
        let decoded = read_artifact_parquet(&dir)?;

        assert_eq!(manifest.row_counts.diagnostics, artifact.diagnostics.len());
        assert_eq!(decoded.diagnostics, artifact.diagnostics);
        Ok(())
    }

    #[test]
    fn back_compat_loads_artifact_without_temporal_tables() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let dir = write_artifact_parquet(
            &empty_artifact("back-compat-without-temporal-tables"),
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )?;

        assert!(!dir.join("commits.parquet").exists());
        assert!(!dir.join("symbol_snapshots.parquet").exists());
        assert!(!dir.join("temporal_edges.parquet").exists());
        assert!(!dir.join("diagnostics.parquet").exists());

        let manifest_path = dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        let row_counts = manifest
            .get_mut("row_counts")
            .and_then(serde_json::Value::as_object_mut)
            .expect("row_counts must be an object");
        row_counts.remove("commits");
        row_counts.remove("symbol_snapshots");
        row_counts.remove("temporal_edges");
        row_counts.remove("diagnostics");
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

        let manifest = read_artifact_header_parquet(&dir)?;
        let decoded = read_artifact_parquet(&dir)?;

        assert_eq!(manifest.row_counts.commits, 0);
        assert_eq!(manifest.row_counts.symbol_snapshots, 0);
        assert_eq!(manifest.row_counts.temporal_edges, 0);
        assert_eq!(manifest.row_counts.diagnostics, 0);
        assert!(decoded.commits.is_empty());
        assert!(decoded.symbol_snapshots.is_empty());
        assert!(decoded.temporal_edges.is_empty());
        assert!(decoded.diagnostics.is_empty());
        Ok(())
    }

    #[test]
    fn manifest_defaults_missing_temporal_shards() -> anyhow::Result<()> {
        let manifest_json = serde_json::json!({
            "graph_index_version": "2",
            "schema_version": "spur-graph-schema-v6",
            "manifest_version": "manifest-version",
            "graph_content_hash": "content-hash",
            "indexed_commit_oid": null,
            "extractor_version": "extractor-version",
            "complete": true,
            "row_counts": {
                "nodes": 0,
                "edges": 0,
                "edges_by_dst": null,
                "edges_unresolved": 0,
                "files": 0,
                "file_manifests": 0,
                "tombstones": 0
            },
            "parquet_writer": {
                "compression": "zstd-3",
                "row_group_size": PARQUET_ROW_GROUP_SIZE
            },
            "edges_by_dst_present": false
        });

        let manifest: GraphArtifactManifest = serde_json::from_value(manifest_json)?;

        assert!(manifest.temporal_shards.is_empty());
        Ok(())
    }

    fn snapshot_key(stable_symbol_id: &str, commit: &str) -> SnapshotKey {
        SnapshotKey {
            stable_symbol_id: stable_symbol_id.to_owned(),
            commit: commit.to_owned(),
        }
    }

    fn snapshot_directory_bytes(dir: &Path) -> anyhow::Result<BTreeMap<PathBuf, Vec<u8>>> {
        fn visit(
            root: &Path,
            dir: &Path,
            snapshot: &mut BTreeMap<PathBuf, Vec<u8>>,
        ) -> anyhow::Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, snapshot)?;
                } else {
                    snapshot.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(&path)?);
                }
            }
            Ok(())
        }

        let mut snapshot = BTreeMap::new();
        visit(dir, dir, &mut snapshot)?;
        Ok(snapshot)
    }

    fn empty_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: crate::schema::GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test-manifest".to_owned(),
            graph_content_hash: graph_content_hash.to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: Vec::new(),
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }
}
