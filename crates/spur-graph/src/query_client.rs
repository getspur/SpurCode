//! Query clients for code graph MCP request handlers.
//!
//! `GraphQueryClient` is the migration boundary from full `GraphIndexArtifact`
//! residency to query-scoped access. `InMemoryClient` preserves the legacy
//! artifact-backed behavior for tests and JSON artifacts, while `ParquetClient`
//! serves hot-path MCP operations directly from Parquet projections so handlers
//! such as `code_search`, `code_read_symbol`, `code_callers`, and `code_callees`
//! do not deserialize and retain the full graph artifact per request.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, bail, Context as _};
use arrow_array::{
    Array as _, BooleanArray, Int32Array, Int64Array, ListArray, RecordBatch, StringArray,
};
use arrow_schema::ArrowError;
use arrow_select::concat::concat_batches;
use futures::TryStreamExt as _;
use globset::Glob;
use parquet::arrow::arrow_reader::{
    ArrowPredicateFn, ArrowReaderMetadata, ParquetRecordBatchReaderBuilder, RowFilter,
};
use parquet::arrow::async_reader::{AsyncFileReader, ParquetRecordBatchStreamBuilder};
use parquet::arrow::ProjectionMask;
use parquet::schema::types::SchemaDescriptor;

use crate::query_hot_index::{HotAdjacencyIndex, HotQueryIndex};
use crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
use crate::store::parquet::{
    read_current_query_edges_parquet, read_filtered_projected_batches, read_projected_batches,
    read_temporal_artifact_parquet, read_temporal_artifact_parquet_for_symbol_history_with_cache,
    ParquetMetadataCache, StringPruningPredicate, PARQUET_ROW_GROUP_SIZE,
};
use crate::temporal::{symbol_history_indexed, GitSha, TemporalIndex};
use crate::{
    artifact_from_facts_for_paths, build_facts_for_paths, compare_symbols, find_callee_edges,
    find_caller_edges, internal_unbounded_search_options, limited_search_result,
    read_artifact_header_parquet, resolve_selector, search_symbols, ChangeKind,
    CommitIndexArtifact, GraphArtifactManifest, GraphEdgeArtifact, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphSymbolArtifact,
    OwnedCalleeRecord, OwnedCallerRecord, RelationKind, SearchOptions, SearchResult, SearchSymbol,
    SelectorResolution, SnapshotKey, CODE_SYMBOL_URI_PREFIX,
};
use crate::{CandidateRow, NodeId, ResolvedSymbol, SearchFilters, SearchMode};

use crate::store::macro_invocations::MacroQueryTables;
use crate::{
    MacroEvidenceAvailability, MacroEvidenceSnapshot, MacroEvidenceSourceScope,
    MacroInvocationFilter, MacroInvocationQueryResult,
};

pub type CodeSelectorResolution = SelectorResolution;

/// Validated snapshot metadata, retained independently of decoded query rows.
struct MacroQueryContext {
    snapshot: Option<MacroEvidenceSnapshot>,
    graph_files: HashSet<String>,
    notebook_owners: Vec<String>,
}

impl MacroQueryContext {
    fn new(artifact: &GraphIndexArtifact) -> anyhow::Result<Self> {
        if let Some(set) = &artifact.macro_invocations {
            set.validate_stored(artifact)?;
        }
        Ok(Self {
            snapshot: artifact
                .macro_invocations
                .as_ref()
                .map(|set| MacroEvidenceSnapshot {
                    graph_content_hash: set.graph_content_hash.clone(),
                    schema_version: set.schema_version.clone(),
                    extractor_version: set.extractor_version.clone(),
                    source_oids: artifact
                        .file_manifests
                        .iter()
                        .filter(|f| physical_rust_path(&f.path))
                        .map(|f| (f.path.clone(), f.content_oid.clone()))
                        .collect(),
                }),
            graph_files: artifact.files.iter().map(|f| f.file_path.clone()).collect(),
            notebook_owners: artifact
                .file_manifests
                .iter()
                .filter(|f| {
                    Path::new(&f.path)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ipynb"))
                })
                .map(|f| f.path.clone())
                .collect(),
        })
    }

    fn result(
        &self,
        filter: &MacroInvocationFilter,
        caller_file: Option<&str>,
    ) -> anyhow::Result<MacroInvocationQueryResult> {
        filter.validate_shape()?;
        let Some(snapshot) = &self.snapshot else {
            return Ok(MacroInvocationQueryResult::unavailable());
        };
        filter.validate_source(&snapshot.source_oids)?;
        let unsupported = filter
            .file_path
            .as_deref()
            .or(caller_file)
            .is_some_and(|p| {
                // A physical source owner wins even when its spelling resembles a
                // notebook identity. Virtual identities are bound to known owners.
                !snapshot.source_oids.contains_key(p)
                    && (!physical_rust_path(p)
                        || self.graph_files.contains(p)
                        || self.notebook_owners.iter().any(|owner| {
                            p.strip_prefix(owner.as_str())
                                .is_some_and(|suffix| suffix.starts_with("#cell:"))
                        }))
            });
        let mut snapshot = snapshot.clone();
        if let Some(path) = &filter.file_path {
            snapshot.source_oids.retain(|p, _| p == path);
        }
        Ok(MacroInvocationQueryResult {
            availability: if unsupported {
                MacroEvidenceAvailability::UnsupportedSource
            } else {
                MacroEvidenceAvailability::Complete
            },
            source_scope: Some(MacroEvidenceSourceScope::PhysicalRustFiles),
            snapshots: vec![snapshot],
            rows: Vec::new(),
            scopes: Vec::new(),
        })
    }
}

fn physical_rust_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

struct ParquetMacroContext {
    context: MacroQueryContext,
    tables: Option<MacroQueryTables>,
}

pub trait GraphQueryClient {
    /// Direct syntax evidence. Custom backends must opt in explicitly.
    fn macro_invocations(
        &self,
        _filter: &crate::MacroInvocationFilter,
    ) -> anyhow::Result<crate::MacroInvocationQueryResult> {
        Ok(MacroInvocationQueryResult::unavailable())
    }

    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult>;
    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord>;
    fn find_unresolved_caller_edges_by_labels(
        &self,
        _target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        Vec::new()
    }
    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord>;
    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution>;
    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>>;
    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>>;
    /// Batched form of [`symbols_by_file`](Self::symbols_by_file): one call
    /// covers every path, so backends with per-query scan cost (parquet) can
    /// answer with a single pass instead of one scan per path.
    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        let mut symbols = Vec::new();
        for path in paths {
            symbols.extend(self.symbols_by_file(path)?);
        }
        Ok(symbols)
    }
    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>>;
    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>>;
    fn file_exists(&self, path: &str) -> anyhow::Result<bool>;
    fn temporal_index(&self) -> Arc<TemporalIndex>;
    fn symbol_history(
        &self,
        commits: &CommitIndexArtifact,
        symbol_id: &str,
    ) -> anyhow::Result<Vec<(GitSha, ChangeKind, SnapshotKey)>> {
        let index = self.temporal_index();
        Ok(symbol_history_indexed(index.as_ref(), commits, symbol_id))
    }
}

impl<T: GraphQueryClient + ?Sized> GraphQueryClient for &T {
    fn macro_invocations(
        &self,
        filter: &MacroInvocationFilter,
    ) -> anyhow::Result<MacroInvocationQueryResult> {
        (**self).macro_invocations(filter)
    }

    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        (**self).search_symbols(opts)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        (**self).find_caller_edges(sid)
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        (**self).find_unresolved_caller_edges_by_labels(target_labels)
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        (**self).find_callee_edges(sid)
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution> {
        (**self).resolve_selector(selector)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        (**self).symbol_by_id(sid)
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        (**self).symbols_by_file(path)
    }

    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        (**self).symbols_by_files(paths)
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        (**self).symbols_by_path_name(path, name)
    }

    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        (**self).file_manifest_by_path(path)
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        (**self).file_exists(path)
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        (**self).temporal_index()
    }

    fn symbol_history(
        &self,
        commits: &CommitIndexArtifact,
        symbol_id: &str,
    ) -> anyhow::Result<Vec<(GitSha, ChangeKind, SnapshotKey)>> {
        (**self).symbol_history(commits, symbol_id)
    }
}

#[derive(Clone)]
pub struct InMemoryClient {
    artifact: Arc<GraphIndexArtifact>,
    temporal_index: Arc<OnceLock<Arc<TemporalIndex>>>,
    macro_context: Arc<OnceLock<Result<MacroQueryContext, String>>>,
}

impl InMemoryClient {
    pub fn new(artifact: Arc<GraphIndexArtifact>) -> Self {
        Self {
            artifact,
            temporal_index: Arc::new(OnceLock::new()),
            macro_context: Arc::new(OnceLock::new()),
        }
    }

    pub fn artifact(&self) -> &GraphIndexArtifact {
        &self.artifact
    }
}

