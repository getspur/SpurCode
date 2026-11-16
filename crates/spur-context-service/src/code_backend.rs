//! Immutable Parquet-backed MCP code surface.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex as StdMutex};

use bytes::Bytes;
use futures::future::{try_join_all, BoxFuture, FutureExt as _};
use parquet::arrow::async_reader::AsyncFileReader;
use parquet::errors::{ParquetError, Result as ParquetResult};
use parquet::file::metadata::{ParquetMetaData, ParquetMetaDataReader};
use serde_json::{json, Value};
use spur_graph::{
    graph_edge_kind_or_default, CodeSelectorResolution, GraphEdgeArtifact, GraphEdgeKind,
    GraphQueryClient, GraphSymbolArtifact, OwnedCalleeRecord, OwnedCallerRecord, ParquetClient,
    ParquetSearchIndex, SearchFilters, SearchMode, SearchOptions,
};
use thiserror::Error;
use tokio::sync::{Mutex, OnceCell};

#[allow(
    dead_code,
    reason = "the shared source-sidecar module includes worker-only writer helpers"
)]
#[path = "source_sidecar.rs"]
mod source_sidecar;

use crate::artifact_cache::{
    validate_bundle_identity, validate_bundle_members, ArtifactBundleIdentity, ArtifactCache,
    ArtifactCacheError, ArtifactIdentity, MaterializedArtifact, MaterializedArtifactBundle,
};
use crate::medallion::SilverManifest;
use crate::package_search::PackageNameFilter;
use crate::serving_registry::{ServingPackage, ServingRegistry};
use source_sidecar::{read_verified_source, SourceSidecarReadError, SOURCE_SIDECAR_FILENAME};

#[derive(Debug, Clone)]
pub struct CatalogRequest {
    pub source: String,
    pub package: Option<String>,
    pub revision_or_ref: Option<String>,
    pub path: Option<String>,
    pub name_filter: Option<String>,
    pub limit: usize,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodeSearchRequest {
    pub source: String,
    pub package: String,
    pub revision_or_ref: Option<String>,
    pub query: String,
    pub symbol_kind: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct CodeReadRequest {
    pub source: String,
    pub selector: String,
    pub context_lines: usize,
}

#[derive(Debug, Clone)]
pub struct CodeEdgesRequest {
    pub source: String,
    pub selector: String,
    pub include_unresolved: bool,
}

#[derive(Debug, Error)]
pub enum CodeBackendError {
    #[error("invalid serving registry")]
    InvalidRegistry,
    #[error("package unavailable")]
    PackageUnavailable,
    #[error("invalid catalog cursor")]
    InvalidCursor,
    #[error("{0}")]
    InvalidSelector(String),
    #[error("ambiguous external selector `{selector}`; candidates: {candidates}")]
    AmbiguousSelector {
        selector: String,
        candidates: String,
    },
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("{0}")]
    Artifact(#[from] ArtifactCacheError),
    #[error("artifact open failed")]
    ArtifactOpen,
    #[error("artifact query failed")]
    ArtifactQuery,
    #[error("source sidecar is corrupt")]
    SourceSidecarCorrupt,
    #[error("source text is unavailable")]
    SourceTextUnavailable,
    #[error("source content OID mismatch")]
    SourceContentOidMismatch,
    #[error("source range is invalid")]
    SourceRangeInvalid,
}

impl CodeBackendError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRegistry => "invalid_serving_registry",
            Self::PackageUnavailable => "package_unavailable",
            Self::InvalidCursor => "invalid_catalog_cursor",
            Self::InvalidSelector(_) => "invalid_external_selector",
            Self::AmbiguousSelector { .. } => "ambiguous_external_selector",
            Self::SymbolNotFound(_) => "symbol_not_found",
            Self::Artifact(error) => error.code(),
            Self::ArtifactOpen => "artifact_open_failed",
            Self::ArtifactQuery => "artifact_query_failed",
            Self::SourceSidecarCorrupt => "source_sidecar_corrupt",
            Self::SourceTextUnavailable => "source_text_unavailable",
            Self::SourceContentOidMismatch => "source_content_oid_mismatch",
            Self::SourceRangeInvalid => "source_range_invalid",
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::PackageUnavailable
            | Self::InvalidCursor
            | Self::InvalidSelector(_)
            | Self::AmbiguousSelector { .. }
            | Self::SymbolNotFound(_) => false,
            Self::Artifact(error) => error.is_retryable(),
            Self::InvalidRegistry
            | Self::ArtifactOpen
            | Self::ArtifactQuery
            | Self::SourceSidecarCorrupt
            | Self::SourceTextUnavailable
            | Self::SourceContentOidMismatch
            | Self::SourceRangeInvalid => true,
        }
    }
}

#[derive(Clone)]
pub struct CodeBackend {
    registry: ServingRegistry,
    cache: ArtifactCache,
    generation_active: Arc<OnceCell<()>>,
    opened: Arc<Mutex<Option<Arc<OpenedPackage>>>>,
    opened_search: Arc<Mutex<Option<Arc<OpenedSearch>>>>,
    search_init: Arc<Mutex<()>>,
}

struct OpenedPackage {
    package: ServingPackage,
    client: ParquetClient,
    _bundle: MaterializedArtifactBundle,
}

struct OpenedSearch {
    package: ServingPackage,
    index: ParquetSearchIndex,
    _manifest: MaterializedArtifact,
}

#[derive(Clone)]
struct ArtifactRangeReader {
    cache: ArtifactCache,
    uri: String,
    bytes: u64,
    etag: String,
    failure: Arc<StdMutex<Option<ArtifactCacheError>>>,
}

