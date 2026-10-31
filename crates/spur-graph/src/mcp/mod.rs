use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use chrono::{DateTime, SecondsFormat, Utc};
use ignore::{DirEntry, WalkBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spur_mcp::local_projects::{
    decorate_project_response, extract_project, with_optional_project_schema, LocalProjectAccess,
    LocalProjectResolver, ResolvedLocalProject,
};

use crate::git_blob_oid;
use crate::store::cache::{emit_base_seed_stats, load_base_seed_for_worktree, BaseArtifactSeed};
use crate::temporal::{
    resolve_symbol_at_indexed, symbol_history, Resolution, ResolutionFailure, TemporalIndex,
};
use crate::{
    artifact_from_facts, bounded_subgraph_with_budget, build_facts, edge_kind, load_artifact,
    read_artifact_parquet, resolve_artifact_location, resolve_selector, resolve_worktree_root_from,
    write_artifact_parquet, CandidateRow, CommitIndexArtifact, GraphArtifactManifest,
    GraphEdgeArtifact, GraphEdgeKind, GraphIndexArtifact, GraphIndexPointer, GraphQueryClient,
    GraphSymbolArtifact, InMemoryClient, OverlayClient, OverlayFinalizationMeasurements,
    OverlayGeneration, OverlayGenerationIdentity, OverlayPathState as GenerationPathState,
    OwnedCalleeRecord, OwnedCallerRecord, ParquetClient, SearchFilters, SearchMode, SearchOptions,
    SearchResult, SearchSymbol, SelectorResolution, SnapshotKey, SubgraphBudget, WriteOptions,
    CODE_SYMBOL_URI_PREFIX,
};

pub use spur_mcp::tools::McpHandlerError;

/// Metadata for a single graph-owned MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Clone)]
pub struct GraphMcpDeps {
    pub rebuild_coordinator: Arc<RebuildCoordinator>,
    pub overlay_fsmonitor_auto: bool,
}

impl Default for GraphMcpDeps {
    fn default() -> Self {
        Self {
            rebuild_coordinator: Arc::new(RebuildCoordinator::new()),
            overlay_fsmonitor_auto: false,
        }
    }
}

impl GraphMcpDeps {
    /// Returns the process-lifetime overlay runtime lifecycle associated with
    /// this dependency set. Keeping the association on the existing rebuild
    /// coordinator preserves source compatibility for callers that construct
    /// `GraphMcpDeps` with a struct literal.
    fn overlay_runtime_lifecycle(&self) -> Arc<OverlayRuntimeLifecycle> {
        overlay_runtime_lifecycle_for(&self.rebuild_coordinator)
    }
}

#[derive(Clone)]
pub struct GraphMcpModule {
    deps: GraphMcpDeps,
    local_projects: LocalProjectAccess,
}

impl Default for GraphMcpModule {
    fn default() -> Self {
        Self::new(GraphMcpDeps::default())
    }
}

impl GraphMcpModule {
    pub fn new(deps: GraphMcpDeps) -> Self {
        Self {
            deps,
            local_projects: LocalProjectAccess::CurrentWorktreeOnly,
        }
    }

    pub fn with_local_projects(deps: GraphMcpDeps, resolver: LocalProjectResolver) -> Self {
        Self {
            deps,
            local_projects: LocalProjectAccess::Catalog(resolver),
        }
    }

    pub fn tools(&self) -> Vec<ToolDefinition> {
        match &self.local_projects {
            LocalProjectAccess::CurrentWorktreeOnly => tool_definitions(),
            LocalProjectAccess::Catalog(_) => local_project_tool_definitions(),
        }
    }

    /// Dispatch a tool call by name. This is the inherent entry point used by
    /// the legacy spur-core dispatcher; the `spur_mcp::ToolModule` impl below
    /// delegates here. Every MCP server that exposes `code_*` goes through
    /// this call, which returns within [`DEFAULT_CODE_TOOL_TIMEOUT`].
    pub async fn dispatch(&self, name: &str, mut args: Value) -> CodeGraphResult {
        let project = extract_project(&mut args, &self.local_projects)?;
        let inherited_root = project
            .as_ref()
            .map(|project| project.root.clone())
            .or_else(scoped_worktree_root);
        #[cfg(test)]
        let scope_barrier = PROJECT_SCOPE_BARRIER_FOR_TEST.try_with(Arc::clone).ok();
        let module = self.clone();
        let tool_name = name.to_string();
        let timeout = code_tool_timeout();
        #[cfg(any(test, feature = "test-support"))]
        let test_delay = CODE_TOOL_DELAY_OVERRIDE.try_with(|delay| *delay).ok();
        let mut task = tokio::spawn({
            let tool_name = tool_name.clone();
            async move {
                let run = async move {
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some(delay) = test_delay.filter(|delay| !delay.is_zero()) {
                        tokio::time::sleep(delay).await;
                    }
                    run_scoped_code_tool(module, &tool_name, args, inherited_root, project).await
                };
                #[cfg(test)]
                if let Some(barrier) = scope_barrier {
                    return PROJECT_SCOPE_BARRIER_FOR_TEST.scope(barrier, run).await;
                }
                run.await
            }
        });
        match tokio::time::timeout(timeout, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_error)) => Err(McpHandlerError::Internal(format!(
                "code graph tool `{tool_name}` task failed: {join_error}"
            ))
            .into()),
            Err(_elapsed) => {
                task.abort();
                Err(code_tool_deadline_exceeded(&tool_name, timeout))
            }
        }
    }

    async fn dispatch_current_project(&self, name: &str, args: Value) -> CodeGraphResult {
        #[cfg(test)]
        wait_for_project_scope_overlap_for_test().await;
        let _runtime_lifecycle = self.deps.overlay_runtime_lifecycle();
        match name {
            "code_resolve" => {
                code_resolve_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_symbol_search" | "code_search" => {
                code_search_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_file_symbols" => {
                code_file_symbols_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_symbol_info" => {
                code_symbol_info_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_read_symbol" => {
                code_read_symbol_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_callers" => {
                code_callers_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_callees" => {
                code_callees_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_subgraph" => {
                code_subgraph_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            "code_symbol_history" => {
                code_symbol_history_response(
                    &args,
                    Arc::clone(&self.deps.rebuild_coordinator),
                    self.deps.overlay_fsmonitor_auto,
                )
                .await
            }
            other => Err(McpHandlerError::InvalidParams(format!(
                "unknown code graph MCP tool: {other}"
            ))
            .into()),
        }
    }
}

/// `spur_mcp::ToolModule` adapter for the code graph module.
///
/// This is the standalone-composition surface: any MCP server (the `spur graph
/// mcp` standalone server, the bundled `spur mcp` server, or future
/// compositions) can register `GraphMcpModule` into a `spur_mcp::ToolRegistry`
/// and dispatch the `code_*` tools without going through spur-core's brain
/// server. The inherent [`GraphMcpModule::dispatch`] does the real work; this
/// impl only maps local types onto the shared `ToolModule` contract and wraps
/// results as MCP text content.
#[async_trait::async_trait]
impl spur_mcp::ToolModule for GraphMcpModule {
    fn tools(&self) -> Vec<spur_mcp::ToolDefinition> {
        GraphMcpModule::tools(self)
            .into_iter()
            .map(|definition| spur_mcp::ToolDefinition {
                name: definition.name,
                description: definition.description,
                input_schema: definition.input_schema,
            })
            .collect()
    }

    async fn call(
        &self,
        ctx: spur_mcp::ToolCallContext<'_>,
        name: &str,
        args: Value,
    ) -> Result<spur_mcp::ToolResponse, spur_mcp::McpError> {
        match self.dispatch(name, args).await {
            Ok(body) => Ok(spur_mcp::ToolResponse::json_text(
                ctx.request_id_value(),
                body,
            )),
            Err(error) => {
                let response = error.into_error_response().await;
                Err(spur_mcp::McpError::new(
                    spur_mcp::ErrorCode(response.code as i32),
                    response.message,
                    response.data,
                ))
            }
        }
    }
}

#[allow(dead_code)]
mod overlay_runtime;
mod overlay_snapshot;
#[cfg(feature = "test-support")]
mod perf_support;
#[allow(dead_code)]
mod request_cache;
mod request_replay;
#[cfg(feature = "test-support")]
pub use perf_support::{
    ExactObservationScope, OverlayFileChange, OverlayProviderLoss, OverlayPublication,
    OverlayRequestDiagnostics, OverlayRequestSample, OverlayRuntimeSnapshot, OverlayRuntimeSupport,
};
use request_replay::RequestReplayClient;

use crate::overlay_watch::{ChangeProviderKind, ChangeSourceSet};
use overlay_runtime::{
    BuiltOverlayGeneration, CompositeSubscriptionFactory, OverlayGenerationBuilder,
    OverlayRuntimeHandle, OverlayRuntimeKey, OverlayRuntimeRegistry, PublishedState,
    PublishedTrust, RuntimeStartupCause, RuntimeStartupError, RuntimeSubscriptionFactory,
};

#[derive(Default)]
struct OverlayRuntimeLifecycle {
    registry: OverlayRuntimeRegistry,
    handles: Mutex<HashMap<OverlayRuntimeKey, RuntimeLifecycleHandle>>,
    starting: Mutex<HashSet<OverlayRuntimeKey>>,
    startup_states: Mutex<HashMap<OverlayRuntimeKey, OverlayRuntimeStartupState>>,
    active_keys: Mutex<HashMap<PathBuf, OverlayRuntimeKey>>,
}

#[derive(Clone, Copy)]
enum OverlayRuntimeStartupState {
    Starting,
    Ready,
    Failed(RuntimeStartupCause),
}

struct RuntimeLifecycleHandle {
    handle: Arc<OverlayRuntimeHandle>,
    subscriptions: Arc<dyn RuntimeSubscriptionFactory>,
}

struct AcquiredOverlayRuntime {
    handle: Arc<OverlayRuntimeHandle>,
    published: Arc<PublishedState>,
}

impl OverlayRuntimeLifecycle {
    #[cfg(test)]
    fn activate(&self, key: &OverlayRuntimeKey) {
        let activated = self.activate_if_current(key, || true);
        debug_assert!(activated);
    }

    fn activate_if_current(
        &self,
        key: &OverlayRuntimeKey,
        base_is_current: impl FnOnce() -> bool,
    ) -> bool {
        let worktree = key.canonical_worktree().to_path_buf();
        let mut active_keys = self
            .active_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Validate while holding the same lock that serializes base changes.
        // A request that opened an old artifact cannot pass this check after a
        // newer base has become authoritative and then reactivate the old key.
        if !base_is_current() {
            if active_keys
                .get(&worktree)
                .is_some_and(|active| active == key)
            {
                active_keys.remove(&worktree);
                self.handles
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(key);
                self.starting
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(key);
                self.startup_states
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(key);
            }
            return false;
        }
        let changed = active_keys
            .insert(worktree.clone(), key.clone())
            .is_none_or(|previous| previous != *key);
        if !changed {
            return true;
        }

        // A graph reindex changes the base identity. Retire the previous
        // worktree actor instead of retaining one watcher per historical base.
        // Keep active_keys locked through both prunes so two base activations
        // cannot leave the key and retained handle maps disagreeing.
        self.handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|candidate, _| candidate.canonical_worktree() != worktree || candidate == key);
        self.starting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|candidate| candidate.canonical_worktree() != worktree || candidate == key);
        self.startup_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|candidate, _| candidate.canonical_worktree() != worktree || candidate == key);
        true
    }

    fn is_active(&self, key: &OverlayRuntimeKey) -> bool {
        self.active_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key.canonical_worktree())
            .is_some_and(|active| active == key)
    }

    fn install_if_active(
        &self,
        key: &OverlayRuntimeKey,
        handle: OverlayRuntimeHandle,
        subscriptions: Arc<dyn RuntimeSubscriptionFactory>,
    ) {
        // Keep the active-key lock through insertion so a concurrent reindex
        // either observes and removes this handle or makes this insertion a
        // no-op. An obsolete asynchronous start can never re-retain its actor.
        let active_keys = self
            .active_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_keys
            .get(key.canonical_worktree())
            .is_none_or(|active| active != key)
        {
            return;
        }
        self.handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key.clone())
            .or_insert_with(|| RuntimeLifecycleHandle {
                handle: Arc::new(handle),
                subscriptions,
            });
        self.startup_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), OverlayRuntimeStartupState::Ready);
    }

    fn acquire(&self, key: &OverlayRuntimeKey) -> Option<AcquiredOverlayRuntime> {
        let handle = self
            .handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .map(|entry| Arc::clone(&entry.handle))?;
        let published = handle.acquire_published();
        Some(AcquiredOverlayRuntime { handle, published })
    }

    fn startup_diagnostics(
        &self,
        key: &OverlayRuntimeKey,
        published: bool,
    ) -> (&'static str, Option<&'static str>) {
        if published {
            return ("ready", None);
        }
        match self
            .startup_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .copied()
        {
            Some(OverlayRuntimeStartupState::Starting) => ("starting", None),
            Some(OverlayRuntimeStartupState::Ready) => ("ready", None),
            Some(OverlayRuntimeStartupState::Failed(cause)) => ("failed", Some(cause.diagnostic())),
            None => ("not_started", None),
        }
    }

    fn record_startup_failure_if_active(
        &self,
        key: &OverlayRuntimeKey,
        cause: RuntimeStartupCause,
    ) {
        let active_keys = self
            .active_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_keys
            .get(key.canonical_worktree())
            .is_some_and(|active| active == key)
        {
            self.startup_states
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(key.clone(), OverlayRuntimeStartupState::Failed(cause));
        }
    }

    fn record_starting_if_active(&self, key: &OverlayRuntimeKey) -> bool {
        let active_keys = self
            .active_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_keys
            .get(key.canonical_worktree())
            .is_none_or(|active| active != key)
        {
            return false;
        }
        self.startup_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), OverlayRuntimeStartupState::Starting);
        true
    }

    fn schedule_start(
        self: &Arc<Self>,
        key: OverlayRuntimeKey,
        builder: Arc<dyn OverlayGenerationBuilder>,
        replace: Option<Arc<OverlayRuntimeHandle>>,
    ) {
        if !self.is_active(&key) {
            return;
        }
        let mut restart_subscriptions = None;
        let mut stale_handle = None;
        if let Some(expected) = replace {
            let mut handles = self
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let should_replace = handles.get(&key).is_some_and(|current| {
                Arc::ptr_eq(&current.handle, &expected)
                    && runtime_requires_fresh_start(&current.handle.acquire_published())
            });
            if should_replace {
                stale_handle = Some(Arc::downgrade(&expected));
                restart_subscriptions = handles.remove(&key).map(|entry| entry.subscriptions);
            } else {
                return;
            }
        } else if self
            .handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&key)
        {
            return;
        }

        let mut starting = self
            .starting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !starting.insert(key.clone()) {
            return;
        }
        drop(starting);
        if !self.record_starting_if_active(&key) {
            self.starting
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key);
            return;
        }

        let lifecycle = Arc::clone(self);
        tokio::spawn(async move {
            if let Some(stale_handle) = stale_handle {
                // A concurrent exact-fallback request may still pin the old
                // handle. Wait for every such request-local pin to drain so
                // the registry's Weak entry cannot resurrect the terminal
                // untrusted runtime during get_or_start.
                while stale_handle.strong_count() > 0 {
                    tokio::task::yield_now().await;
                }
            }
            let subscriptions: Result<Arc<dyn RuntimeSubscriptionFactory>, RuntimeStartupError> =
                match restart_subscriptions {
                    Some(subscriptions) => Ok(subscriptions),
                    None => {
                        let source_root = key.canonical_worktree().to_path_buf();
                        let sources = tokio::task::spawn_blocking(move || {
                            ChangeSourceSet::resolve(&source_root)
                        })
                        .await
                        .context("overlay change-source resolver task failed")
                        .and_then(|resolved| resolved)
                        .map_err(|error| {
                            RuntimeStartupError::new(RuntimeStartupCause::SourceResolution, error)
                        });
                        match sources {
                            Ok(sources) => CompositeSubscriptionFactory::new(&key, sources)
                                .map(|factory| {
                                    let subscriptions: Arc<dyn RuntimeSubscriptionFactory> =
                                        Arc::new(factory);
                                    subscriptions
                                })
                                .map_err(|error| {
                                    RuntimeStartupError::new(
                                        RuntimeStartupCause::SubscriptionConstruction,
                                        error,
                                    )
                                }),
                            Err(error) => Err(error),
                        }
                    }
                };
            let handle = match subscriptions {
                Ok(subscriptions) => lifecycle
                    .registry
                    .get_or_start(key.clone(), Arc::clone(&subscriptions), builder)
                    .await
                    .map(|handle| (handle, subscriptions)),
                Err(error) => Err(error),
            };
            match handle {
                Ok((handle, subscriptions)) => {
                    lifecycle.install_if_active(&key, handle, subscriptions);
                }
                Err(error) => {
                    lifecycle.record_startup_failure_if_active(&key, error.cause());
                    tracing::warn!(
                        target: "spur_graph::mcp",
                        worktree = %key.canonical_worktree().display(),
                        cause = error.cause().diagnostic(),
                        error = %error,
                        "asynchronous overlay runtime start failed"
                    );
                }
            }
            lifecycle
                .starting
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key);
        });
    }
}

fn runtime_requires_fresh_start(published: &PublishedState) -> bool {
    published.provider() == ChangeProviderKind::ExactOnly
        || matches!(published.trust(), PublishedTrust::Untrusted(_))
}

struct RuntimeLifecycleEntry {
    owner: Weak<RebuildCoordinator>,
    lifecycle: Arc<OverlayRuntimeLifecycle>,
}

static OVERLAY_RUNTIME_LIFECYCLES: OnceLock<Mutex<Vec<RuntimeLifecycleEntry>>> = OnceLock::new();

fn overlay_runtime_lifecycle_for(
    rebuild_coordinator: &Arc<RebuildCoordinator>,
) -> Arc<OverlayRuntimeLifecycle> {
    let lifecycles = OVERLAY_RUNTIME_LIFECYCLES.get_or_init(|| Mutex::new(Vec::new()));
    let mut lifecycles = lifecycles
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    lifecycles.retain(|entry| entry.owner.strong_count() > 0);
    if let Some(entry) = lifecycles.iter().find(|entry| {
        entry
            .owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, rebuild_coordinator))
    }) {
        return Arc::clone(&entry.lifecycle);
    }
    let lifecycle = Arc::new(OverlayRuntimeLifecycle::default());
    lifecycles.push(RuntimeLifecycleEntry {
        owner: Arc::downgrade(rebuild_coordinator),
        lifecycle: Arc::clone(&lifecycle),
    });
    lifecycle
}

#[derive(Clone)]
struct McpOverlayGenerationBuilder {
    worktree: PathBuf,
    snapshot_base: overlay_snapshot::SnapshotBase,
    full_base_source: FullBaseArtifactSource,
    #[cfg(test)]
    use_request_cache: bool,
}

#[async_trait::async_trait]
impl OverlayGenerationBuilder for McpOverlayGenerationBuilder {
    async fn exact_scan(&self, key: &OverlayRuntimeKey) -> anyhow::Result<BuiltOverlayGeneration> {
        let key = key.clone();
        let builder = self.clone();
        tokio::task::spawn_blocking(move || builder.build_exact(&key))
            .await
            .context("overlay generation builder task failed")?
    }

    async fn rebuild_incremental(
        &self,
        key: &OverlayRuntimeKey,
        _previous: BuiltOverlayGeneration,
        _changed_paths: BTreeSet<PathBuf>,
    ) -> anyhow::Result<BuiltOverlayGeneration> {
        // Provider events choose when to rebuild. The exact snapshot builder
        // remains the authoritative path/state oracle and OverlayGeneration
        // performs the structurally shared changed-path update.
        self.exact_scan(key).await
    }
}

impl McpOverlayGenerationBuilder {
    fn build_exact(&self, key: &OverlayRuntimeKey) -> anyhow::Result<BuiltOverlayGeneration> {
        let mut changed =
            changed_paths_for_overlay_base(&self.worktree, self.snapshot_base.clone(), true)?;
        let snapshot_identity = changed
            .identity
            .take()
            .map(canonical_overlay_identity)
            .context("exact overlay observation did not produce a snapshot identity")?;
        let generation_identity = overlay_generation_identity(&snapshot_identity);
        let generation_path_state = generation_path_state(&changed.path_state);
        let paths = changed.paths;
        let worktree = self.worktree.clone();
        let cached = request_cache::overlay_delta(snapshot_identity.clone(), || {
            let (artifact, shadowed) =
                OverlayClient::<&dyn GraphQueryClient>::extract_delta(&worktree, &paths)?;
            Ok(request_cache::CachedOverlayDelta { artifact, shadowed })
        })?;
        let full_base_source = self.full_base_source.clone();
        let build_identity = generation_identity.clone();
        #[cfg(test)]
        if !self.use_request_cache {
            fail_overlay_generation_for_test()?;
            let seed = Arc::new(OverlayGeneration::seed(full_base_source.load()?)?);
            let generation = Arc::new(OverlayGeneration::update(
                &seed,
                build_identity,
                &generation_path_state,
                cached.artifact,
            )?);
            return BuiltOverlayGeneration::new(key, generation_identity, generation);
        }
        let generation = request_cache::overlay_generation(snapshot_identity, |seed| {
            fail_overlay_generation_for_test()?;
            let previous = match seed {
                Some(seed) => seed,
                None => Arc::new(OverlayGeneration::seed(full_base_source.load()?)?),
            };
            OverlayGeneration::update(
                &previous,
                build_identity,
                &generation_path_state,
                cached.artifact,
            )
            .map(Arc::new)
        })?;
        BuiltOverlayGeneration::new(key, generation_identity, generation)
    }
}
mod file_oid_cache {
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::fs::{self, Metadata};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use super::current_file_oid;

    const FILE_OID_CACHE_CAPACITY: usize = 4096;

    static FILE_OID_CACHE: OnceLock<Mutex<FileOidCache>> = OnceLock::new();

    pub(super) fn file_oid_match_detail(
        worktree: &Path,
        worktree_head_oid: &str,
        graph_content_hash: &str,
        rel_path: &str,
        indexed_oid: &str,
    ) -> FileOidMatch {
        file_oid_match_inner(
            worktree,
            worktree_head_oid,
            graph_content_hash,
            rel_path,
            indexed_oid,
            None,
        )
    }

    fn file_oid_match_inner(
        worktree: &Path,
        worktree_head_oid: &str,
        graph_content_hash: &str,
        rel_path: &str,
        indexed_oid: &str,
        after_first_stat: Option<&dyn Fn(&Path)>,
    ) -> FileOidMatch {
        let path = worktree.join(rel_path);
        let Some(before) = FileMetadataKey::from_path(&path) else {
            return FileOidMatch::Unknown;
        };
        if let Some(after_first_stat) = after_first_stat {
            after_first_stat(&path);
        }
        let key = FileOidCacheKey {
            worktree_root: worktree.to_path_buf(),
            worktree_head_oid: worktree_head_oid.to_string(),
            graph_content_hash: graph_content_hash.to_string(),
            rel_path: rel_path.to_string(),
            metadata: before.clone(),
        };

        let Ok(mut cache_guard) = cache().lock() else {
            return FileOidMatch::Unknown;
        };
        if let Some(cached_oid) = cache_guard.get(&key) {
            let Some(after) = FileMetadataKey::from_path(&path) else {
                return FileOidMatch::Unknown;
            };
            if before != after {
                return FileOidMatch::Unknown;
            }
            return compare_file_oids(&cached_oid, indexed_oid);
        }
        drop(cache_guard);

        let Some(current_oid) = current_file_oid(worktree, rel_path).ok().flatten() else {
            return FileOidMatch::Unknown;
        };
        let Some(after) = FileMetadataKey::from_path(&path) else {
            return FileOidMatch::Unknown;
        };
        if before != after {
            return FileOidMatch::Unknown;
        }

        let Ok(mut cache_guard) = cache().lock() else {
            return FileOidMatch::Unknown;
        };
        cache_guard.insert(key, current_oid.clone());
        compare_file_oids(&current_oid, indexed_oid)
    }

    #[cfg(test)]
    pub(super) fn file_oid_match_after_first_stat(
        worktree: &Path,
        worktree_head_oid: &str,
        graph_content_hash: &str,
        rel_path: &str,
        indexed_oid: &str,
        after_first_stat: &dyn Fn(&Path),
    ) -> Option<bool> {
        file_oid_match_inner(
            worktree,
            worktree_head_oid,
            graph_content_hash,
            rel_path,
            indexed_oid,
            Some(after_first_stat),
        )
        .as_bool()
    }

    pub(super) fn aggregate_file_oid_report(
        worktree: &Path,
        worktree_head_oid: &str,
        graph_content_hash: &str,
        files: &[(&str, &str)],
    ) -> FileOidAggregateReport {
        let mut dirty_oids = BTreeMap::new();
        let mut saw_unknown = false;
        for (rel_path, indexed_oid) in files {
            match file_oid_match_detail(
                worktree,
                worktree_head_oid,
                graph_content_hash,
                rel_path,
                indexed_oid,
            ) {
                FileOidMatch::Match => {}
                FileOidMatch::Mismatch { current_oid } => {
                    dirty_oids.insert(PathBuf::from(*rel_path), current_oid);
                }
                FileOidMatch::Unknown => saw_unknown = true,
            }
        }

        let verdict = if !dirty_oids.is_empty() {
            Some(false)
        } else if saw_unknown {
            None
        } else {
            Some(true)
        };

        FileOidAggregateReport {
            verdict,
            dirty_oids,
        }
    }

    fn cache() -> &'static Mutex<FileOidCache> {
        FILE_OID_CACHE.get_or_init(|| Mutex::new(FileOidCache::new(FILE_OID_CACHE_CAPACITY)))
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) enum FileOidMatch {
        Match,
        Mismatch { current_oid: [u8; 20] },
        Unknown,
    }

    impl FileOidMatch {
        fn as_bool(&self) -> Option<bool> {
            match self {
                Self::Match => Some(true),
                Self::Mismatch { .. } => Some(false),
                Self::Unknown => None,
            }
        }
    }

    #[derive(Debug)]
    pub(super) struct FileOidAggregateReport {
        pub verdict: Option<bool>,
        pub dirty_oids: BTreeMap<PathBuf, [u8; 20]>,
    }

    fn compare_file_oids(current_oid: &str, indexed_oid: &str) -> FileOidMatch {
        if current_oid == indexed_oid {
            return FileOidMatch::Match;
        }

        match parse_git_oid(current_oid) {
            Some(current_oid) => FileOidMatch::Mismatch { current_oid },
            None => FileOidMatch::Unknown,
        }
    }

    pub(super) fn parse_git_oid(oid: &str) -> Option<[u8; 20]> {
        if oid.len() != 40 {
            return None;
        }
        let mut bytes = [0_u8; 20];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let start = index * 2;
            *byte = u8::from_str_radix(&oid[start..start + 2], 16).ok()?;
        }
        Some(bytes)
    }

    #[derive(Debug)]
    struct FileOidCache {
        entries: HashMap<FileOidCacheKey, String>,
        lru: VecDeque<FileOidCacheKey>,
        capacity: usize,
    }

    impl FileOidCache {
        fn new(capacity: usize) -> Self {
            Self {
                entries: HashMap::new(),
                lru: VecDeque::new(),
                capacity,
            }
        }

        fn get(&mut self, key: &FileOidCacheKey) -> Option<String> {
            let value = self.entries.get(key).cloned()?;
            self.touch(key);
            Some(value)
        }

        fn insert(&mut self, key: FileOidCacheKey, value: String) {
            self.entries.insert(key.clone(), value);
            self.touch(&key);
            while self.entries.len() > self.capacity {
                let Some(expired) = self.lru.pop_front() else {
                    break;
                };
                self.entries.remove(&expired);
            }
        }

        fn touch(&mut self, key: &FileOidCacheKey) {
            self.lru.retain(|candidate| candidate != key);
            self.lru.push_back(key.clone());
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct FileOidCacheKey {
        worktree_root: PathBuf,
        worktree_head_oid: String,
        graph_content_hash: String,
        rel_path: String,
        metadata: FileMetadataKey,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct FileMetadataKey {
        dev: u64,
        ino: u64,
        size: u64,
        mtime_ns: i128,
    }

    impl FileMetadataKey {
        fn from_path(path: &Path) -> Option<Self> {
            let metadata = fs::symlink_metadata(path).ok()?;
            Some(Self::from_metadata(&metadata))
        }
    }

    #[cfg(unix)]
    impl FileMetadataKey {
        fn from_metadata(metadata: &Metadata) -> Self {
            use std::os::unix::fs::MetadataExt;

            let mtime_ns = i128::from(metadata.mtime())
                .saturating_mul(1_000_000_000)
                .saturating_add(i128::from(metadata.mtime_nsec()));

            Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
                size: metadata.size(),
                mtime_ns,
            }
        }
    }

    #[cfg(not(unix))]
    impl FileMetadataKey {
        fn from_metadata(metadata: &Metadata) -> Self {
            let mtime_ns = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos() as i128)
                .unwrap_or_default();

            Self {
                dev: 0,
                ino: 0,
                size: metadata.len(),
                mtime_ns,
            }
        }
    }
}

#[allow(dead_code)]
#[path = "rebuild_singleflight.rs"]
mod rebuild_singleflight;
pub use rebuild_singleflight::RebuildCoordinator;
use rebuild_singleflight::RebuildKey;

static SHARED_REBUILD_COORDINATOR: OnceLock<Arc<RebuildCoordinator>> = OnceLock::new();
static CANONICAL_OVERLAY_IDENTITIES: OnceLock<Mutex<CanonicalOverlayIdentities>> = OnceLock::new();
static REBUILT_PARQUET_RESPONSE_COUNTER: AtomicU64 = AtomicU64::new(0);

const CANONICAL_OVERLAY_IDENTITY_CAPACITY: usize = 8;

#[derive(Clone, PartialEq, Eq, Hash)]
struct VisibleOverlayIdentity {
    canonical_worktree: PathBuf,
    indexed_graph_content_hash: String,
    indexed_head_oid: Option<String>,
    current_head_oid: String,
    normalized_changed_set_fingerprint: [u8; 32],
}

impl From<&overlay_snapshot::SnapshotIdentity> for VisibleOverlayIdentity {
    fn from(identity: &overlay_snapshot::SnapshotIdentity) -> Self {
        Self {
            canonical_worktree: identity.canonical_worktree.clone(),
            indexed_graph_content_hash: identity.indexed_graph_content_hash.clone(),
            indexed_head_oid: identity.indexed_head_oid.clone(),
            current_head_oid: identity.current_head_oid.clone(),
            normalized_changed_set_fingerprint: identity.normalized_changed_set_fingerprint,
        }
    }
}

struct CanonicalOverlayIdentities {
    entries: HashMap<VisibleOverlayIdentity, overlay_snapshot::SnapshotIdentity>,
    lru: VecDeque<VisibleOverlayIdentity>,
}

fn canonical_overlay_identity(
    identity: overlay_snapshot::SnapshotIdentity,
) -> overlay_snapshot::SnapshotIdentity {
    let key = VisibleOverlayIdentity::from(&identity);
    let cache = CANONICAL_OVERLAY_IDENTITIES.get_or_init(|| {
        Mutex::new(CanonicalOverlayIdentities {
            entries: HashMap::new(),
            lru: VecDeque::new(),
        })
    });
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let canonical = cache.entries.entry(key.clone()).or_insert(identity).clone();
    cache.lru.retain(|candidate| candidate != &key);
    cache.lru.push_back(key);
    while cache.entries.len() > CANONICAL_OVERLAY_IDENTITY_CAPACITY {
        let Some(expired) = cache.lru.pop_front() else {
            break;
        };
        cache.entries.remove(&expired);
    }
    canonical
}

pub fn shared_rebuild_coordinator() -> Arc<RebuildCoordinator> {
    Arc::clone(SHARED_REBUILD_COORDINATOR.get_or_init(|| Arc::new(RebuildCoordinator::new())))
}

const MAX_MCP_CODE_SUBGRAPH_RADIUS: u8 = 3;
const DEFAULT_MCP_CODE_SUBGRAPH_MAX_NODES: usize = 40;
const MIN_MCP_CODE_SUBGRAPH_MAX_NODES: usize = 1;
const MAX_MCP_CODE_SUBGRAPH_MAX_NODES: usize = 400;
const DEFAULT_MCP_CODE_SUBGRAPH_MAX_EDGES: usize = 120;
const MIN_MCP_CODE_SUBGRAPH_MAX_EDGES: usize = 1;
const MAX_MCP_CODE_SUBGRAPH_MAX_EDGES: usize = 1200;
const MAX_MCP_CODE_READ_SYMBOL_CONTEXT_LINES: usize = 50;
const GRAPH_POINTER_RELATIVE_PATH: &str = ".spur/graph-index.pointer.json";
const GRAPH_GIT_METADATA_TIMEOUT: Duration = Duration::from_millis(200);
const DEFAULT_GRAPH_REBUILD_LATENCY_BUDGET: Duration = Duration::from_millis(750);
const COLD_OPEN_GRAPH_REBUILD_TIMEOUT: Duration = Duration::from_secs(120);
/// Wall-clock budget for one `code_*` MCP tool call. Every server that
/// dispatches these tools through [`GraphMcpModule::dispatch`] uses this
/// default so a blocked query, lock, or overlay wait returns instead of hanging.
pub const DEFAULT_CODE_TOOL_TIMEOUT: Duration = Duration::from_secs(15);
pub const INCREMENTAL_FAILURES_BEFORE_FULL_REBUILD: u32 = 3;
const MARKDOWN_OVERLAY_EXTENSIONS: &[&str] = &["md", "markdown"];

tokio::task_local! {
    static SCOPED_CODE_GRAPH_WORKTREE_ROOT: PathBuf;
}

#[cfg(test)]
tokio::task_local! {
    static PROJECT_SCOPE_BARRIER_FOR_TEST: Arc<tokio::sync::Barrier>;
}

#[cfg(any(test, feature = "test-support"))]
tokio::task_local! {
    static CODE_TOOL_TIMEOUT_OVERRIDE: Duration;
}

#[cfg(any(test, feature = "test-support"))]
tokio::task_local! {
    static CODE_TOOL_DELAY_OVERRIDE: Duration;
}

#[cfg(test)]
async fn wait_for_project_scope_overlap_for_test() {
    if let Ok(barrier) = PROJECT_SCOPE_BARRIER_FOR_TEST.try_with(Arc::clone) {
        barrier.wait().await;
    }
}

#[cfg(any(test, feature = "test-support"))]
const GRAPH_REBUILD_LATENCY_BUDGET_UNSET_MS: u64 = u64::MAX;
#[cfg(any(test, feature = "test-support"))]
static GRAPH_REBUILD_LATENCY_BUDGET_OVERRIDE_MS: AtomicU64 =
    AtomicU64::new(GRAPH_REBUILD_LATENCY_BUDGET_UNSET_MS);
#[cfg(any(test, feature = "test-support"))]
static GRAPH_REBUILD_DELAY_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "test-support"))]
static INCREMENTAL_REBUILD_FAILURES_REMAINING: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static OVERLAY_GENERATION_FAILURES_REMAINING: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(test, feature = "test-support"))]
static EXACT_OVERLAY_OBSERVATIONS_FOR_TEST: AtomicUsize = AtomicUsize::new(0);
// Temporal resolution error codes (T3 / Phase 1.5 hardening)
const CODE_GRAPH_NOT_FOUND_ERROR_CODE: i64 = -32004;
const CODE_GRAPH_DELETED_ERROR_CODE: i64 = -32005;
const CODE_GRAPH_AMBIGUOUS_ERROR_CODE: i64 = -32006;
const CODE_GRAPH_UNKNOWN_ERROR_CODE: i64 = -32007;
const CODE_GRAPH_TIMEOUT_ERROR_CODE: i64 = -32008;

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        code_resolve_def(),
        code_file_symbols_def(),
        code_symbol_info_def(),
        code_read_symbol_def(),
        code_callers_def(),
        code_callees_def(),
        code_symbol_search_def(),
        code_subgraph_def(),
        code_symbol_history_def(),
    ]
}

/// Returns the graph MCP schemas with an optional named local-project selector.
#[must_use]
pub fn local_project_tool_definitions() -> Vec<ToolDefinition> {
    tool_definitions()
        .into_iter()
        .map(|mut definition| {
            definition.input_schema = with_optional_project_schema(&definition.input_schema);
            definition
        })
        .collect()
}

fn code_resolve_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_resolve".into(),
        description: "Resolve a code selector against the current worktree graph artifact and return only candidate rows. Use before code_subgraph when a selector may be ambiguous.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name"
                },
                "as_of": {
                    "type": "string",
                    "description": "Optional git commit SHA for point-in-time symbol resolution"
                }
            },
            "required": ["selector"]
        }),
    }
}

fn code_file_symbols_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_file_symbols".into(),
        description: "List code symbols declared in one worktree-relative file from the current graph artifact. Rejects absolute paths and paths containing '..'. Supports response_format=full|compact|table.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Worktree-relative file path, e.g. crates/spur-mcp/src/tools.rs"
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "table"],
                    "description": "Output shape. full is the default object-row response; compact omits healthy metadata defaults; table returns symbols as cols/rows with repeated file paths interned in a top-level files array."
                }
            },
            "required": ["file"]
        }),
    }
}

fn code_symbol_info_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_symbol_info".into(),
        description: "Resolve one code symbol and return metadata only: qualified_name, file_path, line_range, symbol_kind, enclosing_scope, uri, and id. Ambiguous selectors return candidate rows.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name"
                },
                "symbol": {
                    "type": "string",
                    "description": "deprecated; use selector. Accepts graph://symbol/<id> or bare hex id."
                }
            }
        }),
    }
}

fn code_read_symbol_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_read_symbol".into(),
        description: "Read the indexed source for one code symbol from the current graph artifact. Select by stable_symbol_id, or by the exact worktree-relative path plus symbol name. Supports response_format=full|compact|source.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "stable_symbol_id": {
                    "type": "string",
                    "description": "Stable symbol id from code_resolve/code_search/code_symbol_info. graph://symbol/<id> is accepted."
                },
                "path": {
                    "type": "string",
                    "description": "Worktree-relative file path. Required with name and mutually exclusive with stable_symbol_id."
                },
                "name": {
                    "type": "string",
                    "description": "Symbol entity_name or qualified_name within path. Required with path and mutually exclusive with stable_symbol_id."
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 50,
                    "default": 0,
                    "description": "Lines of context to include before and after the symbol. Values outside 0..50 are clamped and echoed as requested_context_lines."
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "source"],
                    "description": "Output shape. full is the default source-plus-metadata response; compact omits healthy metadata defaults; source returns only id, name, file, range, source, file_oid, and actionable source signals."
                }
            }
        }),
    }
}

fn code_callers_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_callers".into(),
        description: "List symbols that call the requested code symbol from the current worktree graph artifact. Rows include edge_kind (calls, calls_dyn, references_hof, references_other); calls_dyn rows also include confidence=\"heuristic\". Unresolved rows are hidden by default (include_unresolved=false); counts_by_kind, counts_by_context (production/test/bench breakdown), and unresolved_sample always report what was filtered. Supports response_format=full|compact|table.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name"
                },
                "symbol": {
                    "type": "string",
                    "description": "deprecated; use selector. Accepts graph://symbol/<id> or bare hex id."
                },
                "on_ambiguous": {
                    "type": "string",
                    "enum": ["candidates", "error"],
                    "description": "Ambiguity handling (default: candidates)"
                },
                "include_unresolved": {
                    "type": "boolean",
                    "default": false,
                    "description": "When true, include unresolved caller rows. Default false filters resolved=false rows while counts_by_kind/unresolved_sample still summarize them."
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "table"],
                    "description": "Output shape. full is the default object-row response; compact omits healthy metadata defaults; table returns callers as cols/rows with repeated file paths interned in a top-level files array."
                }
            }
        }),
    }
}

fn code_callees_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_callees".into(),
        description: "List symbols called by the requested code symbol from the current worktree graph artifact. Rows include edge_kind (calls, calls_dyn, references_hof, references_other); calls_dyn rows also include confidence=\"heuristic\". Unresolved rows are hidden by default (include_unresolved=false); counts_by_kind, counts_by_context (production/test/bench breakdown), and unresolved_sample always report what was filtered. Supports response_format=full|compact|table.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name"
                },
                "symbol": {
                    "type": "string",
                    "description": "deprecated; use selector. Accepts graph://symbol/<id> or bare hex id."
                },
                "on_ambiguous": {
                    "type": "string",
                    "enum": ["candidates", "error"],
                    "description": "Ambiguity handling (default: candidates)"
                },
                "include_unresolved": {
                    "type": "boolean",
                    "default": false,
                    "description": "When true, include unresolved callee rows. Default false filters resolved=false rows while counts_by_kind/unresolved_sample still summarize them."
                },
                "as_of": {
                    "type": "string",
                    "description": "Optional git commit SHA for point-in-time symbol resolution"
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "table"],
                    "description": "Output shape. full is the default object-row response; compact omits healthy metadata defaults; table returns callees as cols/rows with repeated file paths interned in a top-level files array."
                }
            }
        }),
    }
}

fn code_symbol_search_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_symbol_search".into(),
        description: "Search the worktree graph artifact for symbols by NAME (exact/prefix/substring). Lexical retrieval over symbol identifiers, not content — returns ranked candidate symbols. For concept/content/natural-language retrieval over docs + code bodies, use code_semantic_search instead. Legacy alias: code_search.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Search term. Non-empty."
                },
                "mode": {
                    "type": "string",
                    "enum": ["exact", "prefix", "substring"],
                    "default": "substring"
                },
                "symbol_kind": {
                    "type": "string",
                    "description": "Optional filter on the artifact's symbol_kind, e.g. function, method, struct, enum, mcp_tool."
                },
                "file": {
                    "type": "string",
                    "description": "Optional exact worktree-relative file path. Mutually exclusive with file_glob."
                },
                "file_glob": {
                    "type": "string",
                    "description": "Optional glob over worktree-relative file_path (e.g. 'crates/spur-mcp/**/*.rs'). Mutually exclusive with file."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 20
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "table"],
                    "description": "Output shape. full is the default candidate-array response; compact omits healthy metadata defaults; table also omits healthy metadata defaults. The candidates array shape is identical across all three formats."
                }
            },
            "required": ["query"]
        }),
    }
}

fn code_subgraph_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_subgraph".into(),
        description: "Get a budgeted code-symbol subgraph from the current worktree graph artifact. Traversal is deterministic BFS; responses cap output with max_nodes/max_edges and report truncated_frontier for continuation. Returns JSON nodes/edges by default, or Mermaid when format=mermaid. Supports response_format=full|compact|table for JSON output.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Code selector: graph://symbol/<id>, bare hex id, qualified name, file-qualified name, or bare symbol name"
                },
                "start_nodes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Continuation roots from a prior truncated_frontier response. Values are bare node ids or graph://symbol/<id> URIs. Mutually exclusive with selector and symbol."
                },
                "symbol": {
                    "type": "string",
                    "description": "deprecated; use selector. Accepts graph://symbol/<id> or bare hex id."
                },
                "on_ambiguous": {
                    "type": "string",
                    "enum": ["candidates", "error"],
                    "description": "Ambiguity handling (default: candidates)"
                },
                "radius": {
                    "type": "integer",
                    "description": "Traversal radius (default: 1, max: 3; larger values are clamped)"
                },
                "max_nodes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 400,
                    "default": 40,
                    "description": "Maximum node rows to return. Values outside 1..400 are clamped and echoed as metadata.requested_max_nodes."
                },
                "max_edges": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1200,
                    "default": 120,
                    "description": "Maximum edge rows to return, including unresolved edges when include_unresolved=true. Values outside 1..1200 are clamped and echoed as metadata.requested_max_edges."
                },
                "format": {
                    "type": "string",
                    "enum": ["json", "mermaid"],
                    "description": "Output format (default: json)"
                },
                "edge_kinds": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["calls", "calls_dyn", "references_hof", "references_other"]
                    },
                    "description": "Optional public edge_kind filter. edge_kinds=[\"calls\"] is strict direct calls only; use calls_dyn separately for heuristic dyn Trait calls."
                },
                "include_unresolved": {
                    "type": "boolean",
                    "default": false,
                    "description": "When true, include unresolved boundary edges. Default false filters target_uri=null edges from the subgraph."
                },
                "as_of": {
                    "type": "string",
                    "description": "Optional git commit SHA for point-in-time symbol resolution"
                },
                "response_format": {
                    "type": "string",
                    "enum": ["full", "compact", "table"],
                    "description": "Output shape. full is the default JSON nodes/edges response; compact omits healthy metadata defaults; table returns nodes and edges as cols/rows with node file paths interned in a top-level files array. Applies to format=json."
                }
            }
        }),
    }
}

fn code_symbol_history_def() -> ToolDefinition {
    ToolDefinition {
        name: "code_symbol_history".into(),
        description: "Return the causal trace of a code symbol across commits, including ChangeKind and snapshot key for each touch. Requires a temporal commit index in the current worktree.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Code symbol URI (graph://symbol/<id>) or bare stable symbol id"
                }
            },
            "required": ["symbol"]
        }),
    }
}

pub async fn code_resolve(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_resolve_with_client).await
}

pub async fn code_search(args: &Value) -> Result<Value, McpHandlerError> {
    let response_format = ResponseFormat::parse(args)?;
    let backend = open_code_search_backend_for_request(None).await?;
    let search = code_search_body_for_client(args, backend.client())?;
    let files = backend.search_response_file_set(&search)?;
    let source = backend.metadata_source();
    let mut body = search.body;
    GraphResponseMetadata::from_source_inner(source, Some(&files))
        .await
        .insert_into_for_format(&mut body, response_format);
    Ok(body)
}

async fn code_search_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args).map_err(CodeGraphError::without_metadata)?;
    let backend = open_code_search_backend_for_request(Some(Arc::clone(&rebuild_coordinator)))
        .await
        .map_err(CodeGraphError::without_metadata)?;
    let source = backend.metadata_source();
    let request_client = RequestReplayClient::new(backend.client());

    if overlay_fsmonitor_auto {
        if let Some(worktree) = current_worktree_root() {
            match overlay_response_for_backend(
                &backend,
                &request_client,
                worktree,
                source.clone(),
                args,
                response_format,
                true,
                overlay_runtime_lifecycle_for(&rebuild_coordinator),
                |args, client| {
                    code_search_with_artifact(args, client).map_err(CodeGraphError::from)
                },
            )
            .await
            {
                Ok(OverlayAttempt::Fresh(fresh_body)) => return Ok(fresh_body),
                Ok(OverlayAttempt::Errored(error)) => return Err(error),
                Ok(OverlayAttempt::StaleBudgetExceeded) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "spur_graph::mcp",
                        error = ?error,
                        "direct code search overlay observation failed; using legacy escalation"
                    );
                }
            }
        }
    }

    let indexed_files = backend.base_file_set().ok();
    let mut preflight =
        GraphResponseMetadata::analyze_source_inner(source.clone(), None, indexed_files.as_deref())
            .await;
    let mut refresh_error = None;
    if let Some(rebuild_candidate) = preflight.rebuild_candidate.take() {
        match attempt_refresh(
            &backend,
            &request_client,
            Arc::clone(&rebuild_coordinator),
            rebuild_candidate,
            source.clone(),
            args,
            response_format,
            GraphRefreshStrategy::OverlayThenRebuild,
            overlay_fsmonitor_auto,
            |args, client| code_search_with_artifact(args, client).map_err(CodeGraphError::from),
        )
        .await
        {
            RefreshOutcome::Fresh(fresh_body) => return Ok(fresh_body),
            RefreshOutcome::Errored(error) => refresh_error = Some(error),
            RefreshOutcome::NotRefreshed(status) => {
                preflight.metadata = preflight.metadata.with_rebuild_status(status);
            }
        }
    }

    let search = match code_search_body_for_client(args, &request_client) {
        Ok(search) => search,
        Err(error) => {
            return Err(refresh_error.unwrap_or_else(|| {
                CodeGraphError::from(error).with_metadata_source(source.clone())
            }));
        }
    };
    let files = backend
        .search_response_file_set(&search)
        .map_err(|error| CodeGraphError::from(error).with_metadata_source(source.clone()))?;
    let mut analysis =
        GraphResponseMetadata::analyze_source_inner(source, Some(&files), None).await;
    if !matches!(preflight.metadata.rebuild_status, RebuildStatus::NotNeeded) {
        analysis.metadata = analysis
            .metadata
            .with_rebuild_status(preflight.metadata.rebuild_status);
    }
    let mut body = search.body;
    analysis
        .metadata
        .insert_into_for_format(&mut body, response_format);
    Ok(body)
}

fn code_search_with_artifact(
    args: &Value,
    client: &dyn GraphQueryClient,
) -> Result<Value, McpHandlerError> {
    Ok(code_search_body_for_client(args, client)?.body)
}

async fn code_graph_backend_value(
    args: &Value,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> Result<Value, McpHandlerError> {
    let response_format = ResponseFormat::parse(args)?;
    code_graph_backend_value_with_format(args, response_format, handler).await
}

async fn code_graph_backend_value_allowing_source(
    args: &Value,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> Result<Value, McpHandlerError> {
    let response_format = ResponseFormat::parse_allowing_source(args)?;
    code_graph_backend_value_with_format(args, response_format, handler).await
}

async fn code_graph_backend_value_with_format(
    args: &Value,
    response_format: ResponseFormat,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> Result<Value, McpHandlerError> {
    let backend = open_code_search_backend_for_request(None).await?;
    let request_client = RequestReplayClient::new(backend.client());
    let mut body = handler(args, &request_client).map_err(CodeGraphError::into_handler_error)?;
    let files = backend.response_file_set_from_body(&request_client, &body)?;
    GraphResponseMetadata::from_source_inner(backend.metadata_source(), Some(&files))
        .await
        .insert_into_for_format(&mut body, response_format);
    Ok(body)
}

async fn code_graph_backend_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> CodeGraphResult {
    code_graph_backend_response_with_refresh(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        handler,
        GraphRefreshStrategy::OverlayThenRebuild,
        false,
        ResponseFormat::Full,
    )
    .await
}

/// Like [`code_graph_backend_response`], but also accepts
/// `response_format: "source"` -- the one format only `code_read_symbol`
/// supports.
async fn code_graph_backend_response_allowing_source(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> CodeGraphResult {
    code_graph_backend_response_with_refresh(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        handler,
        GraphRefreshStrategy::OverlayThenRebuild,
        true,
        ResponseFormat::Full,
    )
    .await
}

async fn code_graph_backend_response_rebuild_only(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> CodeGraphResult {
    code_graph_backend_response_with_refresh(
        args,
        rebuild_coordinator,
        false,
        handler,
        GraphRefreshStrategy::RebuildOnly,
        false,
        ResponseFormat::Full,
    )
    .await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphRefreshStrategy {
    OverlayThenRebuild,
    RebuildOnly,
}

/// Outcome of retrying a code_* lookup against a dirty-worktree overlay or a
/// full/incremental rebuild.
enum RefreshOutcome {
    /// The retry produced a usable, freshness-annotated response body.
    Fresh(Value),
    /// The retry got a fresher view (overlay or rebuilt artifact) but the
    /// handler still errors against it -- more authoritative than whatever
    /// error (if any) the caller already had.
    Errored(CodeGraphError),
    /// The retry could not get a fresher view at all (budget exceeded, or the
    /// rebuild itself failed). The caller decides what to do with its
    /// pre-existing body/error.
    NotRefreshed(RebuildStatus),
}

enum OverlayAttempt {
    Fresh(Value),
    Errored(CodeGraphError),
    StaleBudgetExceeded,
}

enum ExactOverlayResponse {
    Fresh {
        body: Value,
        files: Vec<(String, String)>,
        measurements: OverlayFinalizationMeasurements,
    },
    Errored(CodeGraphError),
}

enum PublishedGenerationRoute {
    Trusted(AcquiredOverlayRuntime),
    Exact(RuntimeExactFallback),
}

struct RuntimeExactFallback {
    lifecycle: Arc<OverlayRuntimeLifecycle>,
    key: OverlayRuntimeKey,
    builder: Arc<dyn OverlayGenerationBuilder>,
    acquired: Option<AcquiredOverlayRuntime>,
    reason: &'static str,
    seed_runtime: bool,
    startup_status: &'static str,
    startup_cause: Option<&'static str>,
}

impl RuntimeExactFallback {
    fn schedule_seed_or_restart(self) {
        let Self {
            lifecycle,
            key,
            builder,
            acquired,
            reason: _,
            seed_runtime,
            startup_status: _,
            startup_cause: _,
        } = self;
        if !seed_runtime {
            return;
        }
        let replace = acquired.as_ref().and_then(|acquired| {
            runtime_requires_fresh_start(&acquired.published).then(|| Arc::clone(&acquired.handle))
        });
        let should_start = acquired.is_none() || replace.is_some();
        // The registry stores a Weak entry. Release the request's stale handle
        // before asking get_or_start for a replacement so the old terminal
        // entry cannot be upgraded and returned again.
        drop(acquired);
        if should_start {
            lifecycle.schedule_start(key, builder, replace);
        }
    }

    fn provider(&self) -> Value {
        self.acquired
            .as_ref()
            .map(|acquired| provider_diagnostic(acquired.published.provider()))
            .map(|provider| Value::String(provider.to_owned()))
            .unwrap_or(Value::Null)
    }

    fn epoch(&self) -> Value {
        self.acquired
            .as_ref()
            .map(|acquired| Value::from(acquired.published.epoch()))
            .unwrap_or(Value::Null)
    }

    fn trust(&self) -> &'static str {
        self.acquired
            .as_ref()
            .map(|acquired| trust_diagnostic(acquired.published.trust()))
            .unwrap_or("unavailable")
    }
}

fn published_generation_route(
    lifecycle: Arc<OverlayRuntimeLifecycle>,
    key: OverlayRuntimeKey,
    builder: Arc<dyn OverlayGenerationBuilder>,
    base_is_current: impl FnOnce() -> bool,
) -> PublishedGenerationRoute {
    if !lifecycle.activate_if_current(&key, base_is_current) {
        return PublishedGenerationRoute::Exact(RuntimeExactFallback {
            lifecycle,
            key,
            builder,
            acquired: None,
            reason: "base_superseded",
            seed_runtime: false,
            startup_status: "inactive",
            startup_cause: None,
        });
    }
    let acquired = lifecycle.acquire(&key);
    if let Some(acquired) = acquired {
        if matches!(acquired.published.trust(), PublishedTrust::Trusted)
            && acquired.published.provider() != ChangeProviderKind::ExactOnly
        {
            return PublishedGenerationRoute::Trusted(acquired);
        }
        let reason = match acquired.published.trust() {
            PublishedTrust::Trusted => "provider_exact_only",
            PublishedTrust::Rebuilding => "runtime_warming",
            PublishedTrust::Untrusted(_) => "runtime_untrusted",
        };
        let (startup_status, startup_cause) = lifecycle.startup_diagnostics(&key, true);
        return PublishedGenerationRoute::Exact(RuntimeExactFallback {
            lifecycle,
            key,
            builder,
            acquired: Some(acquired),
            reason,
            seed_runtime: true,
            startup_status,
            startup_cause,
        });
    }
    let (startup_status, startup_cause) = lifecycle.startup_diagnostics(&key, false);
    PublishedGenerationRoute::Exact(RuntimeExactFallback {
        lifecycle,
        key,
        builder,
        acquired: None,
        reason: "runtime_unavailable",
        seed_runtime: true,
        startup_status,
        startup_cause,
    })
}

fn provider_diagnostic(provider: ChangeProviderKind) -> &'static str {
    match provider {
        ChangeProviderKind::Watchman => "watchman",
        ChangeProviderKind::Notify => "notify",
        ChangeProviderKind::ExactOnly => "exact_only",
    }
}

fn trust_diagnostic(trust: &PublishedTrust) -> &'static str {
    match trust {
        PublishedTrust::Trusted => "trusted",
        PublishedTrust::Rebuilding => "rebuilding",
        PublishedTrust::Untrusted(_) => "untrusted",
    }
}

#[derive(Clone)]
enum FullBaseArtifactSource {
    Parquet(PathBuf),
    InMemory(Arc<GraphIndexArtifact>),
}

impl FullBaseArtifactSource {
    fn load(&self) -> anyhow::Result<Arc<GraphIndexArtifact>> {
        match self {
            Self::Parquet(dir) => read_artifact_parquet(dir).map(Arc::new),
            Self::InMemory(artifact) => Ok(Arc::clone(artifact)),
        }
    }
}

struct PinnedGenerationClient {
    generation: Arc<OverlayGeneration>,
    query_operations: AtomicU64,
}

impl PinnedGenerationClient {
    fn new(generation: Arc<OverlayGeneration>) -> Self {
        Self {
            generation,
            query_operations: AtomicU64::new(0),
        }
    }

    fn record_query(&self) {
        self.query_operations.fetch_add(1, Ordering::Relaxed);
    }

    fn query_operations(&self) -> u64 {
        self.query_operations.load(Ordering::Relaxed)
    }
}

impl GraphQueryClient for PinnedGenerationClient {
    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        self.record_query();
        self.generation.search_symbols(opts)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        self.record_query();
        self.generation.find_caller_edges(sid)
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        self.record_query();
        self.generation
            .find_unresolved_caller_edges_by_labels(target_labels)
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        self.record_query();
        self.generation.find_callee_edges(sid)
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<SelectorResolution> {
        self.record_query();
        self.generation.resolve_selector(selector)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        self.record_query();
        self.generation.symbol_by_id(sid)
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.record_query();
        self.generation.symbols_by_file(path)
    }

    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.record_query();
        self.generation.symbols_by_files(paths)
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.record_query();
        self.generation.symbols_by_path_name(path, name)
    }

    fn file_manifest_by_path(
        &self,
        path: &str,
    ) -> anyhow::Result<Option<crate::GraphFileManifestEntry>> {
        self.record_query();
        self.generation.file_manifest_by_path(path)
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        self.record_query();
        self.generation.file_exists(path)
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        self.record_query();
        self.generation.temporal_index()
    }
}

struct MeasuredOverlayClient<B: GraphQueryClient> {
    overlay: OverlayClient<B>,
    measurements: Mutex<OverlayFinalizationMeasurements>,
}

impl<B: GraphQueryClient> MeasuredOverlayClient<B> {
    fn new(overlay: OverlayClient<B>) -> Self {
        Self {
            overlay,
            measurements: Mutex::new(OverlayFinalizationMeasurements::default()),
        }
    }

    fn measurements(&self) -> OverlayFinalizationMeasurements {
        *self
            .measurements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<B: GraphQueryClient> GraphQueryClient for MeasuredOverlayClient<B> {
    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
        let mut measurements = self
            .measurements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.overlay
            .search_symbols_with_measurements(opts, &mut measurements)
    }

    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
        self.overlay.find_caller_edges(sid)
    }

    fn find_unresolved_caller_edges_by_labels(
        &self,
        target_labels: &HashSet<String>,
    ) -> Vec<OwnedCallerRecord> {
        self.overlay
            .find_unresolved_caller_edges_by_labels(target_labels)
    }

    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
        self.overlay.find_callee_edges(sid)
    }

    fn resolve_selector(&self, selector: &str) -> anyhow::Result<SelectorResolution> {
        self.overlay.resolve_selector(selector)
    }

    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        self.overlay.symbol_by_id(sid)
    }

    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.overlay.symbols_by_file(path)
    }

    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.overlay.symbols_by_files(paths)
    }

    fn symbols_by_path_name(
        &self,
        path: &str,
        name: &str,
    ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.overlay.symbols_by_path_name(path, name)
    }

    fn file_manifest_by_path(
        &self,
        path: &str,
    ) -> anyhow::Result<Option<crate::GraphFileManifestEntry>> {
        self.overlay.file_manifest_by_path(path)
    }

    fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
        self.overlay.file_exists(path)
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        self.overlay.temporal_index()
    }
}

/// Shared escalation ladder after the request freshness preflight: try the
/// cheap dirty-file overlay first (when the strategy allows it), then fall
/// back to a full/incremental rebuild. The handler's first query therefore
/// sees the fresh route, including for a brand-new or renamed symbol that is
/// absent from the static base artifact.
#[allow(clippy::too_many_arguments)]
async fn attempt_refresh(
    backend: &CodeSearchBackend,
    request_client: &(dyn GraphQueryClient + Sync),
    rebuild_coordinator: Arc<RebuildCoordinator>,
    rebuild_candidate: RebuildCandidate,
    source: GraphMetadataSource,
    args: &Value,
    response_format: ResponseFormat,
    refresh_strategy: GraphRefreshStrategy,
    overlay_fsmonitor_auto: bool,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
) -> RefreshOutcome {
    if refresh_strategy == GraphRefreshStrategy::OverlayThenRebuild {
        match overlay_response_for_backend(
            backend,
            request_client,
            rebuild_candidate.worktree.clone(),
            source.clone(),
            args,
            response_format,
            overlay_fsmonitor_auto,
            overlay_runtime_lifecycle_for(&rebuild_coordinator),
            &handler,
        )
        .await
        {
            Ok(OverlayAttempt::Fresh(fresh_body)) => return RefreshOutcome::Fresh(fresh_body),
            Ok(OverlayAttempt::Errored(error)) => return RefreshOutcome::Errored(error),
            Ok(OverlayAttempt::StaleBudgetExceeded) => {
                return RefreshOutcome::NotRefreshed(RebuildStatus::StaleBudgetExceeded);
            }
            Err(error) => {
                tracing::warn!(
                    target: "spur_graph::mcp",
                    error = ?error,
                    "code graph overlay refresh failed; falling back to rebuild"
                );
            }
        }
    }
    let rebuild_started = Instant::now();
    let rebuild = match backend {
        CodeSearchBackend::Parquet(_) => {
            try_rebuild_artifact_from_worktree(Arc::clone(&rebuild_coordinator), rebuild_candidate)
                .await
        }
        CodeSearchBackend::InMemory { artifact, .. } => {
            try_rebuild_artifact(
                Arc::clone(&rebuild_coordinator),
                Arc::clone(artifact),
                rebuild_candidate,
                None,
            )
            .await
        }
    };
    match rebuild {
        RebuildAttempt::Fresh(rebuilt_artifact) => match backend {
            CodeSearchBackend::Parquet(_) => {
                rebuilt_parquet_refresh_outcome(
                    rebuilt_artifact,
                    rebuild_started,
                    args,
                    response_format,
                    &handler,
                )
                .await
            }
            CodeSearchBackend::InMemory { .. } => {
                let client = InMemoryClient::new(Arc::clone(&rebuilt_artifact));
                match handler(args, &client) {
                    Ok(mut fresh_body) => {
                        let fresh_files =
                            response_file_set_from_body(&rebuilt_artifact, &fresh_body);
                        GraphResponseMetadata::analyze_artifact_with_files(
                            &rebuilt_artifact,
                            &fresh_files,
                        )
                        .await
                        .metadata
                        .with_rebuild_status(RebuildStatus::Fresh)
                        .insert_into_for_format(&mut fresh_body, response_format);
                        RefreshOutcome::Fresh(fresh_body)
                    }
                    Err(error) => {
                        RefreshOutcome::Errored(error.with_artifact_metadata(&rebuilt_artifact))
                    }
                }
            }
        },
        RebuildAttempt::StaleBudgetExceeded => {
            RefreshOutcome::NotRefreshed(RebuildStatus::StaleBudgetExceeded)
        }
        RebuildAttempt::StaleRebuildFailed => {
            RefreshOutcome::NotRefreshed(RebuildStatus::StaleRebuildFailed)
        }
    }
}

async fn rebuilt_parquet_refresh_outcome(
    rebuilt_artifact: Arc<GraphIndexArtifact>,
    rebuild_started: Instant,
    args: &Value,
    response_format: ResponseFormat,
    handler: &(impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync),
) -> RefreshOutcome {
    let Some(remaining_budget) = remaining_graph_rebuild_budget(rebuild_started) else {
        return RefreshOutcome::NotRefreshed(RebuildStatus::StaleBudgetExceeded);
    };
    let mut task =
        tokio::task::spawn_blocking(move || RebuiltParquetGraph::from_artifact(&rebuilt_artifact));

    let rebuilt_graph = match tokio::time::timeout(remaining_budget, &mut task).await {
        Ok(Ok(Ok(rebuilt_graph))) => rebuilt_graph,
        Ok(Ok(Err(error))) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "rebuilt code graph Parquet bind failed; serving stale response"
            );
            return RefreshOutcome::NotRefreshed(RebuildStatus::StaleRebuildFailed);
        }
        Ok(Err(error)) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "rebuilt code graph Parquet bind task failed; serving stale response"
            );
            return RefreshOutcome::NotRefreshed(RebuildStatus::StaleRebuildFailed);
        }
        Err(_) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(_rebuilt_graph)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "rebuilt code graph Parquet bind failed after response budget elapsed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "rebuilt code graph Parquet bind task failed after response budget elapsed"
                        );
                    }
                }
            });
            return RefreshOutcome::NotRefreshed(RebuildStatus::StaleBudgetExceeded);
        }
    };

    let client = rebuilt_graph.client();
    let source = GraphMetadataSource::from_parquet_manifest(client.manifest());
    let response = match handler(args, client) {
        Ok(fresh_body) => response_file_set_from_client(client, &fresh_body)
            .map(|fresh_files| (fresh_body, fresh_files))
            .map_err(CodeGraphError::from),
        Err(error) => Err(error),
    };
    drop(rebuilt_graph);

    match response {
        Ok((mut fresh_body, fresh_files)) => {
            GraphResponseMetadata::analyze_source_inner(source, Some(&fresh_files), None)
                .await
                .metadata
                .with_rebuild_status(RebuildStatus::Fresh)
                .insert_into_for_format(&mut fresh_body, response_format);
            RefreshOutcome::Fresh(fresh_body)
        }
        Err(error) => {
            let metadata = GraphResponseMetadata::from_source(source)
                .await
                .with_rebuild_status(RebuildStatus::Fresh);
            RefreshOutcome::Errored(error.with_response_metadata(metadata))
        }
    }
}

async fn code_graph_backend_response_with_refresh(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult + Send + Sync,
    refresh_strategy: GraphRefreshStrategy,
    allow_source: bool,
    default_format: ResponseFormat,
) -> CodeGraphResult {
    let response_format = ResponseFormat::parse_or_inner(args, allow_source, default_format)
        .map_err(CodeGraphError::without_metadata)?;
    let backend = open_code_search_backend_for_request(Some(Arc::clone(&rebuild_coordinator)))
        .await
        .map_err(CodeGraphError::without_metadata)?;
    let source = backend.metadata_source();
    let request_client = RequestReplayClient::new(backend.client());

    // Auto owns a complete exact overlay observer. Enter it before the legacy
    // metadata preflight so one request does not discover the same Git state
    // through two independent paths. The static base may be stale; the
    // certified overlay generation is authoritative for this request.
    if refresh_strategy == GraphRefreshStrategy::OverlayThenRebuild && overlay_fsmonitor_auto {
        if let Some(worktree) = current_worktree_root() {
            match overlay_response_for_backend(
                &backend,
                &request_client,
                worktree,
                source.clone(),
                args,
                response_format,
                true,
                overlay_runtime_lifecycle_for(&rebuild_coordinator),
                &handler,
            )
            .await
            {
                Ok(OverlayAttempt::Fresh(fresh_body)) => return Ok(fresh_body),
                Ok(OverlayAttempt::Errored(error)) => return Err(error),
                Ok(OverlayAttempt::StaleBudgetExceeded) => {
                    // Preserve the existing stale-budget behavior. The legacy
                    // path below may serve the base only with actionable stale
                    // metadata; it cannot bless an uncertified overlay.
                }
                Err(error) => {
                    tracing::warn!(
                        target: "spur_graph::mcp",
                        error = ?error,
                        "direct code graph overlay observation failed; using legacy escalation"
                    );
                }
            }
        }
    }

    // Establish freshness before the first handler call. A dirty request then
    // executes all of its nested graph operations against one pinned
    // generation (or one exact fallback client), never against the base first.
    let indexed_files = backend.base_file_set().ok();
    let mut preflight =
        GraphResponseMetadata::analyze_source_inner(source.clone(), None, indexed_files.as_deref())
            .await;
    let mut refresh_error = None;
    if let Some(rebuild_candidate) = preflight.rebuild_candidate.take() {
        match attempt_refresh(
            &backend,
            &request_client,
            Arc::clone(&rebuild_coordinator),
            rebuild_candidate,
            source.clone(),
            args,
            response_format,
            refresh_strategy,
            overlay_fsmonitor_auto,
            &handler,
        )
        .await
        {
            RefreshOutcome::Fresh(fresh_body) => return Ok(fresh_body),
            RefreshOutcome::Errored(fresh_error) => refresh_error = Some(fresh_error),
            RefreshOutcome::NotRefreshed(status) => {
                preflight.metadata = preflight.metadata.with_rebuild_status(status);
            }
        }
    }

    let mut body = match handler(args, &request_client) {
        Ok(body) => body,
        Err(mut original_error) => {
            if let Some(fresh_error) = refresh_error {
                return Err(fresh_error);
            }
            if original_error.metadata.is_none() && original_error.temporal_code.is_none() {
                original_error.metadata = Some(Box::new(source.clone()));
            }
            return Err(original_error);
        }
    };
    let files = backend
        .response_file_set_from_body(&request_client, &body)
        .map_err(CodeGraphError::from)?;
    let mut analysis =
        GraphResponseMetadata::analyze_source_inner(source, Some(&files), indexed_files.as_deref())
            .await;
    if !matches!(preflight.metadata.rebuild_status, RebuildStatus::NotNeeded) {
        analysis.metadata = analysis
            .metadata
            .with_rebuild_status(preflight.metadata.rebuild_status);
    }
    analysis
        .metadata
        .insert_into_for_format(&mut body, response_format);
    Ok(body)
}

#[expect(
    clippy::too_many_arguments,
    reason = "overlay attempts thread backend, client, worktree, runtime, and handler context in one call"
)]
async fn overlay_response_for_backend(
    backend: &CodeSearchBackend,
    request_client: &(dyn GraphQueryClient + Sync),
    worktree: PathBuf,
    source: GraphMetadataSource,
    args: &Value,
    response_format: ResponseFormat,
    overlay_fsmonitor_auto: bool,
    runtime_lifecycle: Arc<OverlayRuntimeLifecycle>,
    handler: impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult,
) -> Result<OverlayAttempt, CodeGraphError> {
    let snapshot_base = backend.snapshot_base().map_err(|error| {
        CodeGraphError::without_metadata(McpHandlerError::Internal(format!(
            "failed to construct code graph overlay: {error}"
        )))
    })?;
    let full_base_source = backend.full_base_artifact_source();
    let mut runtime_fallback = if overlay_fsmonitor_auto {
        let key = OverlayRuntimeKey::new(
            worktree.clone(),
            snapshot_base.indexed_graph_content_hash.clone(),
        );
        let builder: Arc<dyn OverlayGenerationBuilder> = Arc::new(McpOverlayGenerationBuilder {
            worktree: worktree.clone(),
            snapshot_base: snapshot_base.clone(),
            full_base_source: full_base_source.clone(),
            #[cfg(test)]
            use_request_cache: true,
        });
        match published_generation_route(runtime_lifecycle, key, builder, || {
            backend.is_current_runtime_base(&worktree, &source)
        }) {
            PublishedGenerationRoute::Trusted(acquired) => {
                let generation = Arc::clone(acquired.published.generation());
                let generation_id =
                    opaque_published_generation_id(acquired.published.snapshot_identity());
                let pinned = PinnedGenerationClient::new(generation);
                let metadata = GraphResponseMetadata::from_published_generation(
                    source,
                    acquired.published.snapshot_identity(),
                );
                let mut fresh_body = match handler(args, &pinned) {
                    Ok(body) => body,
                    Err(error) => {
                        return Ok(OverlayAttempt::Errored(
                            error.with_response_metadata(metadata),
                        ));
                    }
                };
                metadata.insert_into_for_format(&mut fresh_body, response_format);
                insert_overlay_generation_diagnostics(
                    &mut fresh_body,
                    published_generation_diagnostics_value(
                        acquired.published.provider(),
                        acquired.published.epoch(),
                        generation_id,
                        pinned.query_operations(),
                    ),
                );
                return Ok(OverlayAttempt::Fresh(fresh_body));
            }
            PublishedGenerationRoute::Exact(fallback) => Some(fallback),
        }
    } else {
        None
    };
    let budget = graph_rebuild_latency_budget();
    let prepare_worktree = worktree.clone();
    let prepare_base = snapshot_base.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        if overlay_fsmonitor_auto {
            prepare_overlay_for_worktree(prepare_worktree, prepare_base, true)
        } else {
            prepare_overlay_for_worktree(prepare_worktree, prepare_base, false)
        }
    });
    let prepared = match tokio::time::timeout(budget, &mut task).await {
        Ok(Ok(Ok(prepared))) => prepared,
        Ok(Ok(Err(error))) => {
            if let Some(fallback) = runtime_fallback.take() {
                fallback.schedule_seed_or_restart();
            }
            return Err(CodeGraphError::without_metadata(McpHandlerError::Internal(
                format!("failed to construct code graph overlay: {error}"),
            )));
        }
        Ok(Err(error)) => {
            if let Some(fallback) = runtime_fallback.take() {
                fallback.schedule_seed_or_restart();
            }
            return Err(CodeGraphError::without_metadata(McpHandlerError::Internal(
                format!("overlay extract task failed: {error}"),
            )));
        }
        Err(_) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "code graph overlay extract failed after response budget elapsed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "code graph overlay extract task failed after response budget elapsed"
                        );
                    }
                }
            });
            if let Some(fallback) = runtime_fallback.take() {
                fallback.schedule_seed_or_restart();
            }
            return Ok(OverlayAttempt::StaleBudgetExceeded);
        }
    };

    if let Some(fallback) = runtime_fallback.take() {
        let cached = prepared.and_then(|prepared| prepared.cached);
        let exact_response = match exact_overlay_response(request_client, cached, args, &handler) {
            Ok(response) => response,
            Err(error) => {
                fallback.schedule_seed_or_restart();
                return Err(error);
            }
        };
        return match exact_response {
            ExactOverlayResponse::Fresh {
                mut body,
                files,
                measurements,
            } => {
                let diagnostics = runtime_exact_fallback_diagnostics_value(&fallback, measurements);
                // Seed cold/warming runtimes and replace terminal untrusted
                // runtimes before any post-query metadata await can cancel.
                fallback.schedule_seed_or_restart();
                GraphResponseMetadata::analyze_source_inner(source, Some(&files), None)
                    .await
                    .metadata
                    .with_rebuild_status(RebuildStatus::Fresh)
                    .insert_into_for_format(&mut body, response_format);
                insert_overlay_generation_diagnostics(&mut body, diagnostics);
                Ok(OverlayAttempt::Fresh(body))
            }
            ExactOverlayResponse::Errored(error) => {
                fallback.schedule_seed_or_restart();
                let metadata = GraphResponseMetadata::from_source(source)
                    .await
                    .with_rebuild_status(RebuildStatus::Fresh);
                Ok(OverlayAttempt::Errored(
                    error.with_response_metadata(metadata),
                ))
            }
        };
    }

    let Some(prepared) = prepared else {
        // No changed paths → identity overlay. Serve the base client directly
        // and preserve the pre-generation response shape.
        let mut fresh_body = handler(args, request_client)?;
        let fresh_files = response_file_set_from_client(request_client, &fresh_body)?;
        GraphResponseMetadata::analyze_source_inner(source, Some(&fresh_files), None)
            .await
            .metadata
            .with_rebuild_status(RebuildStatus::Fresh)
            .insert_into_for_format(&mut fresh_body, response_format);
        return Ok(OverlayAttempt::Fresh(fresh_body));
    };

    debug_assert!(!overlay_fsmonitor_auto);
    match exact_overlay_response(request_client, prepared.cached, args, &handler)? {
        ExactOverlayResponse::Fresh {
            mut body, files, ..
        } => {
            GraphResponseMetadata::analyze_source_inner(source, Some(&files), None)
                .await
                .metadata
                .with_rebuild_status(RebuildStatus::Fresh)
                .insert_into_for_format(&mut body, response_format);
            Ok(OverlayAttempt::Fresh(body))
        }
        ExactOverlayResponse::Errored(error) => Err(error),
    }
}

fn exact_overlay_response(
    request_client: &(dyn GraphQueryClient + Sync),
    cached: Option<request_cache::CachedOverlayDelta>,
    args: &Value,
    handler: &impl Fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult,
) -> Result<ExactOverlayResponse, CodeGraphError> {
    match cached {
        Some(cached) => {
            let overlay =
                OverlayClient::from_artifacts(request_client, cached.artifact, cached.shadowed)
                    .map_err(|_| {
                        CodeGraphError::without_metadata(McpHandlerError::Internal(
                            "failed to construct exact overlay fallback".to_string(),
                        ))
                    })?;
            let measured = MeasuredOverlayClient::new(overlay);
            let body = match handler(args, &measured) {
                Ok(body) => body,
                Err(error) => return Ok(ExactOverlayResponse::Errored(error)),
            };
            let files = response_file_set_from_client(&measured, &body)?;
            Ok(ExactOverlayResponse::Fresh {
                body,
                files,
                measurements: measured.measurements(),
            })
        }
        None => {
            let body = match handler(args, request_client) {
                Ok(body) => body,
                Err(error) => return Ok(ExactOverlayResponse::Errored(error)),
            };
            let files = response_file_set_from_client(request_client, &body)?;
            Ok(ExactOverlayResponse::Fresh {
                body,
                files,
                measurements: OverlayFinalizationMeasurements::default(),
            })
        }
    }
}

fn overlay_generation_identity(
    identity: &overlay_snapshot::SnapshotIdentity,
) -> OverlayGenerationIdentity {
    OverlayGenerationIdentity {
        canonical_worktree: identity.canonical_worktree.clone(),
        indexed_graph_content_hash: identity.indexed_graph_content_hash.clone(),
        indexed_head_oid: identity.indexed_head_oid.clone(),
        current_head_oid: identity.current_head_oid.clone(),
        index_identity: blake3::hash(format!("{:?}", identity.index_identity).as_bytes())
            .to_hex()
            .to_string(),
        normalized_changed_set_fingerprint: identity.normalized_changed_set_fingerprint,
    }
}

fn generation_path_state(
    path_state: &BTreeMap<String, overlay_snapshot::OverlayPathState>,
) -> BTreeMap<String, GenerationPathState> {
    path_state
        .iter()
        .map(|(path, state)| {
            let state = match state {
                overlay_snapshot::OverlayPathState::Tracked(oid) => {
                    GenerationPathState::Tracked(oid.clone())
                }
                overlay_snapshot::OverlayPathState::Untracked(oid) => {
                    GenerationPathState::Untracked(oid.clone())
                }
                overlay_snapshot::OverlayPathState::Deleted => GenerationPathState::Deleted,
            };
            (path.clone(), state)
        })
        .collect()
}

fn opaque_published_generation_id(identity: &OverlayGenerationIdentity) -> String {
    let digest = blake3::hash(format!("{identity:?}").as_bytes())
        .to_hex()
        .to_string();
    format!("gen_{}", &digest[..16])
}

fn finalization_stages(measurements: OverlayFinalizationMeasurements) -> Value {
    json!({
        "shadow_filters": measurements.shadow_filters,
        "result_merges": measurements.result_merges,
        "overlay_sorts": measurements.overlay_sorts,
        "stable_id_deduplications": measurements.stable_id_deduplications,
        "total": measurements.total(),
    })
}

fn published_generation_diagnostics_value(
    provider: ChangeProviderKind,
    epoch: u64,
    generation_id: String,
    query_operations: u64,
) -> Value {
    json!({
        "route": "generation",
        "cache": "reused",
        "provider": provider_diagnostic(provider),
        "epoch": epoch,
        "trust": "trusted",
        "generation_id": generation_id,
        "generation_pins": 1,
        "fallback_reason": Value::Null,
        "startup_status": "ready",
        "startup_cause": Value::Null,
        "full_base_artifact_builds": 0,
        "query_operations": query_operations,
        "validation_observations": 0,
        "response_metadata_scans": 0,
        "response_retry": false,
        "generation_identity_mismatches": 0,
        "finalization_stages": finalization_stages(OverlayFinalizationMeasurements::default()),
    })
}

fn runtime_exact_fallback_diagnostics_value(
    fallback: &RuntimeExactFallback,
    measurements: OverlayFinalizationMeasurements,
) -> Value {
    json!({
        "route": "exact_fallback",
        "cache": "not_applicable",
        "provider": fallback.provider(),
        "epoch": fallback.epoch(),
        "trust": fallback.trust(),
        "generation_id": Value::Null,
        "generation_pins": 0,
        "fallback_reason": fallback.reason,
        "startup_status": fallback.startup_status,
        "startup_cause": fallback.startup_cause,
        "full_base_artifact_builds": 0,
        "validation_observations": 2,
        "response_metadata_scans": 1,
        "response_retry": false,
        "generation_identity_mismatches": 0,
        "finalization_stages": finalization_stages(measurements),
    })
}

fn insert_overlay_generation_diagnostics(body: &mut Value, diagnostics: Value) {
    if let Value::Object(map) = body {
        map.insert("overlay_generation".to_string(), diagnostics);
    }
}

#[allow(dead_code)]
fn overlay_delta_for_worktree(
    worktree: PathBuf,
    base: overlay_snapshot::SnapshotBase,
    overlay_fsmonitor_auto: bool,
) -> anyhow::Result<Option<request_cache::CachedOverlayDelta>> {
    Ok(
        prepare_overlay_for_worktree(worktree, base, overlay_fsmonitor_auto)?
            .and_then(|prepared| prepared.cached),
    )
}

struct PreparedOverlay {
    cached: Option<request_cache::CachedOverlayDelta>,
}

fn prepare_overlay_for_worktree(
    worktree: PathBuf,
    base: overlay_snapshot::SnapshotBase,
    overlay_fsmonitor_auto: bool,
) -> anyhow::Result<Option<PreparedOverlay>> {
    let mut changed = changed_paths_for_overlay_base(&worktree, base, overlay_fsmonitor_auto)?;
    if overlay_fsmonitor_auto {
        changed.identity = changed.identity.map(canonical_overlay_identity);
    }
    let paths = &changed.paths;
    if paths.is_empty() {
        return Ok(None);
    }
    let build = || {
        let (artifact, shadowed) =
            OverlayClient::<&dyn GraphQueryClient>::extract_delta(&worktree, paths)?;
        Ok(request_cache::CachedOverlayDelta { artifact, shadowed })
    };
    let cached = match changed.identity.clone() {
        Some(identity) => request_cache::overlay_delta(identity, build),
        None => build(),
    }?;
    Ok(Some(PreparedOverlay {
        cached: Some(cached),
    }))
}

#[cfg(test)]
fn overlay_client_for_backend<'a>(
    backend: &'a CodeSearchBackend,
    rebuild_candidate: &RebuildCandidate,
) -> anyhow::Result<Option<OverlayClient<&'a (dyn GraphQueryClient + Sync)>>> {
    let worktree = rebuild_candidate.worktree.clone();
    let snapshot_base = backend.snapshot_base()?;
    match overlay_delta_for_worktree(worktree, snapshot_base, false)? {
        Some(cached) => Ok(Some(OverlayClient::from_artifacts(
            backend.client(),
            cached.artifact,
            cached.shadowed,
        )?)),
        None => Ok(None),
    }
}

struct CodeSearchBody {
    body: Value,
    result: SearchResult,
    options: SearchOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseFormat {
    Full,
    Compact,
    Table,
    Source,
}

impl ResponseFormat {
    fn parse(args: &Value) -> Result<Self, McpHandlerError> {
        Self::parse_or_inner(args, false, Self::Full)
    }

    fn parse_allowing_source(args: &Value) -> Result<Self, McpHandlerError> {
        Self::parse_or_inner(args, true, Self::Full)
    }

    fn parse_or_inner(
        args: &Value,
        allow_source: bool,
        default: Self,
    ) -> Result<Self, McpHandlerError> {
        let Some(value) = args.get("response_format") else {
            return Ok(default);
        };
        let expected = if allow_source {
            "`full`, `compact`, `table`, or `source`"
        } else {
            "`full`, `compact`, or `table`"
        };
        match value.as_str() {
            Some("full") => Ok(Self::Full),
            Some("compact") => Ok(Self::Compact),
            Some("table") => Ok(Self::Table),
            Some("source") if allow_source => Ok(Self::Source),
            Some(other) => Err(McpHandlerError::InvalidParams(format!(
                "invalid response_format `{other}`; expected {expected}"
            ))),
            None => Err(McpHandlerError::InvalidParams(format!(
                "field 'response_format' must be a string; expected {expected}"
            ))),
        }
    }
}

enum CodeSearchBackend {
    Parquet(Arc<ParquetClient>),
    InMemory {
        artifact: Arc<GraphIndexArtifact>,
        client: InMemoryClient,
    },
}

impl CodeSearchBackend {
    fn client(&self) -> &(dyn GraphQueryClient + Sync) {
        match self {
            Self::Parquet(client) => client.as_ref(),
            Self::InMemory { client, .. } => client,
        }
    }

    fn metadata_source(&self) -> GraphMetadataSource {
        match self {
            Self::Parquet(client) => {
                GraphMetadataSource::from_parquet_manifest(client.as_ref().manifest())
            }
            Self::InMemory { artifact, .. } => GraphMetadataSource::from_artifact(artifact),
        }
    }

    fn base_file_set(&self) -> anyhow::Result<Vec<(String, String)>> {
        match self {
            Self::Parquet(client) => client.file_oids(),
            Self::InMemory { artifact, .. } => Ok(all_indexed_file_set(artifact)),
        }
    }

    fn snapshot_base(&self) -> anyhow::Result<overlay_snapshot::SnapshotBase> {
        let file_oids = self.base_file_set()?.into_iter().collect();
        Ok(match self {
            Self::Parquet(client) => overlay_snapshot::SnapshotBase {
                indexed_graph_content_hash: client.manifest().graph_content_hash.clone(),
                indexed_head_oid: client.manifest().indexed_commit_oid.clone(),
                file_oids,
            },
            Self::InMemory { artifact, .. } => overlay_snapshot::SnapshotBase {
                indexed_graph_content_hash: artifact.graph_content_hash.clone(),
                indexed_head_oid: None,
                file_oids,
            },
        })
    }

    fn full_base_artifact_source(&self) -> FullBaseArtifactSource {
        match self {
            Self::Parquet(client) => FullBaseArtifactSource::Parquet(client.dir().to_path_buf()),
            Self::InMemory { artifact, .. } => {
                FullBaseArtifactSource::InMemory(Arc::clone(artifact))
            }
        }
    }

    fn is_current_runtime_base(&self, worktree: &Path, source: &GraphMetadataSource) -> bool {
        match self {
            // Re-resolve the persisted base without consulting Git so a
            // request opened before a concurrent reindex cannot reactivate
            // the superseded actor. Checking both canonical path and content
            // identity also covers artifact directories updated in place.
            Self::Parquet(client) => {
                resolve_artifact_location(worktree, None).is_ok_and(|current| {
                    current.path == client.dir()
                        && current.cache_key.graph_content_hash == source.graph_content_hash
                })
            }
            // An in-memory base seed has no durable ordering token. Continue
            // to serve it through the exact path rather than guessing which
            // seed is newer and retaining the wrong watcher.
            Self::InMemory { .. } => false,
        }
    }

    fn search_response_file_set(
        &self,
        search: &CodeSearchBody,
    ) -> Result<Vec<(String, String)>, McpHandlerError> {
        match self {
            Self::Parquet(client) => search_response_file_set_for_parquet(
                client.as_ref(),
                &search.result,
                &search.options,
            ),
            Self::InMemory { artifact, .. } => {
                Ok(empty_code_search_file_set(artifact, &search.body)
                    .unwrap_or_else(|| response_file_set_from_body(artifact, &search.body)))
            }
        }
    }

    fn response_file_set_from_body(
        &self,
        request_client: &dyn GraphQueryClient,
        body: &Value,
    ) -> Result<Vec<(String, String)>, McpHandlerError> {
        match self {
            Self::Parquet(client) => {
                let file_oids = client.file_oids().map_err(|error| {
                    McpHandlerError::Internal(format!(
                        "failed to read graph file manifests from `{}`: {error}",
                        client.dir().display()
                    ))
                })?;
                let mut paths = Vec::new();
                collect_response_file_paths(body, &mut paths);
                let mut files = file_oid_subset(&file_oids, paths);
                if files.is_empty() {
                    if let Some(symbol_id) = response_symbol_id(body) {
                        if let Some(symbol) = request_client
                            .symbol_by_id(symbol_id)
                            .map_err(graph_query_error)?
                        {
                            files = file_oid_subset(&file_oids, [symbol.file_path.as_str()]);
                        }
                    }
                }
                Ok(files)
            }
            Self::InMemory { artifact, .. } => Ok(response_file_set_from_body(artifact, body)),
        }
    }
}

fn code_search_body_for_client(
    args: &Value,
    client: &dyn GraphQueryClient,
) -> Result<CodeSearchBody, McpHandlerError> {
    let request = code_search_options(args)?;
    let options = request.options;
    let result = client.search_symbols(&options).map_err(|error| {
        McpHandlerError::Internal(format!("failed to search graph artifact: {error}"))
    })?;
    let candidates = result
        .candidates
        .iter()
        .map(candidate_row_for_search_symbol)
        .map(candidate_row)
        .collect::<Vec<_>>();

    let mut body = json!({
        "query": options.query.clone(),
        "mode": search_mode_str(options.mode),
        "symbol_kind": options.filters.symbol_kind.clone(),
        "file": options.filters.file.clone(),
        "file_glob": options.filters.file_glob.clone(),
        "limit": options.limit,
        "total_matches": result.total_matches,
        "truncated": result.truncated,
        "candidates": candidates,
    });
    if let Some(requested_limit) = request.requested_limit {
        body["requested_limit"] = requested_limit;
    }
    Ok(CodeSearchBody {
        body,
        result,
        options,
    })
}

async fn code_resolve_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_resolve_with_client,
    )
    .await
}

fn code_resolve_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let selector = selector_arg(args)?;
    let Some(as_of) = parse_as_of(args)? else {
        let candidates = resolve_candidate_rows_for_client(client, selector)?
            .into_iter()
            .map(candidate_row)
            .collect::<Vec<_>>();

        return Ok(json!({ "candidates": candidates }));
    };

    let resolved = match client
        .resolve_selector(selector)
        .map_err(graph_query_error)?
    {
        SelectorResolution::Resolved(resolved) => resolved,
        SelectorResolution::Ambiguous { candidates } => {
            let candidates = candidates
                .into_iter()
                .map(candidate_row)
                .collect::<Vec<_>>();
            return Ok(json!({ "candidates": candidates }));
        }
        SelectorResolution::NotFound => {
            return Err(McpHandlerError::NotFound(format!(
                "symbol {} not found in graph artifact",
                missing_symbol_label(selector)
            ))
            .into());
        }
    };

    let worktree = current_worktree()?;
    let commits = load_commit_index_for_request(&worktree)?;
    let temporal_index = client.temporal_index();
    let resolution = resolve_symbol_as_of(
        temporal_index.artifact(),
        Some(Arc::clone(&temporal_index)),
        &commits,
        &resolved.stable_symbol_id,
        &as_of,
    )?;
    code_resolve_temporal_response_with_client(
        client,
        &resolved.stable_symbol_id,
        &as_of,
        resolution,
    )
}

#[allow(dead_code)]
fn code_resolve_with_artifact(args: &Value, artifact: &GraphIndexArtifact) -> CodeGraphResult {
    let selector = selector_arg(args)?;
    let Some(as_of) = parse_as_of(args)? else {
        let candidates = resolve_candidate_rows(artifact, selector)?
            .into_iter()
            .map(candidate_row)
            .collect::<Vec<_>>();

        return Ok(json!({ "candidates": candidates }));
    };

    let resolved = match resolve_selector(artifact, selector) {
        SelectorResolution::Resolved(resolved) => resolved,
        SelectorResolution::Ambiguous { candidates } => {
            let candidates = candidates
                .into_iter()
                .map(candidate_row)
                .collect::<Vec<_>>();
            return Ok(json!({ "candidates": candidates }));
        }
        SelectorResolution::NotFound => {
            return Err(McpHandlerError::NotFound(format!(
                "symbol {} not found in graph artifact",
                missing_symbol_label(selector)
            ))
            .into());
        }
    };

    let worktree = current_worktree()?;
    let commits = load_commit_index_for_request(&worktree)?;
    let resolution =
        resolve_symbol_as_of(artifact, None, &commits, &resolved.stable_symbol_id, &as_of)?;
    code_resolve_temporal_response(artifact, &resolved.stable_symbol_id, &as_of, resolution)
}

#[allow(dead_code)]
fn code_resolve_with_loaded_artifact(
    args: &Value,
    loaded: &LoadedGraphArtifact,
) -> CodeGraphResult {
    let artifact = loaded.artifact();
    let selector = selector_arg(args)?;
    let Some(as_of) = parse_as_of(args)? else {
        let candidates = resolve_candidate_rows(artifact, selector)?
            .into_iter()
            .map(candidate_row)
            .collect::<Vec<_>>();

        return Ok(json!({ "candidates": candidates }));
    };

    let resolved = match resolve_selector(artifact, selector) {
        SelectorResolution::Resolved(resolved) => resolved,
        SelectorResolution::Ambiguous { candidates } => {
            let candidates = candidates
                .into_iter()
                .map(candidate_row)
                .collect::<Vec<_>>();
            return Ok(json!({ "candidates": candidates }));
        }
        SelectorResolution::NotFound => {
            return Err(McpHandlerError::NotFound(format!(
                "symbol {} not found in graph artifact",
                missing_symbol_label(selector)
            ))
            .into());
        }
    };

    let worktree = current_worktree()?;
    let commits = load_commit_index_for_request(&worktree)?;
    let temporal_index = loaded.temporal_index();
    let resolution = resolve_symbol_as_of(
        artifact,
        Some(temporal_index),
        &commits,
        &resolved.stable_symbol_id,
        &as_of,
    )?;
    code_resolve_temporal_response(artifact, &resolved.stable_symbol_id, &as_of, resolution)
}

#[allow(dead_code)]
fn code_resolve_temporal_response(
    artifact: &GraphIndexArtifact,
    requested_symbol_id: &str,
    as_of: &str,
    resolution: Resolution<String>,
) -> CodeGraphResult {
    match resolution {
        Resolution::Found { value, chain } => {
            let symbol = symbol_by_id(artifact, &value)?;
            let kind = if value == requested_symbol_id {
                "found"
            } else {
                "renamed"
            };
            Ok(json!({
                "candidates": [candidate_row(candidate_row_for_symbol(symbol))],
                "resolution": {
                    "kind": kind,
                    "as_of": as_of,
                    "symbol": symbol_uri(&value),
                    "chain": chain,
                },
            }))
        }
        Resolution::Deleted { last_seen } => Ok(json!({
            "candidates": [],
            "resolution": {
                "kind": "deleted",
                "as_of": as_of,
                "last_seen": last_seen,
            },
        })),
        Resolution::Ambiguous { candidates } => {
            let candidate_rows = candidates
                .iter()
                .map(|candidate| symbol_by_id(artifact, candidate).map(candidate_row_for_symbol))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(candidate_row)
                .collect::<Vec<_>>();
            Ok(json!({
                "candidates": candidate_rows,
                "resolution": {
                    "kind": "ambiguous",
                    "as_of": as_of,
                    "candidates": candidates,
                },
            }))
        }
        Resolution::Unknown { reason } => {
            Err(unknown_resolution_error(requested_symbol_id, as_of, reason))
        }
    }
}

fn code_resolve_temporal_response_with_client(
    client: &dyn GraphQueryClient,
    requested_symbol_id: &str,
    as_of: &str,
    resolution: Resolution<String>,
) -> CodeGraphResult {
    match resolution {
        Resolution::Found { value, chain } => {
            let symbol = symbol_by_id_for_client(client, &value)?;
            let kind = if value == requested_symbol_id {
                "found"
            } else {
                "renamed"
            };
            Ok(json!({
                "candidates": [candidate_row(candidate_row_for_symbol(&symbol))],
                "resolution": {
                    "kind": kind,
                    "as_of": as_of,
                    "symbol": symbol_uri(&value),
                    "chain": chain,
                },
            }))
        }
        Resolution::Deleted { last_seen } => Ok(json!({
            "candidates": [],
            "resolution": {
                "kind": "deleted",
                "as_of": as_of,
                "last_seen": last_seen,
            },
        })),
        Resolution::Ambiguous { candidates } => {
            let candidate_rows = candidates
                .iter()
                .map(|candidate| {
                    symbol_by_id_for_client(client, candidate)
                        .map(|symbol| candidate_row_for_symbol(&symbol))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(candidate_row)
                .collect::<Vec<_>>();
            Ok(json!({
                "candidates": candidate_rows,
                "resolution": {
                    "kind": "ambiguous",
                    "as_of": as_of,
                    "candidates": candidates,
                },
            }))
        }
        Resolution::Unknown { reason } => {
            Err(unknown_resolution_error(requested_symbol_id, as_of, reason))
        }
    }
}

pub async fn code_file_symbols(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_file_symbols_with_client).await
}

async fn code_file_symbols_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_file_symbols_with_client,
    )
    .await
}

fn code_file_symbols_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args)?;
    let file = file_arg(args)?;
    let file = validate_file_path_arg(file)?;
    if !client.file_exists(&file).map_err(graph_query_error)? {
        return Err(McpHandlerError::NotFound(format!(
            "file `{file}` not found in graph artifact"
        ))
        .into());
    }

    let file_symbols = client.symbols_by_file(&file).map_err(graph_query_error)?;
    let candidates = candidate_rows_for_symbols(file_symbols.iter());
    if response_format == ResponseFormat::Table {
        let mut files = TableFileInterner::default();
        let symbols = candidate_table(candidates, &mut files);
        return Ok(table_response(json!({ "symbols": symbols }), files));
    }

    let symbols = candidates
        .into_iter()
        .map(candidate_row)
        .collect::<Vec<_>>();
    Ok(json!({ "symbols": symbols }))
}

pub async fn code_symbol_info(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_symbol_info_with_client).await
}

pub async fn code_symbol_info_rebuild_aware(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_response_rebuild_only(
        args,
        shared_rebuild_coordinator(),
        code_symbol_info_with_client,
    )
    .await
    .map_err(CodeGraphError::into_handler_error)
}

async fn code_symbol_info_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_symbol_info_with_client,
    )
    .await
}

fn code_symbol_info_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let symbol_id = match resolve_code_selector_with_client(args, client)? {
        CodeSelectorResolution::Resolved(symbol_id) => symbol_id,
        CodeSelectorResolution::Ambiguous(candidates) => {
            return Ok(ambiguous_response(candidates));
        }
    };
    let symbol = symbol_by_id_for_client(client, &symbol_id)?;

    Ok(json!({ "symbol": symbol_info_row(&symbol) }))
}

pub async fn code_read_symbol(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value_allowing_source(args, code_read_symbol_with_client).await
}

async fn code_read_symbol_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response_allowing_source(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_read_symbol_with_client,
    )
    .await
}

fn code_read_symbol_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let response_format = ResponseFormat::parse_allowing_source(args)?;
    let symbol = match code_read_symbol_target(args, client)? {
        CodeReadSymbolTarget::Resolved(symbol) => symbol,
        CodeReadSymbolTarget::Ambiguous(candidates) => {
            return Ok(match response_format {
                ResponseFormat::Source => source_ambiguous_response(candidates),
                _ => ambiguous_response(candidates),
            });
        }
    };
    if is_external_symbol_kind(&symbol.symbol_kind) {
        return Ok(bodyless_external_symbol_response(&symbol));
    }
    let context_lines = clamped_usize_arg(
        args,
        "context_lines",
        0,
        0,
        MAX_MCP_CODE_READ_SYMBOL_CONTEXT_LINES,
    )?;
    let manifest = client
        .file_manifest_by_path(&symbol.file_path)
        .map_err(graph_query_error)?
        .ok_or_else(|| {
            McpHandlerError::Internal(format!(
                "graph artifact has no file manifest for `{}`",
                symbol.file_path
            ))
        })?;
    let worktree = current_worktree_root().ok_or_else(|| {
        McpHandlerError::Internal("failed to resolve current worktree root".into())
    })?;
    let file_oid = manifest.content_oid.clone();
    let current_oid = current_file_oid(&worktree, &symbol.file_path)?;
    let stale = current_oid.as_deref() != Some(file_oid.as_str());

    // On a stale file, prefer the current worktree source via a single-file
    // overlay; fall back to the indexed blob when the symbol no longer
    // exists in the edited file (or the overlay cannot be built).
    let current_snapshot = if stale {
        stale_read_current_snapshot(client, &worktree, &symbol)
    } else {
        None
    };
    let source_origin = current_snapshot.as_ref().map(|_| "worktree");
    let (symbol, source_text, served_oid) = match current_snapshot {
        Some((fresh_symbol, current_text, current_oid)) => {
            (fresh_symbol, current_text, current_oid)
        }
        None => {
            let indexed_bytes = read_indexed_file_bytes(&worktree, &symbol.file_path, &file_oid)?;
            let indexed_source = String::from_utf8(indexed_bytes).map_err(|error| {
                McpHandlerError::Internal(format!(
                    "indexed blob `{}` for `{}` is not UTF-8: {error}",
                    file_oid, symbol.file_path
                ))
            })?;
            (symbol, indexed_source, file_oid)
        }
    };
    let source_text = crate::extract::notebook::decoded_source_document(&source_text, &symbol)
        .unwrap_or(source_text);
    let source_range = source_range_with_context(&source_text, &symbol, context_lines.value);
    let source = source_for_line_range(&source_text, source_range);

    if response_format == ResponseFormat::Source {
        return Ok(source_symbol_response(
            &symbol,
            source,
            source_range,
            served_oid,
            &context_lines,
            stale,
            source_origin,
        ));
    }

    let mut body = json!({
        "symbol": symbol_info_row(&symbol),
        "source": source,
        "line_range": {
            "start": source_range[0],
            "end": source_range[1],
        },
        "file_oid": served_oid,
        "context_lines": context_lines.value,
    });
    if let Some(requested_context_lines) = context_lines.requested_value {
        body["requested_context_lines"] = requested_context_lines;
    }
    if stale {
        body["stale"] = Value::Bool(true);
    }
    if let Some(origin) = source_origin {
        body["source_origin"] = Value::String(origin.to_owned());
    }
    Ok(body)
}

/// Best-effort current view of a stale symbol: parse just the symbol's file
/// into a single-file overlay over `client` and return the surviving symbol
/// with the current file text and blob oid. `None` falls back to the indexed
/// source.
fn stale_read_current_snapshot(
    client: &dyn GraphQueryClient,
    worktree: &Path,
    symbol: &GraphSymbolArtifact,
) -> Option<(GraphSymbolArtifact, String, String)> {
    let changed = [PathBuf::from(&symbol.file_path)];
    let overlay = match OverlayClient::new(client, worktree, &changed) {
        Ok(overlay) => overlay,
        Err(error) => {
            tracing::debug!(
                target: "spur_graph::mcp",
                error = %error,
                file = %symbol.file_path,
                "single-file overlay for stale read failed; serving indexed source"
            );
            return None;
        }
    };
    let fresh_symbol = overlay
        .current_symbol_for(&symbol.stable_symbol_id)
        .ok()
        .flatten()?;
    let bytes = read_current_file_bytes(worktree, &fresh_symbol.file_path).ok()?;
    let current_oid = git_blob_oid(&bytes);
    let current_text = String::from_utf8(bytes).ok()?;
    Some((fresh_symbol, current_text, current_oid))
}

pub async fn code_callers(args: &Value) -> Result<Value, McpHandlerError> {
    selected_code_selector(args)?;
    code_graph_backend_value(args, code_callers_with_client).await
}

async fn code_callers_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_callers_with_client,
    )
    .await
}

fn code_callers_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args)?;
    let request = code_traversal_request(args)?;
    let symbol_id = match resolve_code_selector_with_client(args, client)? {
        CodeSelectorResolution::Resolved(symbol_id) => symbol_id,
        CodeSelectorResolution::Ambiguous(candidates) => {
            return Ok(ambiguous_response(candidates));
        }
    };
    let symbol_id =
        resolve_symbol_for_optional_as_of_current_worktree_with_client(client, &symbol_id, args)?;

    let records = client.find_caller_edges(&symbol_id);
    let summary = owned_caller_summary(&records);
    let callers = records
        .into_iter()
        .filter(|record| request.include_unresolved || record.is_resolved())
        .collect::<Vec<_>>();
    if response_format == ResponseFormat::Table {
        let mut files = TableFileInterner::default();
        let callers = owned_caller_table(callers, &mut files);
        return Ok(table_response(
            json!({
                "callers": callers,
                "include_unresolved": request.include_unresolved,
                "counts_by_kind": summary.counts_by_kind,
                "counts_by_context": summary.counts_by_context,
                "unresolved_sample": summary.unresolved_sample,
            }),
            files,
        ));
    }
    let callers = callers
        .into_iter()
        .map(owned_caller_row)
        .collect::<Vec<_>>();
    Ok(json!({
        "callers": callers,
        "include_unresolved": request.include_unresolved,
        "counts_by_kind": summary.counts_by_kind,
        "counts_by_context": summary.counts_by_context,
        "unresolved_sample": summary.unresolved_sample,
    }))
}

pub async fn code_callees(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_callees_with_client).await
}

async fn code_callees_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_callees_with_client,
    )
    .await
}

fn code_callees_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args)?;
    let request = code_traversal_request(args)?;
    let symbol_id = match resolve_code_selector_with_client(args, client)? {
        CodeSelectorResolution::Resolved(symbol_id) => symbol_id,
        CodeSelectorResolution::Ambiguous(candidates) => {
            return Ok(ambiguous_response(candidates));
        }
    };
    let symbol_id =
        resolve_symbol_for_optional_as_of_current_worktree_with_client(client, &symbol_id, args)?;

    let records = client.find_callee_edges(&symbol_id);
    let summary = owned_callee_summary(&records);
    let callees = records
        .into_iter()
        .filter(|record| request.include_unresolved || record.is_resolved())
        .collect::<Vec<_>>();
    if response_format == ResponseFormat::Table {
        let mut files = TableFileInterner::default();
        let callees = owned_callee_table(callees, &mut files);
        return Ok(table_response(
            json!({
                "callees": callees,
                "include_unresolved": request.include_unresolved,
                "counts_by_kind": summary.counts_by_kind,
                "counts_by_context": summary.counts_by_context,
                "unresolved_sample": summary.unresolved_sample,
            }),
            files,
        ));
    }
    let callees = callees
        .into_iter()
        .map(owned_callee_row)
        .collect::<Vec<_>>();
    Ok(json!({
        "callees": callees,
        "include_unresolved": request.include_unresolved,
        "counts_by_kind": summary.counts_by_kind,
        "counts_by_context": summary.counts_by_context,
        "unresolved_sample": summary.unresolved_sample,
    }))
}

pub async fn code_subgraph(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_subgraph_with_client).await
}

async fn code_subgraph_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_subgraph_with_client,
    )
    .await
}

fn code_subgraph_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args)?;
    let request = code_traversal_request(args)?;
    let root_ids = match code_subgraph_root_ids_with_client(args, client)? {
        CodeSubgraphRoots::RootIds(root_ids) => root_ids,
        CodeSubgraphRoots::Ambiguous(candidates) => {
            return Ok(ambiguous_response(candidates));
        }
    };

    let requested_radius = args
        .get("radius")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let radius = requested_radius.min(u64::from(MAX_MCP_CODE_SUBGRAPH_RADIUS)) as u8;
    let warning = (requested_radius > u64::from(MAX_MCP_CODE_SUBGRAPH_RADIUS)).then(|| {
        format!(
            "radius {requested_radius} exceeds max {MAX_MCP_CODE_SUBGRAPH_RADIUS}; clamped to {MAX_MCP_CODE_SUBGRAPH_RADIUS}"
        )
    });
    let format = args
        .get("format")
        .and_then(|value| value.as_str())
        .unwrap_or("json");
    let edge_kinds = parse_edge_kinds(args)?;
    let edge_filter = edge_kinds.as_deref();
    let budget = code_subgraph_budget(args)?;
    let root_refs = root_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let view = client_bounded_subgraph_with_budget(
        client,
        &root_refs,
        radius,
        edge_filter,
        request.include_unresolved,
        budget.budget,
    )?;

    match format {
        "json" => {
            let mut metadata = code_subgraph_metadata(radius, view.truncated, &budget);
            if let Some(warning) = warning {
                metadata["warning"] = Value::String(warning);
            }
            if response_format == ResponseFormat::Table {
                let mut files = TableFileInterner::default();
                let nodes = symbol_table(view.nodes.iter(), &mut files);
                let edges = edge_table(view.edges.iter());
                return Ok(table_response(
                    json!({
                        "nodes": nodes,
                        "edges": edges,
                        "truncated_frontier": view.truncated_frontier,
                        "include_unresolved": request.include_unresolved,
                        "metadata": metadata,
                    }),
                    files,
                ));
            }
            Ok(json!({
                "nodes": view.nodes.iter().map(symbol_row).collect::<Vec<_>>(),
                "edges": view.edges.iter().map(edge_row).collect::<Vec<_>>(),
                "truncated_frontier": view.truncated_frontier,
                "include_unresolved": request.include_unresolved,
                "metadata": metadata,
            }))
        }
        "mermaid" => {
            let mut metadata = code_subgraph_metadata(radius, view.truncated, &budget);
            if let Some(warning) = warning {
                metadata["warning"] = Value::String(warning);
            }
            let mermaid = mermaid_subgraph_owned(&view.nodes, &view.edges);
            Ok(json!({
                "mermaid": mermaid,
                "truncated_frontier": view.truncated_frontier,
                "include_unresolved": request.include_unresolved,
                "metadata": metadata,
            }))
        }
        other => Err(McpHandlerError::InvalidParams(format!(
            "invalid format `{other}`; expected `json` or `mermaid`"
        ))
        .into()),
    }
}

#[allow(dead_code)]
fn code_subgraph_with_artifact(args: &Value, artifact: &GraphIndexArtifact) -> CodeGraphResult {
    code_subgraph_with_artifact_and_temporal(args, artifact, None)
}

#[allow(dead_code)]
fn code_subgraph_with_loaded_artifact(
    args: &Value,
    loaded: &LoadedGraphArtifact,
) -> CodeGraphResult {
    let temporal_index = temporal_index_for_as_of(args, loaded)?;
    code_subgraph_with_artifact_and_temporal(args, loaded.artifact(), temporal_index)
}

#[allow(dead_code)]
fn code_subgraph_with_artifact_and_temporal(
    args: &Value,
    artifact: &GraphIndexArtifact,
    temporal_index: Option<Arc<TemporalIndex>>,
) -> CodeGraphResult {
    let response_format = ResponseFormat::parse(args)?;
    let request = code_traversal_request(args)?;
    let root_ids = match code_subgraph_root_ids(args, artifact, temporal_index)? {
        CodeSubgraphRoots::RootIds(root_ids) => root_ids,
        CodeSubgraphRoots::Ambiguous(candidates) => {
            return Ok(ambiguous_response(candidates));
        }
    };

    let requested_radius = args
        .get("radius")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let radius = requested_radius.min(u64::from(MAX_MCP_CODE_SUBGRAPH_RADIUS)) as u8;
    let warning = (requested_radius > u64::from(MAX_MCP_CODE_SUBGRAPH_RADIUS)).then(|| {
        format!(
            "radius {requested_radius} exceeds max {MAX_MCP_CODE_SUBGRAPH_RADIUS}; clamped to {MAX_MCP_CODE_SUBGRAPH_RADIUS}"
        )
    });
    let format = args
        .get("format")
        .and_then(|value| value.as_str())
        .unwrap_or("json");
    let edge_kinds = parse_edge_kinds(args)?;
    let edge_filter = edge_kinds.as_deref();
    let budget = code_subgraph_budget(args)?;
    let root_refs = root_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let view = bounded_subgraph_with_budget(
        artifact,
        &root_refs,
        radius,
        edge_filter,
        request.include_unresolved,
        budget.budget,
    );

    match format {
        "json" => {
            let mut metadata = code_subgraph_metadata(radius, view.truncated, &budget);
            if let Some(warning) = warning {
                metadata["warning"] = Value::String(warning);
            }
            if response_format == ResponseFormat::Table {
                let mut files = TableFileInterner::default();
                let nodes = symbol_table(view.nodes.iter().copied(), &mut files);
                let edges = edge_table(view.edges.iter().copied());
                return Ok(table_response(
                    json!({
                        "nodes": nodes,
                        "edges": edges,
                        "truncated_frontier": view.truncated_frontier,
                        "include_unresolved": request.include_unresolved,
                        "metadata": metadata,
                    }),
                    files,
                ));
            }
            Ok(json!({
                "nodes": view.nodes.into_iter().map(symbol_row).collect::<Vec<_>>(),
                "edges": view.edges.into_iter().map(edge_row).collect::<Vec<_>>(),
                "truncated_frontier": view.truncated_frontier,
                "include_unresolved": request.include_unresolved,
                "metadata": metadata,
            }))
        }
        "mermaid" => {
            let mut metadata = code_subgraph_metadata(radius, view.truncated, &budget);
            if let Some(warning) = warning {
                metadata["warning"] = Value::String(warning);
            }
            let mermaid = mermaid_subgraph(&view.nodes, &view.edges);
            Ok(json!({
                "mermaid": mermaid,
                "truncated_frontier": view.truncated_frontier,
                "include_unresolved": request.include_unresolved,
                "metadata": metadata,
            }))
        }
        other => Err(McpHandlerError::InvalidParams(format!(
            "invalid format `{other}`; expected `json` or `mermaid`"
        ))
        .into()),
    }
}

pub async fn code_symbol_history(args: &Value) -> Result<Value, McpHandlerError> {
    code_graph_backend_value(args, code_symbol_history_with_client).await
}

async fn code_symbol_history_response(
    args: &Value,
    rebuild_coordinator: Arc<RebuildCoordinator>,
    overlay_fsmonitor_auto: bool,
) -> CodeGraphResult {
    code_graph_backend_response(
        args,
        rebuild_coordinator,
        overlay_fsmonitor_auto,
        code_symbol_history_with_client,
    )
    .await
}

fn code_symbol_history_with_client(args: &Value, client: &dyn GraphQueryClient) -> CodeGraphResult {
    let worktree = current_worktree()?;
    let commits = load_commit_index_for_request(&worktree)?;
    let symbol_id = symbol_id_arg(args)?.to_string();
    let history = client
        .symbol_history(&commits, &symbol_id)
        .map_err(graph_query_error)?;
    let events = code_symbol_history_events(args, &commits, history)?;

    Ok(json!({
        "symbol": symbol_uri(&symbol_id),
        "events": events,
    }))
}

fn code_symbol_history_events(
    args: &Value,
    commits: &CommitIndexArtifact,
    history: Vec<(String, crate::ChangeKind, SnapshotKey)>,
) -> Result<Vec<Value>, McpHandlerError> {
    let reachable = parse_as_of(args)?
        .map(|as_of| reachable_commits(commits, &as_of))
        .transpose()?;
    let commits_by_sha = commits
        .commits
        .iter()
        .map(|commit| (commit.sha.as_str(), commit))
        .collect::<HashMap<_, _>>();

    Ok(history
        .into_iter()
        .filter(|(sha, _, _)| {
            reachable
                .as_ref()
                .is_none_or(|reachable| reachable.contains(sha))
        })
        .map(|(sha, change_kind, key)| {
            if let Some(commit) = commits_by_sha.get(sha.as_str()) {
                json!({
                    "commit": sha,
                    "author_time": commit.author_time,
                    "author_name": commit.author_name,
                    "author_email": commit.author_email,
                    "summary": commit.summary,
                    "change_kind": change_kind,
                    "snapshot": key,
                })
            } else {
                json!({
                    "commit": sha,
                    "change_kind": change_kind,
                    "snapshot": key,
                })
            }
        })
        .collect::<Vec<_>>())
}

pub type CodeGraphResult = Result<Value, CodeGraphError>;
#[allow(dead_code)]
type CodeGraphPayloadResult = Result<GraphResponsePayload, CodeGraphError>;

#[derive(Clone)]
#[allow(dead_code)]
struct LoadedGraphArtifact {
    artifact: Arc<GraphIndexArtifact>,
    rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
    rebuild_key: Option<LoadedRebuildKey>,
}

#[allow(dead_code)]
impl LoadedGraphArtifact {
    fn new(
        artifact: Arc<GraphIndexArtifact>,
        rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
        rebuild_key: Option<LoadedRebuildKey>,
    ) -> Self {
        Self {
            artifact,
            rebuild_coordinator,
            rebuild_key,
        }
    }

    fn artifact(&self) -> &GraphIndexArtifact {
        &self.artifact
    }

    fn temporal_index(&self) -> Arc<TemporalIndex> {
        match (&self.rebuild_coordinator, &self.rebuild_key) {
            (Some(rebuild_coordinator), Some(rebuild_key))
                if rebuild_key.retain_temporal_index_on_miss =>
            {
                rebuild_coordinator.temporal_index_for_artifact(
                    &rebuild_key.worktree,
                    rebuild_key.key.clone(),
                    Arc::clone(&self.artifact),
                )
            }
            (Some(rebuild_coordinator), Some(rebuild_key)) => rebuild_coordinator
                .temporal_index_for_retained_artifact(&rebuild_key.key)
                .unwrap_or_else(|| Arc::new(TemporalIndex::new(Arc::clone(&self.artifact)))),
            _ => Arc::new(TemporalIndex::new(Arc::clone(&self.artifact))),
        }
    }
}

#[allow(dead_code)]
fn temporal_index_for_as_of(
    args: &Value,
    loaded: &LoadedGraphArtifact,
) -> Result<Option<Arc<TemporalIndex>>, McpHandlerError> {
    if parse_as_of(args)?.is_none() {
        return Ok(None);
    }
    if let Ok(backend) = open_existing_code_search_backend_for_request() {
        if backend.metadata_source().graph_content_hash == loaded.artifact().graph_content_hash {
            return Ok(Some(backend.client().temporal_index()));
        }
    }
    Ok(Some(loaded.temporal_index()))
}

#[derive(Debug)]
#[allow(dead_code)]
struct GraphResponsePayload {
    body: Value,
    files: Option<Vec<(String, String)>>,
}

#[allow(dead_code)]
impl GraphResponsePayload {
    fn body(body: Value) -> Self {
        Self { body, files: None }
    }

    fn files_for_metadata(&self, artifact: &GraphIndexArtifact) -> Vec<(String, String)> {
        self.files
            .clone()
            .or_else(|| empty_code_search_file_set(artifact, &self.body))
            .unwrap_or_else(|| response_file_set_from_body(artifact, &self.body))
    }
}

#[derive(Debug)]
pub struct CodeGraphError {
    error: McpHandlerError,
    metadata: Option<Box<GraphMetadataSource>>,
    response_metadata: Option<Box<GraphResponseMetadata>>,
    /// For temporal resolution failures, carries the JSON-RPC error code (e.g. -32005 for Deleted).
    temporal_code: Option<i64>,
    /// For temporal resolution failures, carries the structured error data payload.
    temporal_data: Option<Box<Value>>,
}

impl CodeGraphError {
    fn without_metadata(error: McpHandlerError) -> Self {
        Self {
            error,
            metadata: None,
            response_metadata: None,
            temporal_code: None,
            temporal_data: None,
        }
    }

    fn with_temporal(code: i64, message: String, data: Value) -> Self {
        Self {
            error: McpHandlerError::Internal(message),
            metadata: None,
            response_metadata: None,
            temporal_code: Some(code),
            temporal_data: Some(Box::new(data)),
        }
    }

    fn with_artifact_metadata(mut self, artifact: &GraphIndexArtifact) -> Self {
        if self.metadata.is_none() && self.temporal_code.is_none() {
            self.metadata = Some(Box::new(GraphMetadataSource::from_artifact(artifact)));
        }
        self
    }

    fn with_metadata_source(mut self, source: GraphMetadataSource) -> Self {
        if self.metadata.is_none() && self.temporal_code.is_none() {
            self.metadata = Some(Box::new(source));
        }
        self
    }

    fn with_response_metadata(mut self, metadata: GraphResponseMetadata) -> Self {
        self.response_metadata = Some(Box::new(metadata));
        self
    }

    fn into_handler_error(self) -> McpHandlerError {
        self.error
    }

    pub async fn into_error_response(self) -> CodeGraphErrorResponse {
        let CodeGraphError {
            error,
            metadata,
            response_metadata,
            temporal_code,
            temporal_data,
        } = self;
        if let (Some(code), Some(data)) = (temporal_code, temporal_data) {
            return CodeGraphErrorResponse {
                code,
                message: handler_error_message(&error),
                data: Some(*data),
            };
        }

        let mut response = match error {
            McpHandlerError::InvalidParams(message) => CodeGraphErrorResponse {
                code: -32602,
                message,
                data: None,
            },
            McpHandlerError::NotFound(message) => CodeGraphErrorResponse {
                code: CODE_GRAPH_NOT_FOUND_ERROR_CODE,
                message,
                data: Some(json!({ "kind": "not_found" })),
            },
            McpHandlerError::Unauthorized(message) => CodeGraphErrorResponse {
                code: -32001,
                message,
                data: None,
            },
            McpHandlerError::UpstreamPm(message) | McpHandlerError::Internal(message) => {
                CodeGraphErrorResponse {
                    code: -32603,
                    message,
                    data: None,
                }
            }
        };

        let metadata = if let Some(metadata) = response_metadata {
            Some(metadata.into_value())
        } else if let Some(metadata) = metadata {
            Some(
                GraphResponseMetadata::from_source(*metadata)
                    .await
                    .into_value(),
            )
        } else {
            None
        };
        if let Some(metadata) = metadata {
            match (&mut response.data, metadata) {
                (Some(Value::Object(data)), Value::Object(metadata)) => {
                    data.extend(metadata);
                }
                (_, metadata) => {
                    response.data = Some(metadata);
                }
            }
        }
        response
    }
}

#[derive(Debug, Clone)]
pub struct CodeGraphErrorResponse {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

fn handler_error_message(error: &McpHandlerError) -> String {
    match error {
        McpHandlerError::Internal(message)
        | McpHandlerError::NotFound(message)
        | McpHandlerError::InvalidParams(message)
        | McpHandlerError::Unauthorized(message)
        | McpHandlerError::UpstreamPm(message) => message.clone(),
    }
}

fn resolution_error(code: i64, message: String, data: Value) -> CodeGraphError {
    CodeGraphError::with_temporal(code, message, data)
}

impl From<McpHandlerError> for CodeGraphError {
    fn from(error: McpHandlerError) -> Self {
        Self::without_metadata(error)
    }
}

fn deleted_resolution_error(
    symbol_id: &str,
    as_of: &str,
    last_seen: SnapshotKey,
) -> CodeGraphError {
    resolution_error(
        CODE_GRAPH_DELETED_ERROR_CODE,
        format!("symbol {symbol_id} was deleted at or before commit `{as_of}`"),
        json!({
            "kind": "deleted",
            "last_seen": last_seen,
        }),
    )
}

fn ambiguous_resolution_error(
    symbol_id: &str,
    as_of: &str,
    candidates: Vec<String>,
) -> CodeGraphError {
    resolution_error(
        CODE_GRAPH_AMBIGUOUS_ERROR_CODE,
        format!(
            "symbol {symbol_id} is ambiguous at commit `{as_of}`; candidates: {}",
            candidates.join(", ")
        ),
        json!({
            "kind": "ambiguous",
            "candidates": candidates,
        }),
    )
}

fn unknown_resolution_error(
    symbol_id: &str,
    as_of: &str,
    reason: ResolutionFailure,
) -> CodeGraphError {
    let reason_message = format_resolution_failure(&reason);
    resolution_error(
        CODE_GRAPH_UNKNOWN_ERROR_CODE,
        format!("symbol {symbol_id} could not be resolved at commit `{as_of}` ({reason_message})"),
        json!({
            "kind": "unknown",
            "reason": resolution_failure_data(&reason),
        }),
    )
}

fn resolution_failure_data(reason: &ResolutionFailure) -> Value {
    match reason {
        ResolutionFailure::AnchorCommitNotIndexed(commit) => json!({
            "kind": "anchor_commit_not_indexed",
            "commit": commit,
        }),
        ResolutionFailure::SymbolNotPresentAtAnchor => json!({
            "kind": "symbol_not_present_at_anchor",
        }),
        ResolutionFailure::IndexCorrupt(message) => json!({
            "kind": "index_corrupt",
            "message": message,
        }),
    }
}

#[derive(Debug, Clone)]
struct GraphMetadataSource {
    graph_content_hash: String,
    graph_index_version: String,
    manifest_version: String,
}

impl GraphMetadataSource {
    fn from_artifact(artifact: &GraphIndexArtifact) -> Self {
        Self {
            graph_content_hash: artifact.graph_content_hash.clone(),
            graph_index_version: artifact.header.graph_index_version.clone(),
            manifest_version: artifact.manifest_version.clone(),
        }
    }

    fn from_parquet_manifest(manifest: &GraphArtifactManifest) -> Self {
        Self {
            graph_content_hash: manifest.graph_content_hash.clone(),
            graph_index_version: manifest.graph_index_version.clone(),
            manifest_version: manifest.manifest_version.clone(),
        }
    }
}

#[derive(Debug)]
struct GraphResponseMetadata {
    source: GraphMetadataSource,
    graph_built_at: Option<String>,
    indexed_head_oid: Option<String>,
    worktree_head_oid: Option<String>,
    worktree_dirty: Option<bool>,
    response_file_oids_match: Option<bool>,
    rebuild_status: RebuildStatus,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum RebuildStatus {
    #[default]
    NotNeeded,
    Fresh,
    StaleBudgetExceeded,
    StaleRebuildFailed,
}

impl RebuildStatus {
    fn is_compact_actionable(self) -> bool {
        matches!(self, Self::StaleBudgetExceeded | Self::StaleRebuildFailed)
    }
}

#[derive(Debug)]
struct GraphResponseAnalysis {
    metadata: GraphResponseMetadata,
    rebuild_candidate: Option<RebuildCandidate>,
}

#[derive(Debug)]
struct RebuildCandidate {
    worktree: PathBuf,
    key: RebuildKey,
}

#[derive(Clone)]
#[allow(dead_code)]
struct LoadedRebuildKey {
    worktree: PathBuf,
    key: RebuildKey,
    retain_temporal_index_on_miss: bool,
}

#[allow(dead_code)]
impl LoadedRebuildKey {
    fn retain_on_miss(worktree: PathBuf, key: RebuildKey) -> Self {
        Self {
            worktree,
            key,
            retain_temporal_index_on_miss: true,
        }
    }
}

impl GraphResponseMetadata {
    /// Build response metadata from one trusted immutable publication. This
    /// describes the pinned generation; it does not claim the source could not
    /// change while the handler was executing.
    fn from_published_generation(
        source: GraphMetadataSource,
        identity: &OverlayGenerationIdentity,
    ) -> Self {
        debug_assert_eq!(
            source.graph_content_hash, identity.indexed_graph_content_hash,
            "overlay identity must belong to the opened graph source"
        );
        let pointer = matching_graph_pointer(&identity.canonical_worktree, &source);
        let graph_built_at = pointer.as_ref().and_then(graph_built_at_from_pointer);
        let published_indexed_head_oid = pointer
            .as_ref()
            .and_then(|pointer| non_empty_string(pointer.indexed_commit_oid.clone()))
            .or_else(|| identity.indexed_head_oid.clone());
        let clean_fingerprint = overlay_snapshot::normalized_changed_set_fingerprint(
            std::iter::empty::<(&String, &overlay_snapshot::OverlayPathState)>(),
        );
        let identity_overlay = identity.normalized_changed_set_fingerprint == clean_fingerprint
            && published_indexed_head_oid
                .as_deref()
                .is_none_or(|indexed| indexed == identity.current_head_oid);
        let indexed_head_oid = if identity_overlay {
            published_indexed_head_oid.clone()
        } else {
            // Exact overlay responses normalize a Fresh response to the head
            // that produced the authoritative overlay snapshot.
            Some(identity.current_head_oid.clone())
        };

        Self {
            source,
            graph_built_at,
            indexed_head_oid,
            worktree_head_oid: Some(identity.current_head_oid.clone()),
            worktree_dirty: if identity_overlay && published_indexed_head_oid.is_none() {
                None
            } else {
                Some(false)
            },
            response_file_oids_match: Some(true),
            rebuild_status: if identity_overlay {
                RebuildStatus::NotNeeded
            } else {
                RebuildStatus::Fresh
            },
        }
    }

    #[allow(dead_code)]
    async fn from_artifact_with_files(
        artifact: &GraphIndexArtifact,
        files: &[(String, String)],
    ) -> Self {
        Self::analyze_artifact_with_files(artifact, files)
            .await
            .metadata
    }

    async fn from_source(source: GraphMetadataSource) -> Self {
        Self::from_source_inner(source, None).await
    }

    async fn analyze_artifact_with_files(
        artifact: &GraphIndexArtifact,
        files: &[(String, String)],
    ) -> GraphResponseAnalysis {
        Self::analyze_source_inner(
            GraphMetadataSource::from_artifact(artifact),
            Some(files),
            None,
        )
        .await
    }

    async fn from_source_inner(
        source: GraphMetadataSource,
        response_files: Option<&[(String, String)]>,
    ) -> Self {
        Self::analyze_source_inner(source, response_files, None)
            .await
            .metadata
    }

    async fn analyze_source_inner(
        source: GraphMetadataSource,
        response_files: Option<&[(String, String)]>,
        rebuild_candidate_files: Option<&[(String, String)]>,
    ) -> GraphResponseAnalysis {
        let worktree = current_worktree_root();
        let pointer = worktree
            .as_deref()
            .and_then(|worktree| matching_graph_pointer(worktree, &source));
        let graph_built_at = pointer.as_ref().and_then(graph_built_at_from_pointer);
        let indexed_head_oid = pointer
            .as_ref()
            .and_then(|pointer| non_empty_string(pointer.indexed_commit_oid.clone()));
        let git = match worktree.as_deref() {
            Some(worktree) => worktree_git_metadata(worktree, indexed_head_oid.as_deref()).await,
            None => None,
        };
        let worktree_head_oid = git.as_ref().map(|git| git.head_oid.clone());
        let worktree_dirty = match git.as_ref() {
            Some(git) => compute_worktree_dirty(
                indexed_head_oid.as_deref(),
                &git.head_oid,
                git.has_uncommitted_changes,
            ),
            None if worktree.is_some() => Some(true),
            None => None,
        };
        let mut dirty_oids = BTreeMap::new();
        let response_file_oids_match = match (
            response_files,
            worktree.as_deref(),
            worktree_head_oid.as_deref(),
        ) {
            (Some(files), Some(worktree), Some(worktree_head_oid)) => {
                let files = files
                    .iter()
                    .map(|(rel_path, indexed_oid)| (rel_path.as_str(), indexed_oid.as_str()))
                    .collect::<Vec<_>>();
                let report = file_oid_cache::aggregate_file_oid_report(
                    worktree,
                    worktree_head_oid,
                    &source.graph_content_hash,
                    &files,
                );
                dirty_oids = report.dirty_oids;
                report.verdict
            }
            _ => None,
        };
        let supplemental_oids = match (worktree.as_ref(), git.as_ref()) {
            (Some(worktree), Some(git)) if !git.supplemental_changed.is_empty() => {
                supplemental_changed_oids(worktree, &git.supplemental_changed)
            }
            _ => BTreeMap::new(),
        };
        let rebuild_dirty_oids = match (
            rebuild_candidate_files,
            worktree.as_deref(),
            worktree_head_oid.as_deref(),
        ) {
            (Some(files), Some(worktree), Some(worktree_head_oid)) => {
                let files = files
                    .iter()
                    .map(|(rel_path, indexed_oid)| (rel_path.as_str(), indexed_oid.as_str()))
                    .collect::<Vec<_>>();
                dirty_indexed_file_oids_for_files(
                    worktree,
                    worktree_head_oid,
                    &source.graph_content_hash,
                    &files,
                )
            }
            _ => dirty_oids.clone(),
        };
        let rebuild_candidate = match (worktree.as_ref(), worktree_head_oid.as_deref()) {
            (Some(worktree), Some(head_oid))
                if !rebuild_dirty_oids.is_empty() || !supplemental_oids.is_empty() =>
            {
                let mut key_oids = rebuild_dirty_oids;
                key_oids.extend(supplemental_oids);
                Some(RebuildCandidate {
                    worktree: worktree.clone(),
                    key: RebuildKey::from(head_oid, &key_oids),
                })
            }
            _ => None,
        };

        GraphResponseAnalysis {
            metadata: Self {
                source,
                graph_built_at,
                indexed_head_oid,
                worktree_head_oid,
                worktree_dirty,
                response_file_oids_match,
                rebuild_status: RebuildStatus::NotNeeded,
            },
            rebuild_candidate,
        }
    }

    fn with_rebuild_status(mut self, rebuild_status: RebuildStatus) -> Self {
        self.rebuild_status = rebuild_status;
        match rebuild_status {
            RebuildStatus::Fresh => {
                self.worktree_dirty = Some(false);
                if self.indexed_head_oid.is_none() {
                    self.indexed_head_oid = self.worktree_head_oid.clone();
                }
            }
            RebuildStatus::StaleBudgetExceeded | RebuildStatus::StaleRebuildFailed => {
                self.worktree_dirty = Some(true);
            }
            RebuildStatus::NotNeeded => {}
        }
        self
    }

    fn into_value(self) -> Value {
        json!({
            "graph_content_hash": self.source.graph_content_hash,
            "graph_index_version": self.source.graph_index_version,
            "graph_built_at": self.graph_built_at,
            "indexed_head_oid": self.indexed_head_oid,
            "worktree_head_oid": self.worktree_head_oid,
            "worktree_dirty": self.worktree_dirty,
            "response_file_oids_match": self.response_file_oids_match,
            "rebuild_status": self.rebuild_status,
        })
    }

    fn into_compact_value(self) -> Value {
        let mut metadata = serde_json::Map::new();
        if self.worktree_dirty == Some(true) {
            metadata.insert("worktree_dirty".into(), Value::Bool(true));
            if let (Some(indexed_head_oid), Some(worktree_head_oid)) =
                (self.indexed_head_oid, self.worktree_head_oid)
            {
                if indexed_head_oid != worktree_head_oid {
                    metadata.insert("indexed_head_oid".into(), Value::String(indexed_head_oid));
                    metadata.insert("worktree_head_oid".into(), Value::String(worktree_head_oid));
                }
            }
        }
        if self.response_file_oids_match == Some(false) {
            metadata.insert("response_file_oids_match".into(), Value::Bool(false));
        }
        if self.rebuild_status.is_compact_actionable() {
            metadata.insert("rebuild_status".into(), json!(self.rebuild_status));
        }
        Value::Object(metadata)
    }

    fn insert_into(self, body: &mut Value) {
        self.insert_into_for_format(body, ResponseFormat::Full);
    }

    fn insert_into_for_format(self, body: &mut Value, response_format: ResponseFormat) {
        match response_format {
            ResponseFormat::Full => self.insert_full_into(body),
            ResponseFormat::Compact | ResponseFormat::Table => self.insert_compact_into(body),
            ResponseFormat::Source => {}
        }
    }

    fn insert_full_into(self, body: &mut Value) {
        if let Value::Object(map) = body {
            let Value::Object(metadata) = self.into_value() else {
                return;
            };
            map.extend(metadata);
        }
    }

    fn insert_compact_into(self, body: &mut Value) {
        if let Value::Object(map) = body {
            let Value::Object(metadata) = self.into_compact_value() else {
                return;
            };
            map.extend(metadata);
        }
    }
}

#[allow(dead_code)]
async fn with_loaded_graph_artifact(
    rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
    handler: impl Fn(LoadedGraphArtifact) -> CodeGraphResult + Send + Sync,
) -> CodeGraphResult {
    let artifact =
        Arc::new(load_graph_artifact_for_request().map_err(CodeGraphError::without_metadata)?);
    let rebuild_key = match current_worktree() {
        Ok(worktree) => rebuild_key_for_loaded_artifact(&worktree, &artifact).await,
        Err(_) => None,
    };
    with_loaded_graph_payload(rebuild_coordinator, artifact, rebuild_key, |loaded| {
        handler(loaded).map(GraphResponsePayload::body)
    })
    .await
}

#[allow(dead_code)]
async fn with_loaded_graph_payload(
    rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
    artifact: Arc<GraphIndexArtifact>,
    rebuild_key: Option<LoadedRebuildKey>,
    handler: impl Fn(LoadedGraphArtifact) -> CodeGraphPayloadResult + Send + Sync,
) -> CodeGraphResult {
    let payload = handler(LoadedGraphArtifact::new(
        Arc::clone(&artifact),
        rebuild_coordinator.clone(),
        rebuild_key,
    ))
    .map_err(|error| error.with_artifact_metadata(&artifact))?;
    Ok(with_graph_metadata_for_payload(rebuild_coordinator, artifact, payload, &handler).await)
}

#[allow(dead_code)]
async fn rebuild_key_for_loaded_artifact(
    worktree: &Path,
    artifact: &GraphIndexArtifact,
) -> Option<LoadedRebuildKey> {
    let git = worktree_git_metadata(worktree, None).await?;
    let mut dirty_oids = if git.has_uncommitted_changes {
        dirty_indexed_file_oids(worktree, &git.head_oid, artifact)
    } else {
        BTreeMap::new()
    };
    dirty_oids.extend(supplemental_changed_oids(
        worktree,
        &git.supplemental_changed,
    ));
    Some(LoadedRebuildKey {
        worktree: worktree.to_path_buf(),
        key: RebuildKey::from(&git.head_oid, &dirty_oids),
        retain_temporal_index_on_miss: !git.has_uncommitted_changes
            && git.supplemental_changed.is_empty(),
    })
}

#[allow(dead_code)]
fn dirty_indexed_file_oids(
    worktree: &Path,
    worktree_head_oid: &str,
    artifact: &GraphIndexArtifact,
) -> BTreeMap<PathBuf, [u8; 20]> {
    let files = artifact
        .file_manifests
        .iter()
        .map(|entry| (entry.path.as_str(), entry.content_oid.as_str()))
        .collect::<Vec<_>>();
    dirty_indexed_file_oids_for_files(
        worktree,
        worktree_head_oid,
        &artifact.graph_content_hash,
        &files,
    )
}

fn dirty_indexed_file_oids_for_files(
    worktree: &Path,
    worktree_head_oid: &str,
    graph_content_hash: &str,
    files: &[(&str, &str)],
) -> BTreeMap<PathBuf, [u8; 20]> {
    file_oid_cache::aggregate_file_oid_report(
        worktree,
        worktree_head_oid,
        graph_content_hash,
        files,
    )
    .dirty_oids
}

#[allow(dead_code)]
async fn with_graph_metadata_for_payload(
    rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
    artifact: Arc<GraphIndexArtifact>,
    mut payload: GraphResponsePayload,
    handler: &(impl Fn(LoadedGraphArtifact) -> CodeGraphPayloadResult + Send + Sync),
) -> Value {
    let files = payload.files_for_metadata(&artifact);
    let mut analysis = GraphResponseMetadata::analyze_artifact_with_files(&artifact, &files).await;

    if let (Some(rebuild_coordinator), Some(rebuild_candidate)) =
        (rebuild_coordinator, analysis.rebuild_candidate.take())
    {
        let rebuild_worktree = rebuild_candidate.worktree.clone();
        let rebuild_key = rebuild_candidate.key.clone();
        match try_rebuild_artifact(
            Arc::clone(&rebuild_coordinator),
            Arc::clone(&artifact),
            rebuild_candidate,
            None,
        )
        .await
        {
            RebuildAttempt::Fresh(rebuilt_artifact) => match handler(LoadedGraphArtifact::new(
                Arc::clone(&rebuilt_artifact),
                Some(Arc::clone(&rebuild_coordinator)),
                Some(LoadedRebuildKey::retain_on_miss(
                    rebuild_worktree,
                    rebuild_key,
                )),
            )) {
                Ok(mut fresh_payload) => {
                    let fresh_files = fresh_payload.files_for_metadata(&rebuilt_artifact);
                    GraphResponseMetadata::analyze_artifact_with_files(
                        &rebuilt_artifact,
                        &fresh_files,
                    )
                    .await
                    .metadata
                    .with_rebuild_status(RebuildStatus::Fresh)
                    .insert_into(&mut fresh_payload.body);
                    return fresh_payload.body;
                }
                Err(error) => {
                    tracing::warn!(
                        target: "spur_graph::mcp",
                        error = ?error,
                        "rebuilt code graph response failed; serving stale response"
                    );
                    analysis.metadata = analysis
                        .metadata
                        .with_rebuild_status(RebuildStatus::StaleRebuildFailed);
                }
            },
            RebuildAttempt::StaleBudgetExceeded => {
                analysis.metadata = analysis
                    .metadata
                    .with_rebuild_status(RebuildStatus::StaleBudgetExceeded);
            }
            RebuildAttempt::StaleRebuildFailed => {
                analysis.metadata = analysis
                    .metadata
                    .with_rebuild_status(RebuildStatus::StaleRebuildFailed);
            }
        }
    }

    analysis.metadata.insert_into(&mut payload.body);
    payload.body
}

enum RebuildAttempt {
    Fresh(Arc<GraphIndexArtifact>),
    StaleBudgetExceeded,
    StaleRebuildFailed,
}

struct RebuiltParquetGraph {
    dir: PathBuf,
    client: ParquetClient,
}

impl RebuiltParquetGraph {
    fn from_artifact(artifact: &GraphIndexArtifact) -> anyhow::Result<Self> {
        let dir = rebuilt_parquet_response_dir(&artifact.graph_content_hash);
        let written =
            match write_artifact_parquet(artifact, &dir, WriteOptions::default(), Vec::new()) {
                Ok(written) => written,
                Err(error) => {
                    let _ = fs::remove_dir_all(&dir);
                    return Err(error);
                }
            };
        let client = match ParquetClient::open(&written) {
            Ok(client) => client,
            Err(error) => {
                let _ = fs::remove_dir_all(&written);
                return Err(error);
            }
        };
        Ok(Self {
            dir: written,
            client,
        })
    }

    fn client(&self) -> &ParquetClient {
        &self.client
    }
}

impl Drop for RebuiltParquetGraph {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            if error.kind() != ErrorKind::NotFound {
                tracing::debug!(
                    target: "spur_graph::mcp",
                    path = %self.dir.display(),
                    error = %error,
                    "failed to remove request-local rebuilt Parquet graph"
                );
            }
        }
    }
}

fn rebuilt_parquet_response_dir(graph_content_hash: &str) -> PathBuf {
    let counter = REBUILT_PARQUET_RESPONSE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let digest = blake3::hash(graph_content_hash.as_bytes());
    let digest_hex = digest.to_hex();
    std::env::temp_dir().join(format!(
        "spur-graph-rebuilt-parquet-{}-{since_epoch}-{counter}-{}",
        std::process::id(),
        &digest_hex.as_str()[..16]
    ))
}

fn graph_rebuild_latency_budget() -> Duration {
    #[cfg(any(test, feature = "test-support"))]
    {
        let override_ms = GRAPH_REBUILD_LATENCY_BUDGET_OVERRIDE_MS.load(Ordering::SeqCst);
        if override_ms != GRAPH_REBUILD_LATENCY_BUDGET_UNSET_MS {
            return Duration::from_millis(override_ms);
        }
    }
    DEFAULT_GRAPH_REBUILD_LATENCY_BUDGET
}

fn remaining_graph_rebuild_budget(rebuild_started: Instant) -> Option<Duration> {
    graph_rebuild_latency_budget()
        .checked_sub(rebuild_started.elapsed())
        .filter(|remaining| !remaining.is_zero())
}

#[cfg(any(test, feature = "test-support"))]
fn duration_millis_for_test(duration: Duration) -> u64 {
    duration
        .as_millis()
        .min(u128::from(GRAPH_REBUILD_LATENCY_BUDGET_UNSET_MS - 1)) as u64
}

#[cfg(any(test, feature = "test-support"))]
pub struct GraphRebuildBudgetGuard {
    previous_ms: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for GraphRebuildBudgetGuard {
    fn drop(&mut self) {
        GRAPH_REBUILD_LATENCY_BUDGET_OVERRIDE_MS.store(self.previous_ms, Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_graph_rebuild_latency_budget_for_test(budget: Duration) -> GraphRebuildBudgetGuard {
    let previous_ms = GRAPH_REBUILD_LATENCY_BUDGET_OVERRIDE_MS
        .swap(duration_millis_for_test(budget), Ordering::SeqCst);
    GraphRebuildBudgetGuard { previous_ms }
}

#[cfg(any(test, feature = "test-support"))]
pub struct GraphRebuildDelayGuard {
    previous_ms: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for GraphRebuildDelayGuard {
    fn drop(&mut self) {
        GRAPH_REBUILD_DELAY_MS.store(self.previous_ms, Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_graph_rebuild_delay_for_test(delay: Duration) -> GraphRebuildDelayGuard {
    let previous_ms =
        GRAPH_REBUILD_DELAY_MS.swap(duration_millis_for_test(delay), Ordering::SeqCst);
    GraphRebuildDelayGuard { previous_ms }
}

fn code_tool_timeout() -> Duration {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(timeout) = CODE_TOOL_TIMEOUT_OVERRIDE.try_with(|timeout| *timeout) {
        return timeout;
    }
    DEFAULT_CODE_TOOL_TIMEOUT
}

fn code_tool_deadline_exceeded(tool_name: &str, timeout: Duration) -> CodeGraphError {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    CodeGraphError::with_temporal(
        CODE_GRAPH_TIMEOUT_ERROR_CODE,
        format!("code graph tool `{tool_name}` exceeded {timeout_ms}ms deadline"),
        json!({
            "kind": "timeout",
            "timeout_ms": timeout_ms,
        }),
    )
}

async fn run_scoped_code_tool(
    module: GraphMcpModule,
    tool_name: &str,
    args: Value,
    inherited_root: Option<PathBuf>,
    project: Option<ResolvedLocalProject>,
) -> CodeGraphResult {
    let response = match inherited_root {
        Some(root) => {
            with_worktree_root_for_request(root, module.dispatch_current_project(tool_name, args))
                .await?
        }
        None => module.dispatch_current_project(tool_name, args).await?,
    };
    Ok(decorate_project_response(response, project.as_ref()))
}

#[cfg(any(test, feature = "test-support"))]
pub struct IncrementalRebuildFailureGuard {
    previous_failures: usize,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for IncrementalRebuildFailureGuard {
    fn drop(&mut self) {
        INCREMENTAL_REBUILD_FAILURES_REMAINING.store(self.previous_failures, Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_incremental_rebuild_failures_for_test(
    failures: usize,
) -> IncrementalRebuildFailureGuard {
    let previous_failures = INCREMENTAL_REBUILD_FAILURES_REMAINING.swap(failures, Ordering::SeqCst);
    IncrementalRebuildFailureGuard { previous_failures }
}

#[cfg(test)]
struct OverlayGenerationFailureGuard {
    previous_failures: usize,
}

#[cfg(test)]
impl Drop for OverlayGenerationFailureGuard {
    fn drop(&mut self) {
        OVERLAY_GENERATION_FAILURES_REMAINING.store(self.previous_failures, Ordering::SeqCst);
    }
}

#[cfg(test)]
fn set_overlay_generation_failures_for_test(failures: usize) -> OverlayGenerationFailureGuard {
    let previous_failures = OVERLAY_GENERATION_FAILURES_REMAINING.swap(failures, Ordering::SeqCst);
    OverlayGenerationFailureGuard { previous_failures }
}

#[cfg(any(test, feature = "test-support"))]
fn reset_exact_overlay_observations() {
    EXACT_OVERLAY_OBSERVATIONS_FOR_TEST.store(0, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
fn exact_overlay_observations() -> usize {
    EXACT_OVERLAY_OBSERVATIONS_FOR_TEST.load(Ordering::SeqCst)
}

#[cfg(test)]
fn reset_exact_overlay_observations_for_test() {
    reset_exact_overlay_observations();
}

#[cfg(test)]
fn exact_overlay_observations_for_test() -> usize {
    exact_overlay_observations()
}

fn fail_overlay_generation_for_test() -> anyhow::Result<()> {
    #[cfg(test)]
    if OVERLAY_GENERATION_FAILURES_REMAINING
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
    {
        anyhow::bail!("forced overlay generation build failure for test");
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
async fn apply_graph_rebuild_delay_for_test() {
    let delay_ms = GRAPH_REBUILD_DELAY_MS.load(Ordering::SeqCst);
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

#[cfg(any(test, feature = "test-support"))]
fn fail_incremental_rebuild_for_test() -> anyhow::Result<()> {
    if INCREMENTAL_REBUILD_FAILURES_REMAINING
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
    {
        return Err(anyhow::anyhow!(
            "forced incremental graph rebuild failure for test"
        ));
    }
    Ok(())
}

async fn try_rebuild_artifact(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    previous_artifact: Arc<GraphIndexArtifact>,
    rebuild_candidate: RebuildCandidate,
    base_seed: Option<&'static str>,
) -> RebuildAttempt {
    let rebuild_worktree = rebuild_candidate.worktree.clone();
    let rebuild_key = rebuild_candidate.key.clone();
    let mut task = spawn_incremental_rebuild_task(
        Arc::clone(&rebuild_coordinator),
        previous_artifact,
        rebuild_candidate,
        base_seed,
    );

    match tokio::time::timeout(graph_rebuild_latency_budget(), &mut task).await {
        Ok(Ok(Ok(artifact))) => {
            rebuild_coordinator.reset_incremental_rebuild_failures(&rebuild_key);
            RebuildAttempt::Fresh(artifact)
        }
        Ok(Ok(Err(error))) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "in-memory code graph rebuild failed; serving stale response"
            );
            let failures = rebuild_coordinator.record_incremental_rebuild_failure(&rebuild_key);
            if failures <= INCREMENTAL_FAILURES_BEFORE_FULL_REBUILD {
                return RebuildAttempt::StaleRebuildFailed;
            }

            tracing::warn!(
                target: "spur_graph::mcp",
                failures,
                threshold = INCREMENTAL_FAILURES_BEFORE_FULL_REBUILD,
                "persistent in-memory incremental rebuild failures; attempting full rebuild"
            );
            let full_rebuild_candidate = RebuildCandidate {
                worktree: rebuild_worktree,
                key: rebuild_key.clone(),
            };
            if let Some(artifact) = try_full_rebuild_after_incremental_failure(
                Arc::clone(&rebuild_coordinator),
                full_rebuild_candidate,
            )
            .await
            {
                rebuild_coordinator.reset_incremental_rebuild_failures(&rebuild_key);
                return RebuildAttempt::Fresh(artifact);
            }
            rebuild_coordinator.reset_incremental_rebuild_failures(&rebuild_key);
            RebuildAttempt::StaleRebuildFailed
        }
        Ok(Err(error)) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "in-memory code graph rebuild task failed; serving stale response"
            );
            RebuildAttempt::StaleRebuildFailed
        }
        Err(_) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "in-memory code graph rebuild failed after response budget elapsed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "in-memory code graph rebuild task failed after response budget elapsed"
                        );
                    }
                }
            });
            RebuildAttempt::StaleBudgetExceeded
        }
    }
}

async fn try_full_rebuild_after_incremental_failure(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    rebuild_candidate: RebuildCandidate,
) -> Option<Arc<GraphIndexArtifact>> {
    let mut task = spawn_full_rebuild_task(rebuild_coordinator, rebuild_candidate);
    match tokio::time::timeout(graph_rebuild_latency_budget(), &mut task).await {
        Ok(Ok(Ok(artifact))) => Some(artifact),
        Ok(Ok(Err(error))) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "full code graph rebuild escalation failed; serving stale response"
            );
            None
        }
        Ok(Err(error)) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "full code graph rebuild escalation task failed; serving stale response"
            );
            None
        }
        Err(_) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "full code graph rebuild escalation failed after response budget elapsed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "full code graph rebuild escalation task failed after response budget elapsed"
                        );
                    }
                }
            });
            None
        }
    }
}

async fn try_rebuild_artifact_blocking(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    previous_artifact: Arc<GraphIndexArtifact>,
    rebuild_candidate: RebuildCandidate,
    base_seed: Option<&'static str>,
) -> anyhow::Result<Arc<GraphIndexArtifact>> {
    let task = spawn_incremental_rebuild_task(
        rebuild_coordinator,
        previous_artifact,
        rebuild_candidate,
        base_seed,
    );
    match tokio::time::timeout(COLD_OPEN_GRAPH_REBUILD_TIMEOUT, task).await {
        Ok(Ok(artifact)) => artifact,
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "in-memory graph rebuild task failed: {error}"
        )),
        Err(_) => Err(anyhow::anyhow!(
            "in-memory graph rebuild exceeded hard timeout of {:?}",
            COLD_OPEN_GRAPH_REBUILD_TIMEOUT
        )),
    }
}

fn spawn_incremental_rebuild_task(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    previous_artifact: Arc<GraphIndexArtifact>,
    rebuild_candidate: RebuildCandidate,
    base_seed: Option<&'static str>,
) -> tokio::task::JoinHandle<anyhow::Result<Arc<GraphIndexArtifact>>> {
    let RebuildCandidate { worktree, key } = rebuild_candidate;
    tokio::spawn(async move {
        let rebuild_worktree = worktree.clone();
        rebuild_coordinator
            .get_or_build(rebuild_worktree, key, move || {
                let previous_artifact = Arc::clone(&previous_artifact);
                let worktree = worktree.clone();
                async move {
                    #[cfg(any(test, feature = "test-support"))]
                    apply_graph_rebuild_delay_for_test().await;
                    tokio::task::spawn_blocking(move || {
                        #[cfg(any(test, feature = "test-support"))]
                        fail_incremental_rebuild_for_test()?;
                        let (artifact, _mode, stats) =
                            crate::store::build::artifact_from_facts_incremental(
                                &previous_artifact,
                                &worktree,
                            )?;
                        if let Some(base) = base_seed {
                            emit_base_seed_stats(base, stats);
                        }
                        Ok(Arc::new(artifact))
                    })
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("in-memory graph rebuild task failed: {error}")
                    })?
                }
            })
            .await
    })
}

fn spawn_full_rebuild_task(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    rebuild_candidate: RebuildCandidate,
) -> tokio::task::JoinHandle<anyhow::Result<Arc<GraphIndexArtifact>>> {
    let RebuildCandidate { worktree, key } = rebuild_candidate;
    tokio::spawn(async move {
        let rebuild_worktree = worktree.clone();
        rebuild_coordinator
            .get_or_build(rebuild_worktree, key, move || {
                let worktree = worktree.clone();
                async move {
                    #[cfg(any(test, feature = "test-support"))]
                    apply_graph_rebuild_delay_for_test().await;
                    tokio::task::spawn_blocking(move || {
                        let (facts, _file_counts) = build_facts(&worktree, None)?;
                        let artifact = artifact_from_facts(&facts, &worktree)?;
                        Ok(Arc::new(artifact))
                    })
                    .await
                    .map_err(|error| anyhow::anyhow!("graph rebuild task failed: {error}"))?
                }
            })
            .await
    })
}

async fn try_rebuild_artifact_from_worktree(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    rebuild_candidate: RebuildCandidate,
) -> RebuildAttempt {
    if let Some(seed) = load_base_seed_for_worktree(&rebuild_candidate.worktree) {
        return try_rebuild_artifact_from_seed(rebuild_coordinator, rebuild_candidate, seed).await;
    }

    let mut task = spawn_full_rebuild_task(rebuild_coordinator, rebuild_candidate);

    match tokio::time::timeout(graph_rebuild_latency_budget(), &mut task).await {
        Ok(Ok(Ok(artifact))) => RebuildAttempt::Fresh(artifact),
        Ok(Ok(Err(error))) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "code graph rebuild failed; serving stale response"
            );
            RebuildAttempt::StaleRebuildFailed
        }
        Ok(Err(error)) => {
            tracing::warn!(
                target: "spur_graph::mcp",
                error = %error,
                "code graph rebuild task failed; serving stale response"
            );
            RebuildAttempt::StaleRebuildFailed
        }
        Err(_) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "code graph rebuild failed after response budget elapsed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "spur_graph::mcp",
                            error = %error,
                            "code graph rebuild task failed after response budget elapsed"
                        );
                    }
                }
            });
            RebuildAttempt::StaleBudgetExceeded
        }
    }
}

async fn try_rebuild_artifact_from_seed(
    rebuild_coordinator: Arc<RebuildCoordinator>,
    rebuild_candidate: RebuildCandidate,
    seed: BaseArtifactSeed,
) -> RebuildAttempt {
    try_rebuild_artifact(
        rebuild_coordinator,
        seed.artifact,
        rebuild_candidate,
        Some(seed.base),
    )
    .await
}

#[allow(clippy::result_large_err)]
fn current_worktree() -> Result<PathBuf, McpHandlerError> {
    if let Some(worktree) = scoped_worktree_root() {
        return Ok(worktree);
    }
    let current_dir = std::env::current_dir().map_err(|error| {
        McpHandlerError::Internal(format!("failed to read current directory: {error}"))
    })?;
    Ok(resolve_worktree_root_from(current_dir))
}

pub async fn with_worktree_root_for_request<F, T>(worktree_root: PathBuf, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let worktree_root = resolve_worktree_root_from(worktree_root);
    SCOPED_CODE_GRAPH_WORKTREE_ROOT
        .scope(worktree_root, future)
        .await
}

/// Checks that `worktree_root` has a resolvable, readable graph artifact.
///
/// This probe never creates or rebuilds an artifact.
pub fn ensure_graph_artifact_ready(worktree_root: &Path) -> anyhow::Result<()> {
    let resolved = resolve_artifact_location(worktree_root, None).map_err(|error| {
        anyhow::anyhow!(
            "graph artifact is unavailable for `{}`: {error}",
            worktree_root.display()
        )
    })?;
    request_cache::parquet_client(&resolved.path).map_err(|error| {
        anyhow::anyhow!(
            "graph artifact `{}` is unreadable: {error}",
            resolved.path.display()
        )
    })?;
    Ok(())
}

#[allow(clippy::result_large_err)]
async fn open_code_search_backend_for_request(
    rebuild_coordinator: Option<Arc<RebuildCoordinator>>,
) -> Result<CodeSearchBackend, McpHandlerError> {
    let worktree = current_worktree()?;
    let resolved = match resolve_artifact_location(&worktree, None) {
        Ok(resolved) => resolved,
        Err(_) => {
            let rebuild_coordinator =
                rebuild_coordinator.unwrap_or_else(shared_rebuild_coordinator);
            return open_code_search_backend_from_base_seed(worktree, rebuild_coordinator).await;
        }
    };

    open_resolved_code_search_backend(&worktree, resolved)
}

#[allow(clippy::result_large_err)]
fn open_existing_code_search_backend_for_request() -> Result<CodeSearchBackend, McpHandlerError> {
    let worktree = current_worktree()?;
    let resolved = resolve_artifact_location(&worktree, None)
        .map_err(|_| graph_artifact_missing(&worktree))?;
    open_resolved_code_search_backend(&worktree, resolved)
}

#[allow(clippy::result_large_err)]
fn open_resolved_code_search_backend(
    worktree: &Path,
    resolved: crate::ResolvedArtifact,
) -> Result<CodeSearchBackend, McpHandlerError> {
    let artifact_path = resolved.path;

    request_cache::parquet_client(&artifact_path)
        .map(CodeSearchBackend::Parquet)
        .map_err(|error| {
            if !artifact_path.exists() {
                graph_artifact_missing(worktree)
            } else {
                McpHandlerError::Internal(format!(
                    "failed to open graph artifact `{}`: {error}",
                    artifact_path.display()
                ))
            }
        })
}

#[allow(clippy::result_large_err)]
async fn open_code_search_backend_from_base_seed(
    worktree: PathBuf,
    rebuild_coordinator: Arc<RebuildCoordinator>,
) -> Result<CodeSearchBackend, McpHandlerError> {
    let artifact =
        overlaid_graph_artifact_from_base_seed_for_worktree(worktree, rebuild_coordinator).await?;
    Ok(CodeSearchBackend::InMemory {
        client: InMemoryClient::new(Arc::clone(&artifact)),
        artifact,
    })
}

#[allow(clippy::result_large_err)]
pub async fn overlaid_graph_artifact_from_base_seed_for_worktree(
    worktree: PathBuf,
    rebuild_coordinator: Arc<RebuildCoordinator>,
) -> Result<Arc<GraphIndexArtifact>, McpHandlerError> {
    let Some(seed) = load_base_seed_for_worktree(&worktree) else {
        return Err(graph_artifact_missing(&worktree));
    };
    if base_seed_matches_clean_worktree(&worktree, &seed)
        .await
        .unwrap_or(false)
    {
        return Ok(Arc::clone(&seed.artifact));
    }
    let rebuild_candidate = rebuild_candidate_for_base_seed(
        &worktree,
        &seed.artifact,
        seed.indexed_commit_oid.as_deref(),
    )
    .await;

    match try_rebuild_artifact_blocking(
        rebuild_coordinator,
        seed.artifact,
        rebuild_candidate,
        Some(seed.base),
    )
    .await
    {
        Ok(artifact) => Ok(artifact),
        Err(error) => Err(McpHandlerError::Internal(format!(
            "graph artifact not found and base-seed overlay failed in {}: {error}",
            worktree.display()
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownOverlayBaseDelta {
    pub base_artifact_dir: PathBuf,
    pub changed_markdown_paths: BTreeSet<String>,
    pub deleted_markdown_paths: BTreeSet<String>,
}

pub async fn markdown_overlay_base_delta_for_worktree(
    worktree: &Path,
) -> Option<MarkdownOverlayBaseDelta> {
    let seed = load_base_seed_for_worktree(worktree)?;
    let indexed_commit_oid = seed.indexed_commit_oid.as_deref();
    let git = worktree_git_metadata_with_extensions(
        worktree,
        indexed_commit_oid,
        MARKDOWN_OVERLAY_EXTENSIONS,
        true,
    )
    .await?;
    let head_oid = non_empty_string(Some(git.head_oid.clone()))
        .or_else(|| non_empty_string(seed.indexed_commit_oid.clone()))
        .unwrap_or_else(|| seed.artifact.graph_content_hash.clone());

    let mut changed_markdown_paths = dirty_indexed_file_oids(worktree, &head_oid, &seed.artifact)
        .keys()
        .filter_map(|path| markdown_relative_path(path))
        .collect::<BTreeSet<_>>();
    changed_markdown_paths.extend(
        git.supplemental_changed
            .into_iter()
            .filter(|path| is_markdown_path(Path::new(path))),
    );

    let deleted_markdown_paths = seed
        .artifact
        .file_manifests
        .iter()
        .filter_map(|entry| {
            let relative = Path::new(entry.path.as_str());
            if is_markdown_path(relative) && !worktree.join(relative).is_file() {
                Some(entry.path.clone())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    changed_markdown_paths.retain(|path| !deleted_markdown_paths.contains(path));

    Some(MarkdownOverlayBaseDelta {
        base_artifact_dir: seed.artifact_dir,
        changed_markdown_paths,
        deleted_markdown_paths,
    })
}

async fn base_seed_matches_clean_worktree(
    worktree: &Path,
    seed: &BaseArtifactSeed,
) -> Option<bool> {
    let indexed_commit_oid = non_empty_string(seed.indexed_commit_oid.clone())?;
    tokio::time::timeout(GRAPH_GIT_METADATA_TIMEOUT, async {
        let head_oid = run_git_stdout(worktree, &["rev-parse", "HEAD"]).await?;
        if head_oid != indexed_commit_oid {
            return Some(false);
        }
        let status = run_git_stdout(worktree, &["status", "--porcelain"]).await?;
        Some(status.is_empty())
    })
    .await
    .ok()
    .flatten()
}

async fn rebuild_candidate_for_base_seed(
    worktree: &Path,
    base_artifact: &GraphIndexArtifact,
    fallback_head_oid: Option<&str>,
) -> RebuildCandidate {
    // Prefer the live worktree HEAD, but never fail the request when git is
    // momentarily unavailable — the 200ms probe can time out under load, and a
    // sibling worker mutating the shared `.git` can hold an index lock that makes
    // `git status` exit non-zero. The overlay rebuild walks the filesystem and
    // does not need git; `head_oid` only namespaces the rebuild single-flight key
    // and the file-oid cache, so a stable fallback (the base seed's indexed commit,
    // else the base content hash) is correct.
    let head_oid = match worktree_git_metadata(worktree, fallback_head_oid).await {
        Some(git) => git.head_oid,
        None => fallback_head_oid
            .filter(|oid| !oid.is_empty())
            .unwrap_or(base_artifact.graph_content_hash.as_str())
            .to_string(),
    };
    let dirty_oids = dirty_indexed_file_oids(worktree, &head_oid, base_artifact);
    RebuildCandidate {
        worktree: worktree.to_path_buf(),
        key: RebuildKey::from(&head_oid, &dirty_oids),
    }
}

#[allow(clippy::result_large_err)]
#[allow(dead_code)]
fn load_graph_artifact_for_request() -> Result<GraphIndexArtifact, McpHandlerError> {
    let worktree = current_worktree()?;
    let resolved = resolve_artifact_location(&worktree, None)
        .map_err(|_| graph_artifact_missing(&worktree))?;
    let artifact_path = resolved.path;

    match load_artifact(&artifact_path) {
        Ok(artifact) => Ok(artifact),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == ErrorKind::NotFound) =>
        {
            Err(graph_artifact_missing(&worktree))
        }
        Err(_) if !artifact_path.exists() => Err(graph_artifact_missing(&worktree)),
        Err(error) => Err(McpHandlerError::Internal(format!(
            "failed to load graph artifact `{}`: {error}",
            artifact_path.display()
        ))),
    }
}

fn graph_artifact_missing(worktree: &Path) -> McpHandlerError {
    McpHandlerError::Internal(format!(
        "graph artifact not found; run `spur graph build` in {}",
        worktree.display()
    ))
}

fn load_commit_index_for_request(worktree: &Path) -> Result<CommitIndexArtifact, McpHandlerError> {
    let pointer = crate::store::commit_index::load_pointer(worktree).map_err(|error| {
        McpHandlerError::Internal(format!(
            "failed to load commit index pointer in {}: {error}",
            worktree.display()
        ))
    })?;
    let pointer = pointer.ok_or_else(|| commit_index_missing(worktree))?;
    crate::store::commit_index::load_artifact(worktree, &pointer).map_err(|error| {
        McpHandlerError::Internal(format!(
            "failed to load commit index artifact in {}: {error}",
            worktree.display()
        ))
    })
}

fn commit_index_missing(worktree: &Path) -> McpHandlerError {
    McpHandlerError::Internal(format!(
        "commit index not found; run `spur graph build --history` in {}",
        worktree.display()
    ))
}

#[allow(dead_code)]
fn resolve_symbol_for_optional_as_of(
    artifact: &GraphIndexArtifact,
    temporal_index: Option<Arc<TemporalIndex>>,
    worktree: &Path,
    symbol_id: &str,
    args: &Value,
) -> Result<String, CodeGraphError> {
    let Some(as_of) = parse_as_of(args)? else {
        return Ok(symbol_id.to_string());
    };
    let commits = load_commit_index_for_request(worktree)?;
    temporal_resolution_symbol_id(
        symbol_id,
        &as_of,
        resolve_symbol_as_of(artifact, temporal_index, &commits, symbol_id, &as_of)?,
    )
}

#[allow(dead_code)]
fn resolve_symbol_for_optional_as_of_current_worktree(
    artifact: &GraphIndexArtifact,
    temporal_index: Option<Arc<TemporalIndex>>,
    symbol_id: &str,
    args: &Value,
) -> Result<String, CodeGraphError> {
    if parse_as_of(args)?.is_none() {
        return Ok(symbol_id.to_string());
    }
    let worktree = current_worktree()?;
    resolve_symbol_for_optional_as_of(artifact, temporal_index, &worktree, symbol_id, args)
}

fn resolve_symbol_for_optional_as_of_current_worktree_with_client(
    client: &dyn GraphQueryClient,
    symbol_id: &str,
    args: &Value,
) -> Result<String, CodeGraphError> {
    let Some(as_of) = parse_as_of(args)? else {
        return Ok(symbol_id.to_string());
    };
    let worktree = current_worktree()?;
    let commits = load_commit_index_for_request(&worktree)?;
    let temporal_index = client.temporal_index();
    temporal_resolution_symbol_id(
        symbol_id,
        &as_of,
        resolve_symbol_as_of(
            temporal_index.artifact(),
            Some(Arc::clone(&temporal_index)),
            &commits,
            symbol_id,
            &as_of,
        )?,
    )
}

fn parse_as_of(args: &Value) -> Result<Option<String>, McpHandlerError> {
    match args.get("as_of") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(McpHandlerError::InvalidParams(
            "field 'as_of' must not be empty".to_string(),
        )),
        Some(_) => Err(McpHandlerError::InvalidParams(
            "field 'as_of' must be a string".to_string(),
        )),
    }
}

fn reachable_commits(
    commits: &CommitIndexArtifact,
    as_of: &str,
) -> Result<HashSet<String>, McpHandlerError> {
    if !commits.commits.iter().any(|commit| commit.sha == as_of) {
        return Err(McpHandlerError::InvalidParams(format!(
            "as_of commit `{as_of}` is not indexed"
        )));
    }

    let mut reachable = HashSet::new();
    let mut stack = vec![as_of.to_string()];
    while let Some(sha) = stack.pop() {
        if !reachable.insert(sha.clone()) {
            continue;
        }
        let commit = commits
            .commits
            .iter()
            .find(|commit| commit.sha == sha)
            .ok_or_else(|| {
                McpHandlerError::Internal(format!(
                    "commit index references missing parent commit `{sha}`"
                ))
            })?;
        stack.extend(commit.parents.iter().cloned());
    }

    Ok(reachable)
}

fn temporal_resolution_symbol_id(
    symbol_id: &str,
    as_of: &str,
    resolution: Resolution<String>,
) -> Result<String, CodeGraphError> {
    match resolution {
        Resolution::Found { value, .. } => Ok(value),
        Resolution::Deleted { last_seen } => {
            Err(deleted_resolution_error(symbol_id, as_of, last_seen))
        }
        Resolution::Ambiguous { candidates } => {
            Err(ambiguous_resolution_error(symbol_id, as_of, candidates))
        }
        Resolution::Unknown { reason } => Err(unknown_resolution_error(symbol_id, as_of, reason)),
    }
}

fn resolve_symbol_as_of(
    artifact: &GraphIndexArtifact,
    temporal_index: Option<Arc<TemporalIndex>>,
    commits: &CommitIndexArtifact,
    symbol_id: &str,
    as_of: &str,
) -> Result<Resolution<String>, CodeGraphError> {
    let temporal_index =
        temporal_index.unwrap_or_else(|| Arc::new(TemporalIndex::new(Arc::new(artifact.clone()))));
    if !commits.commits.iter().any(|commit| commit.sha == as_of) {
        return Err(McpHandlerError::InvalidParams(format!(
            "as_of commit `{as_of}` is not indexed"
        ))
        .into());
    }

    let history = symbol_history(temporal_index.as_ref(), commits, symbol_id);
    if history.is_empty() {
        return Err(McpHandlerError::NotFound(format!(
            "symbol {symbol_id} has no temporal history in graph artifact"
        ))
        .into());
    }

    let mut last_unknown = None;
    for (_, _, key) in history {
        match resolve_symbol_at_indexed(
            temporal_index.as_ref(),
            commits,
            &key.stable_symbol_id,
            &key.commit,
            as_of,
        ) {
            Resolution::Found { value, chain } => return Ok(Resolution::Found { value, chain }),
            Resolution::Deleted { last_seen } => return Ok(Resolution::Deleted { last_seen }),
            Resolution::Ambiguous { candidates } => {
                return Ok(Resolution::Ambiguous { candidates });
            }
            Resolution::Unknown { reason } => {
                last_unknown = Some(reason);
            }
        }
    }

    if let Some(reason) = last_unknown {
        return Ok(Resolution::Unknown { reason });
    }

    Err(McpHandlerError::NotFound(format!(
        "symbol {symbol_id} not present at commit `{as_of}`"
    ))
    .into())
}

fn format_resolution_failure(reason: &ResolutionFailure) -> String {
    match reason {
        ResolutionFailure::AnchorCommitNotIndexed(commit) => {
            format!("anchor commit `{commit}` is not indexed")
        }
        ResolutionFailure::SymbolNotPresentAtAnchor => {
            "symbol is not present at anchor commit".to_string()
        }
        ResolutionFailure::IndexCorrupt(message) => format!("index corrupt: {message}"),
    }
}

enum CodeSelectorResolution {
    Resolved(String),
    Ambiguous(Vec<CandidateRow>),
}

enum CodeReadSymbolTarget {
    Resolved(GraphSymbolArtifact),
    Ambiguous(Vec<CandidateRow>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnAmbiguousMode {
    Candidates,
    Error,
}

#[allow(dead_code)]
fn resolve_code_selector(
    args: &Value,
    artifact: &GraphIndexArtifact,
) -> Result<CodeSelectorResolution, McpHandlerError> {
    let selector = selected_code_selector(args)?;
    let on_ambiguous = on_ambiguous_mode(args)?;

    match resolve_selector(artifact, selector) {
        SelectorResolution::Resolved(resolved) => {
            Ok(CodeSelectorResolution::Resolved(resolved.stable_symbol_id))
        }
        SelectorResolution::Ambiguous { candidates: _ }
            if on_ambiguous == OnAmbiguousMode::Error =>
        {
            Err(McpHandlerError::InvalidParams(format!(
                "selector `{selector}` is ambiguous; choose one candidate selector or uri"
            )))
        }
        SelectorResolution::Ambiguous { candidates } => {
            Ok(CodeSelectorResolution::Ambiguous(candidates))
        }
        SelectorResolution::NotFound => Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(selector)
        ))),
    }
}

fn resolve_code_selector_with_client(
    args: &Value,
    client: &dyn GraphQueryClient,
) -> Result<CodeSelectorResolution, McpHandlerError> {
    let selector = selected_code_selector(args)?;
    let on_ambiguous = on_ambiguous_mode(args)?;

    match client
        .resolve_selector(selector)
        .map_err(graph_query_error)?
    {
        SelectorResolution::Resolved(resolved) => {
            Ok(CodeSelectorResolution::Resolved(resolved.stable_symbol_id))
        }
        SelectorResolution::Ambiguous { candidates: _ }
            if on_ambiguous == OnAmbiguousMode::Error =>
        {
            Err(McpHandlerError::InvalidParams(format!(
                "selector `{selector}` is ambiguous; choose one candidate selector or uri"
            )))
        }
        SelectorResolution::Ambiguous { candidates } => {
            Ok(CodeSelectorResolution::Ambiguous(candidates))
        }
        SelectorResolution::NotFound => Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(selector)
        ))),
    }
}

fn selected_code_selector(args: &Value) -> Result<&str, McpHandlerError> {
    let selector = string_arg(args, "selector")?;
    let symbol = string_arg(args, "symbol")?;

    match (selector, symbol) {
        (Some(selector), Some(_)) => {
            tracing::warn!(
                "code graph request included deprecated `symbol` with `selector`; using `selector`"
            );
            Ok(selector)
        }
        (Some(selector), None) => Ok(selector),
        (None, Some(symbol)) => {
            tracing::warn!("code graph request used deprecated `symbol`; use `selector`");
            Ok(symbol)
        }
        (None, None) => Err(McpHandlerError::InvalidParams(
            "Missing required field 'selector' (or deprecated 'symbol')".into(),
        )),
    }
}

fn selector_arg(args: &Value) -> Result<&str, McpHandlerError> {
    string_arg(args, "selector")?
        .ok_or_else(|| McpHandlerError::InvalidParams("Missing required field 'selector'".into()))
}

fn file_arg(args: &Value) -> Result<&str, McpHandlerError> {
    string_arg(args, "file")?
        .ok_or_else(|| McpHandlerError::InvalidParams("Missing required field 'file'".into()))
}

fn code_read_symbol_target(
    args: &Value,
    client: &dyn GraphQueryClient,
) -> Result<CodeReadSymbolTarget, McpHandlerError> {
    let stable_symbol_id = string_arg(args, "stable_symbol_id")?;
    let path = string_arg(args, "path")?;
    let name = string_arg(args, "name")?;

    match (stable_symbol_id, path, name) {
        (Some(stable_symbol_id), None, None) => {
            let symbol_id = missing_symbol_label(stable_symbol_id);
            let symbol = symbol_by_id_for_client(client, symbol_id)?;
            Ok(CodeReadSymbolTarget::Resolved(symbol))
        }
        (None, Some(path), Some(name)) => {
            let path = validate_worktree_relative_path_arg("path", path)?;
            resolve_symbol_by_path_name_for_client(client, &path, name)
        }
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(McpHandlerError::InvalidParams(
            "field 'stable_symbol_id' is mutually exclusive with fields 'path' and 'name'".into(),
        )),
        (None, Some(_), None) | (None, None, Some(_)) => Err(McpHandlerError::InvalidParams(
            "fields 'path' and 'name' must be provided together".into(),
        )),
        (None, None, None) => Err(McpHandlerError::InvalidParams(
            "Missing required field 'stable_symbol_id' or fields 'path' and 'name'".into(),
        )),
    }
}

fn resolve_symbol_by_path_name_for_client(
    client: &dyn GraphQueryClient,
    path: &str,
    name: &str,
) -> Result<CodeReadSymbolTarget, McpHandlerError> {
    let matches = client
        .symbols_by_path_name(path, name)
        .map_err(graph_query_error)?;

    match matches.as_slice() {
        [] => Err(McpHandlerError::NotFound(format!(
            "symbol `{name}` in file `{path}` not found in graph artifact"
        ))),
        [symbol] => Ok(CodeReadSymbolTarget::Resolved(symbol.clone())),
        _ => Ok(CodeReadSymbolTarget::Ambiguous(candidate_rows_for_symbols(
            matches.iter(),
        ))),
    }
}

#[derive(Debug)]
struct CodeSearchRequest {
    options: SearchOptions,
    requested_limit: Option<Value>,
}

#[derive(Debug)]
struct CodeTraversalRequest {
    include_unresolved: bool,
}

#[derive(Debug)]
enum CodeSubgraphRoots {
    RootIds(Vec<String>),
    Ambiguous(Vec<CandidateRow>),
}

#[derive(Debug)]
struct CodeSubgraphBudgetRequest {
    budget: SubgraphBudget,
    requested_max_nodes: Option<Value>,
    requested_max_edges: Option<Value>,
}

#[derive(Debug)]
struct ClampedUsizeArg {
    value: usize,
    requested_value: Option<Value>,
}

#[derive(Debug)]
struct LimitArg {
    limit: usize,
    requested_limit: Option<Value>,
}

fn code_search_options(args: &Value) -> Result<CodeSearchRequest, McpHandlerError> {
    let query = query_arg(args)?;
    let mode = search_mode_arg(args)?;
    let symbol_kind = string_arg(args, "symbol_kind")?.map(str::to_string);
    let file = string_arg(args, "file")?
        .map(validate_file_path_arg)
        .transpose()?;
    let file_glob = string_arg(args, "file_glob")?
        .map(validate_file_glob_arg)
        .transpose()?;
    if file.is_some() && file_glob.is_some() {
        return Err(McpHandlerError::InvalidParams(
            "fields 'file' and 'file_glob' are mutually exclusive".into(),
        ));
    }
    let limit = limit_arg(args)?;

    Ok(CodeSearchRequest {
        options: SearchOptions {
            query,
            mode,
            filters: SearchFilters {
                symbol_kind,
                file,
                file_glob,
            },
            limit: limit.limit,
        },
        requested_limit: limit.requested_limit,
    })
}

fn code_traversal_request(args: &Value) -> Result<CodeTraversalRequest, McpHandlerError> {
    Ok(CodeTraversalRequest {
        include_unresolved: bool_arg(args, "include_unresolved")?.unwrap_or(false),
    })
}

#[allow(dead_code)]
fn code_subgraph_root_ids(
    args: &Value,
    artifact: &GraphIndexArtifact,
    temporal_index: Option<Arc<TemporalIndex>>,
) -> Result<CodeSubgraphRoots, CodeGraphError> {
    if let Some(start_nodes) = start_nodes_arg(args)? {
        if string_arg(args, "selector")?.is_some() || string_arg(args, "symbol")?.is_some() {
            return Err(McpHandlerError::InvalidParams(
                "field 'start_nodes' is mutually exclusive with 'selector' and 'symbol'".into(),
            )
            .into());
        }
        for node_id in &start_nodes {
            ensure_symbol_id_exists(artifact, node_id)?;
        }
        return Ok(CodeSubgraphRoots::RootIds(start_nodes));
    }

    match resolve_code_selector(args, artifact)? {
        CodeSelectorResolution::Resolved(symbol_id) => Ok(CodeSubgraphRoots::RootIds(vec![
            resolve_symbol_for_optional_as_of_current_worktree(
                artifact,
                temporal_index,
                &symbol_id,
                args,
            )?,
        ])),
        CodeSelectorResolution::Ambiguous(candidates) => {
            Ok(CodeSubgraphRoots::Ambiguous(candidates))
        }
    }
}

fn code_subgraph_root_ids_with_client(
    args: &Value,
    client: &dyn GraphQueryClient,
) -> Result<CodeSubgraphRoots, CodeGraphError> {
    if let Some(start_nodes) = start_nodes_arg(args)? {
        if string_arg(args, "selector")?.is_some() || string_arg(args, "symbol")?.is_some() {
            return Err(McpHandlerError::InvalidParams(
                "field 'start_nodes' is mutually exclusive with 'selector' and 'symbol'".into(),
            )
            .into());
        }
        for node_id in &start_nodes {
            ensure_symbol_id_exists_with_client(client, node_id)?;
        }
        return Ok(CodeSubgraphRoots::RootIds(start_nodes));
    }

    match resolve_code_selector_with_client(args, client)? {
        CodeSelectorResolution::Resolved(symbol_id) => Ok(CodeSubgraphRoots::RootIds(vec![
            resolve_symbol_for_optional_as_of_current_worktree_with_client(
                client, &symbol_id, args,
            )?,
        ])),
        CodeSelectorResolution::Ambiguous(candidates) => {
            Ok(CodeSubgraphRoots::Ambiguous(candidates))
        }
    }
}

fn start_nodes_arg(args: &Value) -> Result<Option<Vec<String>>, McpHandlerError> {
    let Some(value) = args.get("start_nodes") else {
        return Ok(None);
    };
    let nodes = value.as_array().ok_or_else(|| {
        McpHandlerError::InvalidParams("field 'start_nodes' must be an array of strings".into())
    })?;
    if nodes.is_empty() {
        return Err(McpHandlerError::InvalidParams(
            "field 'start_nodes' must contain at least one node id".into(),
        ));
    }

    let mut seen = HashSet::new();
    let mut start_nodes = Vec::new();
    for node in nodes {
        let node = node.as_str().ok_or_else(|| {
            McpHandlerError::InvalidParams("field 'start_nodes' must be an array of strings".into())
        })?;
        if node.trim().is_empty() {
            return Err(McpHandlerError::InvalidParams(
                "field 'start_nodes' must not contain empty node ids".into(),
            ));
        }
        let node_id = missing_symbol_label(node);
        if node_id.trim().is_empty() {
            return Err(McpHandlerError::InvalidParams(
                "field 'start_nodes' must not contain empty node ids".into(),
            ));
        }
        let node_id = node_id.to_string();
        if seen.insert(node_id.clone()) {
            start_nodes.push(node_id);
        }
    }

    Ok(Some(start_nodes))
}

#[allow(dead_code)]
fn ensure_symbol_id_exists(
    artifact: &GraphIndexArtifact,
    symbol_id: &str,
) -> Result<(), McpHandlerError> {
    if artifact
        .symbols
        .iter()
        .any(|symbol| symbol.stable_symbol_id == symbol_id)
    {
        Ok(())
    } else {
        Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(symbol_id)
        )))
    }
}

fn ensure_symbol_id_exists_with_client(
    client: &dyn GraphQueryClient,
    symbol_id: &str,
) -> Result<(), McpHandlerError> {
    if client
        .symbol_by_id(symbol_id)
        .map_err(graph_query_error)?
        .is_some()
    {
        Ok(())
    } else {
        Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(symbol_id)
        )))
    }
}

#[derive(Debug)]
struct OwnedSubgraphView {
    nodes: Vec<GraphSymbolArtifact>,
    edges: Vec<GraphEdgeArtifact>,
    truncated_frontier: Vec<String>,
    truncated: bool,
}

fn client_bounded_subgraph_with_budget(
    client: &dyn GraphQueryClient,
    root_ids: &[&str],
    radius: u8,
    edge_kinds: Option<&[GraphEdgeKind]>,
    include_unresolved: bool,
    budget: SubgraphBudget,
) -> Result<OwnedSubgraphView, McpHandlerError> {
    let mut traversal =
        ClientSubgraphTraversal::new(client, edge_kinds, include_unresolved, budget);
    for root_id in root_ids {
        traversal.seed_root(root_id)?;
    }
    traversal.run(radius.min(MAX_MCP_CODE_SUBGRAPH_RADIUS))?;
    Ok(traversal.finish())
}

struct ClientSubgraphTraversal<'a, 'k> {
    client: &'a dyn GraphQueryClient,
    edge_kinds: Option<&'k [GraphEdgeKind]>,
    include_unresolved: bool,
    budget: SubgraphBudget,
    nodes: Vec<GraphSymbolArtifact>,
    edges: Vec<GraphEdgeArtifact>,
    visited_nodes: HashSet<String>,
    visited_edges: HashSet<String>,
    queue: VecDeque<(String, u8)>,
    truncated_frontier: Vec<String>,
    frontier_seen: HashSet<String>,
    truncated: bool,
}

impl<'a, 'k> ClientSubgraphTraversal<'a, 'k> {
    fn new(
        client: &'a dyn GraphQueryClient,
        edge_kinds: Option<&'k [GraphEdgeKind]>,
        include_unresolved: bool,
        budget: SubgraphBudget,
    ) -> Self {
        Self {
            client,
            edge_kinds,
            include_unresolved,
            budget,
            nodes: Vec::new(),
            edges: Vec::new(),
            visited_nodes: HashSet::new(),
            visited_edges: HashSet::new(),
            queue: VecDeque::new(),
            truncated_frontier: Vec::new(),
            frontier_seen: HashSet::new(),
            truncated: false,
        }
    }

    fn seed_root(&mut self, root_id: &str) -> Result<(), McpHandlerError> {
        if self.visited_nodes.contains(root_id) {
            return Ok(());
        }
        let Some(root) = self
            .client
            .symbol_by_id(root_id)
            .map_err(graph_query_error)?
        else {
            return Ok(());
        };
        if self.node_budget_full() {
            self.truncated = true;
            self.add_frontier(root_id);
            return Ok(());
        }

        self.visited_nodes.insert(root_id.to_string());
        self.nodes.push(root);
        self.queue.push_back((root_id.to_string(), 0));
        Ok(())
    }

    fn run(&mut self, radius: u8) -> Result<(), McpHandlerError> {
        while let Some((current_id, depth)) = self.queue.pop_front() {
            if depth >= radius {
                continue;
            }
            self.expand_node(&current_id, depth)?;
        }
        Ok(())
    }

    fn expand_node(&mut self, current_id: &str, depth: u8) -> Result<(), McpHandlerError> {
        for record in self.client.find_callee_edges(current_id) {
            match record {
                OwnedCalleeRecord::Resolved { symbol, edge } => {
                    if edge_matches_subgraph_filter(&edge, self.edge_kinds) {
                        self.try_add_neighbor_edge(edge, symbol, depth + 1);
                    }
                }
                OwnedCalleeRecord::Unresolved { edge, .. } => {
                    if self.include_unresolved
                        && edge_matches_subgraph_filter(&edge, self.edge_kinds)
                    {
                        self.try_add_edge(edge);
                    }
                }
            }
        }

        for record in self.client.find_caller_edges(current_id) {
            match record {
                OwnedCallerRecord::Resolved { caller, edge }
                | OwnedCallerRecord::Unresolved { caller, edge, .. } => {
                    if record_is_unresolved(&edge) && !self.include_unresolved {
                        continue;
                    }
                    if edge_matches_subgraph_filter(&edge, self.edge_kinds) {
                        self.try_add_neighbor_edge(edge, caller, depth + 1);
                    }
                }
            }
        }
        Ok(())
    }

    fn try_add_neighbor_edge(
        &mut self,
        edge: GraphEdgeArtifact,
        symbol: GraphSymbolArtifact,
        depth: u8,
    ) {
        let neighbor_id = symbol.stable_symbol_id.clone();
        if self.visited_nodes.contains(&neighbor_id) {
            self.try_add_edge(edge);
            return;
        }

        let edge_key = subgraph_edge_key(&edge);
        let edge_seen = self.visited_edges.contains(&edge_key);
        if self.node_budget_full() || (!edge_seen && self.edge_budget_full()) {
            self.truncated = true;
            self.add_frontier(&neighbor_id);
            return;
        }

        if !edge_seen {
            self.visited_edges.insert(edge_key);
            self.edges.push(edge);
        }
        self.visited_nodes.insert(neighbor_id.clone());
        self.nodes.push(symbol);
        self.queue.push_back((neighbor_id, depth));
    }

    fn try_add_edge(&mut self, edge: GraphEdgeArtifact) -> bool {
        let edge_key = subgraph_edge_key(&edge);
        if self.visited_edges.contains(&edge_key) {
            return true;
        }
        if self.edge_budget_full() {
            self.truncated = true;
            return false;
        }

        self.visited_edges.insert(edge_key);
        self.edges.push(edge);
        true
    }

    fn node_budget_full(&self) -> bool {
        self.nodes.len() >= self.budget.max_nodes
    }

    fn edge_budget_full(&self) -> bool {
        self.edges.len() >= self.budget.max_edges
    }

    fn add_frontier(&mut self, symbol_id: &str) {
        if self.visited_nodes.contains(symbol_id) {
            return;
        }
        if self.frontier_seen.insert(symbol_id.to_string()) {
            self.truncated_frontier.push(symbol_id.to_string());
        }
    }

    fn finish(self) -> OwnedSubgraphView {
        OwnedSubgraphView {
            nodes: self.nodes,
            edges: self.edges,
            truncated_frontier: self.truncated_frontier,
            truncated: self.truncated,
        }
    }
}

fn record_is_unresolved(edge: &GraphEdgeArtifact) -> bool {
    edge.target_stable_symbol_id.is_none()
}

fn edge_matches_subgraph_filter(
    edge: &GraphEdgeArtifact,
    edge_kinds: Option<&[GraphEdgeKind]>,
) -> bool {
    edge_kinds.is_none_or(|kinds| kinds.contains(&edge_kind(edge)))
}

fn subgraph_edge_key(edge: &GraphEdgeArtifact) -> String {
    format!(
        "{}\0{}\0{}\0{:?}\0{:?}\0{}\0{:?}\0{}\0{:?}",
        edge.source_stable_symbol_id,
        edge.target_stable_symbol_id.as_deref().unwrap_or_default(),
        edge.target_label.as_deref().unwrap_or_default(),
        edge.relation,
        edge.confidence,
        edge.confidence_score.to_bits(),
        edge.change_kind,
        edge.edge_kind.map(edge_kind_str).unwrap_or(""),
        edge.bind_method
    )
}

fn code_subgraph_budget(args: &Value) -> Result<CodeSubgraphBudgetRequest, McpHandlerError> {
    let max_nodes = clamped_usize_arg(
        args,
        "max_nodes",
        DEFAULT_MCP_CODE_SUBGRAPH_MAX_NODES,
        MIN_MCP_CODE_SUBGRAPH_MAX_NODES,
        MAX_MCP_CODE_SUBGRAPH_MAX_NODES,
    )?;
    let max_edges = clamped_usize_arg(
        args,
        "max_edges",
        DEFAULT_MCP_CODE_SUBGRAPH_MAX_EDGES,
        MIN_MCP_CODE_SUBGRAPH_MAX_EDGES,
        MAX_MCP_CODE_SUBGRAPH_MAX_EDGES,
    )?;

    Ok(CodeSubgraphBudgetRequest {
        budget: SubgraphBudget {
            max_nodes: max_nodes.value,
            max_edges: max_edges.value,
        },
        requested_max_nodes: max_nodes.requested_value,
        requested_max_edges: max_edges.requested_value,
    })
}

fn clamped_usize_arg(
    args: &Value,
    field: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<ClampedUsizeArg, McpHandlerError> {
    let Some(value) = args.get(field) else {
        return Ok(ClampedUsizeArg {
            value: default,
            requested_value: None,
        });
    };
    if let Some(limit) = value.as_i64() {
        let clamped = limit.clamp(min as i64, max as i64);
        return Ok(ClampedUsizeArg {
            value: clamped as usize,
            requested_value: (limit != clamped).then(|| json!(limit)),
        });
    }
    if let Some(limit) = value.as_u64() {
        let clamped = limit.clamp(min as u64, max as u64);
        return Ok(ClampedUsizeArg {
            value: clamped as usize,
            requested_value: (limit != clamped).then(|| json!(limit)),
        });
    }

    Err(McpHandlerError::InvalidParams(format!(
        "field '{field}' must be an integer"
    )))
}

fn code_subgraph_metadata(
    radius: u8,
    truncated: bool,
    budget: &CodeSubgraphBudgetRequest,
) -> Value {
    let mut metadata = json!({
        "radius": radius,
        "max_nodes": budget.budget.max_nodes,
        "max_edges": budget.budget.max_edges,
        "truncated": truncated,
    });
    if let Some(requested_max_nodes) = &budget.requested_max_nodes {
        metadata["requested_max_nodes"] = requested_max_nodes.clone();
    }
    if let Some(requested_max_edges) = &budget.requested_max_edges {
        metadata["requested_max_edges"] = requested_max_edges.clone();
    }
    metadata
}

fn symbol_id_arg(args: &Value) -> Result<String, McpHandlerError> {
    let value = args
        .get("symbol")
        .ok_or_else(|| McpHandlerError::InvalidParams("Missing required field 'symbol'".into()))?;
    let s = value
        .as_str()
        .ok_or_else(|| McpHandlerError::InvalidParams("field 'symbol' must be a string".into()))?
        .trim();
    if s.is_empty() {
        return Err(McpHandlerError::InvalidParams(
            "field 'symbol' must not be empty".into(),
        ));
    }
    // Strip the URI prefix if present so callers work with bare IDs.
    Ok(missing_symbol_label(s).to_string())
}

fn query_arg(args: &Value) -> Result<String, McpHandlerError> {
    let value = args
        .get("query")
        .ok_or_else(|| McpHandlerError::InvalidParams("Missing required field 'query'".into()))?;
    let query = value
        .as_str()
        .ok_or_else(|| McpHandlerError::InvalidParams("field 'query' must be a string".into()))?
        .trim();
    if query.is_empty() {
        return Err(McpHandlerError::InvalidParams(
            "field 'query' must not be empty".into(),
        ));
    }
    Ok(query.to_string())
}

fn search_mode_arg(args: &Value) -> Result<SearchMode, McpHandlerError> {
    let Some(value) = args.get("mode") else {
        return Ok(SearchMode::Substring);
    };
    match value.as_str() {
        Some("exact") => Ok(SearchMode::Exact),
        Some("prefix") => Ok(SearchMode::Prefix),
        Some("substring") => Ok(SearchMode::Substring),
        Some(other) => Err(McpHandlerError::InvalidParams(format!(
            "invalid mode `{other}`; expected `exact`, `prefix`, or `substring`"
        ))),
        None => Err(McpHandlerError::InvalidParams(
            "field 'mode' must be a string".into(),
        )),
    }
}

fn limit_arg(args: &Value) -> Result<LimitArg, McpHandlerError> {
    let Some(value) = args.get("limit") else {
        return Ok(LimitArg {
            limit: 20,
            requested_limit: None,
        });
    };
    if let Some(limit) = value.as_i64() {
        let clamped = limit.clamp(1, 200);
        return Ok(LimitArg {
            limit: clamped as usize,
            requested_limit: (limit != clamped).then(|| json!(limit)),
        });
    }
    if let Some(limit) = value.as_u64() {
        let clamped = limit.clamp(1, 200);
        return Ok(LimitArg {
            limit: clamped as usize,
            requested_limit: (limit != clamped).then(|| json!(limit)),
        });
    }
    Err(McpHandlerError::InvalidParams(
        "field 'limit' must be an integer".into(),
    ))
}

fn search_mode_str(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Exact => "exact",
        SearchMode::Prefix => "prefix",
        SearchMode::Substring => "substring",
    }
}

fn validate_file_path_arg(file: &str) -> Result<String, McpHandlerError> {
    validate_worktree_relative_path_arg("file", file)
}

fn validate_file_glob_arg(file_glob: &str) -> Result<String, McpHandlerError> {
    validate_worktree_relative_path_arg("file_glob", file_glob)
}

fn validate_worktree_relative_path_arg(
    field: &str,
    value: &str,
) -> Result<String, McpHandlerError> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(McpHandlerError::InvalidParams(format!(
            "field '{field}' must be a worktree-relative path"
        )));
    }

    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(McpHandlerError::InvalidParams(format!(
                        "field '{field}' must be a UTF-8 path"
                    )));
                };
                normalized.push(part);
            }
            Component::CurDir => {
                return Err(McpHandlerError::InvalidParams(format!(
                    "field '{field}' must not contain '.' path components"
                )));
            }
            Component::ParentDir => {
                return Err(McpHandlerError::InvalidParams(format!(
                    "field '{field}' must not contain '..' path components"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(McpHandlerError::InvalidParams(format!(
                    "field '{field}' must be a worktree-relative path"
                )));
            }
        }
    }

    let normalized = normalized.join("/");
    if normalized != value {
        return Err(McpHandlerError::InvalidParams(format!(
            "field '{field}' must be a normalized worktree-relative path without '.' or '..' components"
        )));
    }

    Ok(normalized)
}

fn string_arg<'a>(args: &'a Value, field: &str) -> Result<Option<&'a str>, McpHandlerError> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    let value = value.as_str().ok_or_else(|| {
        McpHandlerError::InvalidParams(format!("field '{field}' must be a string"))
    })?;
    if value.trim().is_empty() {
        return Err(McpHandlerError::InvalidParams(format!(
            "field '{field}' must not be empty"
        )));
    }
    Ok(Some(value))
}

fn bool_arg(args: &Value, field: &str) -> Result<Option<bool>, McpHandlerError> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| McpHandlerError::InvalidParams(format!("field '{field}' must be a boolean")))
}

fn on_ambiguous_mode(args: &Value) -> Result<OnAmbiguousMode, McpHandlerError> {
    let Some(value) = args.get("on_ambiguous") else {
        return Ok(OnAmbiguousMode::Candidates);
    };
    match value.as_str() {
        Some("candidates") => Ok(OnAmbiguousMode::Candidates),
        Some("error") => Ok(OnAmbiguousMode::Error),
        Some(other) => Err(McpHandlerError::InvalidParams(format!(
            "invalid on_ambiguous `{other}`; expected `candidates` or `error`"
        ))),
        None => Err(McpHandlerError::InvalidParams(
            "field 'on_ambiguous' must be a string".into(),
        )),
    }
}

fn missing_symbol_label(selector: &str) -> &str {
    selector
        .strip_prefix(CODE_SYMBOL_URI_PREFIX)
        .unwrap_or(selector)
}

fn parse_edge_kinds(args: &Value) -> Result<Option<Vec<GraphEdgeKind>>, McpHandlerError> {
    let Some(value) = args.get("edge_kinds") else {
        return Ok(None);
    };
    let kinds = value.as_array().ok_or_else(|| {
        McpHandlerError::InvalidParams("field 'edge_kinds' must be an array of strings".to_string())
    })?;
    kinds
        .iter()
        .map(|kind| {
            let kind = kind.as_str().ok_or_else(|| {
                McpHandlerError::InvalidParams(
                    "field 'edge_kinds' must be an array of strings".to_string(),
                )
            })?;
            serde_json::from_value::<GraphEdgeKind>(Value::String(kind.to_string())).map_err(
                |_| {
                    McpHandlerError::InvalidParams(format!(
                        "invalid edge kind `{kind}`; expected one of calls, calls_dyn, references_hof, references_other"
                    ))
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[allow(dead_code)]
fn resolve_candidate_rows(
    artifact: &GraphIndexArtifact,
    selector: &str,
) -> Result<Vec<CandidateRow>, McpHandlerError> {
    match resolve_selector(artifact, selector) {
        SelectorResolution::Resolved(resolved) => {
            let symbol = symbol_by_id(artifact, &resolved.stable_symbol_id)?;
            Ok(vec![candidate_row_for_symbol(symbol)])
        }
        SelectorResolution::Ambiguous { candidates } => Ok(candidates),
        SelectorResolution::NotFound => Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(selector)
        ))),
    }
}

fn resolve_candidate_rows_for_client(
    client: &dyn GraphQueryClient,
    selector: &str,
) -> Result<Vec<CandidateRow>, McpHandlerError> {
    match client
        .resolve_selector(selector)
        .map_err(graph_query_error)?
    {
        SelectorResolution::Resolved(resolved) => {
            let symbol = symbol_by_id_for_client(client, &resolved.stable_symbol_id)?;
            Ok(vec![candidate_row_for_symbol(&symbol)])
        }
        SelectorResolution::Ambiguous { candidates } => Ok(candidates),
        SelectorResolution::NotFound => Err(McpHandlerError::NotFound(format!(
            "symbol {} not found in graph artifact",
            missing_symbol_label(selector)
        ))),
    }
}

fn symbol_by_id_for_client(
    client: &dyn GraphQueryClient,
    symbol_id: &str,
) -> Result<GraphSymbolArtifact, McpHandlerError> {
    client
        .symbol_by_id(symbol_id)
        .map_err(graph_query_error)?
        .ok_or_else(|| {
            McpHandlerError::Internal(format!(
                "resolved symbol id `{symbol_id}` missing from graph artifact"
            ))
        })
}

fn graph_query_error(error: anyhow::Error) -> McpHandlerError {
    McpHandlerError::Internal(format!("failed to query graph artifact: {error}"))
}

fn symbol_by_id<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Result<&'a GraphSymbolArtifact, McpHandlerError> {
    artifact
        .symbols
        .iter()
        .find(|symbol| symbol.stable_symbol_id == symbol_id)
        .ok_or_else(|| {
            McpHandlerError::Internal(format!(
                "resolved symbol id `{symbol_id}` missing from graph artifact"
            ))
        })
}

fn read_indexed_file_bytes(
    worktree: &Path,
    file_path: &str,
    content_oid: &str,
) -> Result<Vec<u8>, McpHandlerError> {
    if content_oid.starts_with("gitlink:") {
        return Err(McpHandlerError::Internal(format!(
            "indexed source for `{file_path}` points to gitlink `{content_oid}`"
        )));
    }
    if let Some(bytes) = read_git_blob(worktree, content_oid)? {
        return Ok(bytes);
    }

    let current = read_current_file_bytes(worktree, file_path)?;
    if git_blob_oid(&current) == content_oid {
        return Ok(current);
    }

    Err(McpHandlerError::Internal(format!(
        "indexed blob `{content_oid}` for `{file_path}` is not available in git object storage"
    )))
}

fn read_git_blob(worktree: &Path, content_oid: &str) -> Result<Option<Vec<u8>>, McpHandlerError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["cat-file", "-p", content_oid])
        .output()
        .map_err(|error| {
            McpHandlerError::Internal(format!(
                "failed to read indexed blob `{content_oid}`: {error}"
            ))
        })?;

    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        Ok(None)
    }
}

fn current_file_oid(worktree: &Path, file_path: &str) -> Result<Option<String>, McpHandlerError> {
    match fs::read(worktree.join(file_path)) {
        Ok(bytes) => Ok(Some(git_blob_oid(&bytes))),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => {
            Ok(None)
        }
        Err(error) => Err(McpHandlerError::Internal(format!(
            "failed to read current file `{}`: {error}",
            worktree.join(file_path).display()
        ))),
    }
}

fn read_current_file_bytes(worktree: &Path, file_path: &str) -> Result<Vec<u8>, McpHandlerError> {
    fs::read(worktree.join(file_path)).map_err(|error| {
        McpHandlerError::Internal(format!(
            "failed to read current file `{}` while resolving indexed blob: {error}",
            worktree.join(file_path).display()
        ))
    })
}

fn source_range_with_context(
    source: &str,
    symbol: &GraphSymbolArtifact,
    context_lines: usize,
) -> [usize; 2] {
    let line_count = source.split_inclusive('\n').count();
    let symbol_start = symbol.line_range[0].max(1);
    let symbol_end = symbol.line_range[1].max(symbol_start);
    let start = symbol_start.saturating_sub(context_lines).max(1);
    let end = symbol_end
        .saturating_add(context_lines)
        .min(line_count)
        .max(start.saturating_sub(1));
    [start, end]
}

fn source_for_line_range(source: &str, line_range: [usize; 2]) -> String {
    let [start, end] = line_range;
    source
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, line)| {
            let line_no = index + 1;
            (start <= line_no && line_no <= end).then_some(line)
        })
        .collect()
}

fn candidate_rows_for_symbols<'a>(
    symbols: impl IntoIterator<Item = &'a GraphSymbolArtifact>,
) -> Vec<CandidateRow> {
    let mut rows = symbols
        .into_iter()
        .map(candidate_row_for_symbol)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line_range[0].cmp(&right.line_range[0]))
            .then_with(|| left.line_range[1].cmp(&right.line_range[1]))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            .then_with(|| left.id.cmp(&right.id))
    });
    rows
}

fn candidate_row_for_symbol(symbol: &GraphSymbolArtifact) -> CandidateRow {
    let uri = format!("{CODE_SYMBOL_URI_PREFIX}{}", symbol.stable_symbol_id);
    let selector = selector_for_symbol_row(
        &uri,
        &symbol.file_path,
        &symbol.qualified_name,
        &symbol.symbol_kind,
    );

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

fn candidate_row_for_search_symbol(symbol: &SearchSymbol) -> CandidateRow {
    let uri = format!("{CODE_SYMBOL_URI_PREFIX}{}", symbol.stable_symbol_id);
    let selector = selector_for_symbol_row(
        &uri,
        &symbol.file_path,
        &symbol.qualified_name,
        &symbol.symbol_kind,
    );

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

fn ambiguous_response(candidates: Vec<CandidateRow>) -> Value {
    json!({
        "ambiguous": true,
        "candidates": candidates.into_iter().map(candidate_row).collect::<Vec<_>>(),
    })
}

fn source_ambiguous_response(candidates: Vec<CandidateRow>) -> Value {
    json!({
        "ambiguous": true,
        "candidates": candidates.into_iter().map(source_candidate_row).collect::<Vec<_>>(),
    })
}

#[allow(dead_code)]
async fn with_graph_metadata(artifact: &GraphIndexArtifact, mut body: Value) -> Value {
    let files = response_file_set_from_body(artifact, &body);
    GraphResponseMetadata::from_artifact_with_files(artifact, &files)
        .await
        .insert_into(&mut body);
    body
}

fn response_file_set_from_body(
    artifact: &GraphIndexArtifact,
    body: &Value,
) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    collect_response_file_paths(body, &mut paths);
    let mut files = response_file_set_for_paths(artifact, paths);
    if files.is_empty() {
        if let Some(symbol_id) = response_symbol_id(body) {
            if let Ok(symbol) = symbol_by_id(artifact, symbol_id) {
                files = response_file_set_for_paths(artifact, [symbol.file_path.as_str()]);
            }
        }
    }
    files
}

fn response_file_set_from_client(
    client: &dyn GraphQueryClient,
    body: &Value,
) -> Result<Vec<(String, String)>, McpHandlerError> {
    let mut paths = Vec::new();
    collect_response_file_paths(body, &mut paths);
    let mut files = response_file_set_for_client_paths(client, paths)?;
    if files.is_empty() {
        if let Some(symbol_id) = response_symbol_id(body) {
            if let Some(symbol) = client.symbol_by_id(symbol_id).map_err(graph_query_error)? {
                files = response_file_set_for_client_paths(client, [symbol.file_path.as_str()])?;
            }
        }
    }
    Ok(files)
}

fn response_file_set_for_client_paths<'a>(
    client: &dyn GraphQueryClient,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<(String, String)>, McpHandlerError> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for path in paths {
        if !seen.insert(path.to_string()) {
            continue;
        }
        if let Some(manifest) = client
            .file_manifest_by_path(path)
            .map_err(graph_query_error)?
        {
            files.push((manifest.path, manifest.content_oid));
        }
    }
    Ok(files)
}

fn search_response_file_set_for_parquet(
    client: &ParquetClient,
    result: &SearchResult,
    options: &SearchOptions,
) -> Result<Vec<(String, String)>, McpHandlerError> {
    let file_oids = client.file_oids().map_err(|error| {
        McpHandlerError::Internal(format!(
            "failed to read graph file manifests from `{}`: {error}",
            client.dir().display()
        ))
    })?;
    if result.candidates.is_empty() {
        if let Some(file) = options.filters.file.as_deref() {
            return Ok(file_oid_subset(&file_oids, [file]));
        }
        return Ok(file_oids);
    }
    Ok(file_oid_subset(
        &file_oids,
        result
            .candidates
            .iter()
            .map(|symbol| symbol.file_path.as_str()),
    ))
}

fn file_oid_subset<'a>(
    file_oids: &[(String, String)],
    paths: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for path in paths {
        if !seen.insert(path.to_string()) {
            continue;
        }
        if let Some((path, oid)) = file_oids.iter().find(|(candidate, _)| candidate == path) {
            files.push((path.clone(), oid.clone()));
        }
    }
    files
}

fn empty_code_search_file_set(
    artifact: &GraphIndexArtifact,
    body: &Value,
) -> Option<Vec<(String, String)>> {
    let candidates = body.get("candidates")?.as_array()?;
    if !candidates.is_empty() || body.get("query").is_none() || body.get("total_matches").is_none()
    {
        return None;
    }

    if let Some(file) = body.get("file").and_then(Value::as_str) {
        return Some(response_file_set_for_paths(artifact, [file]));
    }

    Some(all_indexed_file_set(artifact))
}

fn all_indexed_file_set(artifact: &GraphIndexArtifact) -> Vec<(String, String)> {
    artifact
        .file_manifests
        .iter()
        .map(|entry| (entry.path.clone(), entry.content_oid.clone()))
        .collect()
}

fn response_file_set_for_paths<'a>(
    artifact: &GraphIndexArtifact,
    paths: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for path in paths {
        if !seen.insert(path.to_string()) {
            continue;
        }
        let Some(indexed_oid) = indexed_file_oid_for_path(artifact, path) else {
            continue;
        };
        files.push((path.to_string(), indexed_oid.to_string()));
    }
    files
}

fn collect_response_file_paths<'a>(value: &'a Value, paths: &mut Vec<&'a str>) {
    match value {
        Value::Object(map) => {
            if let Some(file_path) = map.get("file_path").and_then(Value::as_str) {
                paths.push(file_path);
            }
            if let Some(files) = map.get("files").and_then(Value::as_array) {
                paths.extend(files.iter().filter_map(Value::as_str));
            }
            for value in map.values() {
                collect_response_file_paths(value, paths);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_response_file_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn response_symbol_id(value: &Value) -> Option<&str> {
    value
        .get("symbol")
        .and_then(Value::as_str)
        .map(missing_symbol_label)
}

fn indexed_file_oid_for_path<'a>(artifact: &'a GraphIndexArtifact, path: &str) -> Option<&'a str> {
    artifact
        .file_manifests
        .iter()
        .find(|entry| entry.path == path)
        .map(|entry| entry.content_oid.as_str())
}

#[derive(Clone)]
pub(crate) struct OverlayChangedPaths {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) identity: Option<overlay_snapshot::SnapshotIdentity>,
    pub(crate) path_state: BTreeMap<String, overlay_snapshot::OverlayPathState>,
}

pub fn overlay_changed_oids(
    worktree: &Path,
    base_files: Vec<(String, String)>,
) -> anyhow::Result<BTreeMap<PathBuf, [u8; 20]>> {
    let changed = overlay_changed_oid_hex(worktree, base_files)?;
    Ok(changed
        .into_iter()
        .map(|(path, oid)| {
            let bytes = oid.as_deref().and_then(parse_sha1_oid).unwrap_or([0; 20]);
            (PathBuf::from(path), bytes)
        })
        .collect())
}

fn parse_sha1_oid(hex: &str) -> Option<[u8; 20]> {
    if hex.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let value = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
        out[index] = value;
    }
    Some(out)
}

#[allow(dead_code)]
pub(crate) fn changed_paths_for_overlay(
    worktree: &Path,
    base_files: Vec<(String, String)>,
) -> anyhow::Result<OverlayChangedPaths> {
    let base = overlay_snapshot::SnapshotBase::compatibility(base_files.into_iter().collect());
    changed_paths_for_overlay_base(worktree, base, false)
}

fn changed_paths_for_overlay_base(
    worktree: &Path,
    base: overlay_snapshot::SnapshotBase,
    overlay_fsmonitor_auto: bool,
) -> anyhow::Result<OverlayChangedPaths> {
    #[cfg(any(test, feature = "test-support"))]
    EXACT_OVERLAY_OBSERVATIONS_FOR_TEST.fetch_add(1, Ordering::SeqCst);

    let allowed_extensions = crate::extract::languages::all_supported_extensions();
    let base_file_oids = base.file_oids.clone();
    let (changed, identity, path_state) = if crate::git::detect(worktree).is_some() {
        let capabilities = overlay_capabilities_for_worktree(worktree, overlay_fsmonitor_auto);
        let snapshot = overlay_snapshot::snapshot_with_capabilities(
            worktree,
            base,
            &allowed_extensions,
            capabilities,
        )?;
        let identity = snapshot.identity.clone();
        let changed = snapshot.changed_oid_hex();
        (changed, identity, snapshot.path_state)
    } else {
        let worktree = worktree.canonicalize().map_err(|error| {
            anyhow::anyhow!("failed to canonicalize `{}`: {error}", worktree.display())
        })?;
        let current_oids = current_file_oids_via_fs(&worktree, &allowed_extensions)?;
        let changed = overlay_changed_oid_hex_from_maps(base.file_oids, current_oids)?;
        let path_state = changed
            .iter()
            .map(|(path, oid)| {
                let state = match oid {
                    Some(oid) if base_file_oids.contains_key(path) => {
                        overlay_snapshot::OverlayPathState::Tracked(oid.clone())
                    }
                    Some(oid) => overlay_snapshot::OverlayPathState::Untracked(oid.clone()),
                    None => overlay_snapshot::OverlayPathState::Deleted,
                };
                (path.clone(), state)
            })
            .collect();
        (changed, None, path_state)
    };
    Ok(OverlayChangedPaths {
        paths: changed.keys().map(PathBuf::from).collect(),
        identity,
        path_state,
    })
}

fn overlay_changed_oid_hex(
    worktree: &Path,
    base_files: Vec<(String, String)>,
) -> anyhow::Result<BTreeMap<String, Option<String>>> {
    let worktree = worktree.canonicalize().map_err(|error| {
        anyhow::anyhow!("failed to canonicalize `{}`: {error}", worktree.display())
    })?;
    let allowed_extensions = crate::extract::languages::all_supported_extensions();
    let base_oids = base_files.into_iter().collect::<BTreeMap<_, _>>();
    if crate::git::detect(&worktree).is_some() {
        let base = overlay_snapshot::SnapshotBase::compatibility(base_oids);
        Ok(overlay_snapshot::snapshot_with_capabilities(
            &worktree,
            base,
            &allowed_extensions,
            production_overlay_capabilities(),
        )?
        .changed_oid_hex())
    } else {
        let current_oids = current_file_oids_via_fs(&worktree, &allowed_extensions)?;
        overlay_changed_oid_hex_from_maps(base_oids, current_oids)
    }
}

fn production_overlay_capabilities() -> crate::git::FsmonitorCapabilities {
    crate::git::FsmonitorCapabilities {
        // Task 6 owns the formal p95/correctness release gate. Task 3 exposes
        // the observation seam but must not enable native fsmonitor routing.
        release_enabled: false,
        built_in_supported: false,
        local_filesystem: true,
        watcher_healthy: false,
    }
}

fn overlay_capabilities_for_worktree(
    worktree: &Path,
    overlay_fsmonitor_auto: bool,
) -> crate::git::FsmonitorCapabilities {
    overlay_capabilities_for_worktree_with_probe(
        worktree,
        overlay_fsmonitor_auto,
        crate::git::probe_fsmonitor_capabilities,
    )
}

fn overlay_capabilities_for_worktree_with_probe(
    worktree: &Path,
    overlay_fsmonitor_auto: bool,
    probe: impl FnOnce(&Path, bool, bool) -> crate::git::FsmonitorCapabilities,
) -> crate::git::FsmonitorCapabilities {
    if overlay_fsmonitor_auto {
        probe(worktree, true, true)
    } else {
        production_overlay_capabilities()
    }
}

fn overlay_changed_oid_hex_from_maps(
    base_oids: BTreeMap<String, String>,
    current_oids: BTreeMap<String, String>,
) -> anyhow::Result<BTreeMap<String, Option<String>>> {
    let mut changed = BTreeMap::<String, Option<String>>::new();

    for (rel_path, content_oid) in &current_oids {
        if base_oids
            .get(rel_path)
            .is_none_or(|base_oid| base_oid != content_oid)
        {
            changed.insert(rel_path.clone(), Some(content_oid.clone()));
        }
    }

    for base_path in base_oids.keys() {
        if !current_oids.contains_key(base_path) {
            changed.insert(base_path.clone(), None);
        }
    }

    Ok(changed)
}

fn current_file_oids_via_fs(
    worktree: &Path,
    allowed_extensions: &[&str],
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut current_oids = BTreeMap::new();
    for (rel_path, path) in supported_file_paths_via_fs(worktree, allowed_extensions)? {
        let bytes = fs::read(&path)
            .map_err(|error| anyhow::anyhow!("failed to read `{}`: {error}", path.display()))?;
        current_oids.insert(rel_path, git_blob_oid(&bytes));
    }
    Ok(current_oids)
}

fn supported_file_paths_via_fs(
    worktree: &Path,
    allowed_extensions: &[&str],
) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let canonical_worktree = worktree
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", worktree.display()))?;
    let mut paths = Vec::new();
    for entry in WalkBuilder::new(&canonical_worktree)
        .standard_filters(true)
        .hidden(false)
        .filter_entry(fallback_should_descend)
        .build()
    {
        let entry = entry.map_err(|error| {
            anyhow::anyhow!(
                "strict filesystem fallback traversal failed under `{}`: {error}",
                canonical_worktree.display()
            )
        })?;
        if let Some(error) = entry.error() {
            return Err(anyhow::anyhow!(
                "strict filesystem fallback entry `{}` reported an attached traversal error under `{}`: {error}",
                entry.path().display(),
                canonical_worktree.display()
            ));
        }
        // Filesystem walker entries normally carry a file type. If a future
        // backend omits it, strict certification cannot safely decide whether
        // the path belongs in the complete supported-file set, so fail closed.
        let file_type = entry.file_type().ok_or_else(|| {
            anyhow::anyhow!(
                "strict filesystem fallback entry `{}` had no file type under `{}`",
                entry.path().display(),
                canonical_worktree.display()
            )
        })?;
        if file_type.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    allowed_extensions
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                })
        {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    collect_supported_file_paths(&canonical_worktree, paths)
}

fn fallback_should_descend(entry: &DirEntry) -> bool {
    let Some(file_name) = entry.file_name().to_str() else {
        return true;
    };
    !matches!(file_name, "target" | ".git" | "node_modules")
}

fn collect_supported_file_paths(
    canonical_worktree: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let mut collected = BTreeMap::new();
    for path in paths {
        let relative = strict_worktree_relative_slash_path(canonical_worktree, &path)?;
        if let Some(previous) = collected.get(&relative) {
            return Err(anyhow::anyhow!(
                "duplicate normalized filesystem fallback path `{relative}` from {previous:?} and {path:?}"
            ));
        }
        collected.insert(relative, path);
    }
    Ok(collected)
}

fn strict_worktree_relative_slash_path(
    canonical_worktree: &Path,
    path: &Path,
) -> anyhow::Result<String> {
    let relative = path.strip_prefix(canonical_worktree).with_context(|| {
        format!(
            "filesystem fallback path {path:?} is outside canonical worktree {canonical_worktree:?}"
        )
    })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(anyhow::anyhow!(
                "filesystem fallback path {path:?} has a non-normal relative component"
            ));
        };
        components.push(value.to_str().ok_or_else(|| {
            anyhow::anyhow!("filesystem fallback path {path:?} is not valid UTF-8")
        })?);
    }
    if components.is_empty() {
        return Err(anyhow::anyhow!(
            "filesystem fallback path {path:?} does not name a worktree-relative file"
        ));
    }
    // Native separators cannot occur inside a Normal component, so joining
    // validated UTF-8 components with '/' is injective for walker-produced
    // paths. The collector still rejects repeats explicitly to keep that
    // contract fail-closed if its inputs or normalization ever change.
    Ok(components.join("/"))
}

#[cfg(test)]
fn current_file_oids_via_git(
    worktree: &Path,
    allowed_extensions: &[&str],
) -> anyhow::Result<BTreeMap<String, String>> {
    let dirty_paths: BTreeSet<String> = crate::git::status_dirty_paths(worktree)?
        .into_iter()
        .filter(|entry| overlay_path_has_supported_extension(&entry.path, allowed_extensions))
        .map(|entry| entry.path)
        .collect();

    let mut current_oids = BTreeMap::new();
    for tracked in crate::git::ls_files_with_oids(worktree)? {
        if tracked.is_gitlink
            || !overlay_path_has_supported_extension(&tracked.path, allowed_extensions)
        {
            continue;
        }
        let content_oid = if dirty_paths.contains(&tracked.path) {
            match read_overlay_worktree_content_oid(worktree, &tracked.path)? {
                Some(content_oid) => content_oid,
                // Deleted in the worktree (or replaced by a directory): omit so
                // the base-path pass can record a tombstone.
                None => continue,
            }
        } else {
            // Clean tracked file: index blob oid matches worktree bytes and is
            // enough to detect divergence from a stale graph index.
            tracked.content_oid
        };
        current_oids.insert(tracked.path, content_oid);
    }

    for path in &dirty_paths {
        if current_oids.contains_key(path) {
            continue;
        }
        if let Some(content_oid) = read_overlay_worktree_content_oid(worktree, path)? {
            current_oids.insert(path.clone(), content_oid);
        }
    }

    Ok(current_oids)
}

#[cfg(test)]
fn read_overlay_worktree_content_oid(
    worktree: &Path,
    path: &str,
) -> anyhow::Result<Option<String>> {
    let abs = worktree.join(path);
    match fs::read(&abs) {
        Ok(bytes) => Ok(Some(git_blob_oid(&bytes))),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => {
            Ok(None)
        }
        Err(error) => Err(anyhow::anyhow!(
            "failed to read `{}`: {error}",
            abs.display()
        )),
    }
}

#[cfg(test)]
fn overlay_path_has_supported_extension(path: &str, allowed_extensions: &[&str]) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed_extensions
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn worktree_relative_slash_path(worktree: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(worktree).unwrap_or(path);
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn scoped_worktree_root() -> Option<PathBuf> {
    SCOPED_CODE_GRAPH_WORKTREE_ROOT.try_with(Clone::clone).ok()
}

fn current_worktree_root() -> Option<std::path::PathBuf> {
    scoped_worktree_root().or_else(|| std::env::current_dir().ok().map(resolve_worktree_root_from))
}

fn matching_graph_pointer(
    worktree: &Path,
    source: &GraphMetadataSource,
) -> Option<GraphIndexPointer> {
    let pointer_path = worktree.join(GRAPH_POINTER_RELATIVE_PATH);
    let bytes = std::fs::read(pointer_path).ok()?;
    let pointer: GraphIndexPointer = serde_json::from_slice(&bytes).ok()?;
    if pointer.graph_content_hash == source.graph_content_hash
        && pointer.manifest_version == source.manifest_version
    {
        Some(pointer)
    } else {
        None
    }
}

fn graph_built_at_from_pointer(pointer: &GraphIndexPointer) -> Option<String> {
    let modified = std::fs::metadata(&pointer.canonical_artifact_path)
        .ok()?
        .modified()
        .ok()?;
    let built_at = DateTime::<Utc>::from(modified);
    Some(built_at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| (!value.trim().is_empty()).then_some(value))
}

#[derive(Debug, Clone)]
struct WorktreeGitMetadata {
    head_oid: String,
    has_uncommitted_changes: bool,
    supplemental_changed: Vec<String>,
}

async fn worktree_git_metadata(
    worktree: &Path,
    indexed_head_oid: Option<&str>,
) -> Option<WorktreeGitMetadata> {
    let allowed_extensions = overlay_trigger_extensions();
    worktree_git_metadata_with_extensions(worktree, indexed_head_oid, &allowed_extensions, false)
        .await
}

async fn worktree_git_metadata_with_extensions(
    worktree: &Path,
    indexed_head_oid: Option<&str>,
    allowed_extensions: &[&str],
    include_tracked_supplemental_paths: bool,
) -> Option<WorktreeGitMetadata> {
    tokio::time::timeout(GRAPH_GIT_METADATA_TIMEOUT, async {
        let head_oid = run_git_stdout(worktree, &["rev-parse", "HEAD"]).await?;
        let status = run_git_stdout(
            worktree,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .await?;
        let mut status_report = parse_git_status_for_overlay(
            worktree,
            &status,
            allowed_extensions,
            include_tracked_supplemental_paths,
        );

        if indexed_head_oid.is_some_and(|indexed| indexed != head_oid) {
            let indexed_head_oid = indexed_head_oid?;
            let range = format!("{indexed_head_oid}..HEAD");
            let diff = run_git_stdout(worktree, &["diff", "--name-only", "-z", &range]).await?;
            status_report
                .supplemental_changed
                .extend(supported_git_paths(
                    worktree,
                    diff.split('\0'),
                    allowed_extensions,
                ));
        }

        Some(WorktreeGitMetadata {
            head_oid,
            has_uncommitted_changes: status_report.has_uncommitted_changes,
            supplemental_changed: status_report.supplemental_changed.into_iter().collect(),
        })
    })
    .await
    .ok()
    .flatten()
}

struct GitStatusOverlayReport {
    has_uncommitted_changes: bool,
    supplemental_changed: BTreeSet<String>,
}

fn parse_git_status_for_overlay(
    worktree: &Path,
    status: &str,
    allowed_extensions: &[&str],
    include_tracked_supplemental_paths: bool,
) -> GitStatusOverlayReport {
    let mut has_uncommitted_changes = false;
    let mut supplemental_changed = BTreeSet::new();
    let mut entries = status.split('\0').filter(|entry| !entry.is_empty());

    while let Some(entry) = entries.next() {
        let Some(status_code) = entry.get(..2) else {
            continue;
        };
        let path = entry.get(2..).unwrap_or_default().trim_start();
        let supported_path = supported_git_path(worktree, path, allowed_extensions);
        if status_code == "??" {
            if supported_path.is_some() {
                has_uncommitted_changes = true;
            }
        } else {
            has_uncommitted_changes = true;
        }
        let include_supplemental = status_code == "??"
            || (include_tracked_supplemental_paths
                && status_code_has_supplemental_path(status_code));
        if include_supplemental {
            if let Some(path) = supported_path {
                supplemental_changed.insert(path);
            }
        }

        if status_code.starts_with('R') || status_code.starts_with('C') {
            let _ = entries.next();
        }
    }

    GitStatusOverlayReport {
        has_uncommitted_changes,
        supplemental_changed,
    }
}

fn status_code_has_supplemental_path(status_code: &str) -> bool {
    status_code
        .bytes()
        .any(|status| matches!(status, b'A' | b'D' | b'R' | b'C'))
}

fn overlay_trigger_extensions() -> Vec<&'static str> {
    crate::extract::languages::all_supported_extensions()
        .into_iter()
        .filter(|extension| *extension != "md")
        .collect()
}

fn supported_git_paths<'a>(
    worktree: &Path,
    paths: impl IntoIterator<Item = &'a str>,
    allowed_extensions: &[&str],
) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| !path.is_empty())
        .filter_map(move |path| supported_git_path(worktree, path, allowed_extensions))
        .collect()
}

fn supported_git_path(worktree: &Path, path: &str, allowed_extensions: &[&str]) -> Option<String> {
    let extension = Path::new(path).extension()?.to_str()?;
    if !allowed_extensions
        .iter()
        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    {
        return None;
    }
    Some(worktree_relative_slash_path(worktree, &worktree.join(path)))
}

fn markdown_relative_path(path: &Path) -> Option<String> {
    is_markdown_path(path).then(|| {
        path.components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    })
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            MARKDOWN_OVERLAY_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn supplemental_changed_oids(worktree: &Path, paths: &[String]) -> BTreeMap<PathBuf, [u8; 20]> {
    paths
        .iter()
        .map(|path| {
            let current_oid = fs::read(worktree.join(path))
                .ok()
                .and_then(|bytes| file_oid_cache::parse_git_oid(&git_blob_oid(&bytes)))
                .unwrap_or([0; 20]);
            (PathBuf::from(path), current_oid)
        })
        .collect()
}

fn git_stdout_command(worktree: &Path, args: &[&str]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("git");
    command
        .args(args)
        .current_dir(worktree)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .kill_on_drop(true);
    command
}

async fn run_git_stdout(worktree: &Path, args: &[&str]) -> Option<String> {
    let mut command = git_stdout_command(worktree, args);
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

fn compute_worktree_dirty(
    indexed_head_oid: Option<&str>,
    worktree_head_oid: &str,
    has_uncommitted_changes: bool,
) -> Option<bool> {
    if has_uncommitted_changes {
        Some(true)
    } else {
        indexed_head_oid.map(|indexed_head_oid| indexed_head_oid != worktree_head_oid)
    }
}

fn candidate_row(candidate: CandidateRow) -> Value {
    let external_origin = is_external_symbol_kind(&candidate.symbol_kind)
        .then(|| external_origin_for_qualified_name(&candidate.qualified_name));
    let mut row = json!({
        "selector": candidate.selector,
        "uri": candidate.uri,
        "id": candidate.id,
        "entity_name": candidate.entity_name,
        "qualified_name": candidate.qualified_name,
        "file_path": candidate.file_path,
        "line_range": candidate.line_range,
        "symbol_kind": candidate.symbol_kind,
        "enclosing_scope": candidate.enclosing_scope,
    });
    if let Some(origin) = external_origin {
        row["origin"] = Value::String(origin);
        row["external"] = Value::Bool(true);
        row["workspace_symbol"] = Value::Bool(false);
    }
    row
}

fn source_candidate_row(candidate: CandidateRow) -> Value {
    json!({
        "id": candidate.id,
        "name": candidate.qualified_name,
        "file": candidate.file_path,
        "range": {
            "start": candidate.line_range[0],
            "end": candidate.line_range[1],
        },
        "kind": candidate.symbol_kind,
        "scope": candidate.enclosing_scope,
    })
}

fn source_symbol_response(
    symbol: &GraphSymbolArtifact,
    source: String,
    source_range: [usize; 2],
    file_oid: String,
    context_lines: &ClampedUsizeArg,
    stale: bool,
    source_origin: Option<&str>,
) -> Value {
    let mut body = json!({
        "id": symbol.stable_symbol_id,
        "name": symbol.qualified_name,
        "file": symbol.file_path,
        "range": {
            "start": source_range[0],
            "end": source_range[1],
        },
        "source": source,
        "file_oid": file_oid,
    });
    if context_lines.value != 0 {
        body["context_lines"] = json!(context_lines.value);
    }
    if let Some(requested_context_lines) = &context_lines.requested_value {
        body["requested_context_lines"] = requested_context_lines.clone();
    }
    if stale {
        body["stale"] = Value::Bool(true);
    }
    if let Some(origin) = source_origin {
        body["source_origin"] = Value::String(origin.to_owned());
    }
    body
}

fn symbol_info_row(symbol: &GraphSymbolArtifact) -> Value {
    json!({
        "qualified_name": symbol.qualified_name,
        "entity_name": symbol.entity_name,
        "file_path": symbol.file_path,
        "line_range": symbol.line_range,
        "symbol_kind": symbol.symbol_kind,
        "enclosing_scope": symbol.enclosing_scope,
        "uri": symbol_uri(&symbol.stable_symbol_id),
        "id": symbol.stable_symbol_id,
    })
}

fn is_external_symbol_kind(symbol_kind: &str) -> bool {
    symbol_kind == "external"
}

fn selector_for_symbol_row(
    uri: &str,
    file_path: &str,
    qualified_name: &str,
    symbol_kind: &str,
) -> String {
    if qualified_name.is_empty() || is_external_symbol_kind(symbol_kind) {
        uri.to_owned()
    } else {
        format!("{file_path}::{qualified_name}")
    }
}

fn bodyless_external_symbol_response(symbol: &GraphSymbolArtifact) -> Value {
    json!({
        "symbol": symbol_info_row(symbol),
        "origin": external_origin_for_qualified_name(&symbol.qualified_name),
        "path": symbol.qualified_name,
        "kind": symbol.symbol_kind,
        "body_status": "no indexed body - Tier-3",
        "bodyless": true,
    })
}

fn external_origin_for_qualified_name(qualified_name: &str) -> String {
    let origin = qualified_name
        .split([':', '.', '/'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(qualified_name);
    match origin {
        "std" | "core" | "alloc" => "std".to_owned(),
        other => other.to_owned(),
    }
}

fn symbol_row(symbol: &GraphSymbolArtifact) -> Value {
    json!({
        "uri": symbol_uri(&symbol.stable_symbol_id),
        "entity_name": symbol.entity_name,
        "enclosing_scope": symbol.enclosing_scope,
        "file_path": symbol.file_path,
        "line_range": symbol.line_range,
        "symbol_kind": symbol.symbol_kind,
    })
}

#[derive(Default)]
struct TableFileInterner {
    files: Vec<String>,
    indexes: HashMap<String, usize>,
}

impl TableFileInterner {
    fn intern(&mut self, file_path: &str) -> usize {
        if let Some(index) = self.indexes.get(file_path) {
            return *index;
        }
        let index = self.files.len();
        self.files.push(file_path.to_string());
        self.indexes.insert(file_path.to_string(), index);
        index
    }

    fn into_files(self) -> Vec<String> {
        self.files
    }
}

fn table_response(mut body: Value, files: TableFileInterner) -> Value {
    body["response_format"] = json!("table");
    body["files"] = json!(files.into_files());
    body
}

fn candidate_table(candidates: Vec<CandidateRow>, files: &mut TableFileInterner) -> Value {
    let rows = candidates
        .into_iter()
        .map(|candidate| {
            let file_index = files.intern(&candidate.file_path);
            json!([
                candidate.id,
                candidate.entity_name,
                candidate.qualified_name,
                file_index,
                candidate.line_range[0],
                candidate.line_range[1],
                candidate.symbol_kind,
                candidate.enclosing_scope,
            ])
        })
        .collect::<Vec<_>>();
    json!({
        "cols": [
            "id",
            "entity_name",
            "qualified_name",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
            "enclosing_scope",
        ],
        "rows": rows,
    })
}

fn symbol_table<'a>(
    symbols: impl IntoIterator<Item = &'a GraphSymbolArtifact>,
    files: &mut TableFileInterner,
) -> Value {
    let rows = symbols
        .into_iter()
        .map(|symbol| {
            let file_index = files.intern(&symbol.file_path);
            json!([
                symbol.stable_symbol_id,
                symbol.entity_name,
                symbol.enclosing_scope,
                file_index,
                symbol.line_range[0],
                symbol.line_range[1],
                symbol.symbol_kind,
            ])
        })
        .collect::<Vec<_>>();
    json!({
        "cols": [
            "id",
            "entity_name",
            "enclosing_scope",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
        ],
        "rows": rows,
    })
}

fn owned_caller_table(callers: Vec<OwnedCallerRecord>, files: &mut TableFileInterner) -> Value {
    traversal_table(
        callers
            .into_iter()
            .map(|caller| owned_caller_table_row(caller, files)),
    )
}

fn owned_callee_table(callees: Vec<OwnedCalleeRecord>, files: &mut TableFileInterner) -> Value {
    traversal_table(
        callees
            .into_iter()
            .map(|callee| owned_callee_table_row(callee, files)),
    )
}

fn traversal_table(rows: impl IntoIterator<Item = Value>) -> Value {
    json!({
        "cols": [
            "symbol_id",
            "entity_name",
            "enclosing_scope",
            "file",
            "line_start",
            "line_end",
            "symbol_kind",
            "resolved",
            "target_label",
            "edge_kind",
            "confidence",
            "bind_method",
        ],
        "rows": rows.into_iter().collect::<Vec<_>>(),
    })
}

fn owned_caller_table_row(caller: OwnedCallerRecord, files: &mut TableFileInterner) -> Value {
    match caller {
        OwnedCallerRecord::Resolved { caller, edge } => {
            symbol_traversal_table_row(&caller, &edge, true, None, files)
        }
        OwnedCallerRecord::Unresolved {
            caller,
            edge,
            target_label,
        } => symbol_traversal_table_row(&caller, &edge, false, Some(target_label), files),
    }
}

fn owned_callee_table_row(callee: OwnedCalleeRecord, files: &mut TableFileInterner) -> Value {
    match callee {
        OwnedCalleeRecord::Resolved { symbol, edge } => {
            symbol_traversal_table_row(&symbol, &edge, true, None, files)
        }
        OwnedCalleeRecord::Unresolved { edge, target_label } => {
            unresolved_traversal_table_row(&edge, target_label)
        }
    }
}

fn symbol_traversal_table_row(
    symbol: &GraphSymbolArtifact,
    edge: &GraphEdgeArtifact,
    resolved: bool,
    target_label: Option<String>,
    files: &mut TableFileInterner,
) -> Value {
    let file_index = files.intern(&symbol.file_path);
    let (confidence, bind_method) = compact_traversal_edge_values(edge);
    json!([
        symbol.stable_symbol_id,
        symbol.entity_name,
        symbol.enclosing_scope,
        file_index,
        symbol.line_range[0],
        symbol.line_range[1],
        symbol.symbol_kind,
        resolved,
        target_label,
        edge_kind_str(edge_kind(edge)),
        confidence,
        bind_method,
    ])
}

fn unresolved_traversal_table_row(edge: &GraphEdgeArtifact, target_label: String) -> Value {
    let entity_name = target_label.clone();
    let (confidence, bind_method) = compact_traversal_edge_values(edge);
    json!([
        null,
        entity_name,
        null,
        null,
        null,
        null,
        null,
        false,
        target_label,
        edge_kind_str(edge_kind(edge)),
        confidence,
        bind_method,
    ])
}

fn compact_traversal_edge_values(edge: &GraphEdgeArtifact) -> (Value, Value) {
    let confidence = if edge.bind_method.is_some() || edge_kind(edge) == GraphEdgeKind::CallsDyn {
        json!(edge.confidence)
    } else {
        Value::Null
    };
    let bind_method = edge
        .bind_method
        .as_ref()
        .map(|value| json!(value))
        .unwrap_or(Value::Null);
    (confidence, bind_method)
}

fn edge_table<'a>(edges: impl IntoIterator<Item = &'a GraphEdgeArtifact>) -> Value {
    let rows = edges
        .into_iter()
        .map(|edge| {
            json!([
                edge.source_stable_symbol_id,
                edge.target_stable_symbol_id,
                edge.target_label,
                edge.target_stable_symbol_id.is_some(),
                edge.relation,
                edge_kind_str(edge_kind(edge)),
                edge.confidence,
                edge.confidence_score,
                edge.bind_method,
            ])
        })
        .collect::<Vec<_>>();
    json!({
        "cols": [
            "source_id",
            "target_id",
            "target_label",
            "resolved",
            "relation",
            "edge_kind",
            "confidence",
            "confidence_score",
            "bind_method",
        ],
        "rows": rows,
    })
}

#[derive(Debug)]
struct TraversalSummary {
    counts_by_kind: Value,
    counts_by_context: Value,
    unresolved_sample: Vec<String>,
}

fn classify_file_context(file_path: &str) -> &'static str {
    if file_path.contains("/tests/")
        || file_path.starts_with("tests/")
        || file_path.contains("/benches/")
        || file_path.starts_with("benches/")
    {
        if file_path.contains("/benches/") || file_path.starts_with("benches/") {
            "bench"
        } else {
            "test"
        }
    } else {
        "production"
    }
}

fn owned_caller_summary(records: &[OwnedCallerRecord]) -> TraversalSummary {
    let unresolved = records.iter().filter_map(|record| match record {
        OwnedCallerRecord::Unresolved { target_label, .. } => Some(target_label.as_str()),
        OwnedCallerRecord::Resolved { .. } => None,
    });
    let mut summary = traversal_summary(records.iter().map(OwnedCallerRecord::edge), unresolved);
    let mut production = 0usize;
    let mut test = 0usize;
    let mut bench = 0usize;
    for record in records {
        let file_path = match record {
            OwnedCallerRecord::Resolved { caller, .. }
            | OwnedCallerRecord::Unresolved { caller, .. } => &caller.file_path,
        };
        match classify_file_context(file_path) {
            "production" => production += 1,
            "test" => test += 1,
            "bench" => bench += 1,
            _ => {}
        }
    }
    summary.counts_by_context = json!({
        "production": production,
        "test": test,
        "bench": bench,
    });
    summary
}

fn owned_callee_summary(records: &[OwnedCalleeRecord]) -> TraversalSummary {
    let unresolved = records.iter().filter_map(|record| match record {
        OwnedCalleeRecord::Unresolved { target_label, .. } => Some(target_label.as_str()),
        OwnedCalleeRecord::Resolved { .. } => None,
    });
    let mut summary = traversal_summary(records.iter().map(OwnedCalleeRecord::edge), unresolved);
    let mut production = 0usize;
    let mut test = 0usize;
    let mut bench = 0usize;
    for record in records {
        let file_path = match record {
            OwnedCalleeRecord::Resolved { symbol, .. } => &symbol.file_path,
            OwnedCalleeRecord::Unresolved { .. } => continue,
        };
        match classify_file_context(file_path) {
            "production" => production += 1,
            "test" => test += 1,
            "bench" => bench += 1,
            _ => {}
        }
    }
    summary.counts_by_context = json!({
        "production": production,
        "test": test,
        "bench": bench,
    });
    summary
}

fn traversal_summary<'a>(
    edges: impl IntoIterator<Item = &'a GraphEdgeArtifact>,
    unresolved_labels: impl IntoIterator<Item = &'a str>,
) -> TraversalSummary {
    let mut calls = 0usize;
    let mut calls_dyn = 0usize;
    let mut references_hof = 0usize;
    let mut references_other = 0usize;
    let mut references_address = 0usize;
    let mut unresolved = 0usize;

    for edge in edges {
        match edge_kind(edge) {
            GraphEdgeKind::Calls => calls += 1,
            GraphEdgeKind::CallsDyn => calls_dyn += 1,
            GraphEdgeKind::ReferencesHof => references_hof += 1,
            GraphEdgeKind::ReferencesOther => references_other += 1,
            GraphEdgeKind::ReferencesAddress => references_address += 1,
        }
        if edge.target_stable_symbol_id.is_none() {
            unresolved += 1;
        }
    }

    TraversalSummary {
        counts_by_kind: json!({
            "calls": calls,
            "calls_dyn": calls_dyn,
            "references_hof": references_hof,
            "references_other": references_other,
            "references_address": references_address,
            "unresolved": unresolved,
        }),
        counts_by_context: json!({}),
        unresolved_sample: unresolved_sample(unresolved_labels),
    }
}

fn unresolved_sample<'a>(labels: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut sample = Vec::new();
    let mut bytes = 0usize;

    for label in labels {
        if sample.len() >= 5 || !seen.insert(label) {
            continue;
        }
        let next_bytes = bytes + label.len();
        if next_bytes > 120 {
            break;
        }
        bytes = next_bytes;
        sample.push(label.to_string());
    }

    sample
}

fn owned_caller_row(caller: OwnedCallerRecord) -> Value {
    match caller {
        OwnedCallerRecord::Resolved { caller, edge } => {
            let mut row = symbol_row(&caller);
            add_edge_metadata(&mut row, &edge, true, None);
            row
        }
        OwnedCallerRecord::Unresolved {
            caller,
            edge,
            target_label,
        } => {
            let mut row = symbol_row(&caller);
            add_edge_metadata(&mut row, &edge, false, Some(target_label));
            row
        }
    }
}

fn owned_callee_row(callee: OwnedCalleeRecord) -> Value {
    match callee {
        OwnedCalleeRecord::Resolved { symbol, edge } => {
            let mut row = symbol_row(&symbol);
            add_edge_metadata(&mut row, &edge, true, None);
            row
        }
        OwnedCalleeRecord::Unresolved { edge, target_label } => {
            let entity_name = target_label.clone();
            let mut row = json!({
                "resolved": false,
                "entity_name": entity_name,
                "target_label": target_label,
            });
            add_edge_metadata(&mut row, &edge, false, None);
            row
        }
    }
}

fn add_edge_metadata(
    row: &mut Value,
    edge: &GraphEdgeArtifact,
    resolved: bool,
    unresolved_target_label: Option<String>,
) {
    let Some(map) = row.as_object_mut() else {
        return;
    };
    map.insert("resolved".to_string(), Value::Bool(resolved));
    let kind = edge_kind(edge);
    map.insert(
        "edge_kind".to_string(),
        Value::String(edge_kind_str(kind).to_string()),
    );
    if let Some(target_label) = unresolved_target_label {
        map.insert("target_label".to_string(), Value::String(target_label));
    }
    if let Some(bind_method) = &edge.bind_method {
        map.insert(
            "bind_method".to_string(),
            Value::String(bind_method.clone()),
        );
        map.insert("confidence".to_string(), json!(edge.confidence));
    } else if kind == GraphEdgeKind::CallsDyn {
        map.insert("confidence".to_string(), json!(edge.confidence));
    }
}

fn edge_row(edge: &GraphEdgeArtifact) -> Value {
    json!({
        "source_uri": symbol_uri(&edge.source_stable_symbol_id),
        "target_uri": edge.target_stable_symbol_id.as_ref().map(|id| symbol_uri(id)),
        "target_label": edge.target_label,
        "resolved": edge.target_stable_symbol_id.is_some(),
        "relation": edge.relation,
        "edge_kind": edge_kind_str(edge_kind(edge)),
        "confidence": edge.confidence,
        "confidence_score": edge.confidence_score,
        "bind_method": edge.bind_method.clone(),
    })
}

fn edge_kind_str(edge_kind: GraphEdgeKind) -> &'static str {
    match edge_kind {
        GraphEdgeKind::Calls => "calls",
        GraphEdgeKind::CallsDyn => "calls_dyn",
        GraphEdgeKind::ReferencesHof => "references_hof",
        GraphEdgeKind::ReferencesOther => "references_other",
        GraphEdgeKind::ReferencesAddress => "references_address",
    }
}

fn symbol_uri(symbol_id: &str) -> String {
    format!("{CODE_SYMBOL_URI_PREFIX}{symbol_id}")
}

#[allow(dead_code)]
fn mermaid_subgraph(nodes: &[&GraphSymbolArtifact], edges: &[&GraphEdgeArtifact]) -> String {
    mermaid_subgraph_from_iters(nodes.iter().copied(), edges.iter().copied())
}

fn mermaid_subgraph_owned(nodes: &[GraphSymbolArtifact], edges: &[GraphEdgeArtifact]) -> String {
    mermaid_subgraph_from_iters(nodes.iter(), edges.iter())
}

fn mermaid_subgraph_from_iters<'a>(
    nodes: impl IntoIterator<Item = &'a GraphSymbolArtifact>,
    edges: impl IntoIterator<Item = &'a GraphEdgeArtifact>,
) -> String {
    let mut lines = vec!["graph TD".to_string()];
    for symbol in nodes {
        lines.push(format!(
            "    {}[\"{}\"]",
            mermaid_id(&symbol.stable_symbol_id),
            escape_mermaid_label(&symbol.entity_name)
        ));
    }
    for edge in edges {
        let Some(target_id) = edge.target_stable_symbol_id.as_deref() else {
            continue;
        };
        lines.push(format!(
            "    {} --> {}",
            mermaid_id(&edge.source_stable_symbol_id),
            mermaid_id(target_id)
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

fn mermaid_id(symbol_id: &str) -> String {
    symbol_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn escape_mermaid_label(label: &str) -> String {
    label.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    #[cfg(unix)]
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use crate::query_client::{
        parquet_hot_index_build_observations_for_test,
        reset_parquet_hot_index_build_observations_for_test,
    };
    use crate::{
        ChangeKind, CommitArtifact, Confidence, EdgeEndpoint, GraphFileArtifact,
        GraphFileManifestEntry, GraphIndexHeader, NodeId, RelationKind, RenamePrev,
        SymbolSnapshotArtifact, TemporalEdgeArtifact, WalkStrategy,
    };
    use spur_mcp::local_projects::{
        LocalProjectCatalogStore, LocalProjectError, LocalProjectHealth, LocalProjectResolver,
        LocalProjectValidator, ValidatedLocalProject,
    };

    use super::*;

    const ESCALATION_THRESHOLD: usize = 3;
    static REBUILD_TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    #[derive(Default)]
    struct GraphQueryTrace {
        calls: Mutex<BTreeMap<String, usize>>,
    }

    impl GraphQueryTrace {
        fn record(&self, operation: String) {
            *self
                .calls
                .lock()
                .expect("graph query trace mutex poisoned")
                .entry(operation)
                .or_default() += 1;
        }

        fn snapshot(&self) -> BTreeMap<String, usize> {
            self.calls
                .lock()
                .expect("graph query trace mutex poisoned")
                .clone()
        }
    }

    struct CountingGraphQueryClient {
        inner: InMemoryClient,
        trace: Arc<GraphQueryTrace>,
    }

    impl CountingGraphQueryClient {
        fn new(artifact: Arc<GraphIndexArtifact>) -> Self {
            Self {
                inner: InMemoryClient::new(artifact),
                trace: Arc::new(GraphQueryTrace::default()),
            }
        }

        fn trace(&self) -> BTreeMap<String, usize> {
            self.trace.snapshot()
        }

        fn record(&self, operation: impl Into<String>) {
            self.trace.record(operation.into());
        }
    }

    impl GraphQueryClient for CountingGraphQueryClient {
        fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult> {
            // The caller-visible limit is deliberately excluded: overlay search
            // must reuse the same logical base search at its unbounded limit.
            self.record(format!(
                "search_symbols:{:?}:{}:{:?}",
                opts.mode, opts.query, opts.filters
            ));
            self.inner.search_symbols(opts)
        }

        fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord> {
            self.record(format!("find_caller_edges:{sid}"));
            self.inner.find_caller_edges(sid)
        }

        fn find_unresolved_caller_edges_by_labels(
            &self,
            target_labels: &HashSet<String>,
        ) -> Vec<OwnedCallerRecord> {
            let mut labels = target_labels.iter().cloned().collect::<Vec<_>>();
            labels.sort();
            self.record(format!("find_unresolved_caller_edges_by_labels:{labels:?}"));
            self.inner
                .find_unresolved_caller_edges_by_labels(target_labels)
        }

        fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord> {
            self.record(format!("find_callee_edges:{sid}"));
            self.inner.find_callee_edges(sid)
        }

        fn resolve_selector(&self, selector: &str) -> anyhow::Result<SelectorResolution> {
            self.record(format!("resolve_selector:{selector}"));
            self.inner.resolve_selector(selector)
        }

        fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
            self.record(format!("symbol_by_id:{sid}"));
            self.inner.symbol_by_id(sid)
        }

        fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
            self.record(format!("symbols_by_file:{path}"));
            self.inner.symbols_by_file(path)
        }

        fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
            self.record(format!("symbols_by_files:{paths:?}"));
            self.inner.symbols_by_files(paths)
        }

        fn symbols_by_path_name(
            &self,
            path: &str,
            name: &str,
        ) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
            self.record(format!("symbols_by_path_name:{path}:{name}"));
            self.inner.symbols_by_path_name(path, name)
        }

        fn file_manifest_by_path(
            &self,
            path: &str,
        ) -> anyhow::Result<Option<GraphFileManifestEntry>> {
            self.record(format!("file_manifest_by_path:{path}"));
            self.inner.file_manifest_by_path(path)
        }

        fn file_exists(&self, path: &str) -> anyhow::Result<bool> {
            self.record(format!("file_exists:{path}"));
            self.inner.file_exists(path)
        }

        fn temporal_index(&self) -> Arc<TemporalIndex> {
            self.inner.temporal_index()
        }

        fn symbol_history(
            &self,
            commits: &CommitIndexArtifact,
            symbol_id: &str,
        ) -> anyhow::Result<Vec<(crate::temporal::GitSha, ChangeKind, SnapshotKey)>> {
            self.record(format!("symbol_history:{symbol_id}"));
            self.inner.symbol_history(commits, symbol_id)
        }
    }

    fn search_handler_for_request_replay(
        args: &Value,
        client: &dyn GraphQueryClient,
    ) -> CodeGraphResult {
        code_search_with_artifact(args, client).map_err(CodeGraphError::from)
    }

    async fn code_graph_result_signature(result: CodeGraphResult) -> Value {
        match result {
            Ok(body) => json!({ "ok": body }),
            Err(error) => {
                let response = error.into_error_response().await;
                json!({
                    "error": {
                        "code": response.code,
                        "message": response.message,
                        "data": response.data,
                    }
                })
            }
        }
    }

    fn test_internal_error(message: impl Into<String>) -> CodeGraphError {
        CodeGraphError::without_metadata(McpHandlerError::Internal(message.into()))
    }

    #[test]
    fn default_code_tool_timeout_is_fifteen_seconds() {
        assert_eq!(DEFAULT_CODE_TOOL_TIMEOUT, Duration::from_secs(15));
        assert_eq!(
            DEFAULT_CODE_TOOL_TIMEOUT.as_millis(),
            15_000,
            "server deadline matches the solved wall_ms request"
        );
    }

    #[tokio::test]
    async fn code_tool_dispatch_returns_when_the_call_exceeds_the_deadline() {
        let module = GraphMcpModule::default();
        let started = Instant::now();

        let error = CODE_TOOL_TIMEOUT_OVERRIDE
            .scope(Duration::from_millis(30), async {
                CODE_TOOL_DELAY_OVERRIDE
                    .scope(Duration::from_millis(250), async move {
                        module
                            .dispatch("code_resolve", json!({"selector": "missing_symbol"}))
                            .await
                    })
                    .await
            })
            .await
            .expect_err("a call past the deadline returns an error");
        let response = error.into_error_response().await;

        assert_eq!(response.code, CODE_GRAPH_TIMEOUT_ERROR_CODE);
        assert!(
            response.message.contains("code_resolve"),
            "timeout names the tool: {}",
            response.message
        );
        assert_eq!(
            response.data.as_ref().and_then(|data| data.get("kind")),
            Some(&json!("timeout"))
        );
        assert_eq!(
            response
                .data
                .as_ref()
                .and_then(|data| data.get("timeout_ms")),
            Some(&json!(30))
        );
        assert!(
            started.elapsed() < Duration::from_millis(150),
            "deadline returns before the blocked call finishes, elapsed {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn code_tool_dispatch_keeps_the_caller_worktree_under_the_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn deadline_scope_symbol() {}\n");
        write_current_artifact(root, &artifact);
        let module = GraphMcpModule::default();
        let root = root.to_path_buf();

        let body = with_worktree_root_for_request(root, async move {
            module
                .dispatch(
                    "code_symbol_search",
                    json!({"query": "deadline_scope_symbol", "mode": "exact"}),
                )
                .await
        })
        .await
        .expect("scoped search returns inside the deadline");

        assert_eq!(
            body["candidates"][0]["entity_name"],
            "deadline_scope_symbol"
        );
    }

    #[tokio::test]
    async fn code_graph_result_signature_distinguishes_complete_error_metadata() {
        let error_with_hash = |graph_content_hash: &str| {
            CodeGraphError::without_metadata(McpHandlerError::NotFound(
                "symbol still_missing not found in graph artifact".to_owned(),
            ))
            .with_metadata_source(GraphMetadataSource {
                graph_content_hash: graph_content_hash.to_owned(),
                graph_index_version: "4".to_owned(),
                manifest_version: "1".to_owned(),
            })
        };

        let first = code_graph_result_signature(Err(error_with_hash("graph-hash-a"))).await;
        let second = code_graph_result_signature(Err(error_with_hash("graph-hash-b"))).await;

        assert_eq!(first["error"]["code"], CODE_GRAPH_NOT_FOUND_ERROR_CODE);
        assert_eq!(
            first["error"]["message"],
            "symbol still_missing not found in graph artifact"
        );
        assert_eq!(first["error"]["data"]["kind"], "not_found");
        assert_eq!(first["error"]["data"]["graph_content_hash"], "graph-hash-a");
        assert_eq!(
            second["error"]["data"]["graph_content_hash"],
            "graph-hash-b"
        );
        assert_ne!(
            first, second,
            "observable MCP signatures must preserve distinct graph metadata"
        );
    }

    fn response_digest(value: &Value) -> String {
        let bytes = serde_json::to_vec(value).expect("serialize response signature");
        blake3::hash(&bytes).to_hex().to_string()
    }

    type RequestReplayHandler = fn(&Value, &dyn GraphQueryClient) -> CodeGraphResult;

    struct RequestReplayCase {
        name: &'static str,
        args: Value,
        handler: RequestReplayHandler,
        expected_operations: BTreeMap<String, usize>,
    }

    async fn exercise_request_replay_case(
        scenario: &str,
        case: &RequestReplayCase,
        root: &Path,
        base_artifact: Arc<GraphIndexArtifact>,
        oracle_artifact: Arc<GraphIndexArtifact>,
        changed_paths: &[PathBuf],
    ) -> Vec<String> {
        let counting = CountingGraphQueryClient::new(base_artifact);
        let request_client = RequestReplayClient::new(&counting);

        let _first_result = (case.handler)(&case.args, &request_client);
        let overlay = OverlayClient::new(&request_client, root, changed_paths)
            .expect("construct direct overlay subject");
        let actual = code_graph_result_signature((case.handler)(&case.args, &overlay)).await;

        let oracle = InMemoryClient::new(oracle_artifact);
        let expected = code_graph_result_signature((case.handler)(&case.args, &oracle)).await;
        let actual_digest = response_digest(&actual);
        let oracle_digest = response_digest(&expected);
        let equivalent = actual == expected;
        let trace = counting.trace();

        eprintln!(
            "request_replay trace scenario={scenario} tool={} base_calls={trace:?} \
             actual_digest={actual_digest} oracle_digest={oracle_digest} equivalent={equivalent}",
            case.name
        );

        let mut violations = Vec::new();
        if case.expected_operations.is_empty() {
            violations.push(format!(
                "scenario={scenario} tool={} expected operation map must not be empty",
                case.name
            ));
        }
        if trace != case.expected_operations {
            violations.push(format!(
                "scenario={scenario} tool={} operation map mismatch actual={trace:?} expected={:?}",
                case.name, case.expected_operations
            ));
        }
        if !equivalent {
            violations.push(format!(
                "scenario={scenario} tool={} response mismatch actual={actual:#} expected={expected:#}",
                case.name
            ));
        }
        violations
    }

    #[cfg(unix)]
    #[test]
    fn overlay_snapshot_supported_paths_reject_non_utf8_relative_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().canonicalize().expect("canonical root");
        let path = root.join(OsString::from_vec(b"non-utf8-\x80.rs".to_vec()));
        fs::write(&path, "pub fn invalid_path() {}\n").expect("non-UTF-8 source path");

        let error = supported_file_paths_via_fs(&root, &["rs"])
            .expect_err("fallback path conversion must reject non-UTF-8 paths");

        eprintln!("lossless fallback path trace: rejected={}", path.display());
        assert!(
            error.to_string().contains("UTF-8"),
            "unexpected non-UTF-8 path error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn overlay_snapshot_supported_paths_reject_lossy_normalized_collision() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().canonicalize().expect("canonical root");
        let first = root.join(OsString::from_vec(b"collision-\x80.rs".to_vec()));
        let second = root.join(OsString::from_vec(b"collision-\x81.rs".to_vec()));
        fs::write(&first, "pub fn first() {}\n").expect("first source");
        fs::write(&second, "pub fn second() {}\n").expect("second source");

        let error = supported_file_paths_via_fs(&root, &["rs"])
            .expect_err("distinct paths must never overwrite one normalized fallback key");

        eprintln!(
            "lossless fallback collision trace: rejected_first={} rejected_second={}",
            first.display(),
            second.display()
        );
        assert!(
            error.to_string().contains("UTF-8") || error.to_string().contains("duplicate"),
            "unexpected normalized-collision error: {error:#}"
        );
    }

    #[test]
    fn overlay_snapshot_supported_path_collector_rejects_duplicate_key() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().canonicalize().expect("canonical root");
        let path = root.join("duplicate.rs");
        fs::write(&path, "pub fn duplicate() {}\n").expect("source");

        let error = collect_supported_file_paths(&root, [path.clone(), path.clone()])
            .expect_err("the strict collector must reject duplicate normalized keys");

        eprintln!("lossless fallback duplicate-key trace: rejected={path:?}");
        assert!(
            error.to_string().contains("duplicate normalized"),
            "unexpected duplicate-key error: {error:#}"
        );
    }

    #[test]
    fn overlay_snapshot_clean_repeat_avoids_full_index_sweep() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().join("repo");
        fs::create_dir_all(root.join("src")).expect("src dir");
        init_git_repo(&root);
        let source = b"pub fn clean() {}\n";
        fs::write(root.join("src/lib.rs"), source).expect("source");
        run_git_test(&root, &["add", "src/lib.rs"]);
        run_git_test(&root, &["commit", "-qm", "base"]);
        let base = overlay_snapshot::SnapshotBase::compatibility(BTreeMap::from([(
            "src/lib.rs".to_owned(),
            git_blob_oid(source),
        )]));
        let extensions = crate::extract::languages::all_supported_extensions();

        let first =
            overlay_snapshot::snapshot(&root, base.clone(), &extensions).expect("cold snapshot");
        assert!(first.path_state.is_empty(), "fixture must start clean");
        assert_eq!(first.measurements.full_index_sweeps, 1);

        let second = overlay_snapshot::snapshot(&root, base, &extensions).expect("warm snapshot");
        eprintln!(
            "overlay snapshot measurement: cold_full_index_sweeps={} warm_full_index_sweeps={} \
             warm_hashed_paths={} warm_snapshot_reused={}",
            first.measurements.full_index_sweeps,
            second.measurements.full_index_sweeps,
            second.measurements.hashed_paths.len(),
            second.measurements.snapshot_reused,
        );
        assert!(
            second.path_state.is_empty(),
            "clean repeat must remain clean"
        );
        assert_eq!(
            second.measurements.full_index_sweeps, 0,
            "warm unchanged validation must not repeat a full index sweep"
        );
        assert!(second.measurements.hashed_paths.is_empty());
        assert!(second.measurements.snapshot_reused);
    }

    #[test]
    fn overlay_snapshot_production_route_remains_release_disabled() {
        let capabilities = production_overlay_capabilities();
        assert!(!capabilities.release_enabled);
        assert_eq!(
            crate::git::fsmonitor_status_route(capabilities),
            crate::git::FsmonitorStatusRoute::ExactFallback(
                crate::git::FsmonitorFallbackReason::ReleaseDisabled
            )
        );
    }

    #[test]
    fn graph_mcp_deps_default_keeps_overlay_fsmonitor_off() {
        assert!(!GraphMcpDeps::default().overlay_fsmonitor_auto);
    }

    #[test]
    fn overlay_fsmonitor_off_never_probes_and_returns_release_disabled() {
        let capabilities = overlay_capabilities_for_worktree_with_probe(
            Path::new("configured-repo"),
            false,
            |_, _, _| panic!("Off must not probe Git fsmonitor"),
        );

        assert_eq!(
            crate::git::fsmonitor_status_route(capabilities),
            crate::git::FsmonitorStatusRoute::ExactFallback(
                crate::git::FsmonitorFallbackReason::ReleaseDisabled
            )
        );
    }

    #[test]
    fn overlay_fsmonitor_auto_uses_probe_derived_capabilities() {
        let worktree = Path::new("configured-repo");
        let expected = crate::git::FsmonitorCapabilities {
            release_enabled: true,
            built_in_supported: true,
            local_filesystem: true,
            watcher_healthy: false,
        };

        let actual = overlay_capabilities_for_worktree_with_probe(
            worktree,
            true,
            |observed_worktree, release_enabled, local_filesystem| {
                assert_eq!(observed_worktree, worktree);
                assert!(release_enabled);
                assert!(local_filesystem);
                expected
            },
        );

        assert_eq!(actual, expected);
        assert_eq!(
            crate::git::fsmonitor_status_route(actual),
            crate::git::FsmonitorStatusRoute::ExactFallback(
                crate::git::FsmonitorFallbackReason::WatcherUnhealthy
            ),
            "Auto must preserve the probe's exact-fallback decision"
        );
    }

    #[test]
    fn overlay_changed_paths_fingerprint_tracks_all_changed_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("src dir");
        let old_a = b"pub fn alpha() {}\n";
        let old_b = b"pub fn beta() {}\n";
        fs::write(root.join("src/a.rs"), old_a).expect("a.rs");
        fs::write(root.join("src/b.rs"), old_b).expect("b.rs");
        run_git_test(root, &["add", "src"]);
        run_git_test(root, &["commit", "-qm", "base"]);
        let base_files = vec![
            ("src/a.rs".to_owned(), git_blob_oid(old_a)),
            ("src/b.rs".to_owned(), git_blob_oid(old_b)),
        ];

        fs::write(root.join("src/a.rs"), "pub fn alpha_v2() {}\n").expect("edit a.rs");
        fs::write(root.join("src/b.rs"), "pub fn beta_v2() {}\n").expect("edit b.rs");
        let first = changed_paths_for_overlay(root, base_files.clone()).expect("first changes");

        fs::write(root.join("src/b.rs"), "pub fn beta_v3() {}\n").expect("re-edit b.rs");
        let second = changed_paths_for_overlay(root, base_files).expect("second changes");

        assert_eq!(first.paths, second.paths, "the changed path set is stable");
        assert_ne!(
            first.identity, second.identity,
            "content changes in any tracked overlay path must invalidate the cached delta"
        );
    }

    /// Overlay compares disk/index content to *graph* oids, not to HEAD. A clean
    /// `git status` after commits past the indexed HEAD must still surface those
    /// paths — status-only dirty detection would incorrectly return empty.
    #[test]
    fn changed_paths_for_overlay_detects_head_lag_with_clean_status() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("src dir");

        let v1 = b"pub fn alpha_v1() {}\n";
        fs::write(root.join("src/a.rs"), v1).expect("write v1");
        run_git_test(root, &["add", "src/a.rs"]);
        run_git_test(root, &["commit", "-qm", "v1"]);
        let base_files = vec![("src/a.rs".to_owned(), git_blob_oid(v1))];

        let v2 = b"pub fn alpha_v2() {}\n";
        fs::write(root.join("src/a.rs"), v2).expect("write v2");
        run_git_test(root, &["add", "src/a.rs"]);
        run_git_test(root, &["commit", "-qm", "v2"]);

        let dirty = crate::git::status_dirty_paths(root).expect("status");
        assert!(
            dirty.is_empty(),
            "fixture must be status-clean so HEAD-lag is the only signal"
        );

        let changed = changed_paths_for_overlay(root, base_files).expect("changed paths");
        assert_eq!(
            changed.paths,
            vec![PathBuf::from("src/a.rs")],
            "committed divergence from graph index oids must be detected even when status is clean"
        );

        // Fingerprint must reflect *content* change (Some(oid)), not a false deletion.
        // A status-only dirty set would miss the path entirely or mark it deleted.
        let expected = BTreeMap::from([(
            "src/a.rs".to_owned(),
            overlay_snapshot::OverlayPathState::Tracked(git_blob_oid(v2)),
        )]);
        assert_eq!(
            changed
                .identity
                .expect("complete validated identity")
                .normalized_changed_set_fingerprint,
            overlay_snapshot::normalized_changed_set_fingerprint(expected.iter()),
            "HEAD-lag must surface the live blob oid, not a tombstone"
        );
    }

    /// Perf smoke for the git dirty-set path. Enable with `SPUR_PERF_SMOKE=1`.
    /// Baseline before this change was ~1.0–1.3s full-tree discover+hash on this repo.
    #[test]
    fn changed_paths_for_overlay_git_dirty_set_perf_smoke() {
        if std::env::var_os("SPUR_PERF_SMOKE").is_none() {
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root");
        let allowed = crate::extract::languages::all_supported_extensions();
        let base_files: Vec<(String, String)> = crate::git::ls_files_with_oids(&root)
            .expect("ls-files")
            .into_iter()
            .filter(|entry| {
                !entry.is_gitlink && overlay_path_has_supported_extension(&entry.path, &allowed)
            })
            .map(|entry| (entry.path, entry.content_oid))
            .collect();
        let base_len = base_files.len();

        // Warmup
        let _ = changed_paths_for_overlay(&root, base_files.clone()).expect("warmup");

        let iterations = 5_u32;
        let started = Instant::now();
        let mut last_paths = 0_usize;
        for _ in 0..iterations {
            let changed = changed_paths_for_overlay(&root, base_files.clone()).expect("changed");
            last_paths = changed.paths.len();
        }
        let avg = started.elapsed() / iterations;
        eprintln!(
            "changed_paths_for_overlay git dirty-set: avg={avg:?} over {iterations} iters \
             (base_files={base_len}, changed_paths={last_paths})"
        );
        assert!(
            avg < Duration::from_millis(400),
            "git dirty-set path should stay well under the old ~1s full-tree scan; got {avg:?}"
        );
    }

    #[test]
    fn changed_paths_for_overlay_includes_untracked_and_deleted_in_git_repo() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("src dir");

        let kept = b"pub fn kept() {}\n";
        let doomed = b"pub fn doomed() {}\n";
        fs::write(root.join("src/kept.rs"), kept).expect("kept");
        fs::write(root.join("src/doomed.rs"), doomed).expect("doomed");
        run_git_test(root, &["add", "src/kept.rs", "src/doomed.rs"]);
        run_git_test(root, &["commit", "-qm", "base"]);

        let base_files = vec![
            ("src/kept.rs".to_owned(), git_blob_oid(kept)),
            ("src/doomed.rs".to_owned(), git_blob_oid(doomed)),
        ];

        fs::remove_file(root.join("src/doomed.rs")).expect("delete doomed");
        fs::write(root.join("src/new.rs"), b"pub fn newborn() {}\n").expect("untracked");

        let changed = changed_paths_for_overlay(root, base_files).expect("changed paths");
        let paths: BTreeSet<_> = changed.paths.into_iter().collect();
        assert_eq!(
            paths,
            BTreeSet::from([PathBuf::from("src/doomed.rs"), PathBuf::from("src/new.rs")]),
            "git overlay discovery must include deletions and untracked supported sources"
        );
    }

    #[test]
    fn overlay_client_for_backend_skips_wrap_when_worktree_matches_index() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        let src = b"pub fn alpha() {}\n";
        fs::write(root.join("src/a.rs"), src).expect("a.rs");
        let oid = git_blob_oid(src);

        let artifact = Arc::new(GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "overlay-skip-wrap".to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: "file:src/a.rs".to_owned(),
                path: "src/a.rs".to_owned(),
                content_oid: oid,
                node_ids: vec![NodeId(1)],
            }],
            files: vec![GraphFileArtifact {
                stable_file_id: "file:src/a.rs".to_owned(),
                file_path: "src/a.rs".to_owned(),
            }],
            file_node_ids: vec![NodeId(1)],
            symbols: Vec::new(),
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        });
        let backend = CodeSearchBackend::InMemory {
            client: InMemoryClient::new(Arc::clone(&artifact)),
            artifact,
        };
        let candidate = RebuildCandidate {
            worktree: root.to_path_buf(),
            key: RebuildKey::from("deadbeef", &BTreeMap::new()),
        };

        let overlay =
            overlay_client_for_backend(&backend, &candidate).expect("overlay client construction");
        assert!(
            overlay.is_none(),
            "matching worktree content must skip OverlayClient construction"
        );
    }

    struct OverlayGenerationMcpFixture {
        _dir: Option<tempfile::TempDir>,
        root: PathBuf,
        rebuild_coordinator: Arc<RebuildCoordinator>,
    }

    struct TestRuntimeSubscriptionFactory {
        cursor: crate::overlay_watch::CompositeCursor,
        changes: tokio::sync::watch::Sender<Option<crate::overlay_watch::ChangeBatch>>,
    }

    struct TestRuntimeChangeStream {
        provider: ChangeProviderKind,
        cursor: crate::overlay_watch::CompositeCursor,
        changes: tokio::sync::watch::Receiver<Option<crate::overlay_watch::ChangeBatch>>,
    }

    struct NeverBuildRuntimeGeneration;

    #[async_trait::async_trait]
    impl OverlayGenerationBuilder for NeverBuildRuntimeGeneration {
        async fn exact_scan(
            &self,
            _key: &OverlayRuntimeKey,
        ) -> anyhow::Result<BuiltOverlayGeneration> {
            panic!("a superseded base must not start a generation builder")
        }

        async fn rebuild_incremental(
            &self,
            _key: &OverlayRuntimeKey,
            _previous: BuiltOverlayGeneration,
            _changed_paths: BTreeSet<PathBuf>,
        ) -> anyhow::Result<BuiltOverlayGeneration> {
            panic!("a superseded base must not rebuild a generation")
        }
    }

    struct TestPublishedRuntime {
        subscriptions: Arc<TestRuntimeSubscriptionFactory>,
        lifecycle: Arc<OverlayRuntimeLifecycle>,
        key: OverlayRuntimeKey,
    }

    impl TestPublishedRuntime {
        fn acquired(&self) -> AcquiredOverlayRuntime {
            self.lifecycle
                .acquire(&self.key)
                .expect("installed test runtime")
        }

        fn published(&self) -> Arc<PublishedState> {
            self.acquired().published
        }

        fn send(&self, batch: crate::overlay_watch::ChangeBatch) {
            self.subscriptions.changes.send_replace(Some(batch));
        }

        fn send_changes(
            &self,
            label: &str,
            added: BTreeSet<PathBuf>,
            modified: BTreeSet<PathBuf>,
            deleted: BTreeSet<PathBuf>,
            renamed: BTreeSet<(PathBuf, PathBuf)>,
        ) {
            self.send(crate::overlay_watch::ChangeBatch::Changes {
                cursor: self.cursor(label),
                added,
                modified,
                deleted,
                renamed,
                git_metadata: BTreeSet::new(),
            });
        }

        fn send_git_metadata(&self, label: &str, paths: BTreeSet<PathBuf>) {
            self.send(crate::overlay_watch::ChangeBatch::Changes {
                cursor: self.cursor(label),
                added: BTreeSet::new(),
                modified: BTreeSet::new(),
                deleted: BTreeSet::new(),
                renamed: BTreeSet::new(),
                git_metadata: paths,
            });
        }

        fn send_trust_lost(&self, label: &str) {
            self.send(crate::overlay_watch::ChangeBatch::TrustLost {
                cursor: self.cursor(label),
                reason: crate::overlay_watch::TrustLoss::ChannelDisconnected {
                    provider: ChangeProviderKind::Notify,
                },
            });
        }

        fn cursor(&self, label: &str) -> crate::overlay_watch::CompositeCursor {
            crate::overlay_watch::CompositeCursor::from_entries([(
                self.key.canonical_worktree().to_path_buf(),
                label.to_owned(),
            )])
        }

        async fn wait_for_trusted_epoch_after(&self, previous_epoch: u64) -> Arc<PublishedState> {
            tokio::time::timeout(graph_rebuild_latency_budget(), async {
                loop {
                    let published = self.published();
                    if published.epoch() > previous_epoch
                        && matches!(published.trust(), PublishedTrust::Trusted)
                    {
                        return published;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("runtime actor must publish the requested epoch")
        }

        async fn wait_for_terminal_untrusted(&self) -> Arc<PublishedState> {
            tokio::time::timeout(graph_rebuild_latency_budget(), async {
                loop {
                    let published = self.published();
                    if matches!(
                        published.trust(),
                        PublishedTrust::Untrusted(
                            overlay_runtime::PublishedUntrustedReason::BuildFailed(message)
                        ) if message.contains("exact overlay recovery failed")
                    ) {
                        return published;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("runtime actor must publish terminal recovery failure")
        }

        async fn wait_for_fresh_trusted_handle(
            &self,
            stale: &Weak<OverlayRuntimeHandle>,
        ) -> AcquiredOverlayRuntime {
            tokio::time::timeout(graph_rebuild_latency_budget(), async {
                loop {
                    if let Some(acquired) = self.lifecycle.acquire(&self.key) {
                        if stale.upgrade().is_none()
                            && matches!(acquired.published.trust(), PublishedTrust::Trusted)
                        {
                            return acquired;
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("exact fallback must replace a terminal untrusted runtime")
        }
    }

    #[async_trait::async_trait]
    impl overlay_runtime::RuntimeChangeStream for TestRuntimeChangeStream {
        fn provider(&self) -> ChangeProviderKind {
            self.provider
        }

        fn initial_cursor(&self) -> &crate::overlay_watch::CompositeCursor {
            &self.cursor
        }

        async fn next_batch(&mut self) -> Option<crate::overlay_watch::ChangeBatch> {
            self.changes.changed().await.ok()?;
            self.changes.borrow_and_update().clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeSubscriptionFactory for TestRuntimeSubscriptionFactory {
        async fn arm(
            &self,
            _key: &OverlayRuntimeKey,
        ) -> anyhow::Result<Box<dyn overlay_runtime::RuntimeChangeStream>> {
            Ok(Box::new(TestRuntimeChangeStream {
                provider: ChangeProviderKind::Notify,
                cursor: self.cursor.clone(),
                changes: self.changes.subscribe(),
            }))
        }
    }

    impl OverlayGenerationMcpFixture {
        fn new(source: &str) -> Self {
            let dir = tempfile::tempdir().expect("generation MCP tempdir");
            let root = dir.path().to_path_buf();
            init_git_repo(&root);
            let artifact = artifact_from_source(&root, source);
            fs::write(root.join(".gitignore"), ".spur/\n").expect("write graph ignore");
            run_git_test(&root, &["add", "src/lib.rs", ".gitignore"]);
            run_git_test(&root, &["commit", "-q", "-m", "index generation fixture"]);
            write_current_artifact_indexed_at_head(&root, &artifact);
            Self {
                _dir: Some(dir),
                root,
                rebuild_coordinator: Arc::new(RebuildCoordinator::new()),
            }
        }

        fn for_existing_root(root: PathBuf) -> Self {
            Self {
                _dir: None,
                root,
                rebuild_coordinator: Arc::new(RebuildCoordinator::new()),
            }
        }

        async fn search(&self, query: &str, overlay_fsmonitor_auto: bool) -> Value {
            SCOPED_CODE_GRAPH_WORKTREE_ROOT
                .scope(self.root.clone(), async {
                    code_search_response(
                        &json!({
                            "query": query,
                            "mode": "exact",
                            "response_format": "full",
                        }),
                        Arc::clone(&self.rebuild_coordinator),
                        overlay_fsmonitor_auto,
                    )
                    .await
                })
                .await
                .expect("generation-routed search")
        }

        async fn subgraph(&self, selector: &str) -> Value {
            SCOPED_CODE_GRAPH_WORKTREE_ROOT
                .scope(self.root.clone(), async {
                    code_subgraph_response(
                        &json!({
                            "selector": selector,
                            "radius": 2,
                            "edge_kinds": ["calls"],
                            "include_unresolved": true,
                        }),
                        Arc::clone(&self.rebuild_coordinator),
                        true,
                    )
                    .await
                })
                .await
                .expect("generation-routed subgraph")
        }

        async fn publish_runtime(&self) -> TestPublishedRuntime {
            let root = self.root.clone();
            let rebuild_coordinator = Arc::clone(&self.rebuild_coordinator);
            SCOPED_CODE_GRAPH_WORKTREE_ROOT
                .scope(root.clone(), async move {
                    let backend = open_code_search_backend_for_request(Some(Arc::clone(
                        &rebuild_coordinator,
                    )))
                    .await
                    .expect("open runtime test backend");
                    let snapshot_base = backend.snapshot_base().expect("runtime snapshot base");
                    let key = OverlayRuntimeKey::new(
                        root.clone(),
                        snapshot_base.indexed_graph_content_hash.clone(),
                    );
                    let builder: Arc<dyn OverlayGenerationBuilder> =
                        Arc::new(McpOverlayGenerationBuilder {
                            worktree: root.clone(),
                            snapshot_base,
                            full_base_source: backend.full_base_artifact_source(),
                            use_request_cache: false,
                        });
                    let cursor = crate::overlay_watch::CompositeCursor::from_entries([(
                        root,
                        "test-ready".to_string(),
                    )]);
                    let (changes, _) = tokio::sync::watch::channel(None);
                    let subscriptions =
                        Arc::new(TestRuntimeSubscriptionFactory { cursor, changes });
                    let lifecycle = overlay_runtime_lifecycle_for(&rebuild_coordinator);
                    lifecycle.activate(&key);
                    let runtime_subscriptions: Arc<dyn RuntimeSubscriptionFactory> =
                        subscriptions.clone();
                    let handle = lifecycle
                        .registry
                        .get_or_start(key.clone(), Arc::clone(&runtime_subscriptions), builder)
                        .await
                        .expect("publish deterministic runtime");
                    lifecycle.install_if_active(&key, handle, runtime_subscriptions);
                    TestPublishedRuntime {
                        subscriptions,
                        lifecycle,
                        key,
                    }
                })
                .await
        }
    }

    fn generation_diagnostics(body: &Value) -> &Value {
        body.get("overlay_generation")
            .unwrap_or_else(|| panic!("missing bounded overlay generation diagnostics: {body:#}"))
    }

    fn without_generation_diagnostics(mut body: Value) -> Value {
        body.as_object_mut()
            .expect("MCP response object")
            .remove("overlay_generation");
        body
    }

    async fn assert_published_generation_matches_exact(
        fixture: &OverlayGenerationMcpFixture,
        runtime: &TestPublishedRuntime,
        previous_epoch: u64,
        query: &str,
    ) -> (Arc<PublishedState>, Value) {
        let published = runtime.wait_for_trusted_epoch_after(previous_epoch).await;
        reset_exact_overlay_observations_for_test();
        let auto = fixture.search(query, true).await;
        let diagnostics = generation_diagnostics(&auto);
        assert_eq!(diagnostics["route"], "generation", "{auto:#}");
        assert_eq!(diagnostics["provider"], "notify", "{auto:#}");
        assert_eq!(diagnostics["epoch"], published.epoch(), "{auto:#}");
        assert_eq!(diagnostics["trust"], "trusted", "{auto:#}");
        assert_eq!(diagnostics["generation_pins"], 1, "{auto:#}");
        assert_eq!(diagnostics["validation_observations"], 0, "{auto:#}");
        assert_eq!(diagnostics["response_retry"], false, "{auto:#}");
        assert_eq!(
            exact_overlay_observations_for_test(),
            0,
            "a trusted publication must not run a request-time Git observer"
        );
        let facts = build_facts(&fixture.root, None)
            .expect("extract exact oracle facts")
            .0;
        let oracle_artifact = Arc::new(
            artifact_from_facts(&facts, &fixture.root).expect("build exact oracle artifact"),
        );
        let oracle = code_search_with_artifact(
            &json!({
                "query": query,
                "mode": "exact",
                "response_format": "full",
            }),
            &InMemoryClient::new(oracle_artifact),
        )
        .expect("query exact oracle artifact");
        for field in ["query", "mode", "total_matches", "truncated", "candidates"] {
            assert_eq!(
                auto[field], oracle[field],
                "published generation must equal the freshly rebuilt exact oracle for `{field}`"
            );
        }
        (published, auto)
    }

    #[tokio::test]
    async fn generation_route_reuses_exact_snapshot_and_reports_zero_warm_finalization() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn generation_only() {}\n",
        )
        .expect("dirty generation source");

        let _provider = fixture.publish_runtime().await;
        reset_exact_overlay_observations_for_test();
        let first = fixture.search("generation_only", true).await;
        let second = fixture.search("generation_only", true).await;
        let first_diagnostics = generation_diagnostics(&first);
        let second_diagnostics = generation_diagnostics(&second);

        assert_eq!(first["total_matches"], 1, "{first:#}");
        assert_eq!(second["total_matches"], 1, "{second:#}");
        assert_eq!(first_diagnostics["route"], "generation");
        assert_eq!(second_diagnostics["route"], "generation");
        assert_eq!(first_diagnostics["cache"], "reused");
        assert_eq!(second_diagnostics["cache"], "reused");
        assert_eq!(
            first_diagnostics["generation_id"], second_diagnostics["generation_id"],
            "sequential requests over one exact snapshot must pin the same generation"
        );
        assert_eq!(first_diagnostics["full_base_artifact_builds"], 0);
        assert_eq!(second_diagnostics["full_base_artifact_builds"], 0);
        assert_eq!(second["response_file_oids_match"], true);
        for diagnostics in [first_diagnostics, second_diagnostics] {
            assert_eq!(diagnostics["provider"], "notify");
            assert_eq!(diagnostics["trust"], "trusted");
            assert!(diagnostics["epoch"].as_u64().is_some());
            assert_eq!(diagnostics["generation_pins"], 1);
            assert_eq!(
                diagnostics["validation_observations"], 0,
                "trusted warm requests must not execute an exact Git observer"
            );
            assert_eq!(diagnostics["response_retry"], false);
            assert_eq!(
                diagnostics["response_metadata_scans"], 0,
                "an unchanged final identity must derive metadata from the pinned generation"
            );
            assert_eq!(diagnostics["finalization_stages"]["shadow_filters"], 0);
            assert_eq!(diagnostics["finalization_stages"]["result_merges"], 0);
            assert_eq!(diagnostics["finalization_stages"]["overlay_sorts"], 0);
            assert_eq!(
                diagnostics["finalization_stages"]["stable_id_deduplications"],
                0
            );
            assert_eq!(diagnostics["finalization_stages"]["total"], 0);
        }
        assert_eq!(
            exact_overlay_observations_for_test(),
            0,
            "two sequential trusted warm requests must execute no exact Git observer"
        );
    }

    #[tokio::test]
    async fn generation_route_pins_every_nested_subgraph_query_to_one_identity() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new(
            "pub fn alpha() { beta(); }\npub fn beta() {}\npub fn caller() { alpha(); }\n",
        );
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() { beta(); }\npub fn beta() {}\npub fn caller() { alpha(); }\n// dirty\n",
        )
        .expect("dirty nested-query source");

        let _provider = fixture.publish_runtime().await;
        reset_exact_overlay_observations_for_test();
        let body = fixture.subgraph("alpha").await;
        let diagnostics = generation_diagnostics(&body);
        assert_eq!(diagnostics["route"], "generation");
        assert!(
            diagnostics["query_operations"].as_u64().unwrap_or_default() >= 3,
            "subgraph must perform multiple nested graph operations: {body:#}"
        );
        assert_eq!(diagnostics["generation_identity_mismatches"], 0);
        assert_eq!(diagnostics["generation_pins"], 1);
        assert_eq!(diagnostics["validation_observations"], 0);
        assert_eq!(diagnostics["response_retry"], false);
        assert_eq!(
            exact_overlay_observations_for_test(),
            0,
            "nested subgraph work must retain one published generation without an exact fence"
        );
        assert!(
            diagnostics["generation_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("gen_")),
            "generation identity must be bounded and opaque: {body:#}"
        );
    }

    #[tokio::test]
    async fn generation_route_singleflights_concurrent_identical_requests() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn concurrent_only() {}\n",
        )
        .expect("dirty concurrent source");

        let _provider = fixture.publish_runtime().await;
        reset_exact_overlay_observations_for_test();
        let (first, second) = tokio::join!(
            fixture.search("concurrent_only", true),
            fixture.search("concurrent_only", true),
        );
        let first_diagnostics = generation_diagnostics(&first);
        let second_diagnostics = generation_diagnostics(&second);
        assert_eq!(
            first_diagnostics["generation_id"],
            second_diagnostics["generation_id"]
        );
        assert_eq!(first_diagnostics["validation_observations"], 0);
        assert_eq!(second_diagnostics["validation_observations"], 0);
        assert_eq!(
            exact_overlay_observations_for_test(),
            0,
            "concurrent trusted warm requests must execute no exact Git observer"
        );
        let mut cache_states = vec![
            first_diagnostics["cache"].as_str().unwrap_or_default(),
            second_diagnostics["cache"].as_str().unwrap_or_default(),
        ];
        cache_states.sort_unstable();
        assert_eq!(cache_states, vec!["reused", "reused"]);
        let base_builds = first_diagnostics["full_base_artifact_builds"]
            .as_u64()
            .unwrap_or_default()
            + second_diagnostics["full_base_artifact_builds"]
                .as_u64()
                .unwrap_or_default();
        assert_eq!(
            base_builds, 0,
            "warm requests must not rebuild the full base"
        );
    }

    #[tokio::test]
    async fn generation_route_healthy_error_has_no_handler_retry_or_exact_fence() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let _runtime = fixture.publish_runtime().await;
        let handler_calls = AtomicUsize::new(0);
        reset_exact_overlay_observations_for_test();

        let error = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(fixture.root.clone(), async {
                let backend = open_code_search_backend_for_request(Some(Arc::clone(
                    &fixture.rebuild_coordinator,
                )))
                .await
                .expect("open healthy error backend");
                let source = backend.metadata_source();
                let request_client = RequestReplayClient::new(backend.client());
                let attempt = overlay_response_for_backend(
                    &backend,
                    &request_client,
                    fixture.root.clone(),
                    source,
                    &json!({ "selector": "still_missing" }),
                    ResponseFormat::Full,
                    true,
                    overlay_runtime_lifecycle_for(&fixture.rebuild_coordinator),
                    |args, client| {
                        handler_calls.fetch_add(1, Ordering::SeqCst);
                        code_resolve_with_client(args, client)
                    },
                )
                .await
                .expect("healthy route outcome");
                match attempt {
                    OverlayAttempt::Errored(error) => error,
                    _ => panic!("healthy missing selector must return its pinned handler error"),
                }
            })
            .await;
        let response = error.into_error_response().await;

        assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
        assert_eq!(exact_overlay_observations_for_test(), 0);
        assert_eq!(response.code, CODE_GRAPH_NOT_FOUND_ERROR_CODE);
        assert_eq!(
            response.data.expect("bounded error metadata")["rebuild_status"],
            "not_needed"
        );
    }

    #[tokio::test]
    async fn generation_route_publishes_add_modify_delete_rename_stage_and_head_exactly() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let runtime = fixture.publish_runtime().await;
        let mut epoch = runtime.published().epoch();
        let mut generation_ids = Vec::new();

        fs::write(
            fixture.root.join("src/new.rs"),
            "pub fn immediate_new_file() {}\n",
        )
        .expect("write immediate untracked source");
        runtime.send_changes(
            "add",
            BTreeSet::from([fixture.root.join("src/new.rs")]),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        );
        let (published, added) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "immediate_new_file",
        )
        .await;
        epoch = published.epoch();
        generation_ids.push(generation_diagnostics(&added)["generation_id"].clone());
        assert_eq!(added["total_matches"], 1, "{added:#}");

        fs::write(
            fixture.root.join("src/new.rs"),
            "pub fn modified_new_file() {}\n",
        )
        .expect("modify untracked source");
        runtime.send_changes(
            "modify",
            BTreeSet::new(),
            BTreeSet::from([fixture.root.join("src/new.rs")]),
            BTreeSet::new(),
            BTreeSet::new(),
        );
        let (published, modified) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "modified_new_file",
        )
        .await;
        epoch = published.epoch();
        generation_ids.push(generation_diagnostics(&modified)["generation_id"].clone());
        assert_eq!(modified["total_matches"], 1, "{modified:#}");

        fs::rename(
            fixture.root.join("src/lib.rs"),
            fixture.root.join("src/renamed.rs"),
        )
        .expect("rename tracked source");
        fs::write(
            fixture.root.join("src/renamed.rs"),
            "pub fn renamed_snapshot_only() {}\n",
        )
        .expect("rewrite renamed source");
        runtime.send_changes(
            "rename",
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::from([(
                fixture.root.join("src/lib.rs"),
                fixture.root.join("src/renamed.rs"),
            )]),
        );
        let (published, renamed) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "renamed_snapshot_only",
        )
        .await;
        epoch = published.epoch();
        generation_ids.push(generation_diagnostics(&renamed)["generation_id"].clone());
        assert_eq!(renamed["total_matches"], 1, "{renamed:#}");

        fs::remove_file(fixture.root.join("src/new.rs")).expect("delete untracked source");
        runtime.send_changes(
            "delete",
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::from([fixture.root.join("src/new.rs")]),
            BTreeSet::new(),
        );
        let (published, deleted) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "modified_new_file",
        )
        .await;
        epoch = published.epoch();
        generation_ids.push(generation_diagnostics(&deleted)["generation_id"].clone());
        assert_eq!(deleted["total_matches"], 0, "{deleted:#}");

        fs::write(
            fixture.root.join("src/renamed.rs"),
            "pub fn staged_snapshot_only() {}\n",
        )
        .expect("write staged source");
        run_git_test(&fixture.root, &["add", "-A"]);
        runtime.send_git_metadata("stage", BTreeSet::from([fixture.root.join(".git/index")]));
        let (published, staged) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "staged_snapshot_only",
        )
        .await;
        epoch = published.epoch();
        generation_ids.push(generation_diagnostics(&staged)["generation_id"].clone());
        assert_eq!(staged["total_matches"], 1, "{staged:#}");

        run_git_test(
            &fixture.root,
            &["commit", "-q", "-m", "advance fixture head"],
        );
        runtime.send_git_metadata("head", BTreeSet::from([fixture.root.join(".git/HEAD")]));
        let (_, head) = assert_published_generation_matches_exact(
            &fixture,
            &runtime,
            epoch,
            "staged_snapshot_only",
        )
        .await;
        generation_ids.push(generation_diagnostics(&head)["generation_id"].clone());

        assert!(
            generation_ids.windows(2).all(|pair| pair[0] != pair[1]),
            "each distinct exact snapshot must publish a new generation: {generation_ids:?}"
        );
    }

    #[tokio::test]
    async fn generation_route_shared_commondir_update_invalidates_linked_worktrees() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("linked-worktree tempdir");
        let main_root = dir.path().join("main");
        let linked_root = dir.path().join("linked");
        fs::create_dir_all(&main_root).expect("create main worktree");
        init_git_repo(&main_root);
        let main_artifact = artifact_from_source(&main_root, "pub fn alpha() {}\n");
        fs::write(main_root.join(".gitignore"), ".spur/\n").expect("write graph ignore");
        run_git_test(&main_root, &["add", "src/lib.rs", ".gitignore"]);
        run_git_test(&main_root, &["commit", "-q", "-m", "linked base"]);
        write_current_artifact_indexed_at_head(&main_root, &main_artifact);
        let linked_arg = linked_root.to_str().expect("UTF-8 linked root");
        run_git_test(
            &main_root,
            &["worktree", "add", "-q", "-b", "linked", linked_arg, "HEAD"],
        );
        let linked_artifact = artifact_from_source(&linked_root, "pub fn alpha() {}\n");
        write_current_artifact_indexed_at_head(&linked_root, &linked_artifact);

        let main = OverlayGenerationMcpFixture::for_existing_root(main_root.clone());
        let linked = OverlayGenerationMcpFixture::for_existing_root(linked_root.clone());
        let main_runtime = main.publish_runtime().await;
        let linked_runtime = linked.publish_runtime().await;
        let main_epoch = main_runtime.published().epoch();
        let linked_epoch = linked_runtime.published().epoch();

        fs::write(
            main_root.join("src/lib.rs"),
            "pub fn shared_commondir_head() {}\n",
        )
        .expect("advance main worktree");
        run_git_test(&main_root, &["add", "src/lib.rs"]);
        run_git_test(&main_root, &["commit", "-q", "-m", "shared ref update"]);
        let common = PathBuf::from(git_stdout_test(
            &main_root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ));
        let shared_ref = common.join("refs/heads/main");
        main_runtime.send_git_metadata("shared-main", BTreeSet::from([shared_ref.clone()]));
        linked_runtime.send_git_metadata("shared-linked", BTreeSet::from([shared_ref]));

        let (main_published, main_response) = assert_published_generation_matches_exact(
            &main,
            &main_runtime,
            main_epoch,
            "shared_commondir_head",
        )
        .await;
        let (linked_published, linked_response) = assert_published_generation_matches_exact(
            &linked,
            &linked_runtime,
            linked_epoch,
            "alpha",
        )
        .await;
        assert!(main_published.epoch() > main_epoch);
        assert!(linked_published.epoch() > linked_epoch);
        assert_eq!(main_response["total_matches"], 1, "{main_response:#}");
        assert_eq!(linked_response["total_matches"], 1, "{linked_response:#}");
    }

    #[tokio::test]
    async fn generation_route_reindex_retires_the_superseded_base_actor() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let previous = fixture.publish_runtime().await;
        let previous_acquired = previous.acquired();
        let previous_handle = Arc::downgrade(&previous_acquired.handle);
        let previous_base = previous.key.base_graph_identity().to_owned();
        drop(previous_acquired);

        fs::write(fixture.root.join("src/lib.rs"), "pub fn beta() {}\n")
            .expect("write reindexed source");
        run_git_test(&fixture.root, &["add", "src/lib.rs"]);
        run_git_test(&fixture.root, &["commit", "-q", "-m", "reindex fixture"]);
        let artifact = artifact_from_source(&fixture.root, "pub fn beta() {}\n");
        write_current_artifact_indexed_at_head(&fixture.root, &artifact);

        let late_previous_route = published_generation_route(
            Arc::clone(&previous.lifecycle),
            previous.key.clone(),
            Arc::new(NeverBuildRuntimeGeneration),
            || false,
        );
        let PublishedGenerationRoute::Exact(late_previous_fallback) = late_previous_route else {
            panic!("a late request carrying the superseded base must use exact fallback");
        };
        assert_eq!(late_previous_fallback.reason, "base_superseded");
        late_previous_fallback.schedule_seed_or_restart();
        tokio::task::yield_now().await;
        assert!(previous.lifecycle.acquire(&previous.key).is_none());
        assert!(
            previous
                .lifecycle
                .active_keys
                .lock()
                .expect("retired runtime bases")
                .get(&fixture.root)
                .is_none(),
            "rejecting the active obsolete base must retire it before B is activated"
        );

        tokio::time::timeout(graph_rebuild_latency_budget(), async {
            while previous_handle.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("superseded base actor must stop after its final handle drops");

        let current = fixture.publish_runtime().await;
        assert_ne!(
            current.key.base_graph_identity(),
            previous_base,
            "the test must install a distinct base generation"
        );
        let handles = current
            .lifecycle
            .handles
            .lock()
            .expect("runtime lifecycle handles");
        let retained_count = handles
            .keys()
            .filter(|key| key.canonical_worktree() == fixture.root)
            .count();
        assert_eq!(retained_count, 1);
        assert!(handles.contains_key(&current.key));
        drop(handles);
        assert_eq!(
            current
                .lifecycle
                .active_keys
                .lock()
                .expect("active runtime bases")
                .get(&fixture.root),
            Some(&current.key),
            "the late old request must not reactivate its obsolete base"
        );
        assert!(matches!(
            current.published().trust(),
            PublishedTrust::Trusted
        ));
    }

    #[tokio::test]
    async fn generation_route_terminal_recovery_failure_falls_back_then_restarts() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn fallback_only() {}\n",
        )
        .expect("dirty fallback source");
        let runtime = fixture.publish_runtime().await;
        let concurrent_pin = runtime.acquired().handle;
        let stale = Arc::downgrade(&concurrent_pin);
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn fallback_only() {}\npub fn recovery_changed() {}\n",
        )
        .expect("change exact identity before recovery failure");
        let _failure = set_overlay_generation_failures_for_test(1);
        runtime.send_trust_lost("provider-loss");
        let terminal = runtime.wait_for_terminal_untrusted().await;
        assert!(matches!(terminal.trust(), PublishedTrust::Untrusted(_)));
        let exact = fixture.search("recovery_changed", false).await;
        assert!(
            exact.get("overlay_generation").is_none(),
            "Off mode must retain the existing exact OverlayClient response shape"
        );
        let fallback = fixture.search("recovery_changed", true).await;
        let diagnostics = generation_diagnostics(&fallback);
        assert_eq!(diagnostics["route"], "exact_fallback");
        assert_eq!(diagnostics["fallback_reason"], "runtime_untrusted");
        assert_eq!(diagnostics["provider"], "notify");
        assert_eq!(diagnostics["epoch"], terminal.epoch());
        assert_eq!(diagnostics["trust"], "untrusted");
        assert_eq!(diagnostics["generation_id"], Value::Null);
        assert_eq!(diagnostics["generation_pins"], 0);
        assert_eq!(diagnostics["validation_observations"], 2);
        assert_eq!(diagnostics["response_retry"], false);
        assert_eq!(
            without_generation_diagnostics(fallback),
            exact,
            "generation failure must preserve the whole exact fallback response"
        );

        assert!(
            stale.upgrade().is_some(),
            "the test must retain a concurrent request-local stale pin"
        );
        drop(concurrent_pin);
        let recovered = runtime.wait_for_fresh_trusted_handle(&stale).await;
        assert!(matches!(
            recovered.published.trust(),
            PublishedTrust::Trusted
        ));
        let warm = fixture.search("recovery_changed", true).await;
        assert_eq!(generation_diagnostics(&warm)["route"], "generation");
        assert_eq!(generation_diagnostics(&warm)["validation_observations"], 0);
        assert_eq!(warm["total_matches"], 1, "{warm:#}");
    }

    #[tokio::test]
    async fn generation_route_terminal_fallback_error_still_restarts_without_retry() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let runtime = fixture.publish_runtime().await;
        let concurrent_pin = runtime.acquired().handle;
        let stale = Arc::downgrade(&concurrent_pin);
        let _failure = set_overlay_generation_failures_for_test(1);
        runtime.send_trust_lost("provider-loss-before-error");
        let terminal = runtime.wait_for_terminal_untrusted().await;
        assert!(matches!(terminal.trust(), PublishedTrust::Untrusted(_)));
        let handler_calls = AtomicUsize::new(0);

        let error = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(fixture.root.clone(), async {
                let backend = open_code_search_backend_for_request(Some(Arc::clone(
                    &fixture.rebuild_coordinator,
                )))
                .await
                .expect("open terminal fallback error backend");
                let source = backend.metadata_source();
                let request_client = RequestReplayClient::new(backend.client());
                let attempt = overlay_response_for_backend(
                    &backend,
                    &request_client,
                    fixture.root.clone(),
                    source,
                    &json!({ "selector": "still_missing" }),
                    ResponseFormat::Full,
                    true,
                    overlay_runtime_lifecycle_for(&fixture.rebuild_coordinator),
                    |args, client| {
                        handler_calls.fetch_add(1, Ordering::SeqCst);
                        code_resolve_with_client(args, client)
                    },
                )
                .await
                .expect("terminal exact fallback outcome");
                match attempt {
                    OverlayAttempt::Errored(error) => error,
                    _ => panic!("missing selector must preserve the exact fallback error"),
                }
            })
            .await;
        let response = error.into_error_response().await;

        assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.code, CODE_GRAPH_NOT_FOUND_ERROR_CODE);
        assert_eq!(
            response.data.expect("fallback error metadata")["rebuild_status"],
            "fresh"
        );
        assert!(
            stale.upgrade().is_some(),
            "a concurrent request must still own the terminal handle"
        );
        drop(concurrent_pin);
        let recovered = runtime.wait_for_fresh_trusted_handle(&stale).await;
        assert!(matches!(
            recovered.published.trust(),
            PublishedTrust::Trusted
        ));
        let warm = fixture.search("alpha", true).await;
        assert_eq!(generation_diagnostics(&warm)["route"], "generation");
        assert_eq!(warm["total_matches"], 1, "{warm:#}");
    }

    #[tokio::test]
    async fn generation_route_production_lifecycle_publishes_after_cold_exact_seed() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let module = GraphMcpModule::new(GraphMcpDeps {
            rebuild_coordinator: Arc::clone(&fixture.rebuild_coordinator),
            overlay_fsmonitor_auto: true,
        });

        let first = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(
                fixture.root.clone(),
                module.dispatch(
                    "code_symbol_search",
                    json!({
                        "query": "alpha",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                ),
            )
            .await
            .expect("cold production-shaped search");
        assert_eq!(first["total_matches"], 1, "{first:#}");
        assert_eq!(
            generation_diagnostics(&first)["route"],
            "exact_fallback",
            "the cold request may seed only through the exact route"
        );
        assert_eq!(
            generation_diagnostics(&first)["startup_status"],
            "not_started"
        );
        assert_eq!(generation_diagnostics(&first)["startup_cause"], Value::Null);

        let lifecycle = overlay_runtime_lifecycle_for(&fixture.rebuild_coordinator);
        let key = lifecycle
            .active_keys
            .lock()
            .expect("active production runtime key")
            .get(&fixture.root)
            .cloned()
            .expect("cold request must activate its runtime key");
        let published = tokio::time::timeout(graph_rebuild_latency_budget(), async {
            loop {
                if let Some(acquired) = lifecycle.acquire(&key) {
                    return Some(acquired.published);
                }
                if !lifecycle
                    .starting
                    .lock()
                    .expect("production runtime startup state")
                    .contains(&key)
                {
                    return None;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production runtime startup must reach an observable terminal condition");
        let published = match published {
            Some(published) => published,
            None => {
                let sources = ChangeSourceSet::resolve(&fixture.root)
                    .expect("diagnostic production source resolution");
                let subscriptions: Arc<dyn RuntimeSubscriptionFactory> = Arc::new(
                    CompositeSubscriptionFactory::new(&key, sources)
                        .expect("diagnostic production subscription factory"),
                );
                let builder: Arc<dyn OverlayGenerationBuilder> = SCOPED_CODE_GRAPH_WORKTREE_ROOT
                    .scope(fixture.root.clone(), async {
                        let backend = open_code_search_backend_for_request(Some(Arc::clone(
                            &fixture.rebuild_coordinator,
                        )))
                        .await
                        .expect("diagnostic production backend");
                        Arc::new(McpOverlayGenerationBuilder {
                            worktree: fixture.root.clone(),
                            snapshot_base: backend
                                .snapshot_base()
                                .expect("diagnostic production snapshot base"),
                            full_base_source: backend.full_base_artifact_source(),
                            use_request_cache: true,
                        }) as Arc<dyn OverlayGenerationBuilder>
                    })
                    .await;
                match lifecycle
                    .registry
                    .get_or_start(key.clone(), subscriptions, builder)
                    .await
                {
                    Ok(_) => panic!(
                        "asynchronous production startup completed without installing a handle, \
                         although the same direct startup succeeded"
                    ),
                    Err(error) => panic!("production startup boundary failed: {error:#}"),
                }
            }
        };
        assert!(matches!(published.trust(), PublishedTrust::Trusted));
        assert_ne!(published.provider(), ChangeProviderKind::ExactOnly);

        reset_exact_overlay_observations_for_test();
        let warm = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(
                fixture.root.clone(),
                module.dispatch(
                    "code_symbol_search",
                    json!({
                        "query": "alpha",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                ),
            )
            .await
            .expect("warm production-shaped search");
        let diagnostics = generation_diagnostics(&warm);
        assert_eq!(diagnostics["route"], "generation", "{warm:#}");
        assert_eq!(diagnostics["trust"], "trusted", "{warm:#}");
        assert_eq!(diagnostics["startup_status"], "ready", "{warm:#}");
        assert_eq!(diagnostics["startup_cause"], Value::Null, "{warm:#}");
        assert_eq!(diagnostics["generation_pins"], 1, "{warm:#}");
        assert_eq!(diagnostics["validation_observations"], 0, "{warm:#}");
        assert_eq!(diagnostics["finalization_stages"]["total"], 0, "{warm:#}");
        assert_eq!(exact_overlay_observations_for_test(), 0);
    }

    #[tokio::test]
    async fn generation_route_startup_failure_reports_bounded_cause() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        let module = GraphMcpModule::new(GraphMcpDeps {
            rebuild_coordinator: Arc::clone(&fixture.rebuild_coordinator),
            overlay_fsmonitor_auto: true,
        });
        let _failure = set_overlay_generation_failures_for_test(1);

        let first = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(
                fixture.root.clone(),
                module.dispatch(
                    "code_symbol_search",
                    json!({
                        "query": "alpha",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                ),
            )
            .await
            .expect("cold exact fallback before scripted startup failure");
        assert_eq!(generation_diagnostics(&first)["route"], "exact_fallback");

        let lifecycle = overlay_runtime_lifecycle_for(&fixture.rebuild_coordinator);
        let key = lifecycle
            .active_keys
            .lock()
            .expect("active failed-start key")
            .get(&fixture.root)
            .cloned()
            .expect("cold request must activate failed-start key");
        tokio::time::timeout(graph_rebuild_latency_budget(), async {
            while lifecycle
                .starting
                .lock()
                .expect("failed runtime startup state")
                .contains(&key)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scripted startup failure must become observable");
        assert!(lifecycle.acquire(&key).is_none());

        let failed = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(
                fixture.root.clone(),
                module.dispatch(
                    "code_symbol_search",
                    json!({
                        "query": "alpha",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                ),
            )
            .await
            .expect("exact fallback after scripted startup failure");
        let diagnostics = generation_diagnostics(&failed);
        assert_eq!(diagnostics["route"], "exact_fallback", "{failed:#}");
        assert_eq!(diagnostics["startup_status"], "failed", "{failed:#}");
        assert_eq!(diagnostics["startup_cause"], "exact_seed", "{failed:#}");
        assert!(
            diagnostics["startup_cause"]
                .as_str()
                .is_some_and(|cause| cause.len() <= 32),
            "startup cause must remain a bounded category: {failed:#}"
        );
    }

    #[tokio::test]
    async fn generation_route_unavailable_falls_back_and_off_mode_is_byte_for_byte_unchanged() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let fixture = OverlayGenerationMcpFixture::new("pub fn alpha() {}\n");
        fs::write(
            fixture.root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn off_mode_only() {}\n",
        )
        .expect("dirty Off-mode source");
        let off_before = fixture.search("off_mode_only", false).await;
        let fallback = fixture.search("off_mode_only", true).await;
        let off_after = fixture.search("off_mode_only", false).await;
        let diagnostics = generation_diagnostics(&fallback);
        assert_eq!(diagnostics["route"], "exact_fallback", "{fallback:#}");
        assert_eq!(diagnostics["fallback_reason"], "runtime_unavailable");
        assert_eq!(diagnostics["provider"], Value::Null);
        assert_eq!(diagnostics["epoch"], Value::Null);
        assert_eq!(diagnostics["trust"], "unavailable");
        assert_eq!(diagnostics["generation_id"], Value::Null);
        assert_eq!(diagnostics["response_retry"], false);
        assert_eq!(without_generation_diagnostics(fallback), off_before);
        assert_eq!(
            off_before, off_after,
            "Auto fallback must not mutate Off behavior"
        );
        assert_eq!(off_after["total_matches"], 1, "{off_after:#}");
        assert!(off_after.get("overlay_generation").is_none());
    }

    #[tokio::test]
    async fn request_replay_counts_each_base_operation_once_and_matches_fresh_oracle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let base_source = "pub fn target() { leaf(); }\n\
                           pub fn leaf() {}\n\
                           pub fn caller() { target(); }\n\
                           pub fn old_name() {}\n";
        let base_artifact = Arc::new(artifact_from_source(root, base_source));
        let target_sid = symbol_id_for(&base_artifact, "target");
        let symbols_by_files = "symbols_by_files:[\"src/lib.rs\"]".to_owned();

        let clean_case = RequestReplayCase {
            name: "search-clean",
            args: json!({
                "query": "target",
                "mode": "exact",
                "limit": 1,
            }),
            handler: search_handler_for_request_replay,
            expected_operations: BTreeMap::from([(
                "search_symbols:Exact:target:SearchFilters { symbol_kind: None, file: None, file_glob: None }"
                    .to_owned(),
                1,
            )]),
        };
        let mut violations = exercise_request_replay_case(
            "clean",
            &clean_case,
            root,
            Arc::clone(&base_artifact),
            Arc::clone(&base_artifact),
            &[],
        )
        .await;

        let current_source = "pub fn target() { new_leaf(); }\n\
                              pub fn new_leaf() {}\n\
                              pub fn caller() { target(); }\n\
                              pub fn added() { target(); }\n\
                              pub fn new_name() {}\n";
        fs::write(root.join("src/lib.rs"), current_source).expect("write dirty source");
        let oracle_facts = build_facts(root, None).expect("extract oracle source").0;
        let oracle_artifact = Arc::new(
            artifact_from_facts(&oracle_facts, root).expect("freshly rebuilt oracle artifact"),
        );
        let dirty_paths = [PathBuf::from("src/lib.rs")];
        let dirty_cases = [
            RequestReplayCase {
                name: "search",
                args: json!({
                    "query": "a",
                    "mode": "substring",
                    "limit": 2,
                    "response_format": "full",
                }),
                handler: search_handler_for_request_replay,
                expected_operations: BTreeMap::from([
                    (
                        "search_symbols:Substring:a:SearchFilters { symbol_kind: None, file: None, file_glob: None }"
                            .to_owned(),
                        1,
                    ),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "resolve",
                args: json!({ "selector": "target" }),
                handler: code_resolve_with_client,
                expected_operations: BTreeMap::from([
                    ("resolve_selector:target".to_owned(), 1),
                    (format!("symbol_by_id:{target_sid}"), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "resolve-new-or-renamed",
                args: json!({ "selector": "new_name" }),
                handler: code_resolve_with_client,
                expected_operations: BTreeMap::from([
                    ("resolve_selector:new_name".to_owned(), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "resolve-not-found-error",
                args: json!({ "selector": "still_missing" }),
                handler: code_resolve_with_client,
                expected_operations: BTreeMap::from([
                    ("resolve_selector:still_missing".to_owned(), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "file-symbols",
                args: json!({
                    "file": "src/lib.rs",
                    "response_format": "table",
                }),
                handler: code_file_symbols_with_client,
                expected_operations: BTreeMap::from([
                    ("file_exists:src/lib.rs".to_owned(), 1),
                    ("symbols_by_file:src/lib.rs".to_owned(), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "read-symbol",
                args: json!({
                    "path": "src/lib.rs",
                    "name": "target",
                    "context_lines": 1,
                    "response_format": "source",
                }),
                handler: code_read_symbol_with_client,
                expected_operations: BTreeMap::from([
                    ("file_manifest_by_path:src/lib.rs".to_owned(), 1),
                    ("symbols_by_path_name:src/lib.rs:target".to_owned(), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "callers",
                args: json!({
                    "selector": "target",
                    "include_unresolved": true,
                    "response_format": "table",
                }),
                handler: code_callers_with_client,
                expected_operations: BTreeMap::from([
                    (format!("find_caller_edges:{target_sid}"), 1),
                    (
                        format!(
                            "find_unresolved_caller_edges_by_labels:[\"{target_sid}\", \"target\"]"
                        ),
                        1,
                    ),
                    ("resolve_selector:target".to_owned(), 1),
                    (symbols_by_files.clone(), 1),
                ]),
            },
            RequestReplayCase {
                name: "callees",
                args: json!({
                    "selector": "target",
                    "include_unresolved": true,
                    "response_format": "table",
                }),
                handler: code_callees_with_client,
                expected_operations: BTreeMap::from([
                    (format!("find_callee_edges:{target_sid}"), 1),
                    ("resolve_selector:target".to_owned(), 1),
                    (symbols_by_files, 1),
                ]),
            },
        ];

        let dirty_violations = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                let mut violations = Vec::new();
                for case in &dirty_cases {
                    violations.extend(
                        exercise_request_replay_case(
                            "dirty",
                            case,
                            root,
                            Arc::clone(&base_artifact),
                            Arc::clone(&oracle_artifact),
                            &dirty_paths,
                        )
                        .await,
                    );
                }
                violations
            })
            .await;
        violations.extend(dirty_violations);

        assert!(
            violations.is_empty(),
            "request replay cardinality/equivalence violations:\n{}",
            violations.join("\n")
        );
    }

    #[tokio::test]
    async fn rebuild_only_refresh_rebinds_rebuilt_graph_to_parquet_hot_indices() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn stale_base() {}\n");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index stale base"]);
        write_current_artifact(root, &artifact);
        fs::write(
            root.join("src/lib.rs"),
            "pub fn fresh_caller() { fresh_target(); }\n\
             pub fn fresh_target() {}\n",
        )
        .expect("dirty source");
        reset_parquet_hot_index_build_observations_for_test();

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_graph_backend_response_rebuild_only(
                    &json!({}),
                    Arc::new(RebuildCoordinator::new()),
                    |_, client| {
                        let search = client
                            .search_symbols(&SearchOptions {
                                query: "fresh_".to_owned(),
                                mode: SearchMode::Prefix,
                                filters: SearchFilters::default(),
                                limit: 20,
                            })
                            .map_err(|error| test_internal_error(format!("search: {error:#}")))?;
                        let target = search
                            .candidates
                            .iter()
                            .find(|symbol| symbol.entity_name == "fresh_target")
                            .ok_or_else(|| test_internal_error("fresh target missing"))?;
                        let caller = search
                            .candidates
                            .iter()
                            .find(|symbol| symbol.entity_name == "fresh_caller")
                            .ok_or_else(|| test_internal_error("fresh caller missing"))?;
                        let mut search_names = search
                            .candidates
                            .iter()
                            .map(|symbol| symbol.entity_name.clone())
                            .collect::<Vec<_>>();
                        search_names.sort();
                        let mut caller_names = client
                            .find_caller_edges(&target.stable_symbol_id)
                            .into_iter()
                            .map(|record| match record {
                                OwnedCallerRecord::Resolved { caller, .. }
                                | OwnedCallerRecord::Unresolved { caller, .. } => {
                                    caller.entity_name
                                }
                            })
                            .collect::<Vec<_>>();
                        caller_names.sort();
                        let mut callee_names = client
                            .find_callee_edges(&caller.stable_symbol_id)
                            .into_iter()
                            .filter_map(|record| match record {
                                OwnedCalleeRecord::Resolved { symbol, .. } => {
                                    Some(symbol.entity_name)
                                }
                                OwnedCalleeRecord::Unresolved { .. } => None,
                            })
                            .collect::<Vec<_>>();
                        callee_names.sort();
                        Ok(json!({
                            "search_total": search.total_matches,
                            "search_names": search_names,
                            "caller_names": caller_names,
                            "callee_names": callee_names,
                        }))
                    },
                )
                .await
            })
            .await
            .expect("fresh rebuild response");
        let (query_builds, adjacency_builds) = parquet_hot_index_build_observations_for_test();

        eprintln!(
            "rebuild SoA hot-index evidence query_builds={query_builds} \
             adjacency_builds={adjacency_builds} body={body:#}"
        );
        assert_eq!(body["rebuild_status"], "fresh", "{body:#}");
        assert_eq!(body["search_total"], 2, "{body:#}");
        assert_eq!(
            body["search_names"],
            json!(["fresh_caller", "fresh_target"])
        );
        assert_eq!(body["caller_names"], json!(["fresh_caller"]));
        assert_eq!(body["callee_names"], json!(["fresh_target"]));
        assert_eq!(
            query_builds, 1,
            "rebuilt graph search/caller/callee must build the Parquet-backed RecordBatch hot index"
        );
        assert_eq!(
            adjacency_builds, 1,
            "rebuilt graph caller/callee must build integer-coded HotAdjacencyIndex"
        );
    }

    #[tokio::test]
    async fn request_replay_stale_budget_fallback_returns_whole_base_response() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::ZERO);
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn alpha() {}\n");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);
        fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn overlay_only() {}\n",
        )
        .expect("dirty source");

        let base = code_search_with_artifact(
            &json!({ "query": "a", "mode": "substring", "limit": 20 }),
            &InMemoryClient::new(Arc::new(artifact.clone())),
        )
        .expect("base response");
        let expected_head = git_stdout_test(root, &["rev-parse", "HEAD"]);

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(
                    &json!({ "query": "a", "mode": "substring", "limit": 20 }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await
            .expect("stale-budget response");

        let names = body["candidates"]
            .as_array()
            .expect("candidate array")
            .iter()
            .filter_map(|candidate| candidate["entity_name"].as_str())
            .collect::<Vec<_>>();
        eprintln!(
            "request_replay fallback trace kind=stale_budget digest={} names={names:?} \
             status={} metadata={{graph_hash:{}, head:{}, dirty:{}, oid_match:{}}}",
            response_digest(&body),
            body["rebuild_status"],
            body["graph_content_hash"],
            body["worktree_head_oid"],
            body["worktree_dirty"],
            body["response_file_oids_match"],
        );
        assert_eq!(body["candidates"], base["candidates"]);
        assert_eq!(body["total_matches"], base["total_matches"]);
        assert_eq!(names, vec!["alpha"]);
        assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
        assert_eq!(
            body["graph_index_version"],
            artifact.header.graph_index_version
        );
        assert_eq!(body["worktree_head_oid"], expected_head);
        assert_eq!(body["worktree_dirty"], true);
        assert_eq!(body["response_file_oids_match"], false);
        assert_eq!(body["rebuild_status"], "stale_budget_exceeded");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_replay_overlay_failure_fallback_returns_whole_base_response() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn alpha() {}\n");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);
        let invalid_path = root.join("src/unreadable.rs");
        fs::write(&invalid_path, "pub fn partial_overlay_only() {}\n")
            .expect("unreadable overlay source");
        let mut permissions = fs::metadata(&invalid_path)
            .expect("unreadable source metadata")
            .permissions();
        permissions.set_mode(0);
        fs::set_permissions(&invalid_path, permissions).expect("make overlay source unreadable");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() {}\n// force response-relevant refresh\n",
        )
        .expect("dirty indexed source");

        let args = json!({ "query": "a", "mode": "substring", "limit": 20 });
        let base =
            code_search_with_artifact(&args, &InMemoryClient::new(Arc::new(artifact.clone())))
                .expect("base response");
        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(&args, Arc::new(RebuildCoordinator::new()), false).await
            })
            .await
            .expect("production wrapper must return the whole base response");
        let expected_head = git_stdout_test(root, &["rev-parse", "HEAD"]);

        let names = body["candidates"]
            .as_array()
            .expect("candidate array")
            .iter()
            .filter_map(|candidate| candidate["entity_name"].as_str())
            .collect::<Vec<_>>();
        eprintln!(
            "request_replay fallback trace kind=overlay_failure digest={} names={names:?} \
             status={} metadata={{graph_hash:{}, head:{}, dirty:{}, oid_match:{}}}",
            response_digest(&body),
            body["rebuild_status"],
            body["graph_content_hash"],
            body["worktree_head_oid"],
            body["worktree_dirty"],
            body["response_file_oids_match"],
        );
        assert_eq!(body["candidates"], base["candidates"]);
        assert_eq!(body["total_matches"], base["total_matches"]);
        assert_eq!(names, vec!["alpha"]);
        assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
        assert_eq!(
            body["graph_index_version"],
            artifact.header.graph_index_version
        );
        assert_eq!(body["worktree_head_oid"], expected_head);
        assert_eq!(body["worktree_dirty"], true);
        assert_eq!(body["response_file_oids_match"], false);
        assert_eq!(body["rebuild_status"], "stale_rebuild_failed");

        let missing_message = "symbol still_missing not found in graph artifact".to_owned();
        let reference_error =
            CodeGraphError::without_metadata(McpHandlerError::NotFound(missing_message.clone()))
                .with_metadata_source(GraphMetadataSource::from_artifact(&artifact));
        let (reference, fallback) = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                let reference = reference_error.into_error_response().await;
                let fallback = code_resolve_response(
                    &json!({ "selector": "still_missing" }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
                .expect_err("whole not-found error must survive failed refresh")
                .into_error_response()
                .await;
                (reference, fallback)
            })
            .await;

        assert_eq!(fallback.code, reference.code);
        assert_eq!(fallback.message, reference.message);
        assert_eq!(fallback.data, reference.data);
        assert_eq!(fallback.code, CODE_GRAPH_NOT_FOUND_ERROR_CODE);
        assert_eq!(fallback.message, missing_message);
        let reference_data = reference.data.as_ref().expect("reference error data");
        let fallback_data = fallback.data.as_ref().expect("fallback error data");
        eprintln!(
            "request_replay fallback error reference={{code:{}, message:{:?}, data:{reference_data}}} \
             fallback={{code:{}, message:{:?}, data:{fallback_data}}}",
            reference.code, reference.message, fallback.code, fallback.message,
        );
        assert_eq!(fallback_data["kind"], "not_found");
        assert_eq!(
            fallback_data["graph_content_hash"],
            artifact.graph_content_hash
        );
        assert_eq!(
            fallback_data["graph_index_version"],
            artifact.header.graph_index_version
        );
        assert_eq!(fallback_data["rebuild_status"], "not_needed");
    }

    #[tokio::test]
    async fn request_replay_immediate_untracked_source_invalidates_primed_clean_metadata() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}\n").expect("write source");
        fs::write(root.join(".gitignore"), ".spur/\n").expect("write graph ignore");
        run_git_test(root, &["add", "src/lib.rs", ".gitignore"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        let artifact = artifact_from_source(root, "pub fn alpha() {}\n");
        write_current_artifact(root, &artifact);

        let prime = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(
                    &json!({
                        "query": "alpha",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await
            .expect("prime clean metadata");
        eprintln!(
            "immediate untracked prime total={} status={} dirty={} candidate={}",
            prime["total_matches"],
            prime["rebuild_status"],
            prime["worktree_dirty"],
            prime["candidates"].get(0).unwrap_or(&Value::Null),
        );
        assert_eq!(prime["rebuild_status"], "not_needed");
        assert_eq!(prime["total_matches"], 1, "{prime:#}");
        assert_eq!(prime["candidates"][0]["entity_name"], "alpha");

        fs::write(
            root.join("src/immediate.rs"),
            "pub fn immediate_only() {}\n",
        )
        .expect("write immediate untracked source");
        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(
                    &json!({
                        "query": "immediate_only",
                        "mode": "exact",
                        "response_format": "full",
                    }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await
            .expect("immediate untracked symbol must be visible");

        eprintln!(
            "immediate untracked evidence total={} status={} dirty={} candidate={}",
            body["total_matches"],
            body["rebuild_status"],
            body["worktree_dirty"],
            body["candidates"].get(0).unwrap_or(&Value::Null),
        );
        assert_eq!(body["total_matches"], 1, "{body:#}");
        assert_eq!(body["candidates"][0]["entity_name"], "immediate_only");
        assert_eq!(body["rebuild_status"], "fresh");
    }

    #[derive(Clone)]
    struct ReadyLocalProjectValidator;

    impl LocalProjectValidator for ReadyLocalProjectValidator {
        fn validate(
            &self,
            requested_path: &Path,
        ) -> Result<ValidatedLocalProject, LocalProjectError> {
            let canonical_root =
                requested_path
                    .canonicalize()
                    .map_err(|error| LocalProjectError::InvalidPath {
                        path: requested_path.to_path_buf(),
                        reason: error.to_string(),
                    })?;
            Ok(ValidatedLocalProject {
                canonical_root,
                health: LocalProjectHealth::ready(),
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_named_dispatches_overlap_inside_distinct_scoped_roots() {
        let alpha_dir = tempfile::tempdir().expect("alpha tempdir");
        let beta_dir = tempfile::tempdir().expect("beta tempdir");
        for (root, source) in [
            (alpha_dir.path(), "pub fn alpha_only() {}\n"),
            (beta_dir.path(), "pub fn beta_only() {}\n"),
        ] {
            init_git_repo(root);
            let artifact = artifact_from_source(root, source);
            run_git_test(root, &["add", "src/lib.rs"]);
            run_git_test(root, &["commit", "-q", "-m", "index symbol"]);
            write_current_artifact(root, &artifact);
        }
        let catalog_dir = tempfile::tempdir().expect("catalog tempdir");
        let store = LocalProjectCatalogStore::new(catalog_dir.path().join("projects.toml"));
        store
            .add("alpha", alpha_dir.path(), false)
            .expect("register alpha");
        store
            .add("beta", beta_dir.path(), false)
            .expect("register beta");
        let resolver = LocalProjectResolver::new(store, Arc::new(ReadyLocalProjectValidator));
        let module = GraphMcpModule::with_local_projects(GraphMcpDeps::default(), resolver);
        let overlap = Arc::new(tokio::sync::Barrier::new(2));

        let alpha_task = {
            let module = module.clone();
            let overlap = Arc::clone(&overlap);
            tokio::spawn(PROJECT_SCOPE_BARRIER_FOR_TEST.scope(overlap, async move {
                module
                    .dispatch(
                        "code_symbol_search",
                        json!({"query": "alpha_only", "mode": "exact", "project": "alpha"}),
                    )
                    .await
            }))
        };
        let beta_task = {
            let module = module.clone();
            let overlap = Arc::clone(&overlap);
            tokio::spawn(PROJECT_SCOPE_BARRIER_FOR_TEST.scope(overlap, async move {
                module
                    .dispatch(
                        "code_symbol_search",
                        json!({"query": "beta_only", "mode": "exact", "project": "beta"}),
                    )
                    .await
            }))
        };
        let (alpha, beta) = tokio::join!(alpha_task, beta_task);
        let alpha = alpha.expect("alpha task").expect("alpha dispatch");
        let beta = beta.expect("beta task").expect("beta dispatch");

        assert_eq!(alpha["candidates"][0]["entity_name"], "alpha_only");
        assert_eq!(alpha["project"]["name"], "alpha");
        assert_eq!(beta["candidates"][0]["entity_name"], "beta_only");
        assert_eq!(beta["project"]["name"], "beta");
    }

    #[tokio::test]
    async fn code_search_response_adds_full_metadata_and_clamps_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let source = "pub fn alpha() {}\n\
                      pub fn beta() {}\n";
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), source).expect("write source");
        fs::write(root.join(".gitignore"), ".spur/\n").expect("write graph ignore");
        run_git_test(root, &["add", "src/lib.rs", ".gitignore"]);
        run_git_test(root, &["commit", "-q", "-m", "index symbols"]);
        let artifact = artifact_from_source(root, source);
        write_current_artifact(root, &artifact);

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(
                    &json!({
                        "query": "alp",
                        "mode": "prefix",
                        "limit": 0,
                    }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await
            .expect("search response");

        assert_eq!(body["query"], "alp");
        assert_eq!(body["mode"], "prefix");
        assert_eq!(body["limit"], 1);
        assert_eq!(body["requested_limit"], 0);
        assert_eq!(body["total_matches"], 1);
        assert_eq!(body["candidates"][0]["entity_name"], "alpha");
        assert_eq!(body["graph_content_hash"], artifact.graph_content_hash);
        assert_eq!(body["rebuild_status"], "not_needed");
        assert_eq!(body["response_file_oids_match"], true);
    }

    #[tokio::test]
    async fn code_search_response_refreshes_dirty_empty_search() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn alpha() {}\n");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);

        fs::write(root.join("src/lib.rs"), "pub fn beta() {}\n").expect("rewrite source");

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_search_response(
                    &json!({
                        "query": "beta",
                        "mode": "exact",
                    }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await
            .expect("dirty search should refresh");

        assert_eq!(body["total_matches"], 1, "{body:#?}");
        assert_eq!(body["candidates"][0]["entity_name"], "beta");
        assert_eq!(body["rebuild_status"], "fresh");
        assert_eq!(body["worktree_dirty"], false);
    }

    #[test]
    fn code_subgraph_with_client_tables_budget_and_radius_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let artifact = subgraph_artifact(root);
        let alpha = symbol_id_for(&artifact, "alpha");
        let client = InMemoryClient::new(Arc::new(artifact));

        let body = code_subgraph_with_client(
            &json!({
                "start_nodes": [alpha],
                "response_format": "table",
                "radius": 99,
                "include_unresolved": true,
                "max_nodes": 0,
                "max_edges": 0,
            }),
            &client,
        )
        .expect("table subgraph");

        assert_eq!(body["response_format"], "table");
        assert_eq!(body["include_unresolved"], true);
        assert_eq!(body["metadata"]["radius"], MAX_MCP_CODE_SUBGRAPH_RADIUS);
        assert_eq!(body["metadata"]["max_nodes"], 1);
        assert_eq!(body["metadata"]["max_edges"], 1);
        assert_eq!(body["metadata"]["requested_max_nodes"], 0);
        assert_eq!(body["metadata"]["requested_max_edges"], 0);
        assert_eq!(body["metadata"]["truncated"], true);
        assert!(
            body["metadata"]["warning"]
                .as_str()
                .expect("warning")
                .contains("radius 99 exceeds max"),
            "{body:#?}"
        );
        assert_eq!(body["nodes"]["cols"][0], "id");
        assert_eq!(
            body["nodes"]["rows"].as_array().expect("node rows").len(),
            1
        );
        assert!(body["files"]
            .as_array()
            .expect("interned files")
            .contains(&json!("src/lib.rs")));
    }

    #[test]
    fn code_subgraph_with_client_returns_ambiguous_candidates_and_invalid_format_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let artifact = artifact_from_source(
            root,
            "pub mod left { pub fn duplicate() {} }\n\
             pub mod right { pub fn duplicate() {} }\n",
        );
        let client = InMemoryClient::new(Arc::new(artifact));

        let ambiguous = code_subgraph_with_client(
            &json!({
                "selector": "duplicate",
            }),
            &client,
        )
        .expect("ambiguous response");

        assert_eq!(ambiguous["ambiguous"], true);
        assert_eq!(
            ambiguous["candidates"]
                .as_array()
                .expect("candidate rows")
                .len(),
            2
        );

        let error = code_subgraph_with_client(
            &json!({
                "selector": "left::duplicate",
                "format": "dot",
            }),
            &client,
        )
        .expect_err("invalid format should fail")
        .into_handler_error();
        assert!(
            matches!(error, McpHandlerError::InvalidParams(message) if message.contains("invalid format `dot`"))
        );
    }

    #[test]
    fn code_subgraph_with_artifact_json_filters_unresolved_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let artifact = subgraph_artifact(root);
        let alpha = symbol_id_for(&artifact, "alpha");
        let beta = symbol_id_for(&artifact, "beta");

        let body = code_subgraph_with_artifact_and_temporal(
            &json!({
                "start_nodes": [alpha],
                "radius": 1,
                "edge_kinds": ["calls"],
            }),
            &artifact,
            None,
        )
        .expect("json subgraph");

        assert_eq!(body["include_unresolved"], false);
        assert_eq!(body["metadata"]["radius"], 1);
        assert_eq!(body["metadata"]["truncated"], false);
        assert!(
            body["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .any(|node| node["entity_name"] == "beta"),
            "{body:#?}"
        );
        assert_eq!(
            body["edges"]
                .as_array()
                .expect("edges")
                .iter()
                .filter(|edge| edge["target_uri"] == symbol_uri(&beta))
                .count(),
            1,
            "{body:#?}"
        );
        assert!(
            body["edges"]
                .as_array()
                .expect("edges")
                .iter()
                .all(|edge| edge["resolved"] == true),
            "unresolved edges should be filtered out: {body:#?}"
        );
    }

    #[test]
    fn code_subgraph_with_artifact_mermaid_includes_unresolved_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let artifact = subgraph_artifact(root);
        let alpha = symbol_id_for(&artifact, "alpha");

        let body = code_subgraph_with_artifact_and_temporal(
            &json!({
                "start_nodes": [alpha],
                "format": "mermaid",
                "radius": 1,
                "include_unresolved": true,
            }),
            &artifact,
            None,
        )
        .expect("mermaid subgraph");

        let mermaid = body["mermaid"].as_str().expect("mermaid text");
        assert!(mermaid.starts_with("graph TD"), "{mermaid}");
        assert!(mermaid.contains("alpha"), "{mermaid}");
        assert!(mermaid.contains("beta"), "{mermaid}");
        assert_eq!(body["include_unresolved"], true);
        assert_eq!(body["metadata"]["radius"], 1);
    }

    #[test]
    fn code_resolve_temporal_response_reports_resolution_variants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let (artifact, _commits, old_id, new_id, old_key, new_key) = temporal_rename_fixture(root);

        let renamed = code_resolve_temporal_response(
            &artifact,
            &old_id,
            "c3",
            Resolution::Found {
                value: new_id.clone(),
                chain: vec![old_key.clone(), new_key.clone()],
            },
        )
        .expect("renamed response");
        assert_eq!(renamed["resolution"]["kind"], "renamed");
        assert_eq!(renamed["resolution"]["symbol"], symbol_uri(&new_id));
        assert_eq!(renamed["candidates"][0]["entity_name"], "new_name");

        let found = code_resolve_temporal_response(
            &artifact,
            &new_id,
            "c3",
            Resolution::Found {
                value: new_id.clone(),
                chain: vec![new_key.clone()],
            },
        )
        .expect("found response");
        assert_eq!(found["resolution"]["kind"], "found");

        let deleted = code_resolve_temporal_response(
            &artifact,
            &old_id,
            "c3",
            Resolution::Deleted {
                last_seen: old_key.clone(),
            },
        )
        .expect("deleted response");
        assert_eq!(deleted["resolution"]["kind"], "deleted");
        assert_eq!(deleted["candidates"], json!([]));

        let ambiguous = code_resolve_temporal_response(
            &artifact,
            &old_id,
            "c3",
            Resolution::Ambiguous {
                candidates: vec![old_id.clone(), new_id.clone()],
            },
        )
        .expect("ambiguous response");
        assert_eq!(ambiguous["resolution"]["kind"], "ambiguous");
        assert_eq!(
            ambiguous["candidates"]
                .as_array()
                .expect("candidates")
                .len(),
            2
        );

        let unknown = code_resolve_temporal_response(
            &artifact,
            &old_id,
            "c3",
            Resolution::Unknown {
                reason: ResolutionFailure::SymbolNotPresentAtAnchor,
            },
        )
        .expect_err("unknown temporal response should be an error");
        let response = futures::executor::block_on(unknown.into_error_response());
        assert_eq!(response.code, CODE_GRAPH_UNKNOWN_ERROR_CODE);
        assert_eq!(response.data.expect("unknown data")["kind"], "unknown");
    }

    #[test]
    fn code_resolve_temporal_response_with_client_reports_resolution_variants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let (artifact, _commits, old_id, new_id, old_key, new_key) = temporal_rename_fixture(root);
        let client = InMemoryClient::new(Arc::new(artifact));

        let renamed = code_resolve_temporal_response_with_client(
            &client,
            &old_id,
            "c3",
            Resolution::Found {
                value: new_id.clone(),
                chain: vec![old_key.clone(), new_key.clone()],
            },
        )
        .expect("renamed response");
        assert_eq!(renamed["resolution"]["kind"], "renamed");
        assert_eq!(renamed["candidates"][0]["entity_name"], "new_name");

        let deleted = code_resolve_temporal_response_with_client(
            &client,
            &old_id,
            "c3",
            Resolution::Deleted {
                last_seen: old_key.clone(),
            },
        )
        .expect("deleted response");
        assert_eq!(deleted["resolution"]["kind"], "deleted");

        let ambiguous = code_resolve_temporal_response_with_client(
            &client,
            &old_id,
            "c3",
            Resolution::Ambiguous {
                candidates: vec![old_id.clone(), new_id.clone()],
            },
        )
        .expect("ambiguous response");
        assert_eq!(ambiguous["resolution"]["kind"], "ambiguous");
        assert_eq!(
            ambiguous["candidates"]
                .as_array()
                .expect("candidates")
                .len(),
            2
        );

        let unknown = code_resolve_temporal_response_with_client(
            &client,
            &old_id,
            "c3",
            Resolution::Unknown {
                reason: ResolutionFailure::IndexCorrupt("broken temporal edge".into()),
            },
        )
        .expect_err("unknown temporal response should be an error");
        let response = futures::executor::block_on(unknown.into_error_response());
        assert_eq!(response.code, CODE_GRAPH_UNKNOWN_ERROR_CODE);
        assert_eq!(
            response.data.expect("unknown data")["reason"]["kind"],
            "index_corrupt"
        );
    }

    #[tokio::test]
    async fn code_graph_error_response_maps_codes_and_merges_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = artifact_from_source(root, "pub fn alpha() {}\n");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);

        let invalid = CodeGraphError::from(McpHandlerError::InvalidParams("bad field".into()))
            .into_error_response()
            .await;
        assert_eq!(invalid.code, -32602);
        assert_eq!(invalid.message, "bad field");
        assert_eq!(invalid.data, None);

        let not_found = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                CodeGraphError::from(McpHandlerError::NotFound("missing symbol".into()))
                    .with_artifact_metadata(&artifact)
                    .into_error_response()
                    .await
            })
            .await;
        assert_eq!(not_found.code, CODE_GRAPH_NOT_FOUND_ERROR_CODE);
        let data = not_found.data.expect("metadata data");
        assert_eq!(data["kind"], "not_found");
        assert_eq!(data["graph_content_hash"], artifact.graph_content_hash);
        assert_eq!(data["rebuild_status"], "not_needed");

        let temporal = deleted_resolution_error(
            "old",
            "c3",
            SnapshotKey {
                stable_symbol_id: "old".into(),
                commit: "c2".into(),
            },
        )
        .into_error_response()
        .await;
        assert_eq!(temporal.code, CODE_GRAPH_DELETED_ERROR_CODE);
        assert_eq!(temporal.data.expect("temporal data")["kind"], "deleted");
    }

    #[tokio::test]
    async fn with_graph_metadata_for_payload_refreshes_dirty_payload() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = Arc::new(artifact_from_source(root, "pub fn alpha() {}\n"));
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);

        fs::write(root.join("src/lib.rs"), "pub fn beta() {}\n").expect("rewrite source");
        let payload = GraphResponsePayload::body(json!({
            "symbol": symbol_id_for(&artifact, "alpha"),
            "file_path": "src/lib.rs",
        }));
        let handler = |loaded: LoadedGraphArtifact| {
            let beta = loaded
                .artifact()
                .symbols
                .iter()
                .find(|symbol| symbol.entity_name == "beta")
                .ok_or_else(|| McpHandlerError::NotFound("beta missing after rebuild".into()))?;
            Ok(GraphResponsePayload::body(json!({
                "symbol": beta.stable_symbol_id.clone(),
                "entity_name": beta.entity_name.clone(),
                "file_path": beta.file_path.clone(),
            })))
        };

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                with_graph_metadata_for_payload(
                    Some(Arc::new(RebuildCoordinator::new())),
                    Arc::clone(&artifact),
                    payload,
                    &handler,
                )
                .await
            })
            .await;

        assert_eq!(body["entity_name"], "beta");
        assert_eq!(body["rebuild_status"], "fresh");
        assert_eq!(body["worktree_dirty"], false);
    }

    #[tokio::test]
    async fn with_graph_metadata_for_payload_serves_stale_when_handler_rejects_fresh_artifact() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        let artifact = Arc::new(artifact_from_source(root, "pub fn alpha() {}\n"));
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);
        write_current_artifact(root, &artifact);

        fs::write(root.join("src/lib.rs"), "pub fn beta() {}\n").expect("rewrite source");
        let payload = GraphResponsePayload::body(json!({
            "symbol": symbol_id_for(&artifact, "alpha"),
            "file_path": "src/lib.rs",
        }));
        let handler = |_loaded: LoadedGraphArtifact| -> CodeGraphPayloadResult {
            Err(McpHandlerError::Internal("fresh handler failed".into()).into())
        };

        let body = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                with_graph_metadata_for_payload(
                    Some(Arc::new(RebuildCoordinator::new())),
                    Arc::clone(&artifact),
                    payload,
                    &handler,
                )
                .await
            })
            .await;

        assert_eq!(body["symbol"], symbol_id_for(&artifact, "alpha"));
        assert_eq!(body["rebuild_status"], "stale_rebuild_failed");
        assert_eq!(body["worktree_dirty"], true);
    }

    #[test]
    fn resolve_symbol_as_of_follows_renames_and_reports_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let (artifact, commits, old_id, new_id, _old_key, _new_key) = temporal_rename_fixture(root);
        let temporal_index = Arc::new(TemporalIndex::new(Arc::new(artifact.clone())));

        let resolution = resolve_symbol_as_of(
            &artifact,
            Some(Arc::clone(&temporal_index)),
            &commits,
            &old_id,
            "c3",
        )
        .expect("resolve old symbol at c3");
        assert!(matches!(
            resolution,
            Resolution::Found { value, .. } if value == new_id
        ));

        let invalid_commit = resolve_symbol_as_of(
            &artifact,
            Some(Arc::clone(&temporal_index)),
            &commits,
            &old_id,
            "missing-commit",
        )
        .expect_err("missing as_of commit should fail")
        .into_handler_error();
        assert!(
            matches!(invalid_commit, McpHandlerError::InvalidParams(message) if message.contains("as_of commit `missing-commit` is not indexed"))
        );

        let no_history_artifact = artifact_from_source(root, "pub fn orphan() {}\n");
        let orphan_id = symbol_id_for(&no_history_artifact, "orphan");
        let no_history =
            resolve_symbol_as_of(&no_history_artifact, None, &commits, &orphan_id, "c3")
                .expect_err("symbol without snapshots should fail")
                .into_handler_error();
        assert!(
            matches!(no_history, McpHandlerError::NotFound(message) if message.contains("has no temporal history"))
        );
    }

    #[tokio::test]
    async fn persistent_incremental_failures_escalate_to_full_rebuild() {
        let _rebuild_guard = rebuild_test_guard().await;
        let _budget = set_graph_rebuild_latency_budget_for_test(Duration::from_secs(30));
        let _incremental_failures = set_incremental_rebuild_failures_for_test(usize::MAX);

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}\n").expect("write alpha");

        let facts = build_facts(root, None).expect("extract alpha").0;
        let previous_artifact =
            Arc::new(artifact_from_facts(&facts, root).expect("alpha artifact"));

        fs::write(root.join("src/lib.rs"), "pub fn beta() {}\n").expect("write beta");

        let coordinator = Arc::new(RebuildCoordinator::new());
        let key = RebuildKey::from("test-head", &BTreeMap::new());

        for attempt in 1..=ESCALATION_THRESHOLD {
            assert!(
                matches!(
                    try_rebuild_artifact(
                        Arc::clone(&coordinator),
                        Arc::clone(&previous_artifact),
                        rebuild_candidate(root, key.clone()),
                        None,
                    )
                    .await,
                    RebuildAttempt::StaleRebuildFailed
                ),
                "attempt {attempt} should serve stale after incremental failure"
            );
        }

        let rebuilt = match try_rebuild_artifact(
            Arc::clone(&coordinator),
            Arc::clone(&previous_artifact),
            rebuild_candidate(root, key),
            None,
        )
        .await
        {
            RebuildAttempt::Fresh(artifact) => artifact,
            RebuildAttempt::StaleBudgetExceeded => {
                panic!("full rebuild escalation should not exceed the test budget")
            }
            RebuildAttempt::StaleRebuildFailed => {
                panic!("persistent incremental failures should escalate to a fresh full rebuild")
            }
        };

        assert!(
            rebuilt
                .symbols
                .iter()
                .any(|symbol| symbol.entity_name == "beta"),
            "fresh full rebuild should index the rewritten file"
        );
    }

    #[tokio::test]
    async fn code_read_symbol_preserves_stale_success_when_refresh_loses_symbol() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn stable_symbol() -> bool {\n    true\n}\n",
        )
        .expect("write stable symbol");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index stable symbol"]);

        let facts = build_facts(root, None).expect("extract stable symbol").0;
        let artifact = artifact_from_facts(&facts, root).expect("stable symbol artifact");
        let stable_symbol_id = artifact
            .symbols
            .iter()
            .find(|symbol| symbol.entity_name == "stable_symbol")
            .expect("stable_symbol in artifact")
            .stable_symbol_id
            .clone();
        write_current_artifact(root, &artifact);

        fs::write(
            root.join("src/lib.rs"),
            "pub const REPLACEMENT_SYMBOL: bool = false;\n",
        )
        .expect("replace tracked file content");

        let response = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                code_read_symbol_response(
                    &json!({
                        "stable_symbol_id": stable_symbol_id,
                    }),
                    Arc::new(RebuildCoordinator::new()),
                    false,
                )
                .await
            })
            .await;

        let body = response.expect("stale read should preserve the successful indexed-source body");
        assert_eq!(body["stale"], Value::Bool(true));
        assert!(
            body["source"]
                .as_str()
                .expect("source string")
                .contains("pub fn stable_symbol() -> bool"),
            "stale response should serve the indexed source: {body:#?}"
        );
    }

    #[tokio::test]
    async fn analyze_source_inner_builds_rebuild_candidate_for_dirty_indexed_file_without_response_files(
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}\n").expect("write alpha");
        run_git_test(root, &["add", "src/lib.rs"]);
        run_git_test(root, &["commit", "-q", "-m", "index alpha"]);

        let facts = build_facts(root, None).expect("extract alpha").0;
        let artifact = artifact_from_facts(&facts, root).expect("alpha artifact");
        write_current_artifact(root, &artifact);

        fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() { let _edited = true; }\n",
        )
        .expect("edit tracked file");

        let indexed_files = all_indexed_file_set(&artifact);
        let analysis = SCOPED_CODE_GRAPH_WORKTREE_ROOT
            .scope(root.to_path_buf(), async {
                GraphResponseMetadata::analyze_source_inner(
                    GraphMetadataSource::from_artifact(&artifact),
                    None,
                    Some(&indexed_files),
                )
                .await
            })
            .await;

        assert_eq!(analysis.metadata.worktree_dirty, Some(true));
        assert!(
            analysis.rebuild_candidate.is_some(),
            "plain tracked-file edits must trigger retry candidate computation without response files"
        );
    }

    fn rebuild_candidate(root: &Path, key: RebuildKey) -> RebuildCandidate {
        RebuildCandidate {
            worktree: root.to_path_buf(),
            key,
        }
    }

    async fn rebuild_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
        REBUILD_TEST_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    fn artifact_from_source(root: &Path, source: &str) -> GraphIndexArtifact {
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), source).expect("write source");
        let facts = build_facts(root, None).expect("extract source").0;
        artifact_from_facts(&facts, root).expect("artifact from facts")
    }

    fn subgraph_artifact(root: &Path) -> GraphIndexArtifact {
        let mut artifact = artifact_from_source(
            root,
            "pub fn alpha() {\n\
                 beta();\n\
                 missing();\n\
             }\n\
             pub fn beta() {}\n\
             pub fn gamma() {\n\
                 alpha();\n\
             }\n",
        );
        let alpha = symbol_id_for(&artifact, "alpha");
        artifact.edges.push(GraphEdgeArtifact {
            source_stable_symbol_id: alpha,
            target_stable_symbol_id: None,
            target_label: Some("missing".into()),
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation: RelationKind::Calls,
            confidence: Confidence::Unknown,
            confidence_score: 0.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::Calls),
            bind_method: None,
        });
        artifact
    }

    fn symbol_id_for(artifact: &GraphIndexArtifact, entity_name: &str) -> String {
        artifact
            .symbols
            .iter()
            .find(|symbol| symbol.entity_name == entity_name)
            .unwrap_or_else(|| panic!("symbol {entity_name} not found"))
            .stable_symbol_id
            .clone()
    }

    #[allow(clippy::type_complexity)]
    fn temporal_rename_fixture(
        root: &Path,
    ) -> (
        GraphIndexArtifact,
        CommitIndexArtifact,
        String,
        String,
        SnapshotKey,
        SnapshotKey,
    ) {
        let mut artifact = artifact_from_source(
            root,
            "pub fn old_name() {}\n\
             pub fn new_name() {}\n",
        );
        let old_id = symbol_id_for(&artifact, "old_name");
        let new_id = symbol_id_for(&artifact, "new_name");
        let c1 = CommitArtifact {
            sha: "c1".into(),
            parents: vec![],
            author_time: 0,
            author_name: String::new(),
            author_email: String::new(),
            summary: "add old".into(),
        };
        let c2 = CommitArtifact {
            sha: "c2".into(),
            parents: vec!["c1".into()],
            author_time: 1,
            author_name: String::new(),
            author_email: String::new(),
            summary: "rename old to new".into(),
        };
        let c3 = CommitArtifact {
            sha: "c3".into(),
            parents: vec!["c2".into()],
            author_time: 2,
            author_name: String::new(),
            author_email: String::new(),
            summary: "later".into(),
        };
        let old_key = SnapshotKey {
            stable_symbol_id: old_id.clone(),
            commit: "c1".into(),
        };
        let new_key = SnapshotKey {
            stable_symbol_id: new_id.clone(),
            commit: "c2".into(),
        };
        let old_symbol = artifact
            .symbols
            .iter()
            .find(|symbol| symbol.stable_symbol_id == old_id)
            .expect("old symbol");
        let new_symbol = artifact
            .symbols
            .iter()
            .find(|symbol| symbol.stable_symbol_id == new_id)
            .expect("new symbol");
        let old_snapshot = snapshot_for_symbol(old_symbol, old_key.clone(), "old-anchor");
        let new_snapshot = snapshot_for_symbol(new_symbol, new_key.clone(), "new-anchor");

        artifact.commits = vec![c1.clone(), c2.clone(), c3.clone()];
        artifact.symbol_snapshots = vec![old_snapshot, new_snapshot];
        artifact.temporal_edges = vec![
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Commit { sha: "c1".into() },
                target: EdgeEndpoint::Snapshot {
                    key: old_key.clone(),
                },
                relation: RelationKind::Touches,
                parent: None,
                change_kind: Some(ChangeKind::Added),
            },
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Commit { sha: "c2".into() },
                target: EdgeEndpoint::Snapshot {
                    key: new_key.clone(),
                },
                relation: RelationKind::Touches,
                parent: Some("c1".into()),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(old_key.clone()))),
            },
            TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot {
                    key: old_key.clone(),
                },
                target: EdgeEndpoint::Snapshot {
                    key: new_key.clone(),
                },
                relation: RelationKind::Touches,
                parent: Some("c1".into()),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(old_key.clone()))),
            },
        ];

        let commits = CommitIndexArtifact {
            schema_version: 1,
            commits: vec![c1, c2, c3],
            refs: [("main".into(), "c3".into())].into(),
            indexed_at: "2026-05-20T12:00:00Z".into(),
            walk_strategy: WalkStrategy::Reachable,
        };

        (artifact, commits, old_id, new_id, old_key, new_key)
    }

    fn snapshot_for_symbol(
        symbol: &GraphSymbolArtifact,
        key: SnapshotKey,
        anchor_hash: &str,
    ) -> SymbolSnapshotArtifact {
        SymbolSnapshotArtifact {
            key,
            file_path: symbol.file_path.clone().into(),
            entity_name: symbol.entity_name.clone(),
            symbol_kind: symbol.symbol_kind.clone(),
            enclosing_scope: symbol.enclosing_scope.clone(),
            byte_range: symbol.byte_range,
            line_range: symbol.line_range,
            anchor_hash: anchor_hash.into(),
            tokens: vec![],
        }
    }

    fn init_git_repo(root: &Path) {
        run_git_test(root, &["init", "-q", "-b", "main"]);
        run_git_test(
            root,
            &["config", "user.email", "spur-graph@example.invalid"],
        );
        run_git_test(root, &["config", "user.name", "Spur Graph Test"]);
    }

    fn write_current_artifact(root: &Path, artifact: &GraphIndexArtifact) {
        let artifact_base = root.join(".spur/graph");
        let written = crate::write_artifact_parquet(
            artifact,
            &artifact_base,
            crate::WriteOptions::default(),
            Vec::new(),
        )
        .expect("write worktree graph artifact");
        crate::write_current_pointer(root, &written).expect("write CURRENT pointer");
    }

    fn write_current_artifact_indexed_at_head(root: &Path, artifact: &GraphIndexArtifact) {
        write_current_artifact(root, artifact);
        let artifact_base = root.join(".spur/graph");
        let manifest_path = artifact_base.join("manifest.json");
        let mut manifest: crate::GraphArtifactManifest = serde_json::from_slice(
            &fs::read(&manifest_path).expect("read generation fixture manifest"),
        )
        .expect("decode generation fixture manifest");
        manifest.indexed_commit_oid = Some(git_stdout_test(root, &["rev-parse", "HEAD"]));
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("encode generation fixture manifest"),
        )
        .expect("write generation fixture manifest");
        crate::write_current_pointer(root, &artifact_base)
            .expect("rewrite indexed generation CURRENT pointer");
    }

    fn run_git_test(root: &Path, args: &[&str]) {
        let _ = git_stdout_test(root, args);
    }

    fn git_stdout_test(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git test output must be UTF-8")
            .trim()
            .to_owned()
    }
}