impl GraphQueryClient for InMemoryClient {
    fn macro_invocations(
        &self,
        filter: &MacroInvocationFilter,
    ) -> anyhow::Result<MacroInvocationQueryResult> {
        let context = self
            .macro_context
            .get_or_init(|| MacroQueryContext::new(&self.artifact).map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!(e.clone()))?;
        let caller = if filter.file_path.is_none() && context.snapshot.is_some() {
            filter
                .caller_stable_symbol_id
                .as_deref()
                .map(|sid| self.symbol_by_id(sid))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let mut result = context.result(filter, caller.as_ref().map(|s| s.file_path.as_str()))?;
        if result.availability == MacroEvidenceAvailability::Complete {
            if let Some(set) = &self.artifact.macro_invocations {
                result.rows = set
                    .rows
                    .iter()
                    .filter(|r| filter.matches(r))
                    .cloned()
                    .collect();
                let ids = crate::macro_invocations::referenced_scope_ids(&result.rows);
                result.scopes = set
                    .scopes
                    .iter()
                    .filter(|s| ids.contains(&s.scope.stable_symbol_id))
                    .cloned()
                    .collect();
            }
        }
        result.sort();
        Ok(result)
    }

    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        Ok(search_symbols(&self.artifact, opts))
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        find_caller_edges(&self.artifact, sid)
            .into_iter()
            .map(OwnedCallerRecord::from)
            .collect()
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        if target_labels.is_empty() {
            return Vec::new();
        }
        self.artifact
            .edges
            .iter()
            .filter(|edge| is_caller_relation(edge.relation))
            .filter(|edge| edge.target_stable_symbol_id.is_none())
            .filter(|edge| {
                edge.target_label
                    .as_ref()
                    .is_some_and(|label| target_labels.contains(label))
            })
            .filter_map(|edge| {
                let caller = self
                    .artifact
                    .symbols
                    .iter()
                    .find(|symbol| symbol.stable_symbol_id == edge.source_stable_symbol_id)?;
                Some(OwnedCallerRecord::Unresolved {
                    caller: caller.clone(),
                    edge: edge.clone(),
                    target_label: edge.target_label.clone().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        find_callee_edges(&self.artifact, sid)
            .into_iter()
            .map(OwnedCalleeRecord::from)
            .collect()
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution> {
        Ok(resolve_selector(&self.artifact, selector))
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        Ok(self
            .artifact
            .symbols
            .iter()
            .find(|symbol| symbol.stable_symbol_id == sid)
            .cloned())
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        Ok(self
            .artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.file_path == path)
            .cloned()
            .collect())
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        Ok(self
            .artifact
            .symbols
            .iter()
            .filter(|symbol| {
                symbol.file_path == path
                    && (symbol.entity_name == name || symbol.qualified_name == name)
            })
            .cloned()
            .collect())
    }

    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        Ok(self
            .artifact
            .file_manifests
            .iter()
            .find(|entry| entry.path == path)
            .cloned())
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        Ok(self
            .artifact
            .files
            .iter()
            .any(|entry| entry.file_path == path)
            || self
                .artifact
                .file_manifests
                .iter()
                .any(|entry| entry.path == path))
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        Arc::clone(
            self.temporal_index
                .get_or_init(|| Arc::new(TemporalIndex::new(Arc::clone(&self.artifact)))),
        )
    }
}

pub struct OverlayClient<B: GraphQueryClient> {
    base: B,
    delta: InMemoryClient,
    shadowed: HashSet<String>,
    remap: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OverlayFinalizationMeasurements {
    pub shadow_filters: u64,
    pub result_merges: u64,
    pub overlay_sorts: u64,
    pub stable_id_deduplications: u64,
}

impl OverlayFinalizationMeasurements {
    pub fn total(self) -> u64 {
        self.shadow_filters
            + self.result_merges
            + self.overlay_sorts
            + self.stable_id_deduplications
    }
}

fn empty_overlay_delta_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "overlay-empty".to_owned(),
        graph_content_hash: "overlay-empty".to_owned(),
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

impl<B: GraphQueryClient> OverlayClient<B> {
    pub fn extract_delta(
        root: &Path,
        changed_files: &[PathBuf],
    ) -> anyhow::Result<(Arc<GraphIndexArtifact>, HashSet<String>)> {
        let facts = build_facts_for_paths(root, changed_files)?;
        let artifact = artifact_from_facts_for_paths(&facts, root, changed_files)?;
        let shadowed = changed_files
            .iter()
            .filter_map(|path| normalize_worktree_path(root, path).ok())
            .collect::<HashSet<_>>();
        Ok((Arc::new(artifact), shadowed))
    }

    pub fn new(base: B, root: &Path, changed_files: &[PathBuf]) -> anyhow::Result<Self> {
        if changed_files.is_empty() {
            return Self::from_artifacts(
                base,
                Arc::new(empty_overlay_delta_artifact()),
                HashSet::new(),
            );
        }
        let (artifact, shadowed) = Self::extract_delta(root, changed_files)?;
        Self::from_artifacts(base, artifact, shadowed)
    }

    pub fn from_artifacts(
        base: B,
        delta_artifact: Arc<GraphIndexArtifact>,
        shadowed: HashSet<String>,
    ) -> anyhow::Result<Self> {
        let remap = build_overlay_remap(&base, &delta_artifact)?;
        Ok(Self {
            base,
            delta: InMemoryClient::new(delta_artifact),
            shadowed,
            remap,
        })
    }

    fn is_identity_overlay(&self) -> bool {
        self.shadowed.is_empty() && self.remap.is_empty()
    }

    pub fn search_symbols_with_measurements(
        &self,
        opts: &SearchOptions,
        measurements: &mut OverlayFinalizationMeasurements,
    ) -> anyhow::Result<SearchResult> {
        self.search_symbols_counted(opts, Some(measurements))
    }

    fn search_symbols_counted(
        &self,
        opts: &SearchOptions,
        mut measurements: Option<&mut OverlayFinalizationMeasurements>,
    ) -> anyhow::Result<SearchResult> {
        if self.is_identity_overlay() {
            return self.base.search_symbols(opts);
        }
        let unbounded = internal_unbounded_search_options(opts);
        let mut candidates = self
            .base
            .search_symbols(&unbounded)?
            .candidates
            .into_iter()
            .filter(|symbol| !self.is_shadowed_path(&symbol.file_path))
            .collect::<Vec<_>>();
        if let Some(measurements) = measurements.as_deref_mut() {
            measurements.shadow_filters += 1;
        }

        candidates.extend(self.delta.search_symbols(&unbounded)?.candidates);
        if let Some(measurements) = measurements.as_deref_mut() {
            measurements.result_merges += 1;
        }

        // This orders the separately queried base and delta vectors as one overlay.
        // Ranking sorts inside either query client are not overlay finalization.
        candidates.sort_by(|left, right| compare_symbols(left, right, opts));
        if let Some(measurements) = measurements.as_deref_mut() {
            measurements.overlay_sorts += 1;
        }

        candidates.dedup_by(|left, right| left.stable_symbol_id == right.stable_symbol_id);
        if let Some(measurements) = measurements {
            measurements.stable_id_deduplications += 1;
        }

        let total_matches = candidates.len();
        Ok(limited_search_result(candidates, total_matches, opts.limit))
    }

    fn is_shadowed_path(&self, path: &str) -> bool {
        self.shadowed.contains(path)
    }

    fn delta_defines_label(&self, label: &str) -> bool {
        matches!(
            self.delta.resolve_selector(label),
            Ok(SelectorResolution::Resolved(_)) | Ok(SelectorResolution::Ambiguous { .. })
        )
    }

    fn is_delta_symbol(&self, sid: &str) -> bool {
        self.delta
            .symbol_by_id(sid)
            .ok()
            .flatten()
            .is_some_and(|symbol| self.is_shadowed_path(&symbol.file_path))
    }

    fn remapped_id_for(&self, old_id: &str) -> Option<&str> {
        self.remap.get(old_id).map(String::as_str)
    }

    /// Resolve the current (delta) version of a base symbol after an edit to
    /// its file: the same id when re-extraction kept it stable, or the
    /// remapped successor when the id changed. `None` means the symbol no
    /// longer exists in the changed file.
    pub fn current_symbol_for(
        &self,
        base_symbol_id: &str,
    ) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        if let Some(symbol) = self.delta.symbol_by_id(base_symbol_id)? {
            if self.is_shadowed_path(&symbol.file_path) {
                return Ok(Some(symbol));
            }
        }
        if let Some(new_id) = self.remapped_id_for(base_symbol_id) {
            return self.delta.symbol_by_id(new_id);
        }
        Ok(None)
    }

    fn old_ids_for_new<'a>(&'a self, new_id: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.remap
            .iter()
            .filter(move |(_, mapped)| mapped.as_str() == new_id)
            .map(|(old, _)| old.as_str())
    }

    fn target_labels_for_symbol(symbol: &GraphSymbolArtifact) -> HashSet<String> {
        HashSet::from([
            symbol.entity_name.clone(),
            symbol.qualified_name.clone(),
            symbol.stable_symbol_id.clone(),
        ])
    }

    fn push_caller_record(
        records: &mut Vec<OwnedCallerRecord>,
        seen: &mut HashSet<EdgeDedupeKey>,
        record: OwnedCallerRecord,
    ) {
        if seen.insert(caller_key(&record)) {
            records.push(record);
        }
    }

    fn repoint_caller_record(
        &self,
        record: OwnedCallerRecord,
        target_sid: &str,
    ) -> Option<OwnedCallerRecord> {
        let target = self.symbol_by_id(target_sid).ok().flatten()?;
        match record {
            OwnedCallerRecord::Resolved { caller, mut edge }
            | OwnedCallerRecord::Unresolved {
                caller, mut edge, ..
            } => {
                edge.target_stable_symbol_id = Some(target.stable_symbol_id);
                edge.target_label
                    .get_or_insert_with(|| target.entity_name.clone());
                Some(OwnedCallerRecord::Resolved { caller, edge })
            }
        }
    }

    fn re_resolve_callee_record(&self, record: OwnedCalleeRecord) -> OwnedCalleeRecord {
        match record {
            OwnedCalleeRecord::Resolved { symbol, mut edge } => {
                if let Some(target_id) = edge.target_stable_symbol_id.as_deref() {
                    if let Some(new_id) = self.remapped_id_for(target_id) {
                        edge.target_stable_symbol_id = Some(new_id.to_owned());
                        if let Ok(Some(new_symbol)) = self.symbol_by_id(new_id) {
                            return OwnedCalleeRecord::Resolved {
                                symbol: new_symbol,
                                edge,
                            };
                        }
                    }
                }

                if !self.is_shadowed_path(&symbol.file_path) {
                    return OwnedCalleeRecord::Resolved { symbol, edge };
                }

                let label = edge
                    .target_label
                    .clone()
                    .unwrap_or_else(|| symbol.entity_name.clone());
                if let Some(new_symbol) = self.resolve_label_to_symbol(&label) {
                    edge.target_stable_symbol_id = Some(new_symbol.stable_symbol_id.clone());
                    return OwnedCalleeRecord::Resolved {
                        symbol: new_symbol,
                        edge,
                    };
                }

                edge.target_stable_symbol_id = None;
                OwnedCalleeRecord::Unresolved {
                    edge,
                    target_label: label,
                }
            }
            OwnedCalleeRecord::Unresolved {
                mut edge,
                target_label,
            } => {
                // Unresolved base rows are overwhelmingly std/iterator noise. Only
                // probe labels the delta actually defines — otherwise we fall through
                // to the base Parquet resolve on every `map`/`clone`/`get`.
                if !self.delta_defines_label(&target_label) {
                    return OwnedCalleeRecord::Unresolved { edge, target_label };
                }
                if let Some(symbol) = self.resolve_label_to_symbol(&target_label) {
                    edge.target_stable_symbol_id = Some(symbol.stable_symbol_id.clone());
                    OwnedCalleeRecord::Resolved { symbol, edge }
                } else {
                    OwnedCalleeRecord::Unresolved { edge, target_label }
                }
            }
        }
    }

    fn resolve_label_to_symbol(&self, label: &str) -> Option<GraphSymbolArtifact> {
        match self.resolve_selector(label).ok()? {
            SelectorResolution::Resolved(resolved) => {
                self.symbol_by_id(&resolved.stable_symbol_id).ok().flatten()
            }
            SelectorResolution::Ambiguous { .. } | SelectorResolution::NotFound => None,
        }
    }

    fn filter_base_resolution(
        &self,
        resolution: SelectorResolution,
    ) -> anyhow::Result<SelectorResolution> {
        Ok(match resolution {
            SelectorResolution::Resolved(resolved) => {
                if self.symbol_by_id(&resolved.stable_symbol_id)?.is_some() {
                    SelectorResolution::Resolved(resolved)
                } else {
                    SelectorResolution::NotFound
                }
            }
            SelectorResolution::Ambiguous { candidates } => {
                let symbols = candidates
                    .into_iter()
                    .filter(|candidate| !self.is_shadowed_path(&candidate.file_path))
                    .filter_map(|candidate| self.symbol_by_id(&candidate.id).ok().flatten())
                    .collect::<Vec<_>>();
                resolution_from_symbols(symbols)
            }
            SelectorResolution::NotFound => SelectorResolution::NotFound,
        })
    }
}

impl<B: GraphQueryClient> GraphQueryClient for OverlayClient<B> {
    fn macro_invocations(
        &self,
        filter: &MacroInvocationFilter,
    ) -> anyhow::Result<MacroInvocationQueryResult> {
        // Validate the original request before inferring a caller's file path.
        filter.validate_shape()?;
        if self.is_identity_overlay() {
            return self.base.macro_invocations(filter);
        }
        // Resolve only real caller symbols. Evidence-only scope IDs are never
        // treated as graph callers. Routing by the current owner also preserves
        // UnsupportedSource for notebook callers in a dirty overlay.
        if filter.file_path.is_none() {
            if let Some(caller) = filter
                .caller_stable_symbol_id
                .as_deref()
                .map(|sid| self.symbol_by_id(sid))
                .transpose()?
                .flatten()
            {
                return self.macro_invocations(&MacroInvocationFilter {
                    file_path: Some(caller.file_path),
                    ..filter.clone()
                });
            }
        }
        if filter
            .file_path
            .as_ref()
            .is_some_and(|p| self.is_shadowed_path(p))
        {
            return self.delta.macro_invocations(filter);
        }
        let mut base = self.base.macro_invocations(filter)?;
        if filter.file_path.is_some() {
            return Ok(base);
        }
        let mut delta = self.delta.macro_invocations(filter)?;
        base.rows.retain(|r| !self.is_shadowed_path(&r.file_path));
        base.retain_referenced_scopes();
        for snapshot in &mut base.snapshots {
            snapshot
                .source_oids
                .retain(|p, _| !self.is_shadowed_path(p));
        }
        delta.rows.retain(|r| self.is_shadowed_path(&r.file_path));
        delta.retain_referenced_scopes();
        for snapshot in &mut delta.snapshots {
            snapshot.source_oids.retain(|p, _| self.is_shadowed_path(p));
        }
        base.availability = match (base.availability, delta.availability) {
            (MacroEvidenceAvailability::Complete, MacroEvidenceAvailability::Complete) => {
                MacroEvidenceAvailability::Complete
            }
            (MacroEvidenceAvailability::Unavailable, MacroEvidenceAvailability::Unavailable) => {
                MacroEvidenceAvailability::Unavailable
            }
            _ => MacroEvidenceAvailability::Partial,
        };
        base.source_scope = base.source_scope.or(delta.source_scope);
        base.rows.extend(delta.rows);
        base.scopes.extend(delta.scopes);
        base.snapshots.extend(delta.snapshots);
        base.sort();
        Ok(base)
    }

    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        self.search_symbols_counted(opts, None)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        if self.is_identity_overlay() {
            return self.base.find_caller_edges(sid);
        }
        let Some(target_symbol) = self.symbol_by_id(sid).ok().flatten() else {
            return Vec::new();
        };
        let target_is_shadowed = self.is_shadowed_path(&target_symbol.file_path);
        let target_labels = Self::target_labels_for_symbol(&target_symbol);
        let mut records = Vec::new();
        let mut seen = HashSet::new();

        for record in self.delta.find_caller_edges(sid) {
            Self::push_caller_record(&mut records, &mut seen, record);
        }
        for record in self.base.find_caller_edges(sid) {
            if !caller_record_is_shadowed(&record, &self.shadowed) {
                Self::push_caller_record(&mut records, &mut seen, record);
            }
        }
        // New call edges introduced in changed files whose target lives outside the
        // delta's extraction scope (e.g. a changed file adding a call to an unchanged
        // symbol) land as unresolved-by-label in the delta. Match them to this target
        // and re-point. Delta callers live in changed files, so they are intentionally
        // NOT dropped by the shadowed filter — they are the authoritative new version.
        for record in self
            .delta
            .find_unresolved_caller_edges_by_labels(&target_labels)
        {
            if let Some(record) = self.repoint_caller_record(record, sid) {
                Self::push_caller_record(&mut records, &mut seen, record);
            }
        }
        if target_is_shadowed {
            for record in self
                .base
                .find_unresolved_caller_edges_by_labels(&target_labels)
            {
                if caller_record_is_shadowed(&record, &self.shadowed) {
                    continue;
                }
                if let Some(record) = self.repoint_caller_record(record, sid) {
                    Self::push_caller_record(&mut records, &mut seen, record);
                }
            }
        }

        for old_id in self.old_ids_for_new(sid) {
            for record in self.base.find_caller_edges(old_id) {
                if caller_record_is_shadowed(&record, &self.shadowed) {
                    continue;
                }
                if let Some(record) = self.repoint_caller_record(record, sid) {
                    Self::push_caller_record(&mut records, &mut seen, record);
                }
            }
        }

        records
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        if self.is_identity_overlay() {
            return self.base.find_callee_edges(sid);
        }
        if self.is_delta_symbol(sid) {
            return self
                .delta
                .find_callee_edges(sid)
                .into_iter()
                .map(|record| self.re_resolve_callee_record(record))
                .collect();
        }
        if self.symbol_by_id(sid).ok().flatten().is_none() {
            return Vec::new();
        }
        self.base
            .find_callee_edges(sid)
            .into_iter()
            .map(|record| self.re_resolve_callee_record(record))
            .collect()
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution> {
        if self.is_identity_overlay() {
            return self.base.resolve_selector(selector);
        }
        let delta_resolution = self.delta.resolve_selector(selector)?;
        if !matches!(delta_resolution, SelectorResolution::NotFound) {
            return Ok(delta_resolution);
        }
        let base_resolution = self.base.resolve_selector(selector)?;
        self.filter_base_resolution(base_resolution)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        if let Some(symbol) = self.delta.symbol_by_id(sid)? {
            return Ok(Some(symbol));
        }
        Ok(self
            .base
            .symbol_by_id(sid)?
            .filter(|symbol| !self.is_shadowed_path(&symbol.file_path)))
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        if self.is_shadowed_path(path) {
            self.delta.symbols_by_file(path)
        } else {
            self.base.symbols_by_file(path)
        }
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        if self.is_shadowed_path(path) {
            self.delta.symbols_by_path_name(path, name)
        } else {
            self.base.symbols_by_path_name(path, name)
        }
    }

    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        if self.is_shadowed_path(path) {
            self.delta.file_manifest_by_path(path)
        } else {
            self.base.file_manifest_by_path(path)
        }
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        Ok(self.delta.file_exists(path)?
            || (!self.is_shadowed_path(path) && self.base.file_exists(path)?))
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        self.base.temporal_index()
    }

    fn symbol_history(
        &self,
        commits: &CommitIndexArtifact,
        symbol_id: &str,
    ) -> anyhow::Result<Vec<(GitSha, ChangeKind, SnapshotKey)>> {
        self.base.symbol_history(commits, symbol_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeDedupeKey {
    source_stable_symbol_id: String,
    target_stable_symbol_id: Option<String>,
    target_label: Option<String>,
    import_path: Option<String>,
    receiver_text: Option<String>,
    scope_text: Option<String>,
    relation: RelationKind,
    edge_kind: Option<crate::GraphEdgeKind>,
    bind_method: Option<String>,
}

fn build_overlay_remap<B: GraphQueryClient>(
    base: &B,
    delta_artifact: &GraphIndexArtifact,
) -> anyhow::Result<HashMap<String, String>> {
    let mut remap = HashMap::new();
    if delta_artifact.symbols.is_empty() {
        return Ok(remap);
    }

    // One batched base fetch for the changed paths; per-delta-symbol base
    // queries are full scans on the parquet backend and dominate overlay
    // construction cost.
    let mut changed_paths = delta_artifact
        .symbols
        .iter()
        .map(|symbol| symbol.file_path.clone())
        .collect::<Vec<_>>();
    changed_paths.sort();
    changed_paths.dedup();
    let base_symbols = base.symbols_by_files(&changed_paths)?;
    let mut base_by_path: HashMap<&str, Vec<&GraphSymbolArtifact>> = HashMap::new();
    for base_symbol in &base_symbols {
        base_by_path
            .entry(base_symbol.file_path.as_str())
            .or_default()
            .push(base_symbol);
    }

    for delta_symbol in &delta_artifact.symbols {
        let file_symbols = base_by_path
            .get(delta_symbol.file_path.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let matching_name = |name: &str| {
            file_symbols
                .iter()
                .copied()
                .filter(|base_symbol| {
                    base_symbol.entity_name == name || base_symbol.qualified_name == name
                })
                .collect::<Vec<_>>()
        };

        let mut candidates = matching_name(&delta_symbol.qualified_name);
        if candidates.is_empty() && delta_symbol.qualified_name != delta_symbol.entity_name {
            candidates = matching_name(&delta_symbol.entity_name);
        }
        if candidates.is_empty() {
            candidates = file_symbols
                .iter()
                .copied()
                .filter(|base_symbol| base_symbol.symbol_kind == delta_symbol.symbol_kind)
                .filter(|base_symbol| {
                    ranges_overlap(base_symbol.line_range, delta_symbol.line_range)
                })
                .collect();
        }

        candidates.sort_by(|left, right| {
            line_distance(left.line_range, delta_symbol.line_range)
                .cmp(&line_distance(right.line_range, delta_symbol.line_range))
                .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
        });
        for base_symbol in candidates {
            if base_symbol.stable_symbol_id != delta_symbol.stable_symbol_id {
                remap.insert(
                    base_symbol.stable_symbol_id.clone(),
                    delta_symbol.stable_symbol_id.clone(),
                );
            }
        }
    }
    Ok(remap)
}

fn normalize_worktree_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = if path.is_absolute() {
        let root = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
        path.strip_prefix(&root).unwrap_or(path)
    } else {
        path
    };
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn ranges_overlap(left: [usize; 2], right: [usize; 2]) -> bool {
    left[0] <= right[1] && right[0] <= left[1]
}

fn line_distance(left: [usize; 2], right: [usize; 2]) -> usize {
    left[0].abs_diff(right[0]) + left[1].abs_diff(right[1])
}

fn caller_record_is_shadowed(record: &OwnedCallerRecord, shadowed: &HashSet<String>) -> bool {
    match record {
        OwnedCallerRecord::Resolved { caller, .. }
        | OwnedCallerRecord::Unresolved { caller, .. } => shadowed.contains(&caller.file_path),
    }
}

fn caller_key(record: &OwnedCallerRecord) -> EdgeDedupeKey {
    edge_key(record.edge())
}

fn edge_key(edge: &GraphEdgeArtifact) -> EdgeDedupeKey {
    EdgeDedupeKey {
        source_stable_symbol_id: edge.source_stable_symbol_id.clone(),
        target_stable_symbol_id: edge.target_stable_symbol_id.clone(),
        target_label: edge.target_label.clone(),
        import_path: edge.import_path.clone(),
        receiver_text: edge.receiver_text.clone(),
        scope_text: edge.scope_text.clone(),
        relation: edge.relation,
        edge_kind: edge.edge_kind,
        bind_method: edge.bind_method.clone(),
    }
}

const SEARCH_COLUMNS: [&str; 8] = [
    "stable_symbol_id",
    "file_path",
    "line_start",
    "line_end",
    "entity_name",
    "qualified_name",
    "symbol_kind",
    "enclosing_scope",
];
const SEARCH_PREDICATE_COLUMNS: [&str; 4] =
    ["entity_name", "qualified_name", "file_path", "symbol_kind"];
const FILE_OID_COLUMNS: [&str; 2] = ["path", "content_oid"];
const FILE_COLUMNS: [&str; 2] = ["stable_file_id", "file_path"];
const DIAGNOSTIC_COLUMNS: [&str; 1] = ["message"];
const FILE_MANIFEST_COLUMNS: [&str; 4] = ["stable_file_id", "path", "content_oid", "node_ids"];
const SYMBOL_COLUMNS: [&str; 11] = [
    "stable_symbol_id",
    "file_path",
    "byte_range_start",
    "byte_range_end",
    "line_start",
    "line_end",
    "entity_name",
    "qualified_name",
    "symbol_kind",
    "anchor_hash",
    "enclosing_scope",
];

/// Search-only index constructed from an asynchronous Parquet byte-range reader.
///
/// This preserves the same ranking and result semantics as [`ParquetClient`]
/// without requiring the complete Parquet object to exist on a local filesystem.
pub struct ParquetSearchIndex {
    inner: HotQueryIndex,
}

impl ParquetSearchIndex {
    pub async fn read_async<T>(reader: T) -> anyhow::Result<Self>
    where
        T: AsyncFileReader + Send + Unpin + 'static,
    {
        let builder = ParquetRecordBatchStreamBuilder::new(reader)
            .await
            .context("failed to read remote nodes.parquet metadata")?;
        let row_count =
            builder
                .metadata()
                .row_groups()
                .iter()
                .try_fold(0_usize, |total, row_group| {
                    let rows = usize::try_from(row_group.num_rows())
                        .context("remote nodes.parquet row count exceeds usize")?;
                    total
                        .checked_add(rows)
                        .context("remote nodes.parquet row count overflow")
                })?;
        let projection = ProjectionMask::columns(builder.parquet_schema(), SYMBOL_COLUMNS);
        let stream = builder
            .with_batch_size(row_count.max(1))
            .with_projection(projection)
            .build()
            .context("failed to build remote nodes.parquet reader")?;
        let schema = stream.schema().clone();
        let mut batches = stream
            .try_collect::<Vec<_>>()
            .await
            .context("failed to decode remote nodes.parquet")?;
        let batch = match batches.len() {
            0 => RecordBatch::new_empty(schema),
            1 => batches.remove(0),
            _ => concat_batches(&schema, &batches)
                .context("failed to combine remote nodes.parquet row groups")?,
        };
        drop(batches);
        Ok(Self {
            inner: HotQueryIndex::new(batch)?,
        })
    }

    pub fn search_symbols(&self, options: &SearchOptions) -> SearchResult {
        self.inner.search_symbols(options)
    }
}

#[cfg(test)]
static PARQUET_HOT_QUERY_INDEX_BUILDS_FOR_TEST: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static PARQUET_HOT_ADJACENCY_INDEX_BUILDS_FOR_TEST: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_parquet_hot_index_build_observations_for_test() {
    PARQUET_HOT_QUERY_INDEX_BUILDS_FOR_TEST.store(0, AtomicOrdering::SeqCst);
    PARQUET_HOT_ADJACENCY_INDEX_BUILDS_FOR_TEST.store(0, AtomicOrdering::SeqCst);
}

#[cfg(test)]
pub(crate) fn parquet_hot_index_build_observations_for_test() -> (usize, usize) {
    (
        PARQUET_HOT_QUERY_INDEX_BUILDS_FOR_TEST.load(AtomicOrdering::SeqCst),
        PARQUET_HOT_ADJACENCY_INDEX_BUILDS_FOR_TEST.load(AtomicOrdering::SeqCst),
    )
}

pub struct ParquetClient {
    dir: PathBuf,
    manifest: GraphArtifactManifest,
    metadata_cache: ParquetMetadataCache,
    nodes_metadata: ArrowReaderMetadata,
    search_projection: ProjectionMask,
    hot_query_index: OnceLock<Result<Arc<HotQueryIndex>, SharedHotQueryIndexError>>,
    hot_adjacency_index: OnceLock<Result<Arc<HotAdjacencyIndex>, SharedHotQueryIndexError>>,
    #[cfg(test)]
    hot_query_index_build_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    hot_adjacency_index_build_count: std::sync::atomic::AtomicUsize,
    file_oids: OnceLock<Result<Vec<(String, String)>, SharedFileOidsError>>,
    #[cfg(test)]
    file_oids_load_count: std::sync::atomic::AtomicUsize,
    temporal_index: OnceLock<Arc<TemporalIndex>>,
    macro_context: OnceLock<Result<ParquetMacroContext, String>>,
}

#[derive(Clone)]
struct SharedFileOidsError(Arc<anyhow::Error>);

#[derive(Clone)]
struct SharedHotQueryIndexError(Arc<anyhow::Error>);

impl SharedFileOidsError {
    fn new(error: anyhow::Error) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Debug for SharedFileOidsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for SharedFileOidsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for SharedFileOidsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl SharedHotQueryIndexError {
    fn new(error: anyhow::Error) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Debug for SharedHotQueryIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for SharedHotQueryIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for SharedHotQueryIndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl ParquetClient {
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        if !dir.is_dir() {
            bail!(
                "Parquet artifact directory `{}` does not exist",
                dir.display()
            );
        }
        let manifest = read_artifact_header_parquet(&dir)?;
        if !manifest.complete {
            bail!(
                "refusing to load incomplete Parquet artifact `{}`",
                dir.display()
            );
        }
        let nodes_path = dir.join("nodes.parquet");
        let metadata_cache = ParquetMetadataCache::default();
        let nodes_metadata = metadata_cache.get(&nodes_path)?;
        let search_projection = ProjectionMask::columns(
            nodes_metadata.metadata().file_metadata().schema_descr(),
            SEARCH_COLUMNS,
        );
        Ok(Self {
            dir,
            manifest,
            metadata_cache,
            nodes_metadata,
            search_projection,
            hot_query_index: OnceLock::new(),
            hot_adjacency_index: OnceLock::new(),
            #[cfg(test)]
            hot_query_index_build_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            hot_adjacency_index_build_count: std::sync::atomic::AtomicUsize::new(0),
            file_oids: OnceLock::new(),
            #[cfg(test)]
            file_oids_load_count: std::sync::atomic::AtomicUsize::new(0),
            temporal_index: OnceLock::new(),
            macro_context: OnceLock::new(),
        })
    }

    // No edges, temporal tables, or full graph hydration. The first macro query
    // freezes both compressed tables, validates their complete shared checksum,
    // then checks source manifests and the actual function caller projection.
    // Later queries perform zero disk IO for evidence, using Parquet pruning on
    // immutable bytes. Decoded full evidence is discarded after validation.
    fn load_macro_context(&self) -> anyhow::Result<ParquetMacroContext> {
        let (tables, set) = MacroQueryTables::open(&self.dir, &self.manifest)?;
        let mut artifact = empty_overlay_delta_artifact();
        artifact.graph_content_hash = self.manifest.graph_content_hash.clone();
        if let Some(set) = &set {
            artifact.file_manifests = file_manifests_from_batches(read_projected_batches(
                &self.dir.join("file_manifests.parquet"),
                &self.metadata_cache,
                FILE_MANIFEST_COLUMNS,
            )?)?;
            anyhow::ensure!(
                artifact.file_manifests.len() == self.manifest.row_counts.file_manifests,
                "macro query file_manifests row count mismatch: declared {}, decoded {}",
                self.manifest.row_counts.file_manifests,
                artifact.file_manifests.len()
            );
            artifact.files = self.files()?;
            let callers = set
                .rows
                .iter()
                .filter_map(|r| r.caller_stable_symbol_id.clone())
                .collect::<HashSet<_>>();
            artifact.symbols =
                self.symbols_by_stable_ids(&callers.into_iter().collect::<Vec<_>>())?;
        }
        artifact.macro_invocations = set;
        Ok(ParquetMacroContext {
            context: MacroQueryContext::new(&artifact)?,
            tables,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn manifest(&self) -> &GraphArtifactManifest {
        &self.manifest
    }

    fn hot_query_index(&self) -> anyhow::Result<Arc<HotQueryIndex>> {
        match self.hot_query_index.get_or_init(|| {
            #[cfg(test)]
            self.hot_query_index_build_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            #[cfg(test)]
            PARQUET_HOT_QUERY_INDEX_BUILDS_FOR_TEST.fetch_add(1, AtomicOrdering::SeqCst);
            (|| -> anyhow::Result<Arc<HotQueryIndex>> {
                let symbol_batch = self.read_current_query_symbol_batch()?;
                Ok(Arc::new(HotQueryIndex::new(symbol_batch)?))
            })()
            .map_err(SharedHotQueryIndexError::new)
        }) {
            Ok(index) => Ok(Arc::clone(index)),
            Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }

    fn read_current_query_symbol_batch(&self) -> anyhow::Result<RecordBatch> {
        let nodes_path = self.dir.join("nodes.parquet");
        let file = File::open(&nodes_path)
            .with_context(|| format!("failed to open `{}`", nodes_path.display()))?;
        let builder =
            ParquetRecordBatchReaderBuilder::new_with_metadata(file, self.nodes_metadata.clone());
        let projection = ProjectionMask::columns(builder.parquet_schema(), SYMBOL_COLUMNS);
        let builder = builder
            .with_batch_size(self.manifest.row_counts.nodes.max(1))
            .with_projection(projection);
        let schema = builder.schema().clone();
        let mut reader = builder.build().with_context(|| {
            format!(
                "failed to build Arrow reader for `{}`",
                nodes_path.display()
            )
        })?;

        let Some(batch) = reader
            .next()
            .transpose()
            .with_context(|| format!("failed to decode `{}`", nodes_path.display()))?
        else {
            return Ok(RecordBatch::new_empty(schema));
        };
        if reader
            .next()
            .transpose()
            .with_context(|| format!("failed to decode `{}`", nodes_path.display()))?
            .is_some()
        {
            bail!(
                "expected one current-symbol RecordBatch for `{}`, but reader produced multiple batches",
                nodes_path.display()
            );
        }
        Ok(batch)
    }

    #[cfg(test)]
    fn hot_query_index_build_count(&self) -> usize {
        self.hot_query_index_build_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn hot_adjacency_index(&self) -> anyhow::Result<Arc<HotAdjacencyIndex>> {
        match self.hot_adjacency_index.get_or_init(|| {
            #[cfg(test)]
            self.hot_adjacency_index_build_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            #[cfg(test)]
            PARQUET_HOT_ADJACENCY_INDEX_BUILDS_FOR_TEST.fetch_add(1, AtomicOrdering::SeqCst);
            (|| -> anyhow::Result<Arc<HotAdjacencyIndex>> {
                let symbols = self.hot_query_index()?;
                let edges = read_current_query_edges_parquet(&self.dir)?;
                Ok(Arc::new(HotAdjacencyIndex::new(symbols, edges)))
            })()
            .map_err(SharedHotQueryIndexError::new)
        }) {
            Ok(index) => Ok(Arc::clone(index)),
            Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }

    #[cfg(test)]
    fn hot_adjacency_index_build_count(&self) -> usize {
        self.hot_adjacency_index_build_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn file_oids(&self) -> anyhow::Result<Vec<(String, String)>> {
        match self.file_oids.get_or_init(|| {
            #[cfg(test)]
            self.file_oids_load_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (|| -> anyhow::Result<Vec<(String, String)>> {
                let batches = read_projected_batches(
                    &self.dir.join("file_manifests.parquet"),
                    &self.metadata_cache,
                    FILE_OID_COLUMNS,
                )?;
                let mut rows = Vec::new();
                for batch in batches {
                    let path = string_array_by_name(&batch, "path")?;
                    let content_oid = string_array_by_name(&batch, "content_oid")?;
                    for row in 0..batch.num_rows() {
                        rows.push((
                            required_string_value(path, row, "path")?.to_owned(),
                            required_string_value(content_oid, row, "content_oid")?.to_owned(),
                        ));
                    }
                }
                Ok(rows)
            })()
            .map_err(SharedFileOidsError::new)
        }) {
            Ok(rows) => Ok(rows.clone()),
            Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }

    /// Read the complete graph-file projection without loading symbols or edges.
    pub fn files(&self) -> anyhow::Result<Vec<GraphFileArtifact>> {
        let batches = read_projected_batches(
            &self.dir.join("files.parquet"),
            &self.metadata_cache,
            FILE_COLUMNS,
        )?;
        let mut files = Vec::new();
        for batch in batches {
            let stable_file_id = string_array_by_name(&batch, "stable_file_id")?;
            let file_path = string_array_by_name(&batch, "file_path")?;
            files.reserve(batch.num_rows());
            for row in 0..batch.num_rows() {
                files.push(GraphFileArtifact {
                    stable_file_id: required_string_value(stable_file_id, row, "stable_file_id")?
                        .to_owned(),
                    file_path: required_string_value(file_path, row, "file_path")?.to_owned(),
                });
            }
        }
        Ok(files)
    }

    /// Read graph-build diagnostics without loading any graph data table.
    pub fn diagnostics(&self) -> anyhow::Result<Vec<String>> {
        if self.manifest.row_counts.diagnostics == 0 {
            return Ok(Vec::new());
        }
        let batches = read_projected_batches(
            &self.dir.join("diagnostics.parquet"),
            &self.metadata_cache,
            DIAGNOSTIC_COLUMNS,
        )?;
        let mut diagnostics = Vec::with_capacity(self.manifest.row_counts.diagnostics);
        for batch in batches {
            let message = string_array_by_name(&batch, "message")?;
            diagnostics.reserve(batch.num_rows());
            for row in 0..batch.num_rows() {
                diagnostics.push(required_string_value(message, row, "message")?.to_owned());
            }
        }
        Ok(diagnostics)
    }

    /// Hydrate full symbols for a bounded set of stable IDs in one pruned Parquet query.
    pub fn symbols_by_stable_ids(
        &self,
        stable_ids: &[String],
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        let requested = stable_ids.iter().cloned().collect::<HashSet<_>>();
        let mut symbols = self.symbols_by_ids(&requested)?;
        Ok(stable_ids
            .iter()
            .filter_map(|stable_id| symbols.remove(stable_id))
            .collect())
    }

    fn search_symbols_inner(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        if !matches!(opts.mode, SearchMode::Exact) {
            return Ok(self.hot_query_index()?.search_symbols(opts));
        }
        let nodes_path = self.dir.join("nodes.parquet");
        let mut candidates = Vec::new();
        if let Some(pruning) = exact_search_pruning_predicate(opts) {
            let batches =
                self.filtered_projected_batches(&nodes_path, SEARCH_COLUMNS, &pruning, |schema| {
                    search_row_filter(schema, opts)
                })?;
            for batch in batches {
                candidates.extend(search_symbols_from_batch(&batch)?);
            }
            candidates.sort_by(|left, right| compare_symbols(left, right, opts));
            let total_matches = candidates.len();
            return Ok(limited_search_result(candidates, total_matches, opts.limit));
        }

        let file = File::open(&nodes_path)
            .with_context(|| format!("failed to open `{}`", nodes_path.display()))?;
        let row_filter = search_row_filter(
            self.nodes_metadata
                .metadata()
                .file_metadata()
                .schema_descr(),
            opts,
        );
        let reader =
            ParquetRecordBatchReaderBuilder::new_with_metadata(file, self.nodes_metadata.clone())
                .with_batch_size(PARQUET_ROW_GROUP_SIZE)
                .with_projection(self.search_projection.clone())
                .with_row_filter(row_filter)
                .build()
                .with_context(|| {
                    format!(
                        "failed to build Arrow reader for `{}`",
                        nodes_path.display()
                    )
                })?;

        for batch in reader {
            let batch =
                batch.with_context(|| format!("failed to decode `{}`", nodes_path.display()))?;
            candidates.extend(search_symbols_from_batch(&batch)?);
        }
        candidates.sort_by(|left, right| compare_symbols(left, right, opts));

        let total_matches = candidates.len();
        Ok(limited_search_result(candidates, total_matches, opts.limit))
    }

    fn filtered_projected_batches<const N: usize>(
        &self,
        path: &Path,
        columns: [&str; N],
        pruning_predicate: &StringPruningPredicate,
        row_filter: impl FnOnce(&SchemaDescriptor) -> RowFilter,
    ) -> anyhow::Result<Vec<RecordBatch>> {
        read_filtered_projected_batches(
            path,
            &self.metadata_cache,
            columns,
            pruning_predicate,
            row_filter,
        )
    }

    pub fn try_find_caller_edges(&self, sid: &str) -> anyhow::Result<Vec<OwnedCallerRecord>> {
        self.find_caller_edges_inner(sid)
    }

    pub fn try_find_callee_edges(&self, sid: &str) -> anyhow::Result<Vec<OwnedCalleeRecord>> {
        self.find_callee_edges_inner(sid)
    }

    fn find_caller_edges_inner(&self, target_sid: &str) -> anyhow::Result<Vec<OwnedCallerRecord>> {
        let symbols = self.hot_query_index()?;
        let Some(target_symbol) = symbols.symbol_by_id(target_sid) else {
            return Ok(Vec::new());
        };
        let unresolved_labels = unresolved_target_labels_for_symbol(&target_symbol);
        let index = self.hot_adjacency_index()?;
        Ok(index.caller_records(target_sid, &unresolved_labels))
    }

    fn find_callee_edges_inner(&self, source_sid: &str) -> anyhow::Result<Vec<OwnedCalleeRecord>> {
        if self.hot_query_index()?.symbol_by_id(source_sid).is_none() {
            return Ok(Vec::new());
        }

        let index = self.hot_adjacency_index()?;
        Ok(index.callee_records(source_sid))
    }

    fn symbol_by_stable_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        if let Some(Ok(index)) = self.hot_query_index.get() {
            return Ok(index.symbol_by_id(sid));
        }

        let ids = HashSet::from([sid.to_owned()]);
        Ok(self.symbols_by_ids(&ids)?.remove(sid))
    }

    fn symbols_by_ids(
        &self,
        sids: &HashSet<String>,
    ) -> anyhow::Result<HashMap<String, GraphSymbolArtifact>> {
        if sids.is_empty() {
            return Ok(HashMap::new());
        }
        let pruning = StringPruningPredicate::any_value("stable_symbol_id", sids.iter().cloned());
        let batches = self.filtered_projected_batches(
            &self.dir.join("nodes.parquet"),
            SYMBOL_COLUMNS,
            &pruning,
            |schema| string_in_row_filter(schema, "stable_symbol_id", sids.clone()),
        )?;
        let mut symbols = HashMap::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let symbol = symbol_from_batch_row(&batch, row)?;
                symbols.insert(symbol.stable_symbol_id.clone(), symbol);
            }
        }
        Ok(symbols)
    }

    fn symbols_where_string_eq(
        &self,
        column: &str,
        value: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        let pruning = StringPruningPredicate::eq(column, value);
        let batches = self.filtered_projected_batches(
            &self.dir.join("nodes.parquet"),
            SYMBOL_COLUMNS,
            &pruning,
            |schema| string_eq_row_filter(schema, column, value.to_owned()),
        )?;
        symbols_from_batches(batches)
    }

    fn symbols_where_string_in(
        &self,
        column: &str,
        values: &[String],
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let expected = values.iter().cloned().collect::<HashSet<_>>();
        let pruning = StringPruningPredicate::any_value(column, values.iter().cloned());
        let batches = self.filtered_projected_batches(
            &self.dir.join("nodes.parquet"),
            SYMBOL_COLUMNS,
            &pruning,
            |schema| string_in_row_filter(schema, column, expected),
        )?;
        symbols_from_batches(batches)
    }

    fn symbols_where_all_string_eq(
        &self,
        expected: Vec<(&str, String)>,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        let pruning = StringPruningPredicate::all(
            expected
                .iter()
                .map(|(column, value)| StringPruningPredicate::eq(*column, value.clone())),
        );
        let batches = self.filtered_projected_batches(
            &self.dir.join("nodes.parquet"),
            SYMBOL_COLUMNS,
            &pruning,
            |schema| string_eq_all_row_filter(schema, expected),
        )?;
        symbols_from_batches(batches)
    }

    fn symbols_by_file_path(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_where_string_eq("file_path", path)
    }

    fn symbols_by_file_qualified_name(
        &self,
        path: &str,
        qualified_name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_where_all_string_eq(vec![
            ("file_path", path.to_owned()),
            ("qualified_name", qualified_name.to_owned()),
        ])
    }

    fn symbols_by_file_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        let pruning = StringPruningPredicate::all([
            StringPruningPredicate::eq("file_path", path),
            StringPruningPredicate::any([
                StringPruningPredicate::eq("entity_name", name),
                StringPruningPredicate::eq("qualified_name", name),
            ]),
        ]);
        let batches = self.filtered_projected_batches(
            &self.dir.join("nodes.parquet"),
            SYMBOL_COLUMNS,
            &pruning,
            |schema| path_name_row_filter(schema, path.to_owned(), name.to_owned()),
        )?;
        symbols_from_batches(batches)
    }

    fn file_manifest_by_path_inner(
        &self,
        path: &str,
    ) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        let pruning = StringPruningPredicate::eq("path", path);
        let batches = self.filtered_projected_batches(
            &self.dir.join("file_manifests.parquet"),
            FILE_MANIFEST_COLUMNS,
            &pruning,
            |schema| string_eq_row_filter(schema, "path", path.to_owned()),
        )?;
        let mut manifests = file_manifests_from_batches(batches)?;
        Ok(manifests.pop())
    }

    fn resolve_selector_inner(&self, selector: &str) -> anyhow::Result<SelectorResolution> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Ok(SelectorResolution::NotFound);
        }

        if let Some(symbol_id) = selector.strip_prefix(CODE_SYMBOL_URI_PREFIX) {
            return Ok(self
                .resolve_symbol_by_id(symbol_id)?
                .map(SelectorResolution::Resolved)
                .unwrap_or(SelectorResolution::NotFound));
        }

        if is_bare_stable_symbol_id(selector) {
            if let Some(symbol) = self.resolve_symbol_by_id(selector)? {
                return Ok(SelectorResolution::Resolved(symbol));
            }
        }

        if let Some(file_scoped) = selector
            .strip_prefix("file:")
            .or_else(|| selector.strip_prefix("path:"))
        {
            return self.resolve_file_scoped(file_scoped);
        }

        if let Some(line_resolution) = self.resolve_line_locator(selector)? {
            return Ok(line_resolution);
        }

        if let Some(file_resolution) = self.resolve_file_qualified(selector)? {
            return Ok(file_resolution);
        }

        if !first_token_contains_path_separator(selector) {
            let resolution =
                resolution_from_symbols(self.symbols_where_string_eq("qualified_name", selector)?);
            if !matches!(resolution, SelectorResolution::NotFound) {
                return Ok(resolution);
            }
        }

        if selector.contains("::") {
            return Ok(SelectorResolution::NotFound);
        }

        Ok(resolution_from_symbols(
            self.symbols_where_string_eq("entity_name", selector)?,
        ))
    }

    fn resolve_symbol_by_id(&self, symbol_id: &str) -> anyhow::Result<Option<ResolvedSymbol>> {
        if symbol_id.is_empty() {
            return Ok(None);
        }
        Ok(self
            .symbol_by_stable_id(symbol_id)?
            .as_ref()
            .map(resolved_symbol))
    }

    fn resolve_file_scoped(&self, selector: &str) -> anyhow::Result<SelectorResolution> {
        if let Some(resolution) = self.resolve_line_locator(selector)? {
            return Ok(resolution);
        }
        Ok(self
            .resolve_file_qualified(selector)?
            .unwrap_or(SelectorResolution::NotFound))
    }

    fn resolve_line_locator(&self, selector: &str) -> anyhow::Result<Option<SelectorResolution>> {
        for (file_path, line) in split_file_prefixes(selector, ":") {
            if line.starts_with(':') {
                continue;
            }
            if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            if !self.file_exists_inner(file_path)? {
                continue;
            }
            let Ok(line) = line.parse::<usize>() else {
                continue;
            };
            let symbol = self
                .symbols_by_file_path(file_path)?
                .into_iter()
                .filter(|symbol| symbol.line_range[0] <= line && line <= symbol.line_range[1])
                .max_by(compare_innermost);
            return Ok(Some(
                symbol
                    .as_ref()
                    .map(resolved_symbol)
                    .map(SelectorResolution::Resolved)
                    .unwrap_or(SelectorResolution::NotFound),
            ));
        }
        Ok(None)
    }

    fn resolve_file_qualified(&self, selector: &str) -> anyhow::Result<Option<SelectorResolution>> {
        for (file_path, chain) in split_file_prefixes(selector, "::")
            .into_iter()
            .chain(split_file_prefixes(selector, ":"))
        {
            if !self.file_exists_inner(file_path)? {
                continue;
            }
            let resolution =
                resolution_from_symbols(self.symbols_by_file_qualified_name(file_path, chain)?);
            if !matches!(resolution, SelectorResolution::NotFound) {
                return Ok(Some(resolution));
            }

            let fallback_matches = self
                .symbols_by_file_path(file_path)?
                .into_iter()
                .filter(|symbol| enclosing_scope_entity_name(symbol).as_deref() == Some(chain))
                .collect();
            return Ok(Some(resolution_from_symbols(fallback_matches)));
        }
        Ok(None)
    }

    fn file_exists_inner(&self, path: &str) -> anyhow::Result<bool> {
        Ok(self.file_manifest_by_path_inner(path)?.is_some())
    }
}

impl GraphQueryClient for ParquetClient {
    fn macro_invocations(
        &self,
        filter: &MacroInvocationFilter,
    ) -> anyhow::Result<MacroInvocationQueryResult> {
        let context = self
            .macro_context
            .get_or_init(|| self.load_macro_context().map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!(e.clone()))?;
        let caller = if filter.file_path.is_none() && context.context.snapshot.is_some() {
            filter
                .caller_stable_symbol_id
                .as_deref()
                .map(|sid| self.symbol_by_id(sid))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let mut result = context
            .context
            .result(filter, caller.as_ref().map(|s| s.file_path.as_str()))?;
        if result.availability == MacroEvidenceAvailability::Complete {
            if let Some(tables) = &context.tables {
                (result.rows, result.scopes) = tables.query(&self.manifest, filter)?;
            }
        }
        result.sort();
        Ok(result)
    }

    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        self.search_symbols_inner(opts)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        self.try_find_caller_edges(sid)
            .unwrap_or_else(|error| panic!("failed to query Parquet caller edges: {error:#}"))
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        if target_labels.is_empty() {
            return Vec::new();
        }
        self.hot_adjacency_index()
            .unwrap_or_else(|error| {
                panic!("failed to query Parquet unresolved caller edges: {error:#}")
            })
            .unresolved_caller_records(target_labels)
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        self.try_find_callee_edges(sid)
            .unwrap_or_else(|error| panic!("failed to query Parquet callee edges: {error:#}"))
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution> {
        self.resolve_selector_inner(selector)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        self.symbol_by_stable_id(sid)
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_by_file_path(path)
    }

    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        // Single set-membership scan of nodes.parquet; the default per-path
        // loop would pay one full filtered scan per path.
        self.symbols_where_string_in("file_path", paths)
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.symbols_by_file_path_name(path, name)
    }

    fn file_manifest_by_path(&self, path: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        self.file_manifest_by_path_inner(path)
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        self.file_exists_inner(path)
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        Arc::clone(self.temporal_index.get_or_init(|| {
            let artifact = read_temporal_artifact_parquet(&self.dir).unwrap_or_else(|error| {
                panic!(
                    "failed to build Parquet temporal index from `{}`: {error:#}",
                    self.dir.display()
                )
            });
            Arc::new(TemporalIndex::new(Arc::new(artifact)))
        }))
    }

    fn symbol_history(
        &self,
        commits: &CommitIndexArtifact,
        symbol_id: &str,
    ) -> anyhow::Result<Vec<(GitSha, ChangeKind, SnapshotKey)>> {
        let artifact = read_temporal_artifact_parquet_for_symbol_history_with_cache(
            &self.dir,
            symbol_id,
            &self.metadata_cache,
        )
        .with_context(|| {
            format!(
                "failed to read filtered Parquet temporal artifact from `{}`",
                self.dir.display()
            )
        })?;
        let index = TemporalIndex::new(Arc::new(artifact));
        Ok(symbol_history_indexed(&index, commits, symbol_id))
    }
}

fn search_row_filter(
    parquet_schema: &parquet::schema::types::SchemaDescriptor,
    opts: &SearchOptions,
) -> RowFilter {
    let predicate_columns = search_predicate_columns(opts);
    let projection = ProjectionMask::columns(parquet_schema, predicate_columns);
    let options = opts.clone();
    let glob = options
        .filters
        .file_glob
        .as_deref()
        .and_then(|pattern| Glob::new(pattern).ok())
        .map(|glob| glob.compile_matcher());
    let predicate = move |batch: RecordBatch| -> Result<BooleanArray, ArrowError> {
        let entity_name = string_array_by_name(&batch, "entity_name")?;
        let qualified_name = if matches!(options.mode, SearchMode::Exact) {
            Some(string_array_by_name(&batch, "qualified_name")?)
        } else {
            None
        };
        let file_path = if options.filters.file.is_some() || options.filters.file_glob.is_some() {
            Some(string_array_by_name(&batch, "file_path")?)
        } else {
            None
        };
        let symbol_kind = if options.filters.symbol_kind.is_some() {
            Some(string_array_by_name(&batch, "symbol_kind")?)
        } else {
            None
        };
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            let entity_name = required_string_value(entity_name, row, "entity_name")?;
            let qualified_name = qualified_name
                .map(|array| required_string_value(array, row, "qualified_name"))
                .transpose()?;
            let file_path = file_path
                .map(|array| required_string_value(array, row, "file_path"))
                .transpose()?;
            let symbol_kind = symbol_kind
                .map(|array| required_string_value(array, row, "symbol_kind"))
                .transpose()?;
            keep.push(row_matches(
                entity_name,
                qualified_name,
                file_path,
                symbol_kind,
                &options,
                glob.as_ref(),
            ));
        }
        Ok(BooleanArray::from(keep))
    };
    RowFilter::new(vec![Box::new(ArrowPredicateFn::new(projection, predicate))])
}

fn exact_search_pruning_predicate(opts: &SearchOptions) -> Option<StringPruningPredicate> {
    if !matches!(opts.mode, SearchMode::Exact) {
        return None;
    }

    let mut predicates = vec![StringPruningPredicate::any([
        StringPruningPredicate::eq("entity_name", opts.query.clone()),
        StringPruningPredicate::eq("qualified_name", opts.query.clone()),
    ])];
    if let Some(file) = &opts.filters.file {
        predicates.push(StringPruningPredicate::eq("file_path", file.clone()));
    }
    if let Some(symbol_kind) = &opts.filters.symbol_kind {
        predicates.push(StringPruningPredicate::eq(
            "symbol_kind",
            symbol_kind.clone(),
        ));
    }
    Some(StringPruningPredicate::all(predicates))
}

fn search_predicate_columns(opts: &SearchOptions) -> Vec<&'static str> {
    let mut columns = Vec::with_capacity(SEARCH_PREDICATE_COLUMNS.len());
    columns.push("entity_name");
    if matches!(opts.mode, SearchMode::Exact) {
        columns.push("qualified_name");
    }
    if opts.filters.file.is_some() || opts.filters.file_glob.is_some() {
        columns.push("file_path");
    }
    if opts.filters.symbol_kind.is_some() {
        columns.push("symbol_kind");
    }
    columns
}

fn row_matches(
    entity_name: &str,
    qualified_name: Option<&str>,
    file_path: Option<&str>,
    symbol_kind: Option<&str>,
    opts: &SearchOptions,
    glob: Option<&globset::GlobMatcher>,
) -> bool {
    if !row_matches_query(entity_name, qualified_name, opts) {
        return false;
    }
    row_matches_filters(file_path, symbol_kind, &opts.filters, glob)
}

fn row_matches_query(
    entity_name: &str,
    qualified_name: Option<&str>,
    opts: &SearchOptions,
) -> bool {
    match opts.mode {
        SearchMode::Exact => {
            entity_name == opts.query || qualified_name == Some(opts.query.as_str())
        }
        SearchMode::Prefix => entity_name.starts_with(&opts.query),
        SearchMode::Substring => entity_name.contains(&opts.query),
    }
}

fn row_matches_filters(
    file_path: Option<&str>,
    symbol_kind: Option<&str>,
    filters: &SearchFilters,
    glob: Option<&globset::GlobMatcher>,
) -> bool {
    if filters
        .symbol_kind
        .as_deref()
        .is_some_and(|filter| symbol_kind != Some(filter))
    {
        return false;
    }

    if filters
        .file
        .as_deref()
        .is_some_and(|filter| file_path != Some(filter))
    {
        return false;
    }

    if filters.file_glob.is_some()
        && !file_path.is_some_and(|path| glob.is_some_and(|glob| glob.is_match(path)))
    {
        return false;
    }

    true
}

fn string_eq_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    expected: String,
) -> RowFilter {
    string_in_row_filter(parquet_schema, column, HashSet::from([expected]))
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

fn string_eq_all_row_filter(
    parquet_schema: &SchemaDescriptor,
    expected: Vec<(&str, String)>,
) -> RowFilter {
    let columns = expected
        .iter()
        .map(|(column, _)| *column)
        .collect::<Vec<_>>();
    let projection = ProjectionMask::columns(parquet_schema, columns);
    let expected = expected
        .into_iter()
        .map(|(column, value)| (column.to_owned(), value))
        .collect::<Vec<_>>();
    let predicate = move |batch: RecordBatch| -> Result<BooleanArray, ArrowError> {
        let arrays = expected
            .iter()
            .map(|(column, _)| string_array_by_name(&batch, column))
            .collect::<Result<Vec<_>, _>>()?;
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            keep.push(arrays.iter().zip(&expected).all(|(values, (_, expected))| {
                !values.is_null(row) && values.value(row) == expected
            }));
        }
        Ok(BooleanArray::from(keep))
    };
    RowFilter::new(vec![Box::new(ArrowPredicateFn::new(projection, predicate))])
}

fn path_name_row_filter(
    parquet_schema: &SchemaDescriptor,
    expected_path: String,
    expected_name: String,
) -> RowFilter {
    let projection = ProjectionMask::columns(
        parquet_schema,
        ["file_path", "entity_name", "qualified_name"],
    );
    let predicate = move |batch: RecordBatch| -> Result<BooleanArray, ArrowError> {
        let file_path = string_array_by_name(&batch, "file_path")?;
        let entity_name = string_array_by_name(&batch, "entity_name")?;
        let qualified_name = string_array_by_name(&batch, "qualified_name")?;
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            let path_matches = !file_path.is_null(row) && file_path.value(row) == expected_path;
            let name_matches = (!entity_name.is_null(row)
                && entity_name.value(row) == expected_name)
                || (!qualified_name.is_null(row) && qualified_name.value(row) == expected_name);
            keep.push(path_matches && name_matches);
        }
        Ok(BooleanArray::from(keep))
    };
    RowFilter::new(vec![Box::new(ArrowPredicateFn::new(projection, predicate))])
}

fn unresolved_target_labels_for_symbol(symbol: &GraphSymbolArtifact) -> HashSet<String> {
    HashSet::from([
        symbol.entity_name.clone(),
        symbol.qualified_name.clone(),
        symbol.stable_symbol_id.clone(),
    ])
}

fn is_caller_relation(relation: RelationKind) -> bool {
    matches!(relation, RelationKind::Calls | RelationKind::References)
}

fn is_bare_stable_symbol_id(selector: &str) -> bool {
    selector.len() >= 16
        && selector
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn split_file_prefixes<'a>(selector: &'a str, separator: &str) -> Vec<(&'a str, &'a str)> {
    let mut prefixes = selector
        .match_indices(separator)
        .filter_map(|(index, _)| {
            let file_path = &selector[..index];
            let tail = &selector[index + separator.len()..];
            (!file_path.is_empty()).then_some((file_path, tail))
        })
        .collect::<Vec<_>>();
    prefixes.sort_by_key(|item| std::cmp::Reverse(item.0.len()));
    prefixes
}

fn first_token_contains_path_separator(selector: &str) -> bool {
    selector
        .split("::")
        .next()
        .is_some_and(|token| token.contains('/'))
}

fn compare_innermost(
    left: &GraphSymbolArtifact,
    right: &GraphSymbolArtifact,
) -> std::cmp::Ordering {
    left.line_range[0]
        .cmp(&right.line_range[0])
        .then_with(|| right.line_range[1].cmp(&left.line_range[1]))
        .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
}

fn resolution_from_symbols(symbols: Vec<GraphSymbolArtifact>) -> SelectorResolution {
    let mcp_tool_matches = symbols
        .iter()
        .filter(|symbol| symbol.symbol_kind == "mcp_tool")
        .collect::<Vec<_>>();
    if let [symbol] = mcp_tool_matches.as_slice() {
        return SelectorResolution::Resolved(resolved_symbol(symbol));
    }

    match symbols.as_slice() {
        [] => SelectorResolution::NotFound,
        [symbol] => SelectorResolution::Resolved(resolved_symbol(symbol)),
        _ => SelectorResolution::Ambiguous {
            candidates: candidate_rows(symbols),
        },
    }
}

fn resolved_symbol(symbol: &GraphSymbolArtifact) -> ResolvedSymbol {
    ResolvedSymbol {
        stable_symbol_id: symbol.stable_symbol_id.clone(),
    }
}

fn candidate_rows(symbols: Vec<GraphSymbolArtifact>) -> Vec<CandidateRow> {
    let mut candidates = symbols.iter().map(candidate_row).collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line_range[0].cmp(&right.line_range[0]))
            .then_with(|| left.line_range[1].cmp(&right.line_range[1]))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            .then_with(|| left.id.cmp(&right.id))
    });
    candidates
}