impl ArtifactRangeReader {
    fn validate_range(&self, range: &Range<u64>) -> ParquetResult<()> {
        if range.start > range.end || range.end > self.bytes {
            return Err(ParquetError::General(format!(
                "invalid artifact range {}..{} for {} bytes",
                range.start, range.end, self.bytes
            )));
        }
        Ok(())
    }

    async fn read_range(&self, range: Range<u64>) -> ParquetResult<Bytes> {
        self.validate_range(&range)?;
        let bytes = match self
            .cache
            .fetch_range(&self.uri, range.clone(), Some(&self.etag))
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                let mut failure = self
                    .failure
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                failure.get_or_insert(error);
                return Err(ParquetError::External(Box::new(error)));
            }
        };
        let expected = usize::try_from(range.end - range.start)
            .map_err(|error| ParquetError::External(Box::new(error)))?;
        if bytes.len() != expected {
            return Err(ParquetError::General(
                "artifact range response has an unexpected length".to_owned(),
            ));
        }
        Ok(Bytes::from(bytes))
    }
}

impl AsyncFileReader for ArtifactRangeReader {
    fn get_bytes(&mut self, range: Range<u64>) -> BoxFuture<'_, ParquetResult<Bytes>> {
        async move { self.read_range(range).await }.boxed()
    }

    fn get_byte_ranges(
        &mut self,
        ranges: Vec<Range<u64>>,
    ) -> BoxFuture<'_, ParquetResult<Vec<Bytes>>> {
        async move {
            for range in &ranges {
                self.validate_range(range)?;
            }
            try_join_all(ranges.into_iter().map(|range| {
                let reader = self.clone();
                async move { reader.read_range(range).await }
            }))
            .await
        }
        .boxed()
    }

    fn get_metadata<'a>(
        &'a mut self,
        _options: Option<&'a parquet::arrow::arrow_reader::ArrowReaderOptions>,
    ) -> BoxFuture<'a, ParquetResult<Arc<ParquetMetaData>>> {
        let file_size = self.bytes;
        async move {
            let metadata = ParquetMetaDataReader::new()
                .load_and_finish(&mut *self, file_size)
                .await?;
            Ok(Arc::new(metadata))
        }
        .boxed()
    }
}

impl CodeBackend {
    pub fn new(registry: ServingRegistry, cache: ArtifactCache) -> Result<Self, CodeBackendError> {
        registry
            .validate()
            .map_err(|_| CodeBackendError::InvalidRegistry)?;
        Ok(Self {
            registry,
            cache,
            generation_active: Arc::new(OnceCell::new()),
            opened: Arc::new(Mutex::new(None)),
            opened_search: Arc::new(Mutex::new(None)),
            search_init: Arc::new(Mutex::new(())),
        })
    }

    pub fn generation(&self) -> i64 {
        self.registry.generation
    }

    pub async fn catalog(&self, request: CatalogRequest) -> Result<Value, CodeBackendError> {
        let Some(package) = request.package.as_deref() else {
            return self.catalog_packages(&request).await;
        };

        let Some(revision_or_ref) = request.revision_or_ref.as_deref() else {
            return self
                .catalog_revisions(&request.source, package, &request)
                .await;
        };

        let opened = self
            .open_selected(&request.source, package, Some(revision_or_ref))
            .await?;
        if let Some(path) = request.path.as_deref() {
            let is_file = opened
                .client
                .file_exists(path)
                .map_err(|_| CodeBackendError::ArtifactQuery)?;
            if is_file {
                return catalog_symbols(&opened, path, &request);
            }
        }
        catalog_tree(&opened, &request)
    }