fn candidate_row(symbol: &GraphSymbolArtifact) -> CandidateRow {
    let uri = format!("{CODE_SYMBOL_URI_PREFIX}{}", symbol.stable_symbol_id);
    let selector = if symbol.qualified_name.is_empty() {
        uri.clone()
    } else {
        format!("{}::{}", symbol.file_path, symbol.qualified_name)
    };

    CandidateRow {
        selector,
        uri,
        id: symbol.stable_symbol_id.clone(),
        entity_name: symbol.entity_name.clone(),
        qualified_name: symbol.qualified_name.clone(),
        file_path: symbol.file_path.clone(),
        line_range: symbol.line_range,
        symbol_kind: symbol.symbol_kind.clone(),
        enclosing_scope: symbol.enclosing_scope.clone(),
    }
}

fn enclosing_scope_entity_name(symbol: &GraphSymbolArtifact) -> Option<String> {
    symbol
        .enclosing_scope
        .as_ref()
        .map(|scope| format!("{scope}::{}", symbol.entity_name))
}

fn symbols_from_batches(batches: Vec<RecordBatch>) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
    let mut symbols = Vec::new();
    for batch in batches {
        symbols.extend(symbols_from_batch(&batch)?);
    }
    Ok(symbols)
}

fn search_symbols_from_batch(batch: &RecordBatch) -> anyhow::Result<Vec<SearchSymbol>> {
    let stable_symbol_id = string_array_by_name(batch, "stable_symbol_id")?;
    let file_path = string_array_by_name(batch, "file_path")?;
    let line_start = i32_array_by_name(batch, "line_start")?;
    let line_end = i32_array_by_name(batch, "line_end")?;
    let entity_name = string_array_by_name(batch, "entity_name")?;
    let qualified_name = string_array_by_name(batch, "qualified_name")?;
    let symbol_kind = string_array_by_name(batch, "symbol_kind")?;
    let enclosing_scope = string_array_by_name(batch, "enclosing_scope")?;

    let mut symbols = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        symbols.push(SearchSymbol {
            stable_symbol_id: required_string_value(stable_symbol_id, row, "stable_symbol_id")?
                .to_owned(),
            entity_name: required_string_value(entity_name, row, "entity_name")?.to_owned(),
            qualified_name: required_string_value(qualified_name, row, "qualified_name")?
                .to_owned(),
            file_path: required_string_value(file_path, row, "file_path")?.to_owned(),
            line_range: [
                i32_to_usize(line_start.value(row), "line_start")?,
                i32_to_usize(line_end.value(row), "line_end")?,
            ],
            symbol_kind: required_string_value(symbol_kind, row, "symbol_kind")?.to_owned(),
            enclosing_scope: optional_string_value(enclosing_scope, row),
        });
    }
    Ok(symbols)
}

pub(crate) fn symbol_from_batch_row(
    batch: &RecordBatch,
    row: usize,
) -> anyhow::Result<GraphSymbolArtifact> {
    if row >= batch.num_rows() {
        bail!(
            "symbol row {row} out of range for batch with {} rows",
            batch.num_rows()
        );
    }
    SymbolBatchColumns::new(batch)?.symbol_at(row)
}

fn symbols_from_batch(batch: &RecordBatch) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
    let columns = SymbolBatchColumns::new(batch)?;
    let mut symbols = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        symbols.push(columns.symbol_at(row)?);
    }
    Ok(symbols)
}

pub(crate) struct SymbolBatchColumns<'a> {
    stable_symbol_id: &'a StringArray,
    file_path: &'a StringArray,
    byte_range_start: &'a Int64Array,
    byte_range_end: &'a Int64Array,
    line_start: &'a Int32Array,
    line_end: &'a Int32Array,
    entity_name: &'a StringArray,
    qualified_name: &'a StringArray,
    symbol_kind: &'a StringArray,
    anchor_hash: &'a StringArray,
    enclosing_scope: &'a StringArray,
}

impl<'a> SymbolBatchColumns<'a> {
    pub(crate) fn new(batch: &'a RecordBatch) -> anyhow::Result<Self> {
        Ok(Self {
            stable_symbol_id: string_array_by_name(batch, "stable_symbol_id")?,
            file_path: string_array_by_name(batch, "file_path")?,
            byte_range_start: i64_array_by_name(batch, "byte_range_start")?,
            byte_range_end: i64_array_by_name(batch, "byte_range_end")?,
            line_start: i32_array_by_name(batch, "line_start")?,
            line_end: i32_array_by_name(batch, "line_end")?,
            entity_name: string_array_by_name(batch, "entity_name")?,
            qualified_name: string_array_by_name(batch, "qualified_name")?,
            symbol_kind: string_array_by_name(batch, "symbol_kind")?,
            anchor_hash: string_array_by_name(batch, "anchor_hash")?,
            enclosing_scope: string_array_by_name(batch, "enclosing_scope")?,
        })
    }