    pub async fn search(&self, request: CodeSearchRequest) -> Result<Value, CodeBackendError> {
        let opened = self
            .open_search_selected(
                &request.source,
                &request.package,
                request.revision_or_ref.as_deref(),
            )
            .await?;
        let result = opened.index.search_symbols(&SearchOptions {
            query: request.query,
            mode: SearchMode::Substring,
            filters: SearchFilters {
                symbol_kind: request.symbol_kind,
                file: None,
                file_glob: None,
            },
            limit: request.limit,
        });
        let catalog_generation = opened.package.generation;
        let candidates = result
            .candidates
            .into_iter()
            .map(|symbol| {
                let selector_name = if symbol.qualified_name.is_empty() {
                    symbol.entity_name.as_str()
                } else {
                    symbol.qualified_name.as_str()
                };
                let selector = format!(
                    "pkg:{}@{}::{selector_name}",
                    opened.package.package, opened.package.revision
                );
                let uri = package_symbol_uri(
                    &opened.package.source,
                    &opened.package.package,
                    &opened.package.revision,
                    &symbol.stable_symbol_id,
                );
                json!({
                    "selector": selector,
                    "uri": uri,
                    "id": symbol.stable_symbol_id,
                    "stable_symbol_id": symbol.stable_symbol_id,
                    "source": opened.package.source,
                    "package": opened.package.package,
                    "revision": opened.package.revision,
                    "entity_name": symbol.entity_name,
                    "qualified_name": symbol.qualified_name,
                    "file_path": symbol.file_path,
                    "line_range": symbol.line_range,
                    "symbol_kind": symbol.symbol_kind,
                    "enclosing_scope": symbol.enclosing_scope,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "candidates": candidates,
            "total_matches": result.total_matches,
            "truncated": result.truncated,
            "catalog_generation": catalog_generation,
        }))
    }

    pub async fn read(&self, request: CodeReadRequest) -> Result<Value, CodeBackendError> {
        let (opened, symbol) = self
            .resolve_external_selector(&request.source, &request.selector)
            .await?;
        let symbol = symbol.ok_or_else(|| CodeBackendError::SymbolNotFound(request.selector))?;
        let file_manifest = opened
            .client
            .file_manifest_by_path(&symbol.file_path)
            .map_err(|_| CodeBackendError::ArtifactQuery)?
            .ok_or(CodeBackendError::ArtifactQuery)?;
        let source_text = read_verified_source(
            &opened._bundle.path().join(SOURCE_SIDECAR_FILENAME),
            &opened.package.source_sidecar.sha256,
            opened.package.source_sidecar.bytes,
            &symbol.file_path,
            &file_manifest.content_oid,
        )
        .map_err(source_sidecar_read_error)?;
        let (source, line_range) = slice_symbol_source(
            &source_text,
            symbol.byte_range,
            symbol.line_range,
            request.context_lines,
        )?;
        let selector_name = symbol_selector_name(&symbol);
        let catalog_generation = opened.package.generation;
        Ok(json!({
            "selector": format!(
                "pkg:{}@{}::{selector_name}",
                opened.package.package, opened.package.revision
            ),
            "stable_symbol_id": symbol.stable_symbol_id,
            "package_source": opened.package.source,
            "package": opened.package.package,
            "revision": opened.package.revision,
            "file_path": symbol.file_path,
            "byte_range": symbol.byte_range,
            "line_range": line_range,
            "source": source,
            "catalog_generation": catalog_generation,
        }))
    }

    pub async fn callers(&self, request: CodeEdgesRequest) -> Result<Value, CodeBackendError> {
        let (opened, symbol) = self
            .resolve_external_selector(&request.source, &request.selector)
            .await?;
        let Some(symbol) = symbol else {
            return Ok(empty_edges("callers", opened.package.generation));
        };
        let records = opened
            .client
            .try_find_caller_edges(&symbol.stable_symbol_id)
            .map_err(|_| CodeBackendError::ArtifactQuery)?;
        Ok(caller_response(
            &opened.package,
            &symbol.stable_symbol_id,
            records,
            request.include_unresolved,
        ))
    }

    pub async fn callees(&self, request: CodeEdgesRequest) -> Result<Value, CodeBackendError> {
        let (opened, symbol) = self
            .resolve_external_selector(&request.source, &request.selector)
            .await?;
        let Some(symbol) = symbol else {
            return Ok(empty_edges("callees", opened.package.generation));
        };
        let records = opened
            .client
            .try_find_callee_edges(&symbol.stable_symbol_id)
            .map_err(|_| CodeBackendError::ArtifactQuery)?;
        Ok(callee_response(
            &opened.package,
            &symbol.stable_symbol_id,
            records,
            request.include_unresolved,
        ))
    }

    pub async fn warm_index(
        &self,
        source: &str,
        package: &str,
        revision: &str,
    ) -> Result<Option<Value>, CodeBackendError> {
        let selected = match self.resolve_exact(source, package, revision)? {
            Some(selected) => selected,
            None => return Ok(None),
        };
        let opened = self.open(selected).await?;
        Ok(Some(json!({
            "status": "complete",
            "snapshot_id": opened.package.generation,
            "revision": opened.package.revision,
        })))
    }

    pub async fn index_status(&self, job_id: &str) -> Result<Value, CodeBackendError> {
        let Some((package, revision)) = parse_warm_job_id(job_id) else {
            return Ok(json!({ "status": "not_found" }));
        };
        let selected = match self.resolve_exact("registry:crates-io", package, revision)? {
            Some(selected) => selected,
            None => return Ok(json!({ "status": "not_found" })),
        };
        let opened = self.open(selected).await?;
        Ok(json!({
            "job_id": job_id,
            "status": "complete",
            "revision": opened.package.revision,
            "created_at": "",
            "updated_at": "",
            "attempt": 0,
            "execution_arn": null,
            "snapshot_id": opened.package.generation,
            "row_counts": opened.client.manifest().row_counts,
        }))
    }

    async fn catalog_packages(&self, request: &CatalogRequest) -> Result<Value, CodeBackendError> {
        let filter = PackageNameFilter::new(request.name_filter.as_deref());
        let mut revisions_by_package = BTreeMap::<String, (Vec<String>, Option<String>)>::new();
        for package in self.registry.packages.iter().filter(|candidate| {
            candidate.source == request.source && filter.matches(&candidate.package)
        }) {
            let opened = self.open(package.clone()).await?;
            let entry = revisions_by_package
                .entry(package.package.clone())
                .or_default();
            entry.0.push(opened.package.revision.clone());
            if opened
                .package
                .refs
                .iter()
                .any(|reference| reference == "latest")
            {
                entry.1 = Some(opened.package.revision.clone());
            }
        }

        let cursor = decode_cursor(request.cursor.as_deref(), 2)?;
        let total_matches = revisions_by_package.len();
        let mut rows = revisions_by_package
            .into_iter()
            .filter(|(package, _)| {
                cursor.as_ref().is_none_or(|parts| {
                    (request.source.as_str(), package.as_str())
                        > (parts[0].as_str(), parts[1].as_str())
                })
            })
            .map(|(package, (revisions, latest_revision))| {
                json!({
                    "source": request.source,
                    "package": package,
                    "latest_revision": latest_revision,
                    "revision_count": revisions.len(),
                    "indexed_at": "",
                })
            })
            .collect::<Vec<_>>();
        let (truncated, next_cursor) = trim_page(&mut rows, request.limit, |row| {
            encode_cursor(&[
                row["source"].as_str().unwrap_or_default(),
                row["package"].as_str().unwrap_or_default(),
            ])
        });
        Ok(catalog_page(
            "packages",
            rows,
            total_matches,
            truncated,
            next_cursor,
            self.registry.generation,
        ))
    }

    async fn catalog_revisions(
        &self,
        source: &str,
        package: &str,
        request: &CatalogRequest,
    ) -> Result<Value, CodeBackendError> {
        let mut rows = Vec::new();
        for selected in self
            .registry
            .packages
            .iter()
            .filter(|candidate| candidate.source == source && candidate.package == package)
        {
            let opened = self.open(selected.clone()).await?;
            rows.push(json!({
                "revision": opened.package.revision,
                "revision_kind": opened.package.revision_kind,
                "semver": normalized_semver(&opened.package.revision),
                "indexed_at": "",
                "embeddings_status": "skipped",
                "row_counts": opened.client.manifest().row_counts,
                "generation": opened.package.generation,
                "snapshot_id": opened.package.generation,
                "refs": opened.package.refs,
            }));
        }
        if rows.is_empty() {
            return Err(CodeBackendError::PackageUnavailable);
        }
        rows.sort_by(|left, right| left["revision"].as_str().cmp(&right["revision"].as_str()));
        let cursor = decode_cursor(request.cursor.as_deref(), 1)?;
        let total_matches = rows.len();
        rows.retain(|item| {
            cursor.as_ref().is_none_or(|parts| {
                item["revision"]
                    .as_str()
                    .is_some_and(|revision| revision > parts[0].as_str())
            })
        });
        let (truncated, next_cursor) = trim_page(&mut rows, request.limit, |row| {
            encode_cursor(&[row["revision"].as_str().unwrap_or_default()])
        });
        Ok(catalog_page(
            "revisions",
            rows,
            total_matches,
            truncated,
            next_cursor,
            self.registry.generation,
        ))
    }

    async fn open_selected(
        &self,
        source: &str,
        package: &str,
        revision_or_ref: Option<&str>,
    ) -> Result<Arc<OpenedPackage>, CodeBackendError> {
        let selected = self
            .registry
            .resolve_revision_or_ref(source, package, revision_or_ref.unwrap_or("latest"))
            .map(|package| package.cloned())
            .map_err(|_| CodeBackendError::InvalidRegistry)?
            .ok_or(CodeBackendError::PackageUnavailable)?;
        self.open(selected).await
    }

    async fn open_search_selected(
        &self,
        source: &str,
        package: &str,
        revision_or_ref: Option<&str>,
    ) -> Result<Arc<OpenedSearch>, CodeBackendError> {
        let selected = self
            .registry
            .resolve_revision_or_ref(source, package, revision_or_ref.unwrap_or("latest"))
            .map(|package| package.cloned())
            .map_err(|_| CodeBackendError::InvalidRegistry)?
            .ok_or(CodeBackendError::PackageUnavailable)?;
        self.open_search(selected).await
    }

    async fn open_search(
        &self,
        package: ServingPackage,
    ) -> Result<Arc<OpenedSearch>, CodeBackendError> {
        self.generation_active
            .get_or_try_init(|| async {
                self.cache
                    .activate_generation(self.registry.generation)
                    .await
            })
            .await?;
        if let Some(opened) = self
            .opened_search
            .lock()
            .await
            .as_ref()
            .filter(|opened| opened.package == package)
            .cloned()
        {
            return Ok(opened);
        }
        let _initializing = self.search_init.lock().await;
        if let Some(opened) = self
            .opened_search
            .lock()
            .await
            .as_ref()
            .filter(|opened| opened.package == package)
            .cloned()
        {
            return Ok(opened);
        }

        let bundle = ArtifactBundleIdentity {
            root: ArtifactIdentity {
                generation: package.generation,
                source: package.source.clone(),
                package: package.package.clone(),
                revision: package.revision.clone(),
                artifact: package.graph_manifest.clone(),
            },
            graph_prefix: package.graph_prefix_uri.clone(),
        };
        validate_bundle_identity(&bundle)?;
        let manifest_lease = self.cache.materialize(&bundle.root).await?;
        let manifest_bytes = tokio::fs::read(manifest_lease.path())
            .await
            .map_err(|_| ArtifactCacheError::Filesystem)?;
        let manifest: SilverManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| ArtifactCacheError::InvalidIdentity)?;
        let nodes = validate_bundle_members(&bundle, &manifest)?
            .into_iter()
            .find(|member| member.relative_path == "nodes.parquet")
            .ok_or(ArtifactCacheError::InvalidIdentity)?;
        let reader = ArtifactRangeReader {
            cache: self.cache.clone(),
            uri: nodes.identity.artifact.uri,
            bytes: nodes.identity.artifact.bytes,
            etag: nodes.etag,
            failure: Arc::new(StdMutex::new(None)),
        };
        let range_failure = Arc::clone(&reader.failure);
        let index = ParquetSearchIndex::read_async(reader).await.map_err(|_| {
            let failure = range_failure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            failure.unwrap_or(ArtifactCacheError::IntegrityMismatch)
        })?;
        let opened = Arc::new(OpenedSearch {
            package,
            index,
            _manifest: manifest_lease,
        });
        let mut cached = self.opened_search.lock().await;
        *cached = Some(Arc::clone(&opened));
        Ok(opened)
    }