    pub(crate) fn symbol_at(&self, row: usize) -> anyhow::Result<GraphSymbolArtifact> {
        Ok(GraphSymbolArtifact {
            stable_symbol_id: required_string_value(
                self.stable_symbol_id,
                row,
                "stable_symbol_id",
            )?
            .to_owned(),
            file_path: required_string_value(self.file_path, row, "file_path")?.to_owned(),
            byte_range: [
                i64_to_usize(self.byte_range_start.value(row), "byte_range_start")?,
                i64_to_usize(self.byte_range_end.value(row), "byte_range_end")?,
            ],
            line_range: [
                i32_to_usize(self.line_start.value(row), "line_start")?,
                i32_to_usize(self.line_end.value(row), "line_end")?,
            ],
            entity_name: required_string_value(self.entity_name, row, "entity_name")?.to_owned(),
            qualified_name: required_string_value(self.qualified_name, row, "qualified_name")?
                .to_owned(),
            symbol_kind: required_string_value(self.symbol_kind, row, "symbol_kind")?.to_owned(),
            anchor_hash: required_string_value(self.anchor_hash, row, "anchor_hash")?.to_owned(),
            enclosing_scope: optional_string_value(self.enclosing_scope, row),
        })
    }
}

fn file_manifests_from_batches(
    batches: Vec<RecordBatch>,
) -> anyhow::Result<Vec<GraphFileManifestEntry>> {
    let mut manifests = Vec::new();
    for batch in batches {
        manifests.extend(file_manifests_from_batch(&batch)?);
    }
    Ok(manifests)
}

fn file_manifests_from_batch(batch: &RecordBatch) -> anyhow::Result<Vec<GraphFileManifestEntry>> {
    let stable_file_id = string_array_by_name(batch, "stable_file_id")?;
    let path = string_array_by_name(batch, "path")?;
    let content_oid = string_array_by_name(batch, "content_oid")?;
    let node_ids = list_array_by_name(batch, "node_ids")?;

    let mut manifests = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        manifests.push(GraphFileManifestEntry {
            stable_file_id: required_string_value(stable_file_id, row, "stable_file_id")?
                .to_owned(),
            path: required_string_value(path, row, "path")?.to_owned(),
            content_oid: required_string_value(content_oid, row, "content_oid")?.to_owned(),
            node_ids: required_node_id_list_value(node_ids, row, "node_ids")?,
        });
    }
    Ok(manifests)
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

fn i32_array_by_name<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int32Array, ArrowError> {
    let index = batch.schema().index_of(name)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int32Array>()
        .ok_or_else(|| ArrowError::CastError(format!("expected int32 column `{name}`")))
}

fn i64_array_by_name<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int64Array, ArrowError> {
    let index = batch.schema().index_of(name)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| ArrowError::CastError(format!("expected int64 column `{name}`")))
}

fn list_array_by_name<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a ListArray, ArrowError> {
    let index = batch.schema().index_of(name)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| ArrowError::CastError(format!("expected list column `{name}`")))
}

fn required_string_value<'a>(
    values: &'a StringArray,
    index: usize,
    name: &str,
) -> Result<&'a str, ArrowError> {
    if values.is_null(index) {
        return Err(ArrowError::ComputeError(format!(
            "missing required string column `{name}`"
        )));
    }
    Ok(values.value(index))
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

fn i32_to_usize(value: i32, name: &str) -> anyhow::Result<usize> {
    usize::try_from(value).map_err(|_| anyhow!("column `{name}` has negative value {value}"))
}

fn i64_to_usize(value: i64, name: &str) -> anyhow::Result<usize> {
    usize::try_from(value).map_err(|_| anyhow!("column `{name}` has negative value {value}"))
}