    async fn resolve_external_selector(
        &self,
        default_source: &str,
        selector: &str,
    ) -> Result<(Arc<OpenedPackage>, Option<GraphSymbolArtifact>), CodeBackendError> {
        match parse_external_selector(selector, default_source)? {
            ExternalSelector::Stable {
                source,
                package,
                revision,
                stable_symbol_id,
            } => {
                let opened = self
                    .open_selected(&source, &package, Some(&revision))
                    .await?;
                let symbol = opened
                    .client
                    .symbol_by_id(&stable_symbol_id)
                    .map_err(|_| CodeBackendError::ArtifactQuery)?;
                Ok((opened, symbol))
            }
            ExternalSelector::Named {
                source,
                package,
                revision_or_ref,
                qualified_name,
            } => {
                let opened = self
                    .open_selected(&source, &package, revision_or_ref.as_deref())
                    .await?;
                let symbol = match opened
                    .client
                    .resolve_selector(&qualified_name)
                    .map_err(|_| CodeBackendError::ArtifactQuery)?
                {
                    CodeSelectorResolution::Resolved(resolved) => opened
                        .client
                        .symbol_by_id(&resolved.stable_symbol_id)
                        .map_err(|_| CodeBackendError::ArtifactQuery)?,
                    CodeSelectorResolution::NotFound => None,
                    CodeSelectorResolution::Ambiguous { candidates } => {
                        let candidates = candidates
                            .iter()
                            .map(|candidate| {
                                package_symbol_uri(
                                    &opened.package.source,
                                    &opened.package.package,
                                    &opened.package.revision,
                                    &candidate.id,
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(CodeBackendError::AmbiguousSelector {
                            selector: selector.to_owned(),
                            candidates,
                        });
                    }
                };
                Ok((opened, symbol))
            }
        }
    }

    fn resolve_exact(
        &self,
        source: &str,
        package: &str,
        revision: &str,
    ) -> Result<Option<ServingPackage>, CodeBackendError> {
        self.registry
            .resolve(source, package, revision)
            .map(|package| package.cloned())
            .map_err(|_| CodeBackendError::InvalidRegistry)
    }

    async fn open(&self, package: ServingPackage) -> Result<Arc<OpenedPackage>, CodeBackendError> {
        let expected_source_sidecar_uri =
            format!("{}{SOURCE_SIDECAR_FILENAME}", package.graph_prefix_uri);
        if package.source_sidecar.uri != expected_source_sidecar_uri {
            return Err(CodeBackendError::Artifact(
                ArtifactCacheError::InvalidIdentity,
            ));
        }
        self.generation_active
            .get_or_try_init(|| async {
                self.cache
                    .activate_generation(self.registry.generation)
                    .await
            })
            .await?;
        if let Some(opened) = self
            .opened
            .lock()
            .await
            .as_ref()
            .filter(|opened| opened.package == package)
            .cloned()
        {
            return Ok(opened);
        }
        let identity = ArtifactBundleIdentity {
            root: ArtifactIdentity {
                generation: package.generation,
                source: package.source.clone(),
                package: package.package.clone(),
                revision: package.revision.clone(),
                artifact: package.graph_manifest.clone(),
            },
            graph_prefix: package.graph_prefix_uri.clone(),
        };
        let bundle = self.cache.materialize_bundle(&identity).await?;
        let client =
            ParquetClient::open(bundle.path()).map_err(|_| CodeBackendError::ArtifactOpen)?;
        let opened = Arc::new(OpenedPackage {
            package,
            client,
            _bundle: bundle,
        });
        let mut cached = self.opened.lock().await;
        if let Some(current) = cached
            .as_ref()
            .filter(|current| current.package == opened.package)
        {
            return Ok(Arc::clone(current));
        }
        *cached = Some(Arc::clone(&opened));
        Ok(opened)
    }
}

enum ExternalSelector {
    Named {
        source: String,
        package: String,
        revision_or_ref: Option<String>,
        qualified_name: String,
    },
    Stable {
        source: String,
        package: String,
        revision: String,
        stable_symbol_id: String,
    },
}

fn parse_external_selector(
    selector: &str,
    default_source: &str,
) -> Result<ExternalSelector, CodeBackendError> {
    let selector = selector.trim();
    if let Some(body) = selector.strip_prefix("pkg-symbol://") {
        let mut parts = body.rsplitn(4, '/');
        let stable_symbol_id = parts.next().unwrap_or_default();
        let revision = parts.next().unwrap_or_default();
        let package = parts.next().unwrap_or_default();
        let source = parts.next().unwrap_or_default();
        if [source, package, revision, stable_symbol_id]
            .iter()
            .any(|part| part.is_empty())
        {
            return Err(CodeBackendError::InvalidSelector(format!(
                "external package symbol URI is invalid: {selector}"
            )));
        }
        return Ok(ExternalSelector::Stable {
            source: decode_uri_component(source),
            package: decode_uri_component(package),
            revision: decode_uri_component(revision),
            stable_symbol_id: decode_uri_component(stable_symbol_id),
        });
    }

    let Some(body) = selector.strip_prefix("pkg:") else {
        return Err(CodeBackendError::InvalidSelector(format!(
            "external selector must start with 'pkg:': {selector}"
        )));
    };
    let Some((package_revision, qualified_name)) = body.split_once("::") else {
        return Err(CodeBackendError::InvalidSelector(format!(
            "external selector must include a package and symbol path: {selector}"
        )));
    };
    if qualified_name.is_empty() {
        return Err(CodeBackendError::InvalidSelector(format!(
            "external selector must include a symbol path: {selector}"
        )));
    }
    let (package, revision_or_ref) = match package_revision.split_once('@') {
        Some((package, revision_or_ref)) if !package.is_empty() && !revision_or_ref.is_empty() => {
            (package.to_owned(), Some(revision_or_ref.to_owned()))
        }
        Some(_) => {
            return Err(CodeBackendError::InvalidSelector(format!(
                "external selector has an invalid package revision: {selector}"
            )))
        }
        None if !package_revision.is_empty() => (package_revision.to_owned(), None),
        None => {
            return Err(CodeBackendError::InvalidSelector(format!(
                "external selector must include a package: {selector}"
            )))
        }
    };
    Ok(ExternalSelector::Named {
        source: default_source.to_owned(),
        package,
        revision_or_ref,
        qualified_name: qualified_name.to_owned(),
    })
}

fn source_sidecar_read_error(error: SourceSidecarReadError) -> CodeBackendError {
    match error {
        SourceSidecarReadError::IntegrityMismatch => {
            CodeBackendError::Artifact(ArtifactCacheError::IntegrityMismatch)
        }
        SourceSidecarReadError::Corrupt => CodeBackendError::SourceSidecarCorrupt,
        SourceSidecarReadError::TextUnavailable => CodeBackendError::SourceTextUnavailable,
        SourceSidecarReadError::ContentOidMismatch => CodeBackendError::SourceContentOidMismatch,
    }
}

fn symbol_selector_name(symbol: &GraphSymbolArtifact) -> &str {
    if symbol.qualified_name.is_empty() {
        &symbol.entity_name
    } else {
        &symbol.qualified_name
    }
}

fn candidate_value(package: &ServingPackage, symbol: &GraphSymbolArtifact) -> Value {
    let selector_name = symbol_selector_name(symbol);
    json!({
        "selector": format!(
            "pkg:{}@{}::{selector_name}",
            package.package, package.revision
        ),
        "uri": package_symbol_uri(
            &package.source,
            &package.package,
            &package.revision,
            &symbol.stable_symbol_id,
        ),
        "id": symbol.stable_symbol_id,
        "stable_symbol_id": symbol.stable_symbol_id,
        "source": package.source,
        "package": package.package,
        "revision": package.revision,
        "entity_name": symbol.entity_name,
        "qualified_name": symbol.qualified_name,
        "file_path": symbol.file_path,
        "line_range": symbol.line_range,
        "symbol_kind": symbol.symbol_kind,
        "enclosing_scope": symbol.enclosing_scope,
    })
}

fn edge_kind_name(edge: &GraphEdgeArtifact) -> &'static str {
    match graph_edge_kind_or_default(edge.relation, edge.edge_kind) {
        GraphEdgeKind::Calls => "calls",
        GraphEdgeKind::CallsDyn => "calls_dyn",
        GraphEdgeKind::ReferencesHof => "references_hof",
        GraphEdgeKind::ReferencesOther | GraphEdgeKind::ReferencesAddress => "references_other",
    }
}

fn edge_value(edge: &GraphEdgeArtifact) -> Value {
    json!({
        "source_stable_id": edge.source_stable_symbol_id,
        "target_stable_id": edge.target_stable_symbol_id,
        "target_label": edge.target_label,
        "target_package": null,
        "relation": edge.relation,
        "edge_kind": edge_kind_name(edge),
        "confidence": edge.confidence,
        "confidence_score": edge.confidence_score,
        "bind_method": edge.bind_method,
        "receiver_text": edge.receiver_text,
        "scope_text": edge.scope_text,
    })
}

fn caller_response(
    package: &ServingPackage,
    stable_symbol_id: &str,
    records: Vec<OwnedCallerRecord>,
    include_unresolved: bool,
) -> Value {
    let mut rows = records
        .into_iter()
        .filter_map(|record| match record {
            OwnedCallerRecord::Resolved { caller, edge } => Some(json!({
                "caller": candidate_value(package, &caller),
                "edge": edge_value(&edge),
                "resolved": true,
            })),
            OwnedCallerRecord::Unresolved { caller, edge, .. } if include_unresolved => {
                Some(json!({
                    "caller": candidate_value(package, &caller),
                    "edge": edge_value(&edge),
                    "resolved": false,
                }))
            }
            OwnedCallerRecord::Unresolved { .. } => None,
        })
        .collect::<Vec<_>>();
    sort_edge_rows(&mut rows, "caller");
    edge_response("callers", rows, package.generation, Some(stable_symbol_id))
}

fn callee_response(
    package: &ServingPackage,
    stable_symbol_id: &str,
    records: Vec<OwnedCalleeRecord>,
    include_unresolved: bool,
) -> Value {
    let mut rows = records
        .into_iter()
        .filter_map(|record| match record {
            OwnedCalleeRecord::Resolved { symbol, edge } => Some(json!({
                "callee": candidate_value(package, &symbol),
                "edge": edge_value(&edge),
                "resolved": true,
            })),
            OwnedCalleeRecord::Unresolved { edge, .. } if include_unresolved => Some(json!({
                "callee": null,
                "edge": edge_value(&edge),
                "resolved": false,
            })),
            OwnedCalleeRecord::Unresolved { .. } => None,
        })
        .collect::<Vec<_>>();
    sort_edge_rows(&mut rows, "callee");
    edge_response("callees", rows, package.generation, Some(stable_symbol_id))
}

fn sort_edge_rows(rows: &mut [Value], direction: &str) {
    rows.sort_by_cached_key(|row| {
        let resolved = row["resolved"].as_bool().unwrap_or(false);
        let candidate = &row[direction];
        format!(
            "{}|{}|{:020}|{:020}|{}|{}",
            if resolved { 0 } else { 1 },
            candidate["file_path"].as_str().unwrap_or_default(),
            candidate["line_range"][0].as_u64().unwrap_or_default(),
            candidate["line_range"][1].as_u64().unwrap_or_default(),
            candidate["qualified_name"].as_str().unwrap_or_default(),
            row["edge"]["edge_kind"].as_str().unwrap_or_default(),
        )
    });
}

fn edge_response(
    direction: &str,
    rows: Vec<Value>,
    catalog_generation: i64,
    stable_symbol_id: Option<&str>,
) -> Value {
    let mut calls = 0;
    let mut calls_dyn = 0;
    let mut references_hof = 0;
    let mut references_other = 0;
    let mut unresolved = 0;
    let mut unresolved_labels = BTreeSet::new();
    for row in &rows {
        match row["edge"]["edge_kind"].as_str().unwrap_or_default() {
            "calls" => calls += 1,
            "calls_dyn" => calls_dyn += 1,
            "references_hof" => references_hof += 1,
            _ => references_other += 1,
        }
        if !row["resolved"].as_bool().unwrap_or(false) {
            unresolved += 1;
            if let Some(label) = row["edge"]["target_label"].as_str() {
                unresolved_labels.insert(label.to_owned());
            }
        }
    }
    let mut unresolved_sample = Vec::new();
    let mut sample_bytes = 0;
    for label in unresolved_labels {
        if unresolved_sample.len() >= 5 || sample_bytes + label.len() > 120 {
            break;
        }
        sample_bytes += label.len();
        unresolved_sample.push(label);
    }
    let mut response = json!({
        "counts_by_kind": {
            "calls": calls,
            "calls_dyn": calls_dyn,
            "references_hof": references_hof,
            "references_other": references_other,
            "unresolved": unresolved,
        },
        "unresolved_sample": unresolved_sample,
        "catalog_generation": catalog_generation,
        "stable_symbol_id": stable_symbol_id,
    });
    response[direction] = Value::Array(rows);
    response
}

fn empty_edges(direction: &str, catalog_generation: i64) -> Value {
    edge_response(direction, Vec::new(), catalog_generation, None)
}

fn slice_symbol_source(
    source_text: &str,
    byte_range: [usize; 2],
    line_range: [usize; 2],
    context_lines: usize,
) -> Result<(String, [usize; 2]), CodeBackendError> {
    let [byte_start, byte_end] = byte_range;
    let [line_start, line_end] = line_range;
    if byte_start > byte_end || byte_end > source_text.len() {
        return Err(CodeBackendError::SourceRangeInvalid);
    }
    if context_lines == 0 {
        let source = source_text
            .get(byte_start..byte_end)
            .ok_or(CodeBackendError::SourceRangeInvalid)?
            .to_owned();
        return Ok((source, line_range));
    }

    let line_starts = line_starts(source_text);
    if line_starts.is_empty() {
        return Ok((String::new(), [0, 0]));
    }
    let expanded_start = line_start.saturating_sub(context_lines).max(1);
    let expanded_end = line_end
        .saturating_add(context_lines)
        .min(line_starts.len());
    let start_byte = line_starts[expanded_start - 1];
    let end_byte = if expanded_end < line_starts.len() {
        line_starts[expanded_end]
    } else {
        source_text.len()
    };
    let source = source_text
        .get(start_byte..end_byte)
        .ok_or(CodeBackendError::SourceRangeInvalid)?
        .to_owned();
    Ok((source, [expanded_start, expanded_end]))
}

fn line_starts(source_text: &str) -> Vec<usize> {
    if source_text.is_empty() {
        return Vec::new();
    }
    let mut starts = vec![0];
    for (index, byte) in source_text.bytes().enumerate() {
        if byte == b'\n' && index + 1 < source_text.len() {
            starts.push(index + 1);
        }
    }
    starts
}

fn catalog_symbols(
    opened: &OpenedPackage,
    path: &str,
    request: &CatalogRequest,
) -> Result<Value, CodeBackendError> {
    let mut symbols = opened
        .client
        .symbols_by_file(path)
        .map_err(|_| CodeBackendError::ArtifactQuery)?;
    symbols.retain(|symbol| {
        request.name_filter.as_deref().is_none_or(|filter| {
            symbol.entity_name.contains(filter) || symbol.qualified_name.contains(filter)
        })
    });
    symbols.sort_by(|left, right| {
        left.line_range[0]
            .cmp(&right.line_range[0])
            .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
    });
    let total_matches = symbols.len();
    let cursor = decode_cursor(request.cursor.as_deref(), 2)?;
    let mut rows = symbols
        .into_iter()
        .filter(|symbol| {
            cursor.as_ref().is_none_or(|parts| {
                let line = parts[0].parse::<usize>().unwrap_or_default();
                (symbol.line_range[0], symbol.stable_symbol_id.as_str()) > (line, parts[1].as_str())
            })
        })
        .map(|symbol| {
            let name = if symbol.qualified_name.is_empty() {
                symbol.entity_name.as_str()
            } else {
                symbol.qualified_name.as_str()
            };
            let selector = format!(
                "pkg:{}@{}::{name}",
                opened.package.package, opened.package.revision
            );
            let next = [
                "external_code_read",
                "external_code_callers",
                "external_code_callees",
            ]
            .into_iter()
            .map(|tool| json!({ "tool": tool, "selector": selector }))
            .collect::<Vec<_>>();
            json!({
                "entity_name": symbol.entity_name,
                "qualified_name": symbol.qualified_name,
                "symbol_kind": symbol.symbol_kind,
                "line_range": symbol.line_range,
                "selector": selector,
                "next": next,
                "_cursor_id": symbol.stable_symbol_id,
            })
        })
        .collect::<Vec<_>>();
    let (truncated, next_cursor) = trim_page(&mut rows, request.limit, |row| {
        encode_cursor(&[
            &row["line_range"][0]
                .as_u64()
                .unwrap_or_default()
                .to_string(),
            row["_cursor_id"].as_str().unwrap_or_default(),
        ])
    });
    for row in &mut rows {
        row.as_object_mut()
            .map(|object| object.remove("_cursor_id"));
    }
    Ok(catalog_page(
        "symbols",
        rows,
        total_matches,
        truncated,
        next_cursor,
        opened.package.generation,
    ))
}

fn catalog_tree(
    opened: &OpenedPackage,
    request: &CatalogRequest,
) -> Result<Value, CodeBackendError> {
    let files = opened
        .client
        .files()
        .map_err(|_| CodeBackendError::ArtifactQuery)?;
    let paths = files
        .iter()
        .map(|file| file.file_path.clone())
        .collect::<Vec<_>>();
    let symbols = opened
        .client
        .symbols_by_files(&paths)
        .map_err(|_| CodeBackendError::ArtifactQuery)?;
    let symbol_counts = symbols
        .into_iter()
        .fold(BTreeMap::new(), |mut counts, symbol| {
            *counts.entry(symbol.file_path).or_insert(0_usize) += 1;
            counts
        });
    let prefix = request.path.as_deref().unwrap_or("").trim_matches('/');
    let mut entries = BTreeMap::<String, (String, bool, BTreeSet<String>, usize)>::new();
    for file in files {
        let remainder = if prefix.is_empty() {
            file.file_path.as_str()
        } else if let Some(remainder) = file
            .file_path
            .strip_prefix(prefix)
            .and_then(|value| value.strip_prefix('/'))
        {
            remainder
        } else {
            continue;
        };
        let Some((name, tail)) = remainder
            .split_once('/')
            .map(|(name, tail)| (name, Some(tail)))
            .or_else(|| (!remainder.is_empty()).then_some((remainder, None)))
        else {
            continue;
        };
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let entry = entries
            .entry(name.to_owned())
            .or_insert_with(|| (path, tail.is_some(), BTreeSet::new(), 0));
        entry.1 |= tail.is_some();
        entry.2.insert(file.file_path.clone());
        entry.3 += symbol_counts
            .get(&file.file_path)
            .copied()
            .unwrap_or_default();
    }
    let cursor = decode_cursor(request.cursor.as_deref(), 1)?;
    let total_matches = entries.len();
    let mut rows = entries
        .into_iter()
        .filter(|(name, _)| cursor.as_ref().is_none_or(|parts| name > &parts[0]))
        .map(|(name, (path, directory, files, symbol_count))| {
            json!({
                "name": name,
                "path": path,
                "kind": if directory { "dir" } else { "file" },
                "file_count": files.len(),
                "symbol_count": symbol_count,
            })
        })
        .collect::<Vec<_>>();
    let (truncated, next_cursor) = trim_page(&mut rows, request.limit, |row| {
        encode_cursor(&[row["name"].as_str().unwrap_or_default()])
    });
    Ok(catalog_page(
        "tree",
        rows,
        total_matches,
        truncated,
        next_cursor,
        opened.package.generation,
    ))
}

fn catalog_page(
    level: &str,
    rows: Vec<Value>,
    total_matches: usize,
    truncated: bool,
    next_cursor: Option<String>,
    generation: i64,
) -> Value {
    json!({
        "level": level,
        "rows": rows,
        "total_matches": total_matches,
        "truncated": truncated,
        "next_cursor": next_cursor,
        "catalog_generation": generation,
    })
}

fn trim_page(
    rows: &mut Vec<Value>,
    limit: usize,
    cursor_for: impl FnOnce(&Value) -> String,
) -> (bool, Option<String>) {
    let limit = limit.clamp(1, 200);
    let truncated = rows.len() > limit;
    if truncated {
        rows.truncate(limit);
    }
    let next_cursor = truncated.then(|| rows.last().map(cursor_for)).flatten();
    (truncated, next_cursor)
}

fn encode_uri_component(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

fn decode_uri_component(value: &str) -> String {
    value.replace("%2F", "/").replace("%25", "%")
}

fn encode_cursor(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| encode_uri_component(part))
        .collect::<Vec<_>>()
        .join("/")
}

fn decode_cursor(
    cursor: Option<&str>,
    expected_parts: usize,
) -> Result<Option<Vec<String>>, CodeBackendError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let parts = cursor
        .split('/')
        .map(decode_uri_component)
        .collect::<Vec<_>>();
    if parts.len() != expected_parts || parts.iter().any(String::is_empty) {
        return Err(CodeBackendError::InvalidCursor);
    }
    Ok(Some(parts))
}

fn package_symbol_uri(source: &str, package: &str, revision: &str, stable_id: &str) -> String {
    format!(
        "pkg-symbol://{}/{}/{}/{}",
        encode_uri_component(source),
        encode_uri_component(package),
        encode_uri_component(revision),
        encode_uri_component(stable_id),
    )
}

fn normalized_semver(revision: &str) -> Option<String> {
    let revision = revision.strip_prefix('v').unwrap_or(revision);
    let mut parts = revision.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{major}.{minor}.{patch}"))
}

fn parse_warm_job_id(job_id: &str) -> Option<(&str, &str)> {
    let value = job_id.strip_prefix("pkg:")?;
    let (package, revision) = value.split_once('@')?;
    (!package.is_empty() && !revision.is_empty() && !revision.contains("::"))
        .then_some((package, revision))
}