fn i64_to_node_id(value: i64, name: &str) -> anyhow::Result<NodeId> {
    u64::try_from(value)
        .map(NodeId)
        .map_err(|_| anyhow!("column `{name}` has negative value {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::parquet::write_artifact_parquet;
    use crate::{
        search_symbols, ChangeKind, CommitArtifact, CommitIndexArtifact, Confidence, EdgeEndpoint,
        GraphEdgeKind, GraphIndexHeader, GraphSymbolArtifact, RelationKind, RenamePrev,
        SearchFilters, SearchMode, SnapshotKey, SymbolSnapshotArtifact, TemporalEdgeArtifact,
        WalkStrategy, WriteOptions, GRAPH_INDEX_VERSION_TEMPORAL,
    };
    use arrow_array::ArrayRef;
    use arrow_schema::{DataType, Field, Schema};
    use bytes::Bytes;
    use futures::future::{BoxFuture, FutureExt as _};
    use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};
    use parquet::arrow::async_reader::AsyncFileReader;
    use parquet::arrow::ArrowWriter;
    use parquet::errors::{ParquetError, Result as ParquetResult};
    use parquet::file::metadata::PageIndexPolicy;
    use parquet::file::metadata::{ParquetMetaData, ParquetMetaDataReader};
    use parquet::file::properties::WriterProperties;
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::{Seek as _, SeekFrom, Write as _};
    use std::ops::Range;
    use std::sync::Mutex as StdMutex;

    #[derive(Clone)]
    struct RecordingRangeReader {
        bytes: Bytes,
        ranges: Arc<StdMutex<Vec<Range<u64>>>>,
    }

    impl RecordingRangeReader {
        fn new(bytes: Bytes) -> Self {
            Self {
                bytes,
                ranges: Arc::new(StdMutex::new(Vec::new())),
            }
        }

        fn observed_ranges(&self) -> Vec<Range<u64>> {
            self.ranges.lock().expect("range log lock").clone()
        }
    }

    impl AsyncFileReader for RecordingRangeReader {
        fn get_bytes(&mut self, range: Range<u64>) -> BoxFuture<'_, ParquetResult<Bytes>> {
            self.ranges
                .lock()
                .expect("range log lock")
                .push(range.clone());
            let start = usize::try_from(range.start);
            let end = usize::try_from(range.end);
            let bytes = self.bytes.clone();
            async move {
                let start = start.map_err(|error| ParquetError::External(Box::new(error)))?;
                let end = end.map_err(|error| ParquetError::External(Box::new(error)))?;
                if start > end || end > bytes.len() {
                    return Err(ParquetError::General(format!(
                        "invalid byte range {start}..{end} for {} bytes",
                        bytes.len()
                    )));
                }
                Ok(bytes.slice(start..end))
            }
            .boxed()
        }

        fn get_metadata<'a>(
            &'a mut self,
            _options: Option<&'a parquet::arrow::arrow_reader::ArrowReaderOptions>,
        ) -> BoxFuture<'a, ParquetResult<Arc<ParquetMetaData>>> {
            let file_size = self.bytes.len() as u64;
            async move {
                let metadata = ParquetMetaDataReader::new()
                    .load_and_finish(&mut *self, file_size)
                    .await?;
                Ok(Arc::new(metadata))
            }
            .boxed()
        }
    }

    fn covered_range_bytes(ranges: &[Range<u64>]) -> u64 {
        let mut ranges = ranges.to_vec();
        ranges.sort_by_key(|range| (range.start, range.end));
        let mut covered = 0_u64;
        let mut current: Option<Range<u64>> = None;
        for range in ranges {
            match &mut current {
                Some(active) if range.start <= active.end => {
                    active.end = active.end.max(range.end);
                }
                Some(active) => {
                    covered += active.end - active.start;
                    current = Some(range);
                }
                None => current = Some(range),
            }
        }
        covered + current.map_or(0, |range| range.end - range.start)
    }

    fn artifact(symbols: Vec<GraphSymbolArtifact>) -> GraphIndexArtifact {
        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols,
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn temporal_artifact() -> GraphIndexArtifact {
        let old_snapshot = snapshot("old-root", "commit-a", "foo");
        let new_snapshot = snapshot("new-root", "commit-b", "bar");
        let unrelated_snapshot = snapshot("unrelated", "commit-b", "baz");
        let mut artifact = artifact(Vec::new());
        artifact.header.graph_index_version = GRAPH_INDEX_VERSION_TEMPORAL.to_owned();
        artifact.commits = commits();
        artifact.symbol_snapshots = vec![
            old_snapshot.clone(),
            new_snapshot.clone(),
            unrelated_snapshot.clone(),
        ];
        artifact.temporal_edges = vec![
            temporal_touch("commit-a", old_snapshot.key.clone(), ChangeKind::Added),
            temporal_touch(
                "commit-b",
                new_snapshot.key.clone(),
                ChangeKind::RenamedFrom(RenamePrev::Symbol(old_snapshot.key.clone())),
            ),
            temporal_rename(old_snapshot.key, new_snapshot.key),
            temporal_touch("commit-b", unrelated_snapshot.key, ChangeKind::Added),
        ];
        artifact
    }

    fn commits() -> Vec<CommitArtifact> {
        vec![
            CommitArtifact {
                sha: "commit-a".to_owned(),
                parents: Vec::new(),
                author_time: 1,
                author_name: String::new(),
                author_email: String::new(),
                summary: "add foo".to_owned(),
            },
            CommitArtifact {
                sha: "commit-b".to_owned(),
                parents: vec!["commit-a".to_owned()],
                author_time: 2,
                author_name: String::new(),
                author_email: String::new(),
                summary: "rename foo to bar".to_owned(),
            },
        ]
    }

    fn commit_index(commits: Vec<CommitArtifact>) -> CommitIndexArtifact {
        CommitIndexArtifact {
            schema_version: 7,
            commits,
            refs: BTreeMap::from([("HEAD".to_owned(), "commit-b".to_owned())]),
            indexed_at: "2026-05-26T00:00:00Z".to_owned(),
            walk_strategy: WalkStrategy::Reachable,
        }
    }

    fn snapshot(id: &str, commit: &str, entity_name: &str) -> SymbolSnapshotArtifact {
        SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: id.to_owned(),
                commit: commit.to_owned(),
            },
            file_path: "src/lib.rs".to_owned().into(),
            entity_name: entity_name.to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 8],
            line_range: [1, 2],
            anchor_hash: format!("hash-{id}-{commit}"),
            tokens: vec![entity_name.to_owned()],
        }
    }

    fn temporal_touch(
        commit: &str,
        key: SnapshotKey,
        change_kind: ChangeKind,
    ) -> TemporalEdgeArtifact {
        TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: commit.to_owned(),
            },
            target: EdgeEndpoint::Snapshot { key },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(change_kind),
        }
    }

    fn temporal_rename(from: SnapshotKey, to: SnapshotKey) -> TemporalEdgeArtifact {
        TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot { key: from.clone() },
            target: EdgeEndpoint::Snapshot { key: to },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(from))),
        }
    }

    fn symbol(id: &str, entity_name: &str) -> GraphSymbolArtifact {
        symbol_at(id, entity_name, "src/lib.rs")
    }

    fn symbol_at(id: &str, entity_name: &str, file_path: &str) -> GraphSymbolArtifact {
        GraphSymbolArtifact {
            stable_symbol_id: id.to_owned(),
            file_path: file_path.to_owned(),
            byte_range: [0, 8],
            line_range: [1, 2],
            entity_name: entity_name.to_owned(),
            qualified_name: format!("crate::{entity_name}"),
            symbol_kind: "function".to_owned(),
            anchor_hash: format!("hash-{id}"),
            enclosing_scope: None,
        }
    }

    fn edge(
        source_stable_symbol_id: &str,
        target_stable_symbol_id: Option<&str>,
        target_label: Option<&str>,
        relation: RelationKind,
    ) -> GraphEdgeArtifact {
        GraphEdgeArtifact {
            source_stable_symbol_id: source_stable_symbol_id.to_owned(),
            target_stable_symbol_id: target_stable_symbol_id.map(str::to_owned),
            target_label: target_label.map(str::to_owned),
            import_path: None,
            relation,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: None,
            bind_method: None,
            receiver_text: None,
            scope_text: None,
        }
    }

    fn ids(result: &SearchResult) -> Vec<String> {
        result
            .candidates
            .iter()
            .map(|symbol| symbol.stable_symbol_id.clone())
            .collect()
    }

    fn graph_symbol_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("stable_symbol_id", DataType::Utf8, false),
            Field::new("file_path", DataType::Utf8, false),
            Field::new("byte_range_start", DataType::Int64, false),
            Field::new("byte_range_end", DataType::Int64, false),
            Field::new("line_start", DataType::Int32, false),
            Field::new("line_end", DataType::Int32, false),
            Field::new("entity_name", DataType::Utf8, false),
            Field::new("qualified_name", DataType::Utf8, false),
            Field::new("symbol_kind", DataType::Utf8, false),
            Field::new("anchor_hash", DataType::Utf8, false),
            Field::new("enclosing_scope", DataType::Utf8, true),
        ]));
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(vec!["s1", "s2", "s3"])),
            Arc::new(StringArray::from(vec!["src/a.rs", "src/b.rs", "src/c.rs"])),
            Arc::new(Int64Array::from(vec![0, 10, 20])),
            Arc::new(Int64Array::from(vec![9, 19, 29])),
            Arc::new(Int32Array::from(vec![1, 5, 9])),
            Arc::new(Int32Array::from(vec![2, 6, 10])),
            Arc::new(StringArray::from(vec!["alpha", "beta", "gamma"])),
            Arc::new(StringArray::from(vec![
                "crate::alpha",
                "crate::beta",
                "crate::gamma",
            ])),
            Arc::new(StringArray::from(vec!["function", "struct", "method"])),
            Arc::new(StringArray::from(vec!["hash-a", "hash-b", "hash-c"])),
            Arc::new(StringArray::from(vec![
                Some("crate"),
                None,
                Some("crate::nested"),
            ])),
        ];

        RecordBatch::try_new(schema, columns).expect("symbol batch is valid")
    }

    #[test]
    fn symbol_from_batch_row_matches_bulk_conversion_for_every_row() {
        let batch = graph_symbol_batch();
        let expected = symbols_from_batch(&batch).expect("bulk conversion succeeds");

        assert_eq!(expected.len(), batch.num_rows());
        for (row, expected_symbol) in expected.into_iter().enumerate() {
            let actual = symbol_from_batch_row(&batch, row).expect("row conversion succeeds");
            assert_eq!(actual, expected_symbol);
        }
    }

    #[test]
    fn symbol_from_batch_row_rejects_out_of_range_row() {
        let batch = graph_symbol_batch();

        symbol_from_batch_row(&batch, batch.num_rows())
            .expect_err("out-of-range row must return an error");
    }

    #[test]
    fn in_memory_client_search_symbols_delegates_to_search_symbols() {
        let artifact = Arc::new(artifact(vec![
            symbol("s1", "target"),
            symbol("s2", "target_extra"),
            symbol("s3", "other"),
        ]));
        let options = SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Prefix,
            filters: SearchFilters::default(),
            limit: 20,
        };
        let expected = search_symbols(&artifact, &options);
        let client = InMemoryClient::new(Arc::clone(&artifact));

        let actual = client.search_symbols(&options).expect("search succeeds");

        assert_eq!(ids(&actual), ids(&expected));
        assert_eq!(actual.total_matches, expected.total_matches);
        assert_eq!(actual.truncated, expected.truncated);
    }

    #[test]
    fn non_identity_overlay_search_reports_each_finalization_stage_once() {
        let base = InMemoryClient::new(Arc::new(artifact(vec![symbol_at(
            "base",
            "target",
            "src/lib.rs",
        )])));
        let overlay = OverlayClient::from_artifacts(
            &base,
            Arc::new(artifact(vec![symbol_at(
                "replacement",
                "target",
                "src/lib.rs",
            )])),
            HashSet::from(["src/lib.rs".to_owned()]),
        )
        .expect("construct non-identity overlay");
        let options = SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Exact,
            filters: SearchFilters::default(),
            limit: 20,
        };
        let mut measurements = OverlayFinalizationMeasurements::default();

        overlay
            .search_symbols_with_measurements(&options, &mut measurements)
            .expect("measured overlay search succeeds");

        assert_eq!(
            measurements,
            OverlayFinalizationMeasurements {
                shadow_filters: 1,
                result_merges: 1,
                overlay_sorts: 1,
                stable_id_deduplications: 1,
            }
        );
        assert_eq!(measurements.total(), 4);
    }

    #[test]
    fn identity_overlay_search_reports_zero_finalization_stages() {
        let base = InMemoryClient::new(Arc::new(artifact(vec![symbol("base", "target")])));
        let overlay =
            OverlayClient::from_artifacts(&base, Arc::new(artifact(Vec::new())), HashSet::new())
                .expect("construct identity overlay");
        let options = SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Exact,
            filters: SearchFilters::default(),
            limit: 20,
        };
        let mut measurements = OverlayFinalizationMeasurements::default();

        overlay
            .search_symbols_with_measurements(&options, &mut measurements)
            .expect("measured identity search succeeds");

        assert_eq!(measurements, OverlayFinalizationMeasurements::default());
        assert_eq!(measurements.total(), 0);
    }

    #[test]
    fn internal_search_overlay_shadows_before_public_cap_and_matches_fresh_oracle() {
        let base_symbols = (0..202)
            .map(|index| {
                symbol_at(
                    &format!("base-{index:03}"),
                    &format!("match_{index:03}"),
                    &format!("src/{index:03}.rs"),
                )
            })
            .collect::<Vec<_>>();
        let shadowed_path = base_symbols[0].file_path.clone();
        let replacement = symbol_at("replacement", "replacement", &shadowed_path);
        let fresh_symbols = base_symbols
            .iter()
            .skip(1)
            .cloned()
            .chain(std::iter::once(replacement.clone()))
            .collect::<Vec<_>>();
        let base = InMemoryClient::new(Arc::new(artifact(base_symbols)));
        let overlay = OverlayClient::from_artifacts(
            &base,
            Arc::new(artifact(vec![replacement])),
            HashSet::from([shadowed_path]),
        )
        .expect("construct overlay");
        let oracle = InMemoryClient::new(Arc::new(artifact(fresh_symbols)));
        let options = SearchOptions {
            query: "match_".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters::default(),
            limit: 200,
        };

        let actual = overlay.search_symbols(&options).expect("overlay search");
        let expected = oracle.search_symbols(&options).expect("oracle search");
        let actual_ids = ids(&actual);
        let expected_ids = ids(&expected);
        let actual_digest = blake3::hash(actual_ids.join("\n").as_bytes()).to_hex();
        let expected_digest = blake3::hash(expected_ids.join("\n").as_bytes()).to_hex();

        eprintln!(
            "overlay search evidence internal_total={} public_count={} digest={} \
             oracle_total={} oracle_count={} oracle_digest={}",
            actual.total_matches,
            actual.candidates.len(),
            actual_digest,
            expected.total_matches,
            expected.candidates.len(),
            expected_digest,
        );
        assert_eq!(actual.total_matches, 201);
        assert_eq!(actual.candidates.len(), 200);
        assert!(actual.truncated);
        assert_eq!(actual, expected);
    }

    #[test]
    fn internal_search_parquet_unbounded_sentinel_returns_every_match() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut artifact = artifact(
            (0..202)
                .map(|index| symbol(&format!("s-{index:03}"), &format!("match_{index:03}")))
                .collect(),
        );
        artifact.symbol_node_ids = (1..=artifact.symbols.len())
            .map(|id| NodeId(id as u64))
            .collect();
        let parquet_dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");
        let options = SearchOptions {
            query: "match_".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters::default(),
            limit: usize::MAX,
        };

        let result = parquet.search_symbols(&options).expect("unbounded search");

        assert_eq!(result.total_matches, 202);
        assert_eq!(result.candidates.len(), 202);
        assert!(!result.truncated);
    }

    #[test]
    fn parquet_client_projects_graph_files_and_batches_symbol_hydration() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut graph = artifact(vec![symbol("sym-a", "alpha"), symbol("sym-b", "beta")]);
        graph.symbol_node_ids = vec![NodeId(1), NodeId(2)];
        graph.files = vec![GraphFileArtifact {
            stable_file_id: "file-lib".to_owned(),
            file_path: "src/lib.rs".to_owned(),
        }];
        graph.file_node_ids = vec![NodeId(3)];
        graph.file_manifests = vec![GraphFileManifestEntry {
            stable_file_id: "file-lib".to_owned(),
            path: "src/lib.rs".to_owned(),
            content_oid: "content-lib".to_owned(),
            node_ids: vec![NodeId(1), NodeId(2)],
        }];
        graph.diagnostics = vec!["fixture extraction warning".to_owned()];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())
                .expect("write parquet artifact");
        let client = ParquetClient::open(&parquet_dir).expect("open parquet client");

        let files = client.files().expect("project graph files");
        let diagnostics = client.diagnostics().expect("project diagnostics");
        let symbols = client
            .symbols_by_stable_ids(&["sym-b".to_owned(), "missing".to_owned()])
            .expect("hydrate selected symbols in one batch");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].stable_file_id, "file-lib");
        assert_eq!(files[0].file_path, "src/lib.rs");
        assert_eq!(diagnostics, vec!["fixture extraction warning"]);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].stable_symbol_id, "sym-b");
        assert_eq!(symbols[0].entity_name, "beta");
    }

    #[test]
    fn parquet_client_reuses_one_hot_query_index_for_scan_searches() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut graph = artifact(vec![
            symbol("sym-a", "alpha_target"),
            symbol("sym-b", "beta_target"),
        ]);
        graph.symbol_node_ids = vec![NodeId(1), NodeId(2)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())
                .expect("write parquet artifact");
        let client = ParquetClient::open(&parquet_dir).expect("open parquet client");

        assert_eq!(client.hot_query_index_build_count(), 0);

        for mode in [SearchMode::Prefix, SearchMode::Substring] {
            let result = client
                .search_symbols(&SearchOptions {
                    query: "alpha".to_owned(),
                    mode,
                    filters: SearchFilters::default(),
                    limit: 20,
                })
                .expect("scan search succeeds");
            assert_eq!(result.total_matches, 1);
            assert_eq!(result.candidates[0].stable_symbol_id, "sym-a");
        }

        assert_eq!(client.hot_query_index_build_count(), 1);
    }

    #[tokio::test]
    async fn async_parquet_search_index_matches_local_without_whole_object_read() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut graph = artifact(vec![
            symbol("sym-a", "alpha_target"),
            symbol("sym-b", "beta_target"),
        ]);
        graph.symbol_node_ids = vec![NodeId(1), NodeId(2)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())
                .expect("write parquet artifact");
        let local = ParquetClient::open(&parquet_dir).expect("open local parquet client");
        let options = SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters::default(),
            limit: 20,
        };
        let expected = local.search_symbols(&options).expect("local search");

        let nodes = Bytes::from(std::fs::read(parquet_dir.join("nodes.parquet")).expect("nodes"));
        let reader = RecordingRangeReader::new(nodes.clone());
        let observations = reader.clone();
        let remote = ParquetSearchIndex::read_async(reader)
            .await
            .expect("read remote search index");
        let actual = remote.search_symbols(&options);

        assert_eq!(actual.total_matches, expected.total_matches);
        assert_eq!(actual.truncated, expected.truncated);
        assert_eq!(actual.candidates, expected.candidates);
        let ranges = observations.observed_ranges();
        assert!(!ranges.is_empty());
        assert!(covered_range_bytes(&ranges) < nodes.len() as u64);
    }

    #[tokio::test]
    async fn async_parquet_search_index_reads_multiple_row_groups() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nodes_path = tempdir.path().join("nodes.parquet");
        let batch = graph_symbol_batch();
        let properties = WriterProperties::builder()
            .set_max_row_group_row_count(Some(1))
            .build();
        let mut writer = ArrowWriter::try_new(
            File::create(&nodes_path).expect("create nodes parquet"),
            batch.schema(),
            Some(properties),
        )
        .expect("create nodes writer");
        writer.write(&batch).expect("write nodes batch");
        writer.close().expect("close nodes writer");

        let nodes = Bytes::from(std::fs::read(nodes_path).expect("read nodes parquet"));
        let metadata = ParquetRecordBatchReaderBuilder::try_new(nodes.clone())
            .expect("read nodes metadata")
            .metadata()
            .clone();
        assert_eq!(metadata.num_row_groups(), 3);

        let remote = ParquetSearchIndex::read_async(RecordingRangeReader::new(nodes))
            .await
            .expect("read all remote row groups");
        let actual = remote.search_symbols(&SearchOptions {
            query: "a".to_owned(),
            mode: SearchMode::Substring,
            filters: SearchFilters::default(),
            limit: 20,
        });

        assert_eq!(actual.total_matches, 3);
        let mut actual_ids = ids(&actual);
        actual_ids.sort();
        assert_eq!(actual_ids, vec!["s1", "s2", "s3"]);
    }

    #[test]
    fn parquet_symbol_lookup_reuses_ready_hot_query_index() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![symbol("sym-a", "alpha_target")]);
        graph.symbol_node_ids = vec![NodeId(1)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;

        client.search_symbols(&SearchOptions {
            query: "alpha".to_owned(),
            mode: SearchMode::Prefix,
            filters: SearchFilters::default(),
            limit: 20,
        })?;
        assert_eq!(client.hot_query_index_build_count(), 1);
        std::fs::remove_file(parquet_dir.join("nodes.parquet"))?;

        let symbol = client.symbol_by_id("sym-a")?.expect("cached symbol");

        assert_eq!(symbol.stable_symbol_id, "sym-a");
        assert_eq!(symbol.entity_name, "alpha_target");
        assert_eq!(client.hot_query_index_build_count(), 1);
        Ok(())
    }

    #[test]
    fn parquet_hot_symbol_search_does_not_depend_on_edge_shards() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![symbol("sym-a", "alpha_target")]);
        graph.symbol_node_ids = vec![NodeId(1)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;

        std::fs::remove_file(parquet_dir.join("edges.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_by_dst.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_unresolved.parquet"))?;

        let result = client.search_symbols(&SearchOptions {
            query: "alpha".to_owned(),
            mode: SearchMode::Prefix,
            filters: SearchFilters::default(),
            limit: 20,
        })?;

        assert_eq!(result.total_matches, 1);
        assert_eq!(result.candidates[0].stable_symbol_id, "sym-a");
        Ok(())
    }

    #[test]
    fn parquet_empty_adjacency_queries_do_not_depend_on_edge_shards() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![symbol("sym-a", "alpha_target")]);
        graph.symbol_node_ids = vec![NodeId(1)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;

        std::fs::remove_file(parquet_dir.join("edges.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_by_dst.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_unresolved.parquet"))?;

        assert!(client.try_find_caller_edges("missing")?.is_empty());
        assert!(client.try_find_callee_edges("missing")?.is_empty());
        assert!(client
            .find_unresolved_caller_edges_by_labels(&HashSet::new())
            .is_empty());
        Ok(())
    }

    #[test]
    fn parquet_hot_query_index_preserves_broad_search_ranking_and_counts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let symbols = (0..512)
            .map(|index| {
                let name = format!("shared_symbol_{index:04}");
                symbol(&format!("sym-{index:04}"), &name)
            })
            .collect::<Vec<_>>();
        let mut graph = artifact(symbols);
        graph.symbol_node_ids = (1..=graph.symbols.len())
            .map(|id| NodeId(id as u64))
            .collect();
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())
                .expect("write parquet artifact");
        let client = ParquetClient::open(&parquet_dir).expect("open parquet client");

        for (mode, query) in [(SearchMode::Prefix, "s"), (SearchMode::Substring, "a")] {
            for limit in [0, 20, usize::MAX] {
                let options = SearchOptions {
                    query: query.to_owned(),
                    mode,
                    filters: SearchFilters::default(),
                    limit,
                };
                assert_eq!(
                    client
                        .search_symbols(&options)
                        .expect("hot broad search succeeds"),
                    search_symbols(&graph, &options),
                );
            }
        }
    }

    #[test]
    fn parquet_hot_query_index_reuses_loaded_adjacency() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![
            symbol("source", "alpha_source"),
            symbol("target", "beta_target"),
            symbol("unresolved", "unresolved_caller"),
        ]);
        graph.symbol_node_ids = vec![NodeId(1), NodeId(2), NodeId(3)];
        graph.edges = vec![
            GraphEdgeArtifact {
                source_stable_symbol_id: "source".to_owned(),
                target_stable_symbol_id: Some("target".to_owned()),
                target_label: Some("beta_target".to_owned()),
                import_path: None,
                relation: RelationKind::Calls,
                confidence: Confidence::SyntaxExact,
                confidence_score: 1.0,
                change_kind: None,
                edge_kind: Some(GraphEdgeKind::Calls),
                bind_method: None,
                receiver_text: None,
                scope_text: None,
            },
            GraphEdgeArtifact {
                source_stable_symbol_id: "unresolved".to_owned(),
                target_stable_symbol_id: None,
                target_label: Some("beta_target".to_owned()),
                import_path: None,
                relation: RelationKind::Calls,
                confidence: Confidence::SyntaxExact,
                confidence_score: 1.0,
                change_kind: None,
                edge_kind: Some(GraphEdgeKind::Calls),
                bind_method: None,
                receiver_text: None,
                scope_text: None,
            },
        ];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;

        client.search_symbols(&SearchOptions {
            query: "alpha".to_owned(),
            mode: SearchMode::Prefix,
            filters: SearchFilters::default(),
            limit: 20,
        })?;
        assert_eq!(client.hot_query_index_build_count(), 1);
        assert_eq!(client.hot_adjacency_index_build_count(), 0);

        let initial_callers = client.try_find_caller_edges("target")?;
        let initial_callees = client.try_find_callee_edges("source")?;
        assert_eq!(initial_callers.len(), 2);
        assert_eq!(initial_callees.len(), 1);
        assert_eq!(client.hot_adjacency_index_build_count(), 1);

        std::fs::remove_file(parquet_dir.join("edges.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_by_dst.parquet"))?;
        std::fs::remove_file(parquet_dir.join("edges_unresolved.parquet"))?;
        std::fs::remove_file(parquet_dir.join("nodes.parquet"))?;

        let callers = client.try_find_caller_edges("target")?;
        let callees = client.try_find_callee_edges("source")?;

        assert_eq!(callers.len(), 2);
        assert_eq!(callees.len(), 1);
        assert_eq!(client.hot_query_index_build_count(), 1);
        assert_eq!(client.hot_adjacency_index_build_count(), 1);
        Ok(())
    }

    #[test]
    fn parquet_hot_adjacency_matches_in_memory_caller_callee_records() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![
            symbol("sym-source", "source_fn"),
            symbol("sym-target", "target_fn"),
            symbol("sym-unresolved-caller", "unresolved_caller"),
            symbol("sym-stable-label-caller", "stable_label_caller"),
        ]);
        graph.symbol_node_ids = vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4)];
        graph.edges = vec![
            edge(
                "sym-source",
                Some("sym-target"),
                Some("target_fn"),
                RelationKind::Calls,
            ),
            edge(
                "sym-source",
                None,
                Some("external_target"),
                RelationKind::Calls,
            ),
            edge(
                "sym-unresolved-caller",
                None,
                Some("target_fn"),
                RelationKind::Calls,
            ),
            edge(
                "sym-stable-label-caller",
                None,
                Some("sym-target"),
                RelationKind::References,
            ),
            edge(
                "sym-source",
                Some("sym-target"),
                Some("target_fn"),
                RelationKind::Contains,
            ),
        ];
        let oracle = InMemoryClient::new(Arc::new(graph.clone()));
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;
        let target_labels = HashSet::from([
            "target_fn".to_owned(),
            "crate::target_fn".to_owned(),
            "sym-target".to_owned(),
        ]);

        assert_eq!(
            client.try_find_caller_edges("sym-target")?,
            oracle.find_caller_edges("sym-target")
        );
        assert_eq!(
            client.try_find_callee_edges("sym-source")?,
            oracle.find_callee_edges("sym-source")
        );
        assert_eq!(
            client.find_unresolved_caller_edges_by_labels(&target_labels),
            oracle.find_unresolved_caller_edges_by_labels(&target_labels)
        );
        Ok(())
    }

    #[test]
    fn in_memory_client_temporal_index_is_cached() {
        let client = InMemoryClient::new(Arc::new(artifact(Vec::new())));

        let first = client.temporal_index();
        let second = client.temporal_index();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn parquet_client_loads_page_indexes_for_pruning() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(vec![symbol("sym-a", "alpha")]);
        graph.symbol_node_ids = vec![NodeId(1)];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;

        let client = ParquetClient::open(&parquet_dir)?;

        assert!(client.nodes_metadata.metadata().column_index().is_some());
        assert!(client.nodes_metadata.metadata().offset_index().is_some());
        Ok(())
    }

    #[test]
    fn parquet_client_reuses_non_node_metadata_after_first_query() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut graph = artifact(Vec::new());
        graph.file_manifests = vec![GraphFileManifestEntry {
            stable_file_id: "file-a".to_owned(),
            path: "src/lib.rs".to_owned(),
            content_oid: "content-a".to_owned(),
            node_ids: vec![NodeId(1)],
        }];
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;

        let first = client
            .file_manifest_by_path_inner("src/lib.rs")?
            .expect("manifest exists before footer damage");
        let path = parquet_dir.join("file_manifests.parquet");
        let mut file = OpenOptions::new().write(true).open(&path)?;
        file.seek(SeekFrom::End(-8))?;
        file.write_all(&[0xff; 4])?;
        file.sync_all()?;

        let second = client
            .file_manifest_by_path_inner("src/lib.rs")?
            .expect("cached metadata avoids rereading the damaged footer");

        assert_eq!(second, first);
        Ok(())
    }

    #[test]
    fn exact_symbol_lookup_prunes_non_matching_pages_and_row_groups() -> anyhow::Result<()> {
        const CANDIDATE_COUNT: usize = 128;

        let tempdir = tempfile::tempdir()?;
        let mut symbols = Vec::with_capacity(PARQUET_ROW_GROUP_SIZE + 2);
        for index in 0..(PARQUET_ROW_GROUP_SIZE / 2) {
            symbols.push(symbol(
                &format!("a-{index:08x}-{}", "x".repeat(160)),
                "alpha",
            ));
        }
        let candidates = (0..CANDIDATE_COUNT)
            .map(|index| format!("m-target-{index:03}"))
            .collect::<Vec<_>>();
        symbols.extend(candidates.iter().map(|id| symbol(id, "target")));
        for index in 0..(PARQUET_ROW_GROUP_SIZE / 2 - CANDIDATE_COUNT) {
            symbols.push(symbol(
                &format!("z-{index:08x}-{}", "y".repeat(160)),
                "zeta",
            ));
        }
        for value in &mut symbols {
            value.file_path = "src/a.rs".to_owned();
        }
        let mut low = symbol("a-second", "low");
        low.file_path = "src/z.rs".to_owned();
        let mut high = symbol("z-second", "high");
        high.file_path = "src/z.rs".to_owned();
        symbols.extend([low, high]);

        let mut graph = artifact(symbols);
        graph.symbol_node_ids = (1..=graph.symbols.len())
            .map(|id| NodeId(u64::try_from(id).expect("test node id fits u64")))
            .collect();
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;
        let nodes_path = parquet_dir.join("nodes.parquet");

        let bloom_builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&nodes_path)?)?;
        let second_row_group_bloom = bloom_builder
            .get_row_group_column_bloom_filter(1, 0)?
            .expect("stable_symbol_id Bloom filter exists");
        let target = candidates
            .iter()
            .find(|candidate| !second_row_group_bloom.check(candidate.as_str()))
            .expect("at least one candidate is definitely absent from the second row group")
            .clone();

        let indexed_metadata = ArrowReaderMetadata::load(
            &File::open(&nodes_path)?,
            ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional),
        )?;
        let metadata = indexed_metadata.metadata();
        assert_eq!(metadata.num_row_groups(), 2);
        let first_row_group_pages =
            metadata.offset_index().expect("offset index loaded")[0][0].page_locations();
        assert!(
            first_row_group_pages.len() > 1,
            "fixture must create multiple stable_symbol_id pages"
        );
        let page_to_skip = first_row_group_pages
            .last()
            .expect("first row group has a final page");
        let second_row_group_range = metadata.row_group(1).column(0).byte_range();

        let mut file = OpenOptions::new().write(true).open(&nodes_path)?;
        file.seek(SeekFrom::Start(
            u64::try_from(page_to_skip.offset).expect("page offset is non-negative"),
        ))?;
        file.write_all(&vec![
            0;
            usize::try_from(page_to_skip.compressed_page_size)
                .expect("page size is non-negative")
        ])?;
        file.seek(SeekFrom::Start(second_row_group_range.0))?;
        file.write_all(&vec![
            0;
            usize::try_from(second_row_group_range.1)
                .expect("column chunk size fits usize")
        ])?;
        file.sync_all()?;

        client
            .search_symbols(&SearchOptions {
                query: "m-target".to_owned(),
                mode: SearchMode::Prefix,
                filters: SearchFilters::default(),
                limit: 20,
            })
            .expect_err("full hot-index scan should observe corrupt non-matching data");

        let actual = client
            .symbol_by_id(&target)?
            .expect("indexed pruning skips corrupt non-matching data");

        assert_eq!(actual.stable_symbol_id, target);
        Ok(())
    }

    #[test]
    fn exact_search_prunes_non_matching_row_groups() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let mut symbols = Vec::with_capacity(PARQUET_ROW_GROUP_SIZE + 2);
        for index in 0..PARQUET_ROW_GROUP_SIZE {
            let name = if index == 0 {
                "target".to_owned()
            } else {
                format!("alpha_{index:05}")
            };
            let mut value = symbol(&format!("a-{index:05}"), &name);
            value.file_path = "src/a.rs".to_owned();
            symbols.push(value);
        }
        for (id, name) in [("z-low", "low"), ("z-high", "high")] {
            let mut value = symbol(id, name);
            value.file_path = "src/z.rs".to_owned();
            symbols.push(value);
        }

        let mut graph = artifact(symbols);
        graph.symbol_node_ids = (1..=graph.symbols.len())
            .map(|id| NodeId(u64::try_from(id).expect("test node id fits u64")))
            .collect();
        let parquet_dir =
            write_artifact_parquet(&graph, tempdir.path(), WriteOptions::default(), Vec::new())?;
        let client = ParquetClient::open(&parquet_dir)?;
        let nodes_path = parquet_dir.join("nodes.parquet");
        let metadata = client.nodes_metadata.metadata();
        assert_eq!(metadata.num_row_groups(), 2);

        let entity_name_column = metadata
            .file_metadata()
            .schema_descr()
            .columns()
            .iter()
            .position(|column| column.path().string() == "entity_name")
            .expect("entity_name column exists");
        let second_row_group_range = metadata
            .row_group(1)
            .column(entity_name_column)
            .byte_range();
        let mut file = OpenOptions::new().write(true).open(&nodes_path)?;
        file.seek(SeekFrom::Start(second_row_group_range.0))?;
        file.write_all(&vec![
            0;
            usize::try_from(second_row_group_range.1)
                .expect("column chunk size fits usize")
        ])?;
        file.sync_all()?;

        let result = client.search_symbols(&SearchOptions {
            query: "target".to_owned(),
            mode: SearchMode::Exact,
            filters: SearchFilters::default(),
            limit: 20,
        })?;

        assert_eq!(result.total_matches, 1);
        assert_eq!(result.candidates[0].entity_name, "target");
        Ok(())
    }

    #[test]
    fn parquet_client_symbol_history_returns_rename_chain_without_caching_full_temporal_index() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = temporal_artifact();
        let commits = commit_index(artifact.commits.clone());
        let expected = InMemoryClient::new(Arc::new(artifact.clone()))
            .symbol_history(&commits, "new-root")
            .expect("in-memory symbol history succeeds");
        let parquet_dir = write_artifact_parquet(
            &artifact,
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");

        let actual = parquet
            .symbol_history(&commits, "new-root")
            .expect("parquet symbol history succeeds");

        assert_eq!(actual, expected);
        assert_eq!(
            actual,
            vec![
                (
                    "commit-a".to_owned(),
                    ChangeKind::Added,
                    SnapshotKey {
                        stable_symbol_id: "old-root".to_owned(),
                        commit: "commit-a".to_owned(),
                    },
                ),
                (
                    "commit-b".to_owned(),
                    ChangeKind::RenamedFrom(RenamePrev::Symbol(SnapshotKey {
                        stable_symbol_id: "old-root".to_owned(),
                        commit: "commit-a".to_owned(),
                    })),
                    SnapshotKey {
                        stable_symbol_id: "new-root".to_owned(),
                        commit: "commit-b".to_owned(),
                    },
                ),
            ]
        );
        assert!(parquet.temporal_index.get().is_none());
    }

    #[test]
    fn parquet_client_file_oids_are_memoized_for_the_open_manifest() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let parquet_dir = write_artifact_parquet(
            &artifact(Vec::new()),
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");
        let first = parquet.file_oids().expect("first file OID read");

        std::fs::remove_file(parquet_dir.join("file_manifests.parquet"))
            .expect("remove backing file after opened-manifest read");
        let second = parquet
            .file_oids()
            .expect("opened manifest must reuse memoized file OIDs");

        assert_eq!(second, first);
    }

    #[test]
    fn parquet_client_file_oids_memoizes_the_full_error_chain() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let parquet_dir = write_artifact_parquet(
            &artifact(Vec::new()),
            tempdir.path(),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");
        let file_manifests = parquet_dir.join("file_manifests.parquet");
        let file_manifests_bytes =
            std::fs::read(&file_manifests).expect("read file manifest fixture");
        std::fs::remove_file(&file_manifests).expect("remove file manifest fixture");

        let cold = parquet
            .file_oids()
            .expect_err("cold file OID load must fail with the backing file absent");
        let cold_chain = cold.chain().map(ToString::to_string).collect::<Vec<_>>();

        std::fs::write(&file_manifests, file_manifests_bytes)
            .expect("restore file manifest before warm call");
        let warm = parquet
            .file_oids()
            .expect_err("warm file OID load must replay the memoized failure");
        let warm_chain = warm.chain().map(ToString::to_string).collect::<Vec<_>>();
        let load_count = parquet
            .file_oids_load_count
            .load(std::sync::atomic::Ordering::SeqCst);

        eprintln!(
            "file_oids cold_chain={cold_chain:?} warm_chain={warm_chain:?} load_count={load_count}"
        );
        assert!(
            cold_chain.len() >= 2,
            "cold failure must preserve its context and nested source: {cold_chain:?}"
        );
        assert_eq!(
            warm_chain, cold_chain,
            "warm replay must preserve every source"
        );
        assert_eq!(load_count, 1, "the underlying Parquet load must run once");
    }
}
