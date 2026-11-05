//! TUI mention registry — the public `spur_tui::mentions` facade.
//!
//! Snapshot resolution (filesystem traversal, TTL caching, source-swap
//! and root invalidation) is delegated to [`spur_mentions::MentionEngine`]:
//! the session sources (`worker`, `issue`, `datasource`) implement the
//! neutral [`spur_mentions::MentionSource`] themselves with their TUI-only
//! row state in the sidecar each build writes (sud-m3), while the
//! filesystem/code-graph sources go through thin engine adapters that
//! park their TUI rows + extras. This module keeps the TUI surface the app
//! and pickers consume (`query`, `prepare_query_work`/`run_query_work`,
//! snapshot setters, code-payload retention, worker-slot composition and
//! agent-model-catalog probing — decoupling spec §4.1). Ranking reuses the
//! neutral engine primitives (`spur_mentions::rank`) for tier/bucket
//! ordering while the empty-query section layout, code-specific scoring,
//! and code-payload hydration stay frontend policy here.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use spur_acp::{
    agent_model_catalog::{self, ConfigOptionChoice},
    DatasourceEntry, SessionId,
};
use spur_mentions::{
    rank, MentionEngine, MentionEntry as NeutralMentionEntry, MentionId,
    MentionSource as NeutralMentionSource, QueryOptions, SnapshotResolution, SourceBuildError,
    SourceContext, SourceSnapshot, SystemClock,
};

use super::code_graph::source::{
    entry_for_candidate, CodeGraphMentionSource, CodeMentionCandidate, CodePayloadSnapshot,
};
use super::datasource_source::DatasourceMentionSource;
use super::entry::{MentionEntry, MentionKind, MentionSource};
use super::file_source::FileMentionSource;
use super::issue_source::{IssueMentionDescriptor, IssueMentionSource};
use super::session::SessionMentionSource;
use super::worker_source::{WorkerMentionDescriptor, WorkerMentionSource};
use spur_graph::{CodeMentionPayload, CODE_SYMBOL_URI_PREFIX};

pub const CODE_GRAPH_INDEX_ENV: &str = "SPUR_CODE_GRAPH_INDEX";
pub const CODE_GRAPH_MISSING_HINT: &str = "Run 'spur graph build' to enable code-graph mentions";
const CODE_GRAPH_POINTER_SCHEMA: &str = "spur-graph-pointer-v1";
const CODE_GRAPH_POINTER_PATH: &str = ".spur/graph-index.pointer.json";

/// Maximum number of worker rows pinned to the top of the empty-query
/// picker view. See design spec §4.4 / §10.1.
pub(super) const WORKER_PIN_CAP: usize = 4;
const FILE_CAP: usize = 6;
const ISSUE_CAP: usize = 3;
const DATASOURCE_CAP: usize = 4;
const CODE_CAP: usize = 3;

/// Facade-side materialization of one source snapshot: the TUI rows plus
/// the adapter-owned extras (code payloads/candidates, datasource prompt
/// hints) that travel with them. Freshness, cache identity, and TTL are
/// the engine's concern; each materialization belongs to the exact engine
/// snapshot that produced it, including its sidecar and payload stores.
struct SourceSnapshotData {
    entries: Arc<Vec<MentionEntry>>,
    code_payloads: HashMap<String, Arc<CodeMentionPayload>>,
    code_index: Option<CodeQueryIndex>,
    datasource_hints: HashMap<String, Arc<String>>,
}

/// Retain frontend data for each engine generation, and expose only the
/// generations selected for the current query. Weak references let replaced
/// or invalidated engine snapshots release their materializations too.
#[derive(Default)]
struct SnapshotMaterializations {
    active: HashMap<&'static str, u64>,
    generations: HashMap<(&'static str, u64), (Weak<SourceSnapshot>, SourceSnapshotData)>,
}

impl SnapshotMaterializations {
    fn begin_query(&mut self) {
        self.active.clear();
        self.generations
            .retain(|_, (snapshot, _)| snapshot.strong_count() > 0);
    }

    fn insert(
        &mut self,
        name: &'static str,
        snapshot: &Arc<SourceSnapshot>,
        data: SourceSnapshotData,
    ) {
        self.generations.insert(
            (name, snapshot.generation),
            (Arc::downgrade(snapshot), data),
        );
    }

    fn select(&mut self, name: &'static str, snapshot: &SourceSnapshot) {
        self.active.insert(name, snapshot.generation);
    }

    fn get(&self, name: &str) -> Option<&SourceSnapshotData> {
        let (&name, &generation) = self.active.get_key_value(name)?;
        self.generations
            .get(&(name, generation))
            .map(|(_, data)| data)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut SourceSnapshotData> {
        let (&name, &generation) = self.active.get_key_value(name)?;
        self.generations
            .get_mut(&(name, generation))
            .map(|(_, data)| data)
    }

    fn values(&self) -> impl Iterator<Item = &SourceSnapshotData> {
        self.active.keys().filter_map(|name| self.get(name))
    }

    fn all_values_mut(&mut self) -> impl Iterator<Item = &mut SourceSnapshotData> {
        self.generations.values_mut().map(|(_, data)| data)
    }

    fn clear(&mut self) {
        self.active.clear();
        self.generations.clear();
    }

    fn remove(&mut self, name: &str) {
        self.active.remove(name);
        self.generations.retain(|(key, _), _| *key != name);
    }
}

#[derive(Clone)]
struct CodeQueryIndex {
    candidates: Arc<Vec<CodeMentionCandidate>>,
    payload_snapshot: Option<CodePayloadSnapshot>,
    source: Arc<Mutex<Box<dyn MentionSource>>>,
}

#[derive(Clone)]
struct MentionSourceSlot {
    name: &'static str,
    kind: SourceSlotKind,
}

/// Which flavor of source a slot holds (decoupling spec §5 Phase M3).
#[derive(Clone)]
enum SourceSlotKind {
    /// TUI-trait source (`file`, `code_graph`): builds run through
    /// [`TuiEngineSourceAdapter`], which parks the TUI rows + extras for
    /// the facade to adopt. Code payloads/candidates and payload
    /// hydration stay reachable through the TUI trait handle.
    Tui {
        source: Arc<Mutex<Box<dyn MentionSource>>>,
        /// Build output parked by the engine-side adapter for the facade
        /// to adopt when the engine reports a freshly built snapshot.
        built: Arc<Mutex<Option<SourceSnapshotData>>>,
    },
    /// Session source (`worker`, `issue`, `datasource`): implements the
    /// neutral `spur_mentions::MentionSource` directly, so the engine
    /// builds it without a wrapper. Each build refreshes the source's
    /// sidecar (+ adapter-owned extras); the facade adopts the engine's
    /// snapshot by rejoining it with that sidecar.
    Session {
        source: Arc<Mutex<Box<dyn SessionMentionSource>>>,
    },
}

impl MentionSourceSlot {
    fn new(source: Box<dyn MentionSource>) -> Self {
        let name = source.name();
        Self {
            name,
            kind: SourceSlotKind::Tui {
                source: Arc::new(Mutex::new(source)),
                built: Arc::new(Mutex::new(None)),
            },
        }
    }

    fn session(source: Box<dyn SessionMentionSource>) -> Self {
        let name = source.slot_name();
        Self {
            name,
            kind: SourceSlotKind::Session {
                source: Arc::new(Mutex::new(source)),
            },
        }
    }

    /// The engine-facing registration for this slot.
    fn engine_adapter(&self) -> Box<dyn NeutralMentionSource> {
        match &self.kind {
            SourceSlotKind::Tui { source, built } => Box::new(TuiEngineSourceAdapter {
                name: self.name,
                source: Arc::clone(source),
                built: Arc::clone(built),
            }),
            SourceSlotKind::Session { source } => Box::new(SessionEngineSourceAdapter {
                name: self.name,
                source: Arc::clone(source),
            }),
        }
    }

    /// Facade-side materialization of a snapshot the engine just built
    /// through this slot. TUI-flavor slots read the data their adapter
    /// parked; session-flavor slots rejoin the neutral entries with the
    /// sidecar their source retained and collect the adapter-owned extras.
    fn adopt_built(&self, snapshot: &SourceSnapshot) -> Option<SourceSnapshotData> {
        match &self.kind {
            SourceSlotKind::Tui { built, .. } => built
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take(),
            SourceSlotKind::Session { source } => {
                let guard = source.lock().unwrap_or_else(|error| error.into_inner());
                let entries = Arc::new(super::sidecar::rejoin_snapshot(
                    &snapshot.entries,
                    guard.sidecar(),
                ));
                let datasource_hints = guard.datasource_hints().cloned().unwrap_or_default();
                Some(SourceSnapshotData {
                    entries,
                    code_payloads: HashMap::new(),
                    code_index: None,
                    datasource_hints,
                })
            }
        }
    }

    /// Whether `other` is the same slot instance (name + shared source
    /// identity) — the staleness check deferred builds apply before
    /// publishing their results.
    fn same_identity(&self, other: &Self) -> bool {
        self.name == other.name
            && match (&self.kind, &other.kind) {
                (
                    SourceSlotKind::Tui { source: left, .. },
                    SourceSlotKind::Tui { source: right, .. },
                ) => Arc::ptr_eq(left, right),
                (
                    SourceSlotKind::Session { source: left },
                    SourceSlotKind::Session { source: right },
                ) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }

    /// Current data-revision token of a session-flavor slot (the value the
    /// engine folds into its cache key); `None` for TUI-flavor slots.
    fn session_source_token(&self) -> Option<u64> {
        match &self.kind {
            SourceSlotKind::Session { source } => Some(
                source
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .source_token(),
            ),
            SourceSlotKind::Tui { .. } => None,
        }
    }
}

/// Neutral [`spur_mentions::MentionSource`] view of one TUI-flavor source
/// slot (file / code graph). Registered in the registry's
/// [`MentionEngine`] so snapshot builds, TTL accounting, and cache
/// identity live engine-side; the TUI rows and extras produced by each
/// build are parked on the shared `built` cell for the facade to adopt.
struct TuiEngineSourceAdapter {
    name: &'static str,
    source: Arc<Mutex<Box<dyn MentionSource>>>,
    built: Arc<Mutex<Option<SourceSnapshotData>>>,
}

impl NeutralMentionSource for TuiEngineSourceAdapter {
    fn key(&self) -> &str {
        self.name
    }

    fn build(
        &mut self,
        root: &Path,
        _context: &SourceContext,
    ) -> Result<SourceSnapshot, SourceBuildError> {
        let data = build_source_snapshot_data(&self.source, root).map_err(SourceBuildError::new)?;
        let snapshot = SourceSnapshot::new(neutral_entries_of(&data.entries), 0);
        *self.built.lock().unwrap_or_else(|error| error.into_inner()) = Some(data);
        Ok(snapshot)
    }
}

/// Neutral engine view of one session-flavor slot (worker / issue /
/// datasource). The source itself implements the neutral trait; this shim
/// only shares it with the engine (and forwards the data-revision token
/// so snapshot swaps invalidate cache identity immediately).
struct SessionEngineSourceAdapter {
    name: &'static str,
    source: Arc<Mutex<Box<dyn SessionMentionSource>>>,
}

impl NeutralMentionSource for SessionEngineSourceAdapter {
    fn key(&self) -> &str {
        self.name
    }

    fn source_token(&self) -> u64 {
        self.source
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .source_token()
    }

    fn build(
        &mut self,
        root: &Path,
        context: &SourceContext,
    ) -> Result<SourceSnapshot, SourceBuildError> {
        self.source
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .build(root, context)
    }
}

/// Neutral entry view of one TUI snapshot's rows: the shared crate's row
/// shape (id/kind/uri/display/secondary/search_text) with the TUI-only
/// fields left to the facade's parked snapshot data. Kinds map through the
/// M1 sidecar's category projection.
fn neutral_entries_of(entries: &[MentionEntry]) -> Vec<NeutralMentionEntry> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| NeutralMentionEntry {
            id: MentionId::new(index as u64),
            kind: super::sidecar::neutral_kind(&entry.kind),
            uri: entry.uri.clone(),
            display: entry.display.clone(),
            secondary: entry.secondary.clone(),
            search_text: entry.search_text.clone(),
            insert_text: None,
        })
        .collect()
}

/// Scope for completion cache lookup. Dashboard pre-session composition
/// has no real ACP session id yet, while session-detail composition does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionScope<'a> {
    PreSession,
    Session(&'a SessionId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CompletionScopeKey {
    PreSession,
    Session(SessionId),
}

impl From<CompletionScope<'_>> for CompletionScopeKey {
    fn from(scope: CompletionScope<'_>) -> Self {
        match scope {
            CompletionScope::PreSession => CompletionScopeKey::PreSession,
            CompletionScope::Session(session) => CompletionScopeKey::Session(session.clone()),
        }
    }
}

pub struct MentionRegistry {
    sources: Vec<MentionSourceSlot>,
    /// Engine-owned snapshot cache: traversal builds, TTL accounting,
    /// cache identity (canonical root + source key + profile + token), and
    /// invalidation all run through the per-source seams here.
    engine: MentionEngine,
    /// Facade-side materializations selected by the engine's snapshot identity.
    snapshots: SnapshotMaterializations,
    retained_code_payloads: HashMap<String, Arc<CodeMentionPayload>>,
    pinned_code_payload_uris: HashSet<String>,
    code_graph_hint: Option<&'static str>,
    code_graph_token: Option<CodeGraphToken>,
    code_graph_auto_discovery: bool,
    agent_model_catalog_path: Option<PathBuf>,
    pending_agent_model_catalog_probe_requests: HashSet<String>,
    /// Workers whose catalog probe was handed to the app (drained) and has
    /// not reported completion yet. Surfaced to pickers as a hint so the
    /// missing model/effort slots are explainable while the probe runs.
    agent_model_catalog_probes_in_flight: HashSet<String>,
    /// Test observation for the synchronous compatibility API. Production
    /// pickers receive the slot in `MentionQueryResult`.
    #[cfg(test)]
    last_worker_open_slot: Option<OpenWorkerSlot>,
    matcher: Matcher,
    #[cfg(any(test, debug_assertions))]
    query_call_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodeGraphToken {
    Pointer {
        canonical_artifact_path: PathBuf,
        graph_content_hash: String,
        manifest_version: String,
        indexed_commit_oid: Option<String>,
    },
    Resolved {
        artifact_path: PathBuf,
        cache_key: spur_graph::ArtifactCacheKey,
        modified: Option<SystemTime>,
        len: u64,
    },
}

#[derive(Debug)]
struct CodeGraphCandidate {
    explicit_override: Option<PathBuf>,
    token: CodeGraphToken,
}

enum CodeGraphSourceUpdate {
    ClearAllCaches,
    ClearCodeGraphCache,
}

pub(crate) struct MentionQueryWork {
    sources: Vec<Arc<Vec<MentionEntry>>>,
    code_indexes: Vec<CodeQueryIndex>,
    query: String,
    limit: usize,
    composed_entry: Option<MentionEntry>,
    open_slot: Option<OpenWorkerSlot>,
}

impl MentionQueryWork {
    pub(crate) fn entry_count(&self) -> usize {
        self.sources
            .iter()
            .map(|entries| entries.len())
            .sum::<usize>()
            + self
                .code_indexes
                .iter()
                .map(|index| index.candidates.len())
                .sum::<usize>()
    }
}

pub(crate) struct MentionQueryResult {
    pub(crate) entries: Vec<MentionEntry>,
    pub(crate) code_payloads: Vec<(String, Arc<CodeMentionPayload>)>,
    pub(crate) open_slot: Option<OpenWorkerSlot>,
}

pub(crate) enum MentionQueryPreparation {
    Ready(MentionQueryWork),
    ReadyAndBuild {
        work: MentionQueryWork,
        build: MentionCacheBuildWork,
    },
}

pub(crate) struct MentionCacheBuildWork {
    sources: Vec<MentionSourceSlot>,
    cwd: PathBuf,
}

struct MentionCacheBuildUpdate {
    source: MentionSourceSlot,
    data: SourceSnapshotData,
}

pub(crate) struct MentionCacheBuildResult {
    cwd: PathBuf,
    updates: Vec<MentionCacheBuildUpdate>,
}

/// Query options the facade drives every engine seam with: the TUI
/// compatibility filesystem profile (fingerprinted into every cache key).
fn engine_query_options() -> QueryOptions {
    QueryOptions::new()
}

impl MentionRegistry {
    fn with_source_slots(slots: Vec<MentionSourceSlot>) -> Self {
        let mut engine = MentionEngine::new(Arc::new(SystemClock));
        for slot in &slots {
            engine.register_source(slot.engine_adapter(), false);
        }
        Self {
            sources: slots,
            engine,
            snapshots: SnapshotMaterializations::default(),
            retained_code_payloads: HashMap::new(),
            pinned_code_payload_uris: HashSet::new(),
            code_graph_hint: None,
            code_graph_token: None,
            code_graph_auto_discovery: false,
            agent_model_catalog_path: None,
            pending_agent_model_catalog_probe_requests: HashSet::new(),
            agent_model_catalog_probes_in_flight: HashSet::new(),
            #[cfg(test)]
            last_worker_open_slot: None,
            matcher: Matcher::new(Config::DEFAULT),
            #[cfg(any(test, debug_assertions))]
            query_call_count: 0,
        }
    }

    /// Source list for direct (single-agent) sessions. Files only.
    pub fn for_direct_session() -> Self {
        Self::with_source_slots(vec![MentionSourceSlot::new(Box::new(
            FileMentionSource::new(),
        ))])
    }

    /// Source list for brain sessions. Files + workers.
    /// `workers` is the snapshot derived from the agent registry.
    pub fn for_brain_session(workers: Vec<super::WorkerMentionDescriptor>) -> Self {
        Self::with_source_slots(vec![
            MentionSourceSlot::new(Box::new(FileMentionSource::new())),
            MentionSourceSlot::session(Box::new(WorkerMentionSource::new(workers))),
        ])
    }

    pub fn with_code_graph(mut self, artifact_path: impl Into<PathBuf>) -> Self {
        self.code_graph_auto_discovery = false;
        let artifact_path = artifact_path.into();
        self.set_code_graph_source(
            None,
            Some(artifact_path),
            None,
            CodeGraphSourceUpdate::ClearAllCaches,
        );
        self.code_graph_hint = None;
        self
    }

    /// Opt-in runtime code-graph source registration.
    ///
    /// Resolution order:
    /// 1. `SPUR_CODE_GRAPH_INDEX=<path>` env var, if set and non-empty.
    /// 2. `<worktree_root>/.spur/graph/CURRENT`.
    /// 3. `<worktree_root>/.spur/graph-index.pointer.json`.
    pub fn with_code_graph_from_env(mut self) -> Self {
        self.code_graph_auto_discovery = true;
        let worktree_root = spur_graph::resolve_worktree_root();
        self.refresh_code_graph_registration(&worktree_root);
        self
    }

    fn refresh_code_graph_registration(&mut self, cwd: &Path) {
        if !self.code_graph_auto_discovery {
            return;
        }
        let worktree_root = spur_graph::resolve_worktree_root_from(cwd.to_path_buf());
        match discover_code_graph_candidate(&worktree_root) {
            Some(candidate) => {
                if self.code_graph_token.as_ref() != Some(&candidate.token)
                    || !self.has_code_graph_source()
                {
                    self.set_code_graph_source(
                        Some(worktree_root.clone()),
                        candidate.explicit_override,
                        Some(candidate.token),
                        CodeGraphSourceUpdate::ClearCodeGraphCache,
                    );
                }
                self.code_graph_hint = None;
            }
            None => {
                self.remove_code_graph_source();
                self.code_graph_token = None;
                self.code_graph_hint = Some(CODE_GRAPH_MISSING_HINT);
            }
        }
    }

    fn set_code_graph_source(
        &mut self,
        worktree_root: Option<PathBuf>,
        explicit_override: Option<PathBuf>,
        token: Option<CodeGraphToken>,
        cache_update: CodeGraphSourceUpdate,
    ) {
        let source = MentionSourceSlot::new(match worktree_root {
            Some(worktree_root) => Box::new(CodeGraphMentionSource::for_worktree(
                worktree_root,
                explicit_override,
            )) as Box<dyn MentionSource>,
            None => Box::new(CodeGraphMentionSource::new(
                explicit_override.expect("manual code graph source requires an artifact path"),
            )) as Box<dyn MentionSource>,
        });
        // One engine registration per source key: the new adapter replaces
        // the previous registration (its generation counter carries over).
        self.engine.register_source(source.engine_adapter(), false);
        if let Some(index) = self
            .sources
            .iter()
            .position(|source| source.name == "code_graph")
        {
            self.sources[index] = source;
        } else {
            let insert_at = self
                .sources
                .iter()
                .position(|source| source.name != "file")
                .unwrap_or(self.sources.len());
            self.sources.insert(insert_at, source);
        }
        self.code_graph_token = token;
        self.code_graph_hint = None;
        match cache_update {
            CodeGraphSourceUpdate::ClearAllCaches => self.clear_cache(),
            CodeGraphSourceUpdate::ClearCodeGraphCache => self.clear_cache_for("code_graph"),
        }
    }

    fn has_code_graph_source(&self) -> bool {
        self.sources
            .iter()
            .any(|source| source.name == "code_graph")
    }

    fn remove_code_graph_source(&mut self) {
        let previous_len = self.sources.len();
        self.sources.retain(|source| source.name != "code_graph");
        if self.sources.len() != previous_len {
            self.clear_cache_for("code_graph");
        }
    }

    /// Back-compat alias used by tests and any caller that doesn't
    /// know the session role. Equivalent to `for_direct_session()`.
    pub fn new() -> Self {
        Self::for_direct_session()
    }

    /// Drop all cached per-session indexes. Call after the agent
    /// registry reloads so the next `query()` rebuilds with the
    /// fresh worker snapshot.
    ///
    /// Currently has no caller — wired up when live config-reload
    /// support is added (out of scope for v1).
    pub fn clear_cache(&mut self) {
        self.engine.clear_cache();
        self.snapshots.clear();
    }

    fn clear_cache_for(&mut self, name: &'static str) {
        self.engine.invalidate_source(name);
        self.snapshots.remove(name);
    }

    pub fn lookup_code_payload(&self, uri: &str) -> Option<&CodeMentionPayload> {
        self.retained_code_payloads
            .get(uri)
            .or_else(|| {
                self.snapshots
                    .values()
                    .find_map(|cached| cached.code_payloads.get(uri))
            })
            .map(Arc::as_ref)
    }

    pub(crate) fn install_query_code_payloads(
        &mut self,
        payloads: Vec<(String, Arc<CodeMentionPayload>)>,
    ) {
        self.retained_code_payloads
            .retain(|uri, _| self.pinned_code_payload_uris.contains(uri));
        let Some(cached) = self.snapshots.get_mut("code_graph") else {
            return;
        };
        cached
            .code_payloads
            .retain(|uri, _| !is_graph_symbol_uri(uri));
        cached.code_payloads.extend(payloads);
    }

    pub(crate) fn retain_code_payload_on_accept(&mut self, uri: &str) {
        if !is_graph_symbol_uri(uri) {
            return;
        }
        let payload = self
            .snapshots
            .get("code_graph")
            .and_then(|cached| cached.code_payloads.get(uri))
            .cloned();
        if let Some(payload) = payload {
            self.retained_code_payloads.insert(uri.to_owned(), payload);
            self.pinned_code_payload_uris.insert(uri.to_owned());
        }
    }

    pub fn lookup_datasource_hint(&self, uri: &str) -> Option<&str> {
        self.snapshots
            .values()
            .find_map(|cached| cached.datasource_hints.get(uri))
            .map(Arc::as_ref)
            .map(String::as_str)
    }

    pub fn code_graph_hint(&self) -> Option<&'static str> {
        self.code_graph_hint
    }

    pub fn retain_code_payloads_for_uris<'a>(&mut self, uris: impl IntoIterator<Item = &'a str>) {
        let keep: std::collections::HashSet<&str> = uris.into_iter().collect();
        self.retained_code_payloads
            .retain(|uri, _| keep.contains(uri.as_str()));
        self.pinned_code_payload_uris.clear();
        for cached in self.snapshots.all_values_mut() {
            cached
                .code_payloads
                .retain(|uri, _| !is_graph_symbol_uri(uri) || keep.contains(uri.as_str()));
        }
    }

    pub fn drain_agent_model_catalog_probe_requests(&mut self) -> Vec<String> {
        let mut requests: Vec<String> = self
            .pending_agent_model_catalog_probe_requests
            .drain()
            .collect();
        requests.sort();
        self.agent_model_catalog_probes_in_flight
            .extend(requests.iter().cloned());
        requests
    }

    /// Clears a worker from the in-flight probe set once its background
    /// agent-model-catalog probe reports completion (success or failure).
    pub fn mark_agent_model_catalog_probe_completed(&mut self, worker_name: &str) {
        self.agent_model_catalog_probes_in_flight
            .remove(worker_name);
    }

    /// Human-readable hint while agent-model-catalog probes are pending or
    /// in flight, e.g. `fetching codex models…`. `None` when nothing is
    /// being probed. Pending requests count too: the compose that marks a
    /// request is the same pass that renders the degraded cascade, so the
    /// hint must be visible before the app has drained the request.
    pub fn agent_model_catalog_probe_hint(&self) -> Option<String> {
        let mut names: Vec<&str> = self
            .pending_agent_model_catalog_probe_requests
            .iter()
            .chain(self.agent_model_catalog_probes_in_flight.iter())
            .map(String::as_str)
            .collect();
        if names.is_empty() {
            return None;
        }
        names.sort_unstable();
        names.dedup();
        Some(format!("fetching {} models\u{2026}", names.join(", ")))
    }

    /// Takes the open cascade slot captured by the most recent synchronous
    /// `query()` call. Used by compatibility tests for the public query API.
    #[cfg(test)]
    pub(crate) fn take_worker_open_slot(&mut self) -> Option<OpenWorkerSlot> {
        self.last_worker_open_slot.take()
    }

    pub fn set_issue_snapshot(&mut self, issues: Vec<IssueMentionDescriptor>) {
        // Ordering invariant: callers must pass issues newest-first.
        // Empty-query `@` preserves that order within the ISSUE_CAP slice.
        self.update_session_source("issue", |token| {
            Box::new(IssueMentionSource::with_token(issues, token))
        });
    }

    pub fn set_worker_snapshot_in_place(&mut self, workers: Vec<WorkerMentionDescriptor>) {
        self.update_session_source("worker", |token| {
            Box::new(WorkerMentionSource::with_token(workers, token))
        });
    }

    pub fn set_datasource_snapshot(&mut self, entries: Vec<DatasourceEntry>) {
        self.update_session_source("datasource", |token| {
            Box::new(DatasourceMentionSource::with_token(entries, token))
        });
    }

    /// Swap (or append) one session source. The replacement source is
    /// stamped with `previous token + 1`, so the engine cache key changes
    /// on every swap (data-driven invalidation); both the engine-side
    /// snapshots and the facade materialization are dropped so the next
    /// query rebuilds from the fresh data.
    fn update_session_source(
        &mut self,
        name: &'static str,
        make_source: impl FnOnce(u64) -> Box<dyn SessionMentionSource>,
    ) {
        let next_token = self
            .sources
            .iter()
            .find(|slot| slot.name == name)
            .and_then(|slot| slot.session_source_token())
            .map_or(0, |token| token.wrapping_add(1));
        let slot = MentionSourceSlot::session(make_source(next_token));
        self.engine.register_source(slot.engine_adapter(), false);
        if let Some(existing) = self.sources.iter_mut().find(|slot| slot.name == name) {
            *existing = slot;
        } else {
            self.sources.push(slot);
        }
        self.clear_cache_for(name);
    }

    pub fn query(
        &mut self,
        scope: CompletionScope<'_>,
        cwd: &Path,
        query: &str,
        limit: usize,
    ) -> Vec<MentionEntry> {
        let work = self.prepare_query_work(scope, cwd, query, limit);
        let result = self.run_query_work(work);
        #[cfg(test)]
        {
            self.last_worker_open_slot = result.open_slot.clone();
        }
        result.entries
    }

    pub(crate) fn prepare_query_work(
        &mut self,
        scope: CompletionScope<'_>,
        cwd: &Path,
        query: &str,
        limit: usize,
    ) -> MentionQueryWork {
        match self.prepare_query_work_inner(scope, cwd, query, limit, false) {
            MentionQueryPreparation::Ready(work) => work,
            MentionQueryPreparation::ReadyAndBuild { .. } => {
                unreachable!("synchronous query preparation cannot defer cache builds")
            }
        }
    }

    pub(crate) fn prepare_query_work_nonblocking(
        &mut self,
        scope: CompletionScope<'_>,
        cwd: &Path,
        query: &str,
        limit: usize,
    ) -> MentionQueryPreparation {
        self.prepare_query_work_inner(scope, cwd, query, limit, true)
    }

    fn prepare_query_work_inner(
        &mut self,
        scope: CompletionScope<'_>,
        cwd: &Path,
        query: &str,
        limit: usize,
        defer_code_graph_build: bool,
    ) -> MentionQueryPreparation {
        #[cfg(test)]
        {
            self.last_worker_open_slot = None;
        }
        #[cfg(any(test, debug_assertions))]
        {
            self.query_call_count += 1;
        }
        let _scope_key = CompletionScopeKey::from(scope);
        let _span =
            tracing::debug_span!("mention_registry_query", query_len = query.len()).entered();
        self.refresh_code_graph_registration(cwd);
        // Staleness is engine-owned: a source is stale when it has no fresh
        // snapshot under the current root/profile/token identity. (The old
        // registry's code-graph token-unchanged TTL bypass is subsumed:
        // TTL-driven rebuilds are cheap because the source's content-hash
        // cache short-circuits artifact reloads, and results are identical.)
        let options = engine_query_options();
        self.snapshots.begin_query();
        let mut deferred_sources = Vec::new();
        for source in &self.sources {
            if let Some(snapshot) = self.engine.cached_snapshot(cwd, source.name, &options) {
                self.snapshots.select(source.name, &snapshot);
                continue;
            }
            if defer_code_graph_build && source.name == "code_graph" {
                // Only this identity's previous generation may serve while
                // its replacement is being built in the background.
                if let Some(snapshot) = self.engine.retained_snapshot(cwd, source.name, &options) {
                    self.snapshots.select(source.name, &snapshot);
                }
                deferred_sources.push(source.clone());
                continue;
            }
            match self.engine.resolve_snapshot(cwd, source.name, &options) {
                SnapshotResolution::Built(snapshot) => {
                    if let Some(data) = source.adopt_built(&snapshot) {
                        self.snapshots.insert(source.name, &snapshot, data);
                    }
                    self.snapshots.select(source.name, &snapshot);
                }
                SnapshotResolution::Fresh(snapshot) | SnapshotResolution::Retained(snapshot) => {
                    self.snapshots.select(source.name, &snapshot);
                }
                SnapshotResolution::Failed(_) => {}
            }
        }
        let composed = if query.is_empty() {
            None
        } else {
            let worker_entries: Vec<&MentionEntry> = self
                .snapshots
                .get("worker")
                .map(|cached| cached.entries.iter().collect())
                .unwrap_or_default();
            Self::compose_worker_mention(
                cwd,
                &worker_entries,
                query,
                &self.snapshots,
                self.agent_model_catalog_path.as_ref(),
                &mut self.pending_agent_model_catalog_probe_requests,
                &mut self.matcher,
            )
        };
        let sources = self
            .sources
            .iter()
            .filter_map(|source| self.snapshots.get(source.name))
            .map(|cached| Arc::clone(&cached.entries))
            .collect();
        let code_indexes = self
            .sources
            .iter()
            .filter_map(|source| self.snapshots.get(source.name))
            .filter_map(|cached| cached.code_index.clone())
            .collect();
        let (composed_entry, open_slot) = match composed {
            Some(composed) => (Some(composed.entry), composed.open_slot),
            None => (None, None),
        };

        let work = MentionQueryWork {
            sources,
            code_indexes,
            query: query.to_string(),
            limit,
            composed_entry,
            open_slot,
        };
        if deferred_sources.is_empty() {
            MentionQueryPreparation::Ready(work)
        } else {
            MentionQueryPreparation::ReadyAndBuild {
                work,
                build: MentionCacheBuildWork {
                    sources: deferred_sources,
                    cwd: cwd.to_path_buf(),
                },
            }
        }
    }

    pub(crate) fn apply_cache_build(&mut self, result: MentionCacheBuildResult) {
        let options = engine_query_options();
        let MentionCacheBuildResult { cwd, updates } = result;
        for update in updates {
            let is_current = self
                .sources
                .iter()
                .any(|source| source.same_identity(&update.source));
            if !is_current {
                continue;
            }
            // Publish the deferred build engine-side (stamping the next
            // generation under the caller's root identity), then retain its
            // materialization. Selection happens on the next query: a late
            // background result must not change the currently visible root.
            let snapshot = SourceSnapshot::new(neutral_entries_of(&update.data.entries), 0);
            if let Some(snapshot) =
                self.engine
                    .publish_snapshot(&cwd, update.source.name, &options, snapshot)
            {
                self.snapshots
                    .insert(update.source.name, &snapshot, update.data);
            } else {
                tracing::warn!(
                    source = update.source.name,
                    "deferred mention build published for an unregistered source"
                );
            }
        }
    }

    pub(crate) fn run_query_work(&mut self, work: MentionQueryWork) -> MentionQueryResult {
        let result = score_mention_query(work, &mut self.matcher);
        self.install_query_code_payloads(result.code_payloads.clone());
        result
    }

    #[cfg(test)]
    pub(crate) fn from_sources_for_test(sources: Vec<Box<dyn MentionSource>>) -> Self {
        Self::with_source_slots(sources.into_iter().map(MentionSourceSlot::new).collect())
    }

    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn query_call_count_for_test(&self) -> usize {
        self.query_call_count
    }

    #[cfg(test)]
    pub(crate) fn set_agent_model_catalog_path_for_test(&mut self, path: PathBuf) {
        self.agent_model_catalog_path = Some(path);
    }

    /// Engine cache token currently exposed by a session source (`None`
    /// when the named source is absent or not session-backed). Test
    /// observation for the snapshot-replacement token pins.
    #[cfg(test)]
    pub(crate) fn session_source_token(&self, name: &str) -> Option<u64> {
        self.sources
            .iter()
            .find(|slot| slot.name == name)
            .and_then(|slot| slot.session_source_token())
    }

    fn compose_worker_mention(
        cwd: &Path,
        entries: &[&MentionEntry],
        query: &str,
        snapshots: &SnapshotMaterializations,
        catalog_path: Option<&PathBuf>,
        pending_probe_requests: &mut HashSet<String>,
        matcher: &mut Matcher,
    ) -> Option<ComposedWorkerMention> {
        let tokens = query_tokens(query);
        let first = tokens.first()?;
        let worker_entries: Vec<&MentionEntry> = entries
            .iter()
            .copied()
            .filter(|entry| entry.kind == MentionKind::Worker)
            .collect();
        let worker_entry = Self::high_confidence_entry_match(first.text, &worker_entries, matcher)?;
        let worker_name = worker_entry
            .uri
            .strip_prefix("worker://")
            .unwrap_or(worker_entry.display.trim_start_matches("worker:"))
            .split('?')
            .next()
            .unwrap_or_default()
            .to_string();
        if worker_name.is_empty() {
            return None;
        }
        let worker_kind = Self::worker_kind(snapshots, &worker_name);
        let catalog_entry = Self::worker_catalog_entry(catalog_path, &worker_name);
        if Self::catalog_probe_needed(
            catalog_entry.as_ref(),
            worker_entry.worker_cli_identity.as_deref(),
        ) {
            pending_probe_requests.insert(worker_name.clone());
        }
        let model_candidates_empty = catalog_entry
            .as_ref()
            .map(|entry| entry.models.is_empty())
            .unwrap_or(true);
        let effort_candidates_empty = catalog_entry
            .as_ref()
            .map(|entry| entry.efforts.is_empty())
            .unwrap_or(true);

        let mut token_index = 1usize;
        let mut slot = WorkerMentionSlot::Worker;
        let mut agent = None;
        let mut model = None;
        let mut effort = None;
        let mut consumed_end = first.end;
        let mut unconsumed_suffix = None;
        let mut open_slot = None;

        loop {
            match slot {
                WorkerMentionSlot::Worker => slot = WorkerMentionSlot::Agent,
                WorkerMentionSlot::Agent => {
                    let candidates = agent_slot_candidates(cwd, worker_kind);
                    if candidates.is_empty() {
                        slot = WorkerMentionSlot::Model;
                        continue;
                    }
                    let is_final = model_candidates_empty && effort_candidates_empty;
                    let Some(token) = tokens.get(token_index) else {
                        open_slot = Some(OpenWorkerSlot {
                            kind: WorkerSlotKind::Agent,
                            splice_at: consumed_end,
                            filter_text: String::new(),
                            candidates,
                            is_final,
                        });
                        break;
                    };
                    if token.text.is_empty() {
                        consumed_end = token.end;
                        token_index += 1;
                        slot = WorkerMentionSlot::Model;
                        continue;
                    }
                    match Self::high_confidence_slot_match(token.text, &candidates, matcher) {
                        Some(candidate) => {
                            agent = Some(candidate.value);
                            consumed_end = token.end;
                            token_index += 1;
                            slot = WorkerMentionSlot::Model;
                        }
                        None => {
                            open_slot = Some(OpenWorkerSlot {
                                kind: WorkerSlotKind::Agent,
                                splice_at: consumed_end,
                                filter_text: token.text.to_string(),
                                candidates,
                                is_final,
                            });
                            unconsumed_suffix = Some(query[consumed_end..].to_string());
                            break;
                        }
                    }
                }
                WorkerMentionSlot::Model => {
                    let candidates = catalog_entry
                        .as_ref()
                        .map(|entry| config_choice_candidates(&entry.models))
                        .unwrap_or_default();
                    if candidates.is_empty() {
                        slot = WorkerMentionSlot::Effort;
                        continue;
                    }
                    let is_final = effort_candidates_empty;
                    let Some(token) = tokens.get(token_index) else {
                        open_slot = Some(OpenWorkerSlot {
                            kind: WorkerSlotKind::Model,
                            splice_at: consumed_end,
                            filter_text: String::new(),
                            candidates,
                            is_final,
                        });
                        break;
                    };
                    if token.text.is_empty() {
                        consumed_end = token.end;
                        token_index += 1;
                        slot = WorkerMentionSlot::Effort;
                        continue;
                    }
                    match Self::high_confidence_slot_match(token.text, &candidates, matcher) {
                        Some(candidate) => {
                            model = Some(candidate.value);
                            consumed_end = token.end;
                            token_index += 1;
                            slot = WorkerMentionSlot::Effort;
                        }
                        None => {
                            open_slot = Some(OpenWorkerSlot {
                                kind: WorkerSlotKind::Model,
                                splice_at: consumed_end,
                                filter_text: token.text.to_string(),
                                candidates,
                                is_final,
                            });
                            unconsumed_suffix = Some(query[consumed_end..].to_string());
                            break;
                        }
                    }
                }
                WorkerMentionSlot::Effort => {
                    let candidates = catalog_entry
                        .as_ref()
                        .map(|entry| config_choice_candidates(&entry.efforts))
                        .unwrap_or_default();
                    if candidates.is_empty() {
                        slot = WorkerMentionSlot::Done;
                        continue;
                    }
                    let Some(token) = tokens.get(token_index) else {
                        open_slot = Some(OpenWorkerSlot {
                            kind: WorkerSlotKind::Effort,
                            splice_at: consumed_end,
                            filter_text: String::new(),
                            candidates,
                            is_final: true,
                        });
                        break;
                    };
                    if token.text.is_empty() {
                        consumed_end = token.end;
                        token_index += 1;
                        slot = WorkerMentionSlot::Done;
                        continue;
                    }
                    match Self::high_confidence_slot_match(token.text, &candidates, matcher) {
                        Some(candidate) => {
                            effort = Some(candidate.value);
                            consumed_end = token.end;
                            token_index += 1;
                            slot = WorkerMentionSlot::Done;
                        }
                        None => {
                            open_slot = Some(OpenWorkerSlot {
                                kind: WorkerSlotKind::Effort,
                                splice_at: consumed_end,
                                filter_text: token.text.to_string(),
                                candidates,
                                is_final: true,
                            });
                            unconsumed_suffix = Some(query[consumed_end..].to_string());
                            break;
                        }
                    }
                }
                WorkerMentionSlot::Done => {
                    if let Some(token) = tokens.get(token_index) {
                        unconsumed_suffix = Some(
                            query[consumed_end..token.start].to_string() + &query[token.start..],
                        );
                    }
                    break;
                }
            }
        }

        let uri = composed_worker_uri(
            &worker_name,
            agent.as_deref(),
            model.as_deref(),
            effort.as_deref(),
        );
        let atom_text = composed_worker_atom(
            &worker_name,
            agent.as_deref(),
            model.as_deref(),
            effort.as_deref(),
        );
        let mut composed = worker_entry.clone();
        composed.display = atom_text.trim_start_matches('@').to_string();
        composed.uri = uri;
        composed.atom_text = Some(atom_text);
        composed.agent = agent;
        composed.model = model;
        composed.effort = effort;
        composed.unconsumed_suffix = unconsumed_suffix.filter(|suffix| !suffix.is_empty());
        let open_slot = open_slot.map(|mut slot| {
            slot.candidates = filter_slot_candidates(slot.candidates, &slot.filter_text, matcher);
            slot
        });
        Some(ComposedWorkerMention {
            entry: composed,
            open_slot,
        })
    }

    fn worker_kind(snapshots: &SnapshotMaterializations, worker_name: &str) -> spur_acp::AgentKind {
        snapshots
            .get("worker")
            .and_then(|cached| {
                cached.entries.iter().find_map(|entry| {
                    let name = entry
                        .uri
                        .strip_prefix("worker://")?
                        .split('?')
                        .next()
                        .unwrap_or_default();
                    (name == worker_name).then_some(entry.worker_kind).flatten()
                })
            })
            .unwrap_or_else(|| spur_acp::AgentKind::from_name(worker_name))
    }

    fn worker_catalog_entry(
        catalog_path: Option<&PathBuf>,
        worker_name: &str,
    ) -> Option<agent_model_catalog::WorkerCatalogEntry> {
        let path = catalog_path
            .cloned()
            .or_else(agent_model_catalog::cache_path)?;
        agent_model_catalog::read(&path)?
            .entries
            .get(worker_name)
            .cloned()
    }

    fn catalog_probe_needed(
        entry: Option<&agent_model_catalog::WorkerCatalogEntry>,
        cli_identity: Option<&str>,
    ) -> bool {
        match entry {
            None => true,
            Some(entry) => cli_identity
                .map(|identity| entry.is_stale(chrono::Utc::now(), identity))
                .unwrap_or(false),
        }
    }

    fn high_confidence_entry_match<'a>(
        token: &str,
        entries: &[&'a MentionEntry],
        matcher: &mut Matcher,
    ) -> Option<&'a MentionEntry> {
        let candidates: Vec<SlotCandidate> = entries
            .iter()
            .map(|entry| SlotCandidate {
                value: entry
                    .uri
                    .strip_prefix("worker://")
                    .unwrap_or(entry.display.as_str())
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                label: entry.display.clone(),
                description: entry.secondary.clone(),
            })
            .collect();
        let matched = Self::high_confidence_slot_match(token, &candidates, matcher)?;
        entries.iter().copied().find(|entry| {
            entry.uri == format!("worker://{}", matched.value)
                || entry.display == format!("worker:{}", matched.value)
        })
    }

    fn high_confidence_slot_match(
        token: &str,
        candidates: &[SlotCandidate],
        matcher: &mut Matcher,
    ) -> Option<SlotCandidate> {
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        // Exact text dominates: typing a candidate's full name resolves the
        // slot even when that name prefixes a sibling candidate.
        let mut exact = candidates.iter().filter(|candidate| {
            candidate.value.eq_ignore_ascii_case(token)
                || candidate.label.eq_ignore_ascii_case(token)
        });
        if let (Some(only), None) = (exact.next(), exact.next()) {
            return Some(only.clone());
        }
        // Otherwise the match must be UNIQUE among high-confidence
        // candidates. Auto-advancing on an ambiguous prefix would silently
        // pick one of several matching candidates the user never saw;
        // keeping the slot open turns the token into a visible filter.
        let pattern = Pattern::parse(token, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut qualifying = candidates.iter().filter_map(|candidate| {
            let score = slot_candidate_score(candidate, token, &pattern, matcher, &mut buf)?;
            high_confidence_score(candidate, token, score).then_some(candidate)
        });
        match (qualifying.next(), qualifying.next()) {
            (Some(only), None) => Some(only.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerMentionSlot {
    Worker,
    Agent,
    Model,
    Effort,
    Done,
}

/// Which cascade slot an [`OpenWorkerSlot`] describes. Lets a picker UI
/// know which field on the composed entry a selected candidate fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerSlotKind {
    Agent,
    Model,
    Effort,
}

/// A cascade slot the user hasn't resolved past yet, with real candidates
/// available -- surfaced so a picker UI can show them as selectable rows
/// instead of forcing blind typing. `splice_at` is a byte offset into the
/// same `query` string passed to `MentionRegistry::query`, marking where a
/// picked candidate's value (plus a trailing `,` unless `is_final`) should
/// be spliced in.
#[derive(Debug, Clone)]
pub(crate) struct OpenWorkerSlot {
    pub(crate) kind: WorkerSlotKind,
    pub(crate) splice_at: usize,
    pub(crate) filter_text: String,
    pub(crate) candidates: Vec<SlotCandidate>,
    /// `true` when no later slot has real candidates, so picking one of
    /// `candidates` finishes the mention (closes the picker) rather than
    /// advancing to the next slot's list.
    pub(crate) is_final: bool,
}

/// Result of [`MentionRegistry::compose_worker_mention`]: the composed
/// single-row entry (used for continued blind typing and as the source of
/// already-resolved `agent`/`model`/`effort` values), plus an optional
/// open slot for a picker UI to render as a candidate list.
struct ComposedWorkerMention {
    entry: MentionEntry,
    open_slot: Option<OpenWorkerSlot>,
}

#[derive(Debug, Clone)]
struct QueryToken<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SlotCandidate {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) description: Option<String>,
}

/// Splits on `,` rather than whitespace: the InputBar's completion-trigger
/// detector closes the mention popup on the first whitespace character
/// typed (see `completion_trigger.rs`), so a worker/agent/model/effort
/// cascade typed live in the TUI can never contain a space. `,` was chosen
/// over `/` because real catalog model values use `/` for provider
/// qualification (e.g. opencode's `provider/model` ids).
///
/// Unlike whitespace splitting, consecutive commas are *not* collapsed:
/// an empty segment between two commas (`codex,,gpt-5`) is preserved as
/// an empty token, which `compose_worker_mention` reads as "explicitly
/// skip this slot" (needed when a worker has real agent-profile
/// candidates but the user wants to jump straight to model/effort). A
/// single trailing comma with nothing typed after it yet does *not*
/// produce a phantom empty token, so mid-typing state still reads as
/// "waiting", not "skip".
fn query_tokens(query: &str) -> Vec<QueryToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = 0;
    for (idx, ch) in query.char_indices() {
        if ch == ',' {
            tokens.push(QueryToken {
                text: &query[start..idx],
                start,
                end: idx,
            });
            start = idx + ch.len_utf8();
        }
    }
    if start < query.len() {
        tokens.push(QueryToken {
            text: &query[start..],
            start,
            end: query.len(),
        });
    }
    tokens
}

fn agent_slot_candidates(cwd: &Path, kind: spur_acp::AgentKind) -> Vec<SlotCandidate> {
    // Committed `.spur/agents/*.md` ∪ accepted explore-pool agents. Load is
    // the single resolution path so picker candidates stay consistent with
    // delegation (`AgentProfile::load` pool fallback).
    spur_core::agent_profiles::AgentProfile::candidate_names(cwd)
        .into_iter()
        .filter_map(
            |name| match spur_core::agent_profiles::AgentProfile::load(cwd, &name) {
                Ok(Some(profile))
                    if spur_core::agent_profiles::render::render_for_kind(&profile, kind)
                        .is_some() =>
                {
                    Some(SlotCandidate {
                        value: profile.name,
                        label: name,
                        description: Some(profile.description),
                    })
                }
                Ok(_) => None,
                Err(error) => {
                    tracing::debug!(
                        profile = %name,
                        error = %error,
                        "Skipping invalid agent profile for worker mention picker"
                    );
                    None
                }
            },
        )
        .collect()
}

fn config_choice_candidates(choices: &[ConfigOptionChoice]) -> Vec<SlotCandidate> {
    choices
        .iter()
        .map(|choice| SlotCandidate {
            value: choice.value.clone(),
            label: choice.name.clone(),
            description: choice.description.clone(),
        })
        .collect()
}

fn slot_candidate_score(
    candidate: &SlotCandidate,
    token: &str,
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
) -> Option<u32> {
    let mut best = pattern_score(pattern, matcher, buf, &candidate.value);
    if candidate.label != candidate.value {
        best = best.max(pattern_score(pattern, matcher, buf, &candidate.label));
    }
    if let Some(description) = &candidate.description {
        best = best.max(pattern_score(pattern, matcher, buf, description));
    }
    if candidate_text_matches(&candidate.value, token)
        || candidate_text_matches(&candidate.label, token)
    {
        best = Some(best.unwrap_or(0).saturating_add(10_000));
    }
    best
}

/// Fuzzy-filters and ranks slot candidates for picker-list display, using
/// the same underlying scoring as `high_confidence_slot_match` but without
/// its stricter auto-accept-while-typing threshold -- a lower-scoring
/// match is still worth *showing* even if it isn't confident enough to
/// silently commit while the user is still typing.
pub(crate) fn filter_slot_candidates(
    candidates: Vec<SlotCandidate>,
    filter_text: &str,
    matcher: &mut Matcher,
) -> Vec<SlotCandidate> {
    let token = filter_text.trim();
    if token.is_empty() {
        return candidates;
    }
    let pattern = Pattern::parse(token, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, SlotCandidate)> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let score = slot_candidate_score(&candidate, token, &pattern, matcher, &mut buf)?;
            Some((score, candidate))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.value.cmp(&b.1.value)));
    scored.into_iter().map(|(_, c)| c).collect()
}

fn high_confidence_score(candidate: &SlotCandidate, token: &str, score: u32) -> bool {
    score >= 10_000
        || score >= 60
        || candidate_text_matches(&candidate.value, token)
        || candidate_text_matches(&candidate.label, token)
}

fn candidate_text_matches(haystack: &str, token: &str) -> bool {
    let haystack = haystack.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    !token.is_empty()
        && (haystack == token
            || haystack.starts_with(&token)
            || (token.len() >= 3 && haystack.contains(&token)))
}

fn composed_worker_uri(
    worker_name: &str,
    agent: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    let mut uri = format!("worker://{worker_name}");
    let mut sep = '?';
    for (key, value) in [("agent", agent), ("model", model), ("effort", effort)] {
        if let Some(value) = value {
            uri.push(sep);
            sep = '&';
            uri.push_str(key);
            uri.push('=');
            uri.push_str(value);
        }
    }
    uri
}

fn composed_worker_atom(
    worker_name: &str,
    agent: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    let mut atom = format!("@worker:{worker_name}");
    for (key, value) in [("agent", agent), ("model", model), ("effort", effort)] {
        if let Some(value) = value {
            atom.push(' ');
            atom.push_str(key);
            atom.push('=');
            atom.push_str(value);
        }
    }
    atom
}

/// Builds the final composed `MentionEntry` for picking `picked_value` in
/// `slot_kind`, given the entry already resolved so far (whatever
/// `agent`/`model`/`effort` earlier slots already filled). Used when a
/// picker UI accepts a row from an `is_final` [`OpenWorkerSlot`]'s
/// candidate list -- mirrors what blind-typing-through would have
/// produced for the same combination.
pub(crate) fn finalize_worker_slot_pick(
    entry: &MentionEntry,
    slot_kind: WorkerSlotKind,
    picked_value: &str,
) -> MentionEntry {
    let worker_name = entry
        .uri
        .strip_prefix("worker://")
        .unwrap_or(entry.display.trim_start_matches("worker:"))
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    let mut agent = entry.agent.clone();
    let mut model = entry.model.clone();
    let mut effort = entry.effort.clone();
    match slot_kind {
        WorkerSlotKind::Agent => agent = Some(picked_value.to_string()),
        WorkerSlotKind::Model => model = Some(picked_value.to_string()),
        WorkerSlotKind::Effort => effort = Some(picked_value.to_string()),
    }
    let uri = composed_worker_uri(
        &worker_name,
        agent.as_deref(),
        model.as_deref(),
        effort.as_deref(),
    );
    let atom_text = composed_worker_atom(
        &worker_name,
        agent.as_deref(),
        model.as_deref(),
        effort.as_deref(),
    );
    let mut finalized = entry.clone();
    finalized.display = atom_text.trim_start_matches('@').to_string();
    finalized.uri = uri;
    finalized.atom_text = Some(atom_text);
    finalized.agent = agent;
    finalized.model = model;
    finalized.effort = effort;
    finalized
}

fn is_graph_symbol_uri(uri: &str) -> bool {
    uri.starts_with("graph://symbol/")
}

fn dedup_file_entries_with_code_files(entries: &mut Vec<&MentionEntry>) {
    let code_file_paths: HashSet<String> = entries
        .iter()
        .filter(|entry| entry.kind == MentionKind::CodeFile)
        .map(|entry| mention_path_key(&entry.display))
        .collect();
    if code_file_paths.is_empty() {
        return;
    }

    entries.retain(|entry| {
        entry.kind != MentionKind::File
            || !code_file_paths.contains(&mention_path_key(&entry.display))
    });
}

fn mention_path_key(path: &str) -> String {
    let mut relative = path;
    while let Some(stripped) = relative.strip_prefix("./") {
        relative = stripped;
    }
    relative.replace('\\', "/")
}

/// Build one source's facade-side snapshot data by running the TUI source
/// under `cwd` and collecting its rows plus adapter-owned extras (code
/// payloads/candidates, datasource prompt hints). Shared by the
/// engine-side adapter builds (inline) and the deferred worker-thread
/// builds; freshness and cache identity stay engine-owned.
fn build_source_snapshot_data(
    source: &Arc<Mutex<Box<dyn MentionSource>>>,
    cwd: &Path,
) -> Result<SourceSnapshotData, String> {
    let mut builder = match source.lock() {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!(error = %error, "mention source snapshot lock poisoned");
            return Err(format!("source lock poisoned: {error}"));
        }
    };
    let entries = match builder.build(cwd) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(error = %error, "mention source snapshot build failed");
            return Err(error.to_string());
        }
    };
    let code_payloads = builder
        .code_payloads()
        .iter()
        .map(|(uri, payload)| (uri.clone(), Arc::clone(payload)))
        .collect();
    let code_candidates = builder.code_candidates();
    let code_index = (!code_candidates.is_empty()).then(|| CodeQueryIndex {
        candidates: code_candidates,
        payload_snapshot: builder.code_payload_snapshot(),
        source: Arc::clone(source),
    });
    let datasource_hints = builder
        .datasource_hints()
        .iter()
        .map(|(uri, hint)| (uri.clone(), Arc::clone(hint)))
        .collect();
    Ok(SourceSnapshotData {
        entries: Arc::new(entries),
        code_payloads,
        code_index,
        datasource_hints,
    })
}

pub(crate) fn build_mention_caches(work: MentionCacheBuildWork) -> MentionCacheBuildResult {
    let updates = work
        .sources
        .into_iter()
        .filter_map(|slot| {
            // Deferred builds are a code-graph concern (the only source
            // whose traversal is too heavy for the UI thread); session
            // sources never defer and are skipped defensively.
            let SourceSlotKind::Tui { source, .. } = &slot.kind else {
                return None;
            };
            let data = build_source_snapshot_data(source, &work.cwd).ok()?;
            Some(MentionCacheBuildUpdate { source: slot, data })
        })
        .collect();

    MentionCacheBuildResult {
        cwd: work.cwd,
        updates,
    }
}

pub(crate) fn score_mention_query(
    work: MentionQueryWork,
    matcher: &mut Matcher,
) -> MentionQueryResult {
    let MentionQueryWork {
        sources,
        code_indexes,
        query,
        limit,
        mut composed_entry,
        open_slot,
    } = work;
    let total_len: usize = sources.iter().map(|entries| entries.len()).sum();
    let mut all_entries: Vec<&MentionEntry> = Vec::with_capacity(total_len);
    for entries in &sources {
        all_entries.extend(entries.iter());
    }
    dedup_file_entries_with_code_files(&mut all_entries);
    let entries = all_entries.as_slice();

    if query.is_empty() {
        let mut workers: Vec<&MentionEntry> = entries
            .iter()
            .filter(|entry| entry.kind == MentionKind::Worker)
            .copied()
            .collect();
        workers.sort_by(|a, b| {
            a.display
                .len()
                .cmp(&b.display.len())
                .then(a.display.cmp(&b.display))
        });
        workers.truncate(WORKER_PIN_CAP);
        let workers: Vec<MentionEntry> = workers.into_iter().cloned().collect();

        let mut files: Vec<&MentionEntry> = entries
            .iter()
            .filter(|entry| matches!(entry.kind, MentionKind::File | MentionKind::Directory))
            .copied()
            .collect();
        files.sort_by(|a, b| {
            path_depth(&a.display)
                .cmp(&path_depth(&b.display))
                .then(a.display.len().cmp(&b.display.len()))
                .then(a.display.cmp(&b.display))
                .then(a.uri.cmp(&b.uri))
        });
        files.truncate(FILE_CAP);
        let files: Vec<MentionEntry> = files.into_iter().cloned().collect();

        let mut issues: Vec<(usize, &MentionEntry)> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind == MentionKind::Issue)
            .map(|(index, entry)| (index, *entry))
            .collect();
        issues.sort_by(|(idx_a, a), (idx_b, b)| {
            idx_a
                .cmp(idx_b)
                .then(issue_id(a).cmp(issue_id(b)))
                .then(a.uri.cmp(&b.uri))
        });
        issues.truncate(ISSUE_CAP);
        let issues: Vec<MentionEntry> =
            issues.into_iter().map(|(_, entry)| entry.clone()).collect();

        let mut datasources: Vec<&MentionEntry> = entries
            .iter()
            .filter(|entry| entry.kind == MentionKind::Datasource)
            .copied()
            .collect();
        datasources.sort_by(|a, b| {
            a.display
                .cmp(&b.display)
                .then(a.secondary.cmp(&b.secondary))
                .then(a.uri.cmp(&b.uri))
        });
        datasources.truncate(DATASOURCE_CAP);
        let datasources: Vec<MentionEntry> = datasources.into_iter().cloned().collect();

        let mut code_graph: Vec<MentionEntry> = entries
            .iter()
            .filter(|entry| matches!(entry.kind, MentionKind::CodeFile | MentionKind::CodeSymbol))
            .map(|entry| (*entry).clone())
            .collect();
        code_graph.sort_by(|a, b| {
            empty_code_kind_rank(&a.kind)
                .cmp(&empty_code_kind_rank(&b.kind))
                .then(path_depth(&a.display).cmp(&path_depth(&b.display)))
                .then(a.display.len().cmp(&b.display.len()))
                .then(a.display.cmp(&b.display))
                .then(a.uri.cmp(&b.uri))
        });
        code_graph.truncate(CODE_CAP);
        let mut selected_code = Vec::new();
        if code_graph.len() < CODE_CAP {
            let remaining = CODE_CAP - code_graph.len();
            let candidates = if let [index] = code_indexes.as_slice() {
                index
                    .candidates
                    .iter()
                    .take(remaining)
                    .map(|candidate| (0, candidate))
                    .collect::<Vec<_>>()
            } else {
                let mut candidates = code_indexes
                    .iter()
                    .enumerate()
                    .flat_map(|(source_index, index)| {
                        index
                            .candidates
                            .iter()
                            .map(move |candidate| (source_index, candidate))
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by(|(_, left), (_, right)| {
                    left.entity_name
                        .len()
                        .cmp(&right.entity_name.len())
                        .then(left.entity_name.cmp(&right.entity_name))
                        .then(left.stable_symbol_id.cmp(&right.stable_symbol_id))
                });
                candidates.truncate(remaining);
                candidates
            };
            for (source_index, candidate) in candidates {
                let entry = entry_for_candidate(candidate);
                selected_code.push((entry.uri.clone(), source_index));
                code_graph.push(entry);
            }
        }

        let mut rows = Vec::new();
        append_section_rows(&mut rows, "Workers", &workers);
        append_section_rows(&mut rows, "Files", &files);
        append_section_rows(&mut rows, "Issues", &issues);
        append_section_rows(&mut rows, "Data", &datasources);
        append_section_rows(&mut rows, "Code", &code_graph);
        rows.truncate(limit);
        let code_payloads = hydrate_code_rows(&mut rows, &selected_code, &code_indexes);
        return MentionQueryResult {
            entries: rows,
            code_payloads,
            open_slot,
        };
    }

    // The composed worker row supersedes its base worker entry, but it must
    // not suppress issue/file/code rows matching the same text.
    let composed_base_uri = composed_entry.as_ref().map(|entry| {
        entry
            .uri
            .split('?')
            .next()
            .unwrap_or(&entry.uri)
            .to_string()
    });
    let pattern = Pattern::parse(&query, CaseMatching::Smart, Normalization::Smart);
    let code_pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<RankedMentionRef<'_>> = entries
        .iter()
        .filter_map(|entry| {
            buf.clear();
            let rank = if matches!(entry.kind, MentionKind::CodeFile | MentionKind::CodeSymbol) {
                code_match_rank(entry, &query, &code_pattern, matcher, &mut buf)?
            } else {
                let haystack = entry.search_text.as_deref().unwrap_or(&entry.display);
                pattern.score(nucleo_matcher::Utf32Str::new(haystack, &mut buf), matcher)?
            };
            Some(RankedMentionRef {
                rank,
                tier: tier_of_kind(&entry.kind),
                value: RankedMentionValue::Entry(entry),
            })
        })
        .collect();
    for (source_index, index) in code_indexes.iter().enumerate() {
        scored.extend(index.candidates.iter().filter_map(|candidate| {
            let rank =
                code_candidate_match_rank(candidate, &query, &code_pattern, matcher, &mut buf)?;
            Some(RankedMentionRef {
                rank,
                tier: tier_of_kind(&MentionKind::CodeSymbol),
                value: RankedMentionValue::CodeSymbol {
                    source_index,
                    candidate,
                },
            })
        }));
    }
    let max_rank = scored.iter().map(|mention| mention.rank).max().unwrap_or(0);
    scored.sort_by(|a, b| typed_query_cmp(a, b, max_rank));
    let is_base_worker = |entry: &MentionEntry| {
        composed_base_uri
            .as_deref()
            .is_some_and(|base| entry.kind == MentionKind::Worker && entry.uri == base)
    };
    let mut rows = Vec::new();
    let mut selected_code = Vec::new();
    let mut composed_emitted = false;
    for ranked in scored.into_iter().take(limit) {
        match ranked.value {
            RankedMentionValue::Entry(entry) => {
                if !composed_emitted && is_base_worker(entry) {
                    if let Some(entry) = composed_entry.clone() {
                        rows.push(entry);
                        composed_emitted = true;
                        continue;
                    }
                }
                rows.push(entry.clone());
            }
            RankedMentionValue::CodeSymbol {
                source_index,
                candidate,
            } => {
                let entry = entry_for_candidate(candidate);
                selected_code.push((entry.uri.clone(), source_index));
                rows.push(entry);
            }
        }
    }
    if let Some(entry) = composed_entry.take() {
        let composed_uri = entry.uri.clone();
        if !composed_emitted {
            rows.insert(0, entry);
            rows.truncate(limit.max(1));
        }
        if open_slot.is_some() {
            if let Some(pos) = rows.iter().position(|row| row.uri == composed_uri) {
                if pos > 0 {
                    let row = rows.remove(pos);
                    rows.insert(0, row);
                }
            }
        }
    }

    let code_payloads = hydrate_code_rows(&mut rows, &selected_code, &code_indexes);
    MentionQueryResult {
        entries: rows,
        code_payloads,
        open_slot,
    }
}

#[derive(Debug, Clone)]
struct RankedMentionRef<'a> {
    rank: u32,
    /// Cross-kind tier, precomputed through [`tier_of_kind`] (the neutral
    /// engine's tier table) so the O(n log n) comparator never allocates.
    tier: u8,
    value: RankedMentionValue<'a>,
}

#[derive(Debug, Clone)]
enum RankedMentionValue<'a> {
    Entry(&'a MentionEntry),
    CodeSymbol {
        source_index: usize,
        candidate: &'a CodeMentionCandidate,
    },
}

impl RankedMentionRef<'_> {
    fn stable_cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.value, &other.value) {
            (RankedMentionValue::Entry(left), RankedMentionValue::Entry(right)) => {
                stable_tie_key(left).cmp(stable_tie_key(right))
            }
            (
                RankedMentionValue::CodeSymbol {
                    candidate: left, ..
                },
                RankedMentionValue::CodeSymbol {
                    candidate: right, ..
                },
            ) => left.stable_symbol_id.cmp(&right.stable_symbol_id),
            (
                RankedMentionValue::CodeSymbol { candidate, .. },
                RankedMentionValue::Entry(entry),
            ) => prefixed_key_cmp(
                CODE_SYMBOL_URI_PREFIX,
                candidate.stable_symbol_id.as_ref(),
                stable_tie_key(entry),
            ),
            (
                RankedMentionValue::Entry(entry),
                RankedMentionValue::CodeSymbol { candidate, .. },
            ) => prefixed_key_cmp(
                CODE_SYMBOL_URI_PREFIX,
                candidate.stable_symbol_id.as_ref(),
                stable_tie_key(entry),
            )
            .reverse(),
        }
    }

    #[cfg(test)]
    fn stable_key(&self) -> String {
        match &self.value {
            RankedMentionValue::Entry(entry) => stable_tie_key(entry).to_owned(),
            RankedMentionValue::CodeSymbol { candidate, .. } => {
                format!("{CODE_SYMBOL_URI_PREFIX}{}", candidate.stable_symbol_id)
            }
        }
    }
}

fn prefixed_key_cmp(prefix: &str, suffix: &str, other: &str) -> std::cmp::Ordering {
    prefix.bytes().chain(suffix.bytes()).cmp(other.bytes())
}

fn code_match_rank(
    entry: &MentionEntry,
    query: &str,
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
) -> Option<u32> {
    let path = code_entry_path(entry);
    match entry.kind {
        MentionKind::CodeSymbol => {
            if eq_ignore_ascii_case(&entry.display, query) {
                return Some(if entry.code_scope.is_none() {
                    u32::MAX
                } else {
                    u32::MAX - 1
                });
            }

            if let Some(score) = pattern_score(pattern, matcher, buf, &entry.display)
                .or_else(|| prefix_score(&entry.display, query))
            {
                return Some(score);
            }

            let path = path?;
            pattern_score(pattern, matcher, buf, path).or_else(|| path_prefix_score(path, query))
        }
        MentionKind::CodeFile => {
            let path = path?;
            if path_segment_exact(path, query) {
                return Some(u32::MAX);
            }

            pattern_score(pattern, matcher, buf, path).or_else(|| path_prefix_score(path, query))
        }
        MentionKind::File
        | MentionKind::Directory
        | MentionKind::Worker
        | MentionKind::Issue
        | MentionKind::Datasource => None,
    }
}

fn code_candidate_match_rank(
    candidate: &CodeMentionCandidate,
    query: &str,
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
) -> Option<u32> {
    if eq_ignore_ascii_case(candidate.entity_name.as_ref(), query) {
        return Some(if candidate.enclosing_scope.is_none() {
            u32::MAX
        } else {
            u32::MAX - 1
        });
    }

    if let Some(score) = pattern_score(pattern, matcher, buf, candidate.entity_name.as_ref())
        .or_else(|| prefix_score(candidate.entity_name.as_ref(), query))
    {
        return Some(score);
    }

    pattern_score(pattern, matcher, buf, candidate.file_path.as_ref())
        .or_else(|| path_prefix_score(candidate.file_path.as_ref(), query))
}

fn hydrate_code_rows(
    rows: &mut Vec<MentionEntry>,
    selected: &[(String, usize)],
    code_indexes: &[CodeQueryIndex],
) -> Vec<(String, Arc<CodeMentionPayload>)> {
    let returned = rows
        .iter()
        .filter(|entry| entry.kind == MentionKind::CodeSymbol)
        .map(|entry| entry.uri.as_str())
        .collect::<HashSet<_>>();
    let mut ids_by_source = vec![Vec::new(); code_indexes.len()];
    for (uri, source_index) in selected {
        if !returned.contains(uri.as_str()) {
            continue;
        }
        let Some(stable_id) = uri.strip_prefix(CODE_SYMBOL_URI_PREFIX) else {
            continue;
        };
        if let Some(ids) = ids_by_source.get_mut(*source_index) {
            ids.push(stable_id.to_owned());
        }
    }

    let mut payloads = Vec::new();
    for (source_index, stable_ids) in ids_by_source.into_iter().enumerate() {
        if stable_ids.is_empty() {
            continue;
        }
        let Some(index) = code_indexes.get(source_index) else {
            continue;
        };
        let hydrated = if let Some(snapshot) = &index.payload_snapshot {
            snapshot.hydrate_code_payloads(&stable_ids)
        } else {
            let source = match index.source.lock() {
                Ok(source) => source,
                Err(error) => {
                    tracing::warn!(error = %error, "code mention payload source lock poisoned");
                    continue;
                }
            };
            source.hydrate_code_payloads(&stable_ids)
        };
        match hydrated {
            Ok(hydrated) => payloads.extend(hydrated),
            Err(error) => {
                tracing::warn!(error = %error, "code mention payload hydration failed");
            }
        }
    }

    let hydrated = payloads
        .iter()
        .map(|(uri, _)| uri.as_str())
        .collect::<HashSet<_>>();
    let selected = selected
        .iter()
        .map(|(uri, _)| uri.as_str())
        .collect::<HashSet<_>>();
    rows.retain(|entry| {
        entry.kind != MentionKind::CodeSymbol
            || !selected.contains(entry.uri.as_str())
            || hydrated.contains(entry.uri.as_str())
    });
    payloads
}

fn pattern_score(
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
    haystack: &str,
) -> Option<u32> {
    buf.clear();
    pattern.score(nucleo_matcher::Utf32Str::new(haystack, buf), matcher)
}

fn prefix_score(value: &str, query: &str) -> Option<u32> {
    value
        .to_ascii_lowercase()
        .starts_with(&query.to_ascii_lowercase())
        .then_some(query.len() as u32)
}

fn path_prefix_score(path: &str, query: &str) -> Option<u32> {
    let query = query.to_ascii_lowercase();
    path_segments_and_stems(path)
        .any(|segment| segment.to_ascii_lowercase().starts_with(&query))
        .then_some(query.len() as u32)
}

fn path_segment_exact(path: &str, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    path_segments_and_stems(path).any(|segment| segment.to_ascii_lowercase() == query)
}

fn path_segments_and_stems(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .flat_map(|segment| {
            let stem = segment.rsplit_once('.').map(|(stem, _)| stem);
            std::iter::once(segment).chain(stem)
        })
}

fn code_entry_path(entry: &MentionEntry) -> Option<&str> {
    match entry.kind {
        MentionKind::CodeFile | MentionKind::CodeSymbol => entry.code_path.as_deref(),
        MentionKind::File
        | MentionKind::Directory
        | MentionKind::Worker
        | MentionKind::Issue
        | MentionKind::Datasource => None,
    }
}

fn eq_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn path_depth(path: &str) -> usize {
    path.trim_end_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

fn empty_code_kind_rank(kind: &MentionKind) -> u8 {
    match kind {
        MentionKind::CodeFile => 0,
        MentionKind::CodeSymbol => 1,
        MentionKind::File
        | MentionKind::Directory
        | MentionKind::Worker
        | MentionKind::Issue
        | MentionKind::Datasource => 2,
    }
}

fn stable_tie_key(entry: &MentionEntry) -> &str {
    if matches!(entry.kind, MentionKind::CodeFile | MentionKind::CodeSymbol) {
        entry.uri.as_str()
    } else {
        entry.display.as_str()
    }
}

fn append_section_rows(
    rows: &mut Vec<MentionEntry>,
    header: &'static str,
    section: &[MentionEntry],
) {
    if section.is_empty() {
        return;
    }
    rows.push(MentionEntry {
        section_header: Some(header),
        kind: MentionKind::File,
        uri: String::new(),
        display: format!("── {header} ──"),
        secondary: None,
        agent: None,
        model: None,
        effort: None,
        worker_kind: None,
        worker_cli_identity: None,
        code_path: None,
        code_scope: None,
        tag: None,
        search_text: None,
        atom_text: None,
        unconsumed_suffix: None,
        issue_preview: None,
    });
    rows.extend(section.iter().cloned());
}

fn issue_id(entry: &MentionEntry) -> &str {
    entry
        .issue_preview
        .as_ref()
        .map(|issue| issue.id.as_str())
        .unwrap_or(entry.display.as_str())
}

/// Cross-kind tier for a TUI row: the neutral engine's tier table
/// (`spur_mentions::rank::{tier_of, FILE_TREE_TIER, CODE_TIER}` with the
/// TUI's custom-kind mapping worker→1, issue→2, datasource→3), projected
/// onto the TUI kind enum so the comparator path stays allocation-free.
fn tier_of_kind(kind: &MentionKind) -> u8 {
    match kind {
        MentionKind::File | MentionKind::Directory => rank::FILE_TREE_TIER,
        MentionKind::Worker => 1,
        MentionKind::Issue => 2,
        MentionKind::Datasource => 3,
        MentionKind::CodeFile | MentionKind::CodeSymbol => rank::CODE_TIER,
    }
}

fn typed_query_cmp(
    a: &RankedMentionRef<'_>,
    b: &RankedMentionRef<'_>,
    max_rank: u32,
) -> std::cmp::Ordering {
    // Transitive comparator over the engine's ranking primitives: bucket
    // rank (desc, via `spur_mentions::rank::score_bucket`), then tier rank
    // (asc), then raw rank (desc), then the stable key (frontend policy
    // here because compact code candidates have no materialized row).
    let bucket_a = rank::score_bucket(a.rank, max_rank);
    let bucket_b = rank::score_bucket(b.rank, max_rank);
    bucket_b
        .cmp(&bucket_a)
        .then(a.tier.cmp(&b.tier))
        .then(b.rank.cmp(&a.rank))
        .then_with(|| a.stable_cmp(b))
}

fn discover_code_graph_candidate(worktree_root: &Path) -> Option<CodeGraphCandidate> {
    if let Some(path) = std::env::var_os(CODE_GRAPH_INDEX_ENV).filter(|v| !v.is_empty()) {
        let explicit_override = PathBuf::from(path);
        let resolved =
            spur_graph::resolve_artifact_location(worktree_root, Some(&explicit_override)).ok()?;
        let token = token_for_resolved(worktree_root, &resolved, Some(&explicit_override));
        return Some(CodeGraphCandidate {
            explicit_override: Some(explicit_override),
            token,
        });
    }

    let resolved = spur_graph::resolve_artifact_location(worktree_root, None).ok()?;
    let token = token_for_resolved(worktree_root, &resolved, None);
    Some(CodeGraphCandidate {
        explicit_override: None,
        token,
    })
}

fn token_for_resolved(
    worktree_root: &Path,
    resolved: &spur_graph::ResolvedArtifact,
    explicit_override: Option<&Path>,
) -> CodeGraphToken {
    if explicit_override
        .and_then(|path| canonical_artifact_path(worktree_root, path))
        .as_ref()
        == Some(&resolved.path)
    {
        return resolved_token(resolved);
    }

    if !current_pointer_selected(worktree_root, resolved) {
        if let Some(pointer_token) = pointer_token(worktree_root, resolved) {
            return pointer_token;
        }
    }

    resolved_token(resolved)
}

fn resolved_token(resolved: &spur_graph::ResolvedArtifact) -> CodeGraphToken {
    let metadata = fs::metadata(&resolved.path).ok();
    CodeGraphToken::Resolved {
        artifact_path: resolved.path.clone(),
        cache_key: resolved.cache_key.clone(),
        modified: metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok()),
        len: metadata.map(|metadata| metadata.len()).unwrap_or(0),
    }
}

fn current_pointer_selected(worktree_root: &Path, resolved: &spur_graph::ResolvedArtifact) -> bool {
    spur_graph::read_current_pointer(worktree_root)
        .ok()
        .as_ref()
        == Some(&resolved.path)
}

fn pointer_token(
    worktree_root: &Path,
    resolved: &spur_graph::ResolvedArtifact,
) -> Option<CodeGraphToken> {
    let pointer_path = worktree_root.join(CODE_GRAPH_POINTER_PATH);
    let bytes = fs::read(&pointer_path).ok()?;
    let pointer: spur_graph::GraphIndexPointer = serde_json::from_slice(&bytes).ok()?;
    if pointer.schema != CODE_GRAPH_POINTER_SCHEMA {
        return None;
    }
    let canonical_artifact_path =
        canonical_artifact_path(worktree_root, &pointer.canonical_artifact_path)?;
    if canonical_artifact_path != resolved.path {
        return None;
    }
    Some(CodeGraphToken::Pointer {
        canonical_artifact_path,
        graph_content_hash: pointer.graph_content_hash,
        manifest_version: pointer.manifest_version,
        indexed_commit_oid: pointer.indexed_commit_oid,
    })
}

fn canonical_artifact_path(worktree_root: &Path, path: &Path) -> Option<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    };
    path.canonicalize().ok()
}

impl Default for MentionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Instant;

    use super::*;
    use crate::mentions::{
        IssueMentionDescriptor, IssueMentionSource, MentionKind, MentionSource,
        WorkerMentionDescriptor, WorkerMentionSource,
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use filetime::{set_file_mtime, FileTime};
    use spur_acp::agent_model_catalog::{
        self, AgentModelCatalogV1, ConfigOptionChoice, WorkerCatalogEntry,
    };
    use spur_acp::AgentKind;
    use spur_graph::{
        write_artifact_parquet, write_current_pointer, GraphFileArtifact, GraphFileManifestEntry,
        GraphIndexArtifact, GraphIndexHeader, GraphIndexPointer, NodeId, SourceKind, WriteOptions,
    };
    use spur_pm::PmSource;

    const TEST_GRAPH_INDEX_VERSION: &str = "fixture-2026-05-11";

    fn issue(id: &str, title: &str, assignee: Option<&str>) -> IssueMentionDescriptor {
        IssueMentionDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            source: PmSource::Beads,
            status: "open".to_string(),
            assignee: assignee.map(str::to_string),
            priority: None,
            issue_type: Some("task".to_string()),
            labels: vec!["mentions".to_string()],
            url: format!("https://example.test/{id}"),
            description: None,
        }
    }

    fn worker(name: &str, kind: AgentKind) -> WorkerMentionDescriptor {
        WorkerMentionDescriptor {
            name: name.to_string(),
            kind,
            cli_identity: "test".to_string(),
            description: None,
            tier: None,
        }
    }

    fn write_agent_profile(root: &Path, name: &str) {
        let dir = root.join(".spur/agents");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {name} profile\n---\nbody\n"),
        )
        .unwrap();
    }

    /// Vendor a clean explore-pool agent only — no `.spur/agents/` Apply step.
    fn write_pool_agent_only(root: &Path, name: &str) {
        use spur_core::explore::catalog::ItemKind;
        use spur_core::explore::pool::{GateRecord, Manifest, ManifestItem};

        let pin = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let source = "acme/agents";
        let pool_dir = spur_core::explore::store::local_pool_dir(root, source, name, pin);
        fs::create_dir_all(&pool_dir).unwrap();
        fs::write(
            pool_dir.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {name} from pool\n---\npool body\n"),
        )
        .unwrap();
        Manifest {
            sources: vec![],
            items: vec![ManifestItem {
                name: name.to_string(),
                kind: ItemKind::Agent,
                source: source.to_string(),
                rel_path: format!("agents/{name}.md"),
                pinned_commit: pin.to_string(),
                content_sha256: "0".repeat(64),
                license: None,
                gate: GateRecord {
                    verdict: "clean".to_string(),
                    justification: None,
                    decided_at_epoch: None,
                },
            }],
        }
        .save(root)
        .unwrap();
    }

    fn choice(value: &str, name: &str) -> ConfigOptionChoice {
        ConfigOptionChoice {
            value: value.to_string(),
            name: name.to_string(),
            description: None,
        }
    }

    fn write_catalog(
        root: &Path,
        worker_name: &str,
        models: Vec<ConfigOptionChoice>,
        efforts: Vec<ConfigOptionChoice>,
    ) -> PathBuf {
        let path = root.join("agent-model-catalog.json");
        let mut entries = HashMap::new();
        entries.insert(
            worker_name.to_string(),
            WorkerCatalogEntry {
                probed_at: Utc::now(),
                cli_identity: "test".to_string(),
                models,
                efforts,
            },
        );
        agent_model_catalog::write(
            &path,
            &AgentModelCatalogV1 {
                version: 1,
                entries,
            },
        )
        .unwrap();
        path
    }

    #[test]
    fn query_matches_issue_search_text_not_just_display() {
        let mut registry =
            test_session_registry(vec![Box::new(IssueMentionSource::new(vec![issue(
                "bd-1",
                "Picker rows",
                Some("alice"),
            )]))]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "alice", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].uri, "issue://beads/bd-1");
    }

    #[test]
    fn worker_query_bare_worker_exposes_agent_slot_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("agent slot should be open");

        assert_eq!(open_slot.kind, WorkerSlotKind::Agent);
        assert_eq!(open_slot.filter_text, "");
        assert!(!open_slot.is_final, "codex still has model/effort slots");
        assert!(open_slot
            .candidates
            .iter()
            .any(|c| c.value == "spur-narrow-implementer"));
    }

    #[test]
    fn worker_query_exposes_explore_pool_agents_without_spur_agents_apply() {
        let tmp = tempfile::tempdir().unwrap();
        // Pool-only: mirrors spur-notebook where /explore shows agents but
        // `.spur/agents/` was never written by Apply.
        // Unique name avoids false-greens from a machine-global explore pool.
        write_pool_agent_only(tmp.path(), "pool-only-mention-agent");
        assert!(
            !tmp.path().join(".spur/agents").exists(),
            "fixture must not create .spur/agents"
        );
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("agent slot should open from pool agents alone");

        assert_eq!(open_slot.kind, WorkerSlotKind::Agent);
        assert!(
            open_slot
                .candidates
                .iter()
                .any(|c| c.value == "pool-only-mention-agent"),
            "pool agent must appear without Apply; candidates={:?}",
            open_slot
                .candidates
                .iter()
                .map(|c| &c.value)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn worker_matching_typed_query_keeps_issue_rows_without_duplicating_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        // Point at a nonexistent catalog so the cascade has no model/effort
        // candidates regardless of the developer machine's real cache.
        registry.set_agent_model_catalog_path_for_test(tmp.path().join("catalog.json"));
        registry.set_issue_snapshot(vec![issue("bd-7", "Coordinate codex work", None)]);

        let hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);
        let debug: Vec<_> = hits
            .iter()
            .map(|hit| (&hit.kind, &hit.display, &hit.uri))
            .collect();

        assert_eq!(
            hits.iter()
                .filter(|hit| hit.kind == MentionKind::Worker)
                .count(),
            1,
            "composed row must replace the base worker entry, not duplicate it: {debug:?}"
        );
        assert!(
            hits.iter()
                .any(|hit| hit.kind == MentionKind::Issue && hit.uri == "issue://beads/bd-7"),
            "issue rows matching the same text must stay reachable: {debug:?}"
        );
    }

    #[test]
    fn worker_query_partial_slot_text_is_exposed_as_filter() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex,gpt-5", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("agent slot should still be open on a non-matching token");

        assert_eq!(open_slot.kind, WorkerSlotKind::Agent);
        assert_eq!(open_slot.filter_text, "gpt-5");
    }

    #[test]
    fn probe_hint_tracks_in_flight_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);

        assert_eq!(registry.agent_model_catalog_probe_hint(), None);
        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);
        assert_eq!(
            registry.agent_model_catalog_probe_hint().as_deref(),
            Some("fetching codex models\u{2026}"),
            "a pending (not yet drained) probe request must already surface the hint"
        );
        assert_eq!(
            registry.drain_agent_model_catalog_probe_requests(),
            vec!["codex".to_string()]
        );
        assert_eq!(
            registry.agent_model_catalog_probe_hint().as_deref(),
            Some("fetching codex models\u{2026}"),
            "drained probe requests must surface as an in-flight hint"
        );

        registry.mark_agent_model_catalog_probe_completed("codex");
        assert_eq!(
            registry.agent_model_catalog_probe_hint(),
            None,
            "completion must clear the hint"
        );
    }

    #[test]
    fn worker_query_unique_slot_prefix_still_advances() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "rust-reviewer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex,ru", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("resolved agent should advance to an open model slot");

        assert_eq!(
            open_slot.kind,
            WorkerSlotKind::Model,
            "a unique high-confidence prefix must resolve the agent slot"
        );
    }

    #[test]
    fn worker_query_ambiguous_slot_prefix_stays_open_with_all_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "rust-reviewer");
        write_agent_profile(tmp.path(), "rust-tester");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex,rust", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("ambiguous prefix must keep the agent slot open");

        assert_eq!(open_slot.kind, WorkerSlotKind::Agent);
        assert_eq!(open_slot.filter_text, "rust");
        let labels: Vec<&str> = open_slot
            .candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect();
        assert!(
            labels.contains(&"rust-reviewer") && labels.contains(&"rust-tester"),
            "both matching agents must stay visible as filter results, got {labels:?}"
        );
    }

    #[test]
    fn worker_query_exact_slot_text_advances_even_with_prefix_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "rust-reviewer");
        write_agent_profile(tmp.path(), "rust-reviewer-strict");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,rust-reviewer",
            10,
        );
        let open_slot = registry
            .take_worker_open_slot()
            .expect("exact agent text should advance to an open model slot");

        assert_eq!(
            open_slot.kind,
            WorkerSlotKind::Model,
            "an exact candidate name must resolve the slot even when it prefixes a sibling"
        );
    }

    #[test]
    fn worker_query_exposes_model_slot_after_agent_resolved() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex,narrow", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("model slot should be open");

        assert_eq!(open_slot.kind, WorkerSlotKind::Model);
        assert!(!open_slot.is_final, "codex still has an effort slot");
        assert!(open_slot.candidates.iter().any(|c| c.value == "gpt-5"));
    }

    #[test]
    fn worker_query_effort_slot_is_always_final() {
        let tmp = tempfile::tempdir().unwrap();
        let catalog_path = write_catalog(
            tmp.path(),
            "gemini",
            vec![choice("gemini-2.5-pro", "Gemini 2.5 Pro")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("gemini", AgentKind::Gemini)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let _hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "gemini,gemini-2.5-pro",
            10,
        );
        let open_slot = registry
            .take_worker_open_slot()
            .expect("effort slot should be open");

        assert_eq!(open_slot.kind, WorkerSlotKind::Effort);
        assert!(open_slot.is_final);
    }

    #[test]
    fn worker_query_no_catalog_yet_marks_agent_slot_final() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);

        let _hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);
        let open_slot = registry
            .take_worker_open_slot()
            .expect("agent slot should be open even with no catalog entry yet");

        assert_eq!(open_slot.kind, WorkerSlotKind::Agent);
        assert!(
            open_slot.is_final,
            "no catalog entry yet means model/effort are both empty"
        );
    }

    #[test]
    fn worker_query_composes_agent_model_and_effort_slots() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5"), choice("gpt-5-mini", "GPT-5 mini")],
            vec![choice("high", "High"), choice("low", "Low")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,narrow,gpt-5,high",
            10,
        );
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");

        assert_eq!(
            composed.uri,
            "worker://codex?agent=spur-narrow-implementer&model=gpt-5&effort=high"
        );
        assert_eq!(
            composed.display,
            "worker:codex agent=spur-narrow-implementer model=gpt-5 effort=high"
        );
        assert_eq!(
            composed.atom_text.as_deref(),
            Some("@worker:codex agent=spur-narrow-implementer model=gpt-5 effort=high")
        );
        assert_eq!(composed.agent.as_deref(), Some("spur-narrow-implementer"));
        assert_eq!(composed.model.as_deref(), Some("gpt-5"));
        assert_eq!(composed.effort.as_deref(), Some("high"));
    }

    #[test]
    fn worker_query_double_comma_explicitly_skips_agent_slot() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,,gpt-5,high",
            10,
        );
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");

        assert_eq!(composed.uri, "worker://codex?model=gpt-5&effort=high");
        assert_eq!(composed.agent, None);
        assert_eq!(composed.model.as_deref(), Some("gpt-5"));
        assert_eq!(composed.effort.as_deref(), Some("high"));
    }

    #[test]
    fn worker_query_double_comma_explicitly_skips_model_slot() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,narrow,,high",
            10,
        );
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");

        assert_eq!(
            composed.uri,
            "worker://codex?agent=spur-narrow-implementer&effort=high"
        );
        assert_eq!(composed.agent.as_deref(), Some("spur-narrow-implementer"));
        assert_eq!(composed.model, None);
        assert_eq!(composed.effort.as_deref(), Some("high"));
    }

    #[test]
    fn worker_query_single_trailing_comma_stays_open_not_a_skip() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex,", 10);
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");

        assert_eq!(composed.uri, "worker://codex");
        assert_eq!(composed.agent, None);
        assert_eq!(composed.unconsumed_suffix, None);
    }

    #[test]
    fn worker_query_auto_skips_empty_agent_pool_to_model_slot() {
        let tmp = tempfile::tempdir().unwrap();
        let catalog_path = write_catalog(
            tmp.path(),
            "gemini",
            vec![choice("gemini-2.5-pro", "Gemini 2.5 Pro")],
            Vec::new(),
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("gemini", AgentKind::Gemini)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "gemini,gemini-2.5-pro",
            10,
        );
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");

        assert_eq!(composed.uri, "worker://gemini?model=gemini-2.5-pro");
        assert_eq!(composed.agent, None);
        assert_eq!(composed.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(composed.effort, None);
    }

    #[test]
    fn worker_query_marks_missing_catalog_entry_for_background_probe() {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);

        let hits = registry.query(CompletionScope::PreSession, tmp.path(), "codex", 10);

        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");
        assert_eq!(composed.uri, "worker://codex");
        assert_eq!(
            registry.drain_agent_model_catalog_probe_requests(),
            vec!["codex".to_string()]
        );
    }

    #[test]
    fn worker_query_uses_stale_catalog_entry_and_marks_background_probe() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("agent-model-catalog.json");
        let mut entries = HashMap::new();
        entries.insert(
            "codex".to_string(),
            WorkerCatalogEntry {
                probed_at: Utc::now() - ChronoDuration::hours(25),
                cli_identity: "test".to_string(),
                models: vec![choice("gpt-5", "GPT-5")],
                efforts: vec![choice("high", "High")],
            },
        );
        agent_model_catalog::write(
            &path,
            &AgentModelCatalogV1 {
                version: 1,
                entries,
            },
        )
        .unwrap();
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,gpt-5,high",
            10,
        );

        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("composed worker row");
        assert_eq!(composed.uri, "worker://codex?model=gpt-5&effort=high");
        assert_eq!(
            registry.drain_agent_model_catalog_probe_requests(),
            vec!["codex".to_string()]
        );
    }

    #[test]
    fn worker_query_reparse_reopens_edited_slot_and_clears_after_it() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        write_agent_profile(tmp.path(), "reviewer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            vec![choice("high", "High")],
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let edited = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,reviewer",
            10,
        );
        let composed = edited
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("reparsed worker row");

        assert_eq!(composed.uri, "worker://codex?agent=reviewer");
        assert_eq!(composed.agent.as_deref(), Some("reviewer"));
        assert_eq!(composed.model, None);
        assert_eq!(composed.effort, None);
    }

    #[test]
    fn worker_query_tracks_unconsumed_suffix_after_slot_miss() {
        let tmp = tempfile::tempdir().unwrap();
        write_agent_profile(tmp.path(), "spur-narrow-implementer");
        let catalog_path = write_catalog(
            tmp.path(),
            "codex",
            vec![choice("gpt-5", "GPT-5")],
            Vec::new(),
        );
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        registry.set_agent_model_catalog_path_for_test(catalog_path);

        let hits = registry.query(
            CompletionScope::PreSession,
            tmp.path(),
            "codex,no-such-agent,should,stay,text",
            10,
        );
        let composed = hits
            .iter()
            .find(|hit| hit.kind == MentionKind::Worker)
            .expect("worker row with unconsumed suffix");

        assert_eq!(composed.uri, "worker://codex");
        assert_eq!(composed.agent, None);
        assert_eq!(composed.model, None);
        assert_eq!(composed.effort, None);
        assert_eq!(
            composed.unconsumed_suffix.as_deref(),
            Some(",no-such-agent,should,stay,text")
        );
    }

    #[test]
    fn code_graph_pointer_appearing_after_startup_lazy_registers_source() {
        let _guard = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let _restore = ProcessEnvRestore::enter(root);

        let mut registry = MentionRegistry::for_direct_session().with_code_graph_from_env();
        assert_eq!(registry.code_graph_hint(), Some(CODE_GRAPH_MISSING_HINT));

        let artifact_path = root.join(".spur/canonical/graph-a.parquet");
        write_graph_fixture(&artifact_path, "lazy-file", "src/lazy.rs", "hash-a");
        write_pointer(
            root,
            &artifact_path,
            "hash-a",
            "manifest-a",
            Some("commit-a"),
        );

        let hits = registry.query(CompletionScope::PreSession, root, "lazy", 10);

        assert!(
            hits.iter()
                .any(|hit| hit.kind == MentionKind::CodeFile && hit.display == "src/lazy.rs"),
            "expected lazy-registered code graph row, got {hits:?}"
        );
        assert_eq!(registry.code_graph_hint(), None);
    }

    #[test]
    fn code_graph_pointer_path_change_reloads_immediately() {
        let _guard = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let _restore = ProcessEnvRestore::enter(root);

        let artifact_a = root.join(".spur/canonical/graph-a.parquet");
        let artifact_b = root.join(".spur/canonical/graph-b.parquet");
        write_graph_fixture(&artifact_a, "first-file", "src/first.rs", "hash-a");
        write_graph_fixture(&artifact_b, "second-file", "src/second.rs", "hash-b");
        write_pointer(root, &artifact_a, "hash-a", "manifest-a", Some("commit-a"));

        let mut registry = MentionRegistry::for_direct_session().with_code_graph_from_env();
        let first = registry.query(CompletionScope::PreSession, root, "first", 10);
        assert!(first.iter().any(|hit| hit.display == "src/first.rs"));

        write_pointer(root, &artifact_b, "hash-b", "manifest-b", Some("commit-b"));
        let second = registry.query(CompletionScope::PreSession, root, "second", 10);

        assert!(
            second.iter().any(|hit| hit.display == "src/second.rs"),
            "expected pointer path change to reload graph rows, got {second:?}"
        );
    }

    #[test]
    fn code_graph_pointer_hash_change_reloads_even_when_artifact_metadata_is_stable() {
        let _guard = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let _restore = ProcessEnvRestore::enter(root);

        let artifact_path = root.join(".spur/canonical/graph.parquet");
        let stable_mtime = FileTime::from_unix_time(1_700_000_000, 0);
        write_graph_fixture(&artifact_path, "old-file", "src/old.rs", "hash-a");
        set_file_mtime(&artifact_path, stable_mtime).unwrap();
        let original_len = fs::metadata(&artifact_path).unwrap().len();
        write_pointer(
            root,
            &artifact_path,
            "hash-a",
            "manifest-a",
            Some("commit-a"),
        );

        let mut registry = MentionRegistry::for_direct_session().with_code_graph_from_env();
        let old = registry.query(CompletionScope::PreSession, root, "old", 10);
        assert!(old.iter().any(|hit| hit.display == "src/old.rs"));

        write_graph_fixture(&artifact_path, "new-file", "src/new.rs", "hash-b");
        assert_eq!(
            fs::metadata(&artifact_path).unwrap().len(),
            original_len,
            "fixture rewrite must keep length stable to prove pointer-token invalidation"
        );
        set_file_mtime(&artifact_path, stable_mtime).unwrap();
        write_pointer(
            root,
            &artifact_path,
            "hash-b",
            "manifest-b",
            Some("commit-b"),
        );

        let new = registry.query(CompletionScope::PreSession, root, "new", 10);

        assert!(
            new.iter().any(|hit| hit.display == "src/new.rs"),
            "expected pointer hash change to force source reload, got {new:?}"
        );
    }

    #[test]
    fn code_graph_env_override_still_wins_over_pointer() {
        let _guard = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let env_artifact = root.join("env-graph.parquet");
        let pointer_artifact = root.join(".spur/canonical/pointer-graph.parquet");
        write_graph_fixture(&env_artifact, "env-file", "src/env.rs", "env-hash");
        write_graph_fixture(
            &pointer_artifact,
            "pointer-file",
            "src/pointer.rs",
            "pointer-hash",
        );
        write_pointer(
            root,
            &pointer_artifact,
            "pointer-hash",
            "manifest-pointer",
            Some("commit-pointer"),
        );
        let _restore = ProcessEnvRestore::enter(root).with_env_override(&env_artifact);

        let mut registry = MentionRegistry::for_direct_session().with_code_graph_from_env();
        let hits = registry.query(CompletionScope::PreSession, root, "env", 10);

        assert!(hits.iter().any(|hit| hit.display == "src/env.rs"));
        assert!(
            !hits.iter().any(|hit| hit.display == "src/pointer.rs"),
            "env override should ignore pointer artifact rows, got {hits:?}"
        );
    }

    #[test]
    fn code_graph_auto_discovery_uses_current_parquet() {
        let _guard = crate::theme::runtime::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let _restore = ProcessEnvRestore::enter(root);

        let parquet_dir = write_artifact_parquet(
            &graph_artifact("parquet-file", "src/parquet.rs", "parquet-hash"),
            &root.join(".git/spur-graph/artifacts/test"),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        write_current_pointer(root, &parquet_dir).expect("write CURRENT pointer");

        let mut registry = MentionRegistry::for_direct_session().with_code_graph_from_env();
        let hits = registry.query(CompletionScope::PreSession, root, "parquet", 10);

        assert!(
            hits.iter()
                .any(|hit| hit.kind == MentionKind::CodeFile && hit.display == "src/parquet.rs"),
            "expected resolver to prefer CURRENT parquet artifact, got {hits:?}"
        );
        assert_eq!(registry.code_graph_hint(), None);
    }

    #[test]
    fn snapshot_replacement_changes_the_engine_source_token() {
        // sud-m3 RED-first pin: worker/issue/datasource sources implement the
        // neutral `spur_mentions::MentionSource` directly, and their
        // data-revision token must change whenever the snapshot is replaced
        // so the engine cache key (root + key + profile + token) invalidates
        // immediately instead of waiting out the TTL.
        let mut registry =
            MentionRegistry::for_brain_session(vec![worker("codex", AgentKind::CodexAcp)]);
        let before = registry
            .session_source_token("worker")
            .expect("worker session source registered");
        registry.set_worker_snapshot_in_place(vec![worker("gemini", AgentKind::Gemini)]);
        let after = registry
            .session_source_token("worker")
            .expect("worker session source still registered");
        assert_ne!(
            before, after,
            "replacing a snapshot must change the engine cache token"
        );
        // Monotonic across repeated swaps: no aliasing back to an old token.
        registry.set_worker_snapshot_in_place(vec![worker("codex", AgentKind::CodexAcp)]);
        let again = registry
            .session_source_token("worker")
            .expect("worker session source registered");
        assert_ne!(after, again);
    }

    #[test]
    fn issue_and_datasource_snapshot_replacements_change_their_source_tokens() {
        let mut registry = MentionRegistry::for_brain_session(Vec::new());
        registry.set_issue_snapshot(vec![issue("bd-1", "Old title", None)]);
        let issue_before = registry
            .session_source_token("issue")
            .expect("issue session source registered");
        registry.set_datasource_snapshot(vec![]);
        let datasource_before = registry
            .session_source_token("datasource")
            .expect("datasource session source registered");

        registry.set_issue_snapshot(vec![issue("bd-2", "New title", None)]);
        registry.set_datasource_snapshot(vec![]);

        assert_ne!(
            issue_before,
            registry
                .session_source_token("issue")
                .expect("issue session source registered")
        );
        assert_ne!(
            datasource_before,
            registry
                .session_source_token("datasource")
                .expect("datasource session source registered")
        );
    }

    #[test]
    fn set_issue_snapshot_clears_cache_for_next_query() {
        let mut registry =
            test_session_registry(vec![Box::new(IssueMentionSource::new(vec![issue(
                "bd-1",
                "Old title",
                None,
            )]))]);

        let first = registry.query(CompletionScope::PreSession, Path::new("."), "old", 10);
        assert_eq!(first[0].display, "bd-1 Old title");

        registry.set_issue_snapshot(vec![issue("bd-2", "New title", None)]);
        let old = registry.query(CompletionScope::PreSession, Path::new("."), "old", 10);
        let new = registry.query(CompletionScope::PreSession, Path::new("."), "new", 10);

        assert!(old.is_empty());
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].display, "bd-2 New title");
    }

    #[test]
    fn set_issue_snapshot_does_not_invalidate_file_cache() {
        let file_build_count = Arc::new(AtomicUsize::new(0));
        let mut registry = test_slots(vec![
            MentionSourceSlot::new(Box::new(CountingSource {
                name: "file",
                entries: vec![mention(MentionKind::File, 1, "src/main.rs".into())],
                build_count: Arc::clone(&file_build_count),
            })),
            MentionSourceSlot::session(Box::new(IssueMentionSource::new(vec![issue(
                "bd-1",
                "Old title",
                None,
            )]))),
        ]);

        let _ = registry.query(CompletionScope::PreSession, Path::new("."), "", 16);
        assert_eq!(file_build_count.load(Ordering::Relaxed), 1);

        registry.set_issue_snapshot(vec![issue("bd-2", "New title", None)]);
        let _ = registry.query(CompletionScope::PreSession, Path::new("."), "", 16);

        assert_eq!(file_build_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn presession_to_session_transition_reuses_global_caches() {
        let file_build_count = Arc::new(AtomicUsize::new(0));
        let code_build_count = Arc::new(AtomicUsize::new(0));
        let session_id = SessionId("session-1".to_string());
        let mut registry = test_slots(vec![
            MentionSourceSlot::new(Box::new(CountingSource {
                name: "file",
                entries: vec![mention(MentionKind::File, 1, "src/lib.rs".into())],
                build_count: Arc::clone(&file_build_count),
            })),
            MentionSourceSlot::new(Box::new(CountingSource {
                name: "code",
                entries: vec![mention(MentionKind::CodeFile, 2, "src/code.rs".into())],
                build_count: Arc::clone(&code_build_count),
            })),
            MentionSourceSlot::session(Box::new(WorkerMentionSource::new(vec![
                WorkerMentionDescriptor {
                    name: "alpha".into(),
                    kind: AgentKind::Generic,
                    cli_identity: "alpha".into(),
                    description: None,
                    tier: None,
                },
            ]))),
        ]);

        let _ = registry.query(CompletionScope::PreSession, Path::new("."), "", 16);
        let _ = registry.query(
            CompletionScope::Session(&session_id),
            Path::new("."),
            "",
            16,
        );

        assert_eq!(file_build_count.load(Ordering::Relaxed), 1);
        assert_eq!(code_build_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn query_uses_smart_case_matching() {
        let mut registry = test_session_registry(vec![Box::new(IssueMentionSource::new(vec![
            issue("bd-1", "deploy prod", None),
            issue("bd-2", "Deploy Prod", None),
        ]))]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "Deploy", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].display, "bd-2 Deploy Prod");
    }

    struct StaticSource {
        name: &'static str,
        entries: Vec<MentionEntry>,
    }

    struct RootScopedSource {
        fail: Arc<std::sync::atomic::AtomicBool>,
        hints: Vec<(String, Arc<String>)>,
    }

    impl MentionSource for RootScopedSource {
        fn name(&self) -> &'static str {
            "code_graph"
        }

        fn build(&mut self, cwd: &Path) -> anyhow::Result<Vec<MentionEntry>> {
            anyhow::ensure!(!self.fail.load(Ordering::Relaxed), "fixture build failure");
            let root = cwd.display().to_string();
            self.hints = vec![("datasource://shared".into(), Arc::new(root.clone()))];
            Ok(vec![MentionEntry {
                uri: "file:///shared".into(),
                display: "shared".into(),
                tag: Some(root),
                ..MentionEntry::default()
            }])
        }

        fn datasource_hints(&self) -> &[(String, Arc<String>)] {
            &self.hints
        }
    }

    fn root_scoped_registry() -> (
        MentionRegistry,
        Arc<std::sync::atomic::AtomicBool>,
        Arc<spur_mentions::ManualClock>,
    ) {
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut registry = test_registry(vec![Box::new(RootScopedSource {
            fail: Arc::clone(&fail),
            hints: Vec::new(),
        })]);
        let clock = Arc::new(spur_mentions::ManualClock::new());
        registry.engine = MentionEngine::new(clock.clone());
        for source in &registry.sources {
            registry
                .engine
                .register_source(source.engine_adapter(), false);
        }
        (registry, fail, clock)
    }

    #[test]
    fn failed_rebuild_restores_the_matching_root_rows_and_hints() {
        let (mut registry, fail, clock) = root_scoped_registry();
        let a = Path::new("root-a");
        let b = Path::new("root-b");
        let first = registry.query(CompletionScope::PreSession, a, "shared", 10);
        registry.query(CompletionScope::PreSession, b, "shared", 10);
        clock.advance(registry.engine.cache_ttl() + std::time::Duration::from_nanos(1));
        fail.store(true, Ordering::Relaxed);

        let retained = registry.query(CompletionScope::PreSession, a, "shared", 10);
        assert_eq!(retained, first);
        assert_eq!(
            registry.lookup_datasource_hint("datasource://shared"),
            Some("root-a")
        );
    }

    #[test]
    fn failed_build_at_a_new_root_does_not_borrow_another_roots_rows() {
        let (mut registry, fail, _) = root_scoped_registry();
        registry.query(
            CompletionScope::PreSession,
            Path::new("root-a"),
            "shared",
            10,
        );
        fail.store(true, Ordering::Relaxed);
        let rows = registry.query(
            CompletionScope::PreSession,
            Path::new("root-b"),
            "shared",
            10,
        );
        assert!(rows.is_empty(), "{rows:?}");
        assert!(registry
            .lookup_datasource_hint("datasource://shared")
            .is_none());
    }

    #[test]
    fn deferred_root_build_does_not_expose_another_roots_rows_or_hints() {
        let (mut registry, _, _) = root_scoped_registry();
        let a = Path::new("root-a");
        let b = Path::new("root-b");
        let first = registry.query(CompletionScope::PreSession, a, "shared", 10);
        let MentionQueryPreparation::ReadyAndBuild { work, build } =
            registry.prepare_query_work_nonblocking(CompletionScope::PreSession, b, "shared", 10)
        else {
            panic!("uncached code source must defer its build");
        };
        assert!(registry.run_query_work(work).entries.is_empty());
        assert!(registry
            .lookup_datasource_hint("datasource://shared")
            .is_none());

        assert_eq!(
            registry.query(CompletionScope::PreSession, a, "shared", 10),
            first
        );
        registry.apply_cache_build(build_mention_caches(build));
        // An older background request may finish after the user changes root.
        assert_eq!(
            registry.lookup_datasource_hint("datasource://shared"),
            Some("root-a")
        );
        assert_eq!(
            registry.query(CompletionScope::PreSession, a, "shared", 10),
            first
        );
        registry.query(CompletionScope::PreSession, b, "shared", 10);
        assert_eq!(
            registry.lookup_datasource_hint("datasource://shared"),
            Some("root-b")
        );
    }

    impl MentionSource for StaticSource {
        fn build(&mut self, _cwd: &Path) -> anyhow::Result<Vec<MentionEntry>> {
            Ok(self.entries.clone())
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    struct CompactCodeSource {
        candidates: Arc<Vec<CodeMentionCandidate>>,
    }

    impl MentionSource for CompactCodeSource {
        fn build(&mut self, _cwd: &Path) -> anyhow::Result<Vec<MentionEntry>> {
            Ok(Vec::new())
        }

        fn name(&self) -> &'static str {
            "code_graph"
        }

        fn code_candidates(&self) -> Arc<Vec<CodeMentionCandidate>> {
            Arc::clone(&self.candidates)
        }

        fn hydrate_code_payloads(
            &self,
            stable_symbol_ids: &[String],
        ) -> anyhow::Result<Vec<(String, Arc<CodeMentionPayload>)>> {
            Ok(stable_symbol_ids
                .iter()
                .map(|stable_id| {
                    let uri = format!("{CODE_SYMBOL_URI_PREFIX}{stable_id}");
                    (
                        uri.clone(),
                        Arc::new(CodeMentionPayload {
                            authoritative: spur_graph::CodeMentionAuthoritative {
                                display: stable_id.clone(),
                                uri,
                                kind: spur_graph::CodeMentionKind::Symbol,
                                file_path: "src/lib.rs".to_owned(),
                                validation: spur_graph::CodeMentionValidationSpec::SymbolRange {
                                    path: "src/lib.rs".to_owned(),
                                    line_range: [1, 1],
                                    byte_range: [0, 1],
                                    entity_name: stable_id.clone(),
                                    anchor_hash: "anchor".to_owned(),
                                },
                            },
                            extraction_hints: spur_graph::CodeMentionExtractionHints {
                                line_range: Some([1, 1]),
                                byte_range: Some([0, 1]),
                                symbol_kind: Some("fn".to_owned()),
                                entity_name: Some(stable_id.clone()),
                                qualified_name: stable_id.clone(),
                            },
                            display_meta: spur_graph::CodeMentionDisplayMeta {
                                enclosing_scope: None,
                                graph_index_version: "test".to_owned(),
                            },
                        }),
                    )
                })
                .collect())
        }
    }

    fn compact_candidate(stable_id: &str, entity_name: &str) -> CodeMentionCandidate {
        CodeMentionCandidate {
            stable_symbol_id: stable_id.into(),
            entity_name: entity_name.into(),
            file_path: Arc::from("src/lib.rs"),
            line_range: [1, 1],
            symbol_kind: Arc::from("fn"),
            enclosing_scope: None,
        }
    }

    #[test]
    fn accepted_code_payload_survives_cache_rebuild_until_submit_retention_prunes_it() {
        let mut registry = test_registry(vec![Box::new(CompactCodeSource {
            candidates: Arc::new(vec![
                compact_candidate("symbol-alpha", "alpha"),
                compact_candidate("symbol-beta", "beta"),
            ]),
        })]);
        let alpha_uri = "graph://symbol/symbol-alpha";
        let beta_uri = "graph://symbol/symbol-beta";

        let alpha = registry.query(CompletionScope::PreSession, Path::new("."), "alpha", 1);
        assert_eq!(alpha[0].uri, alpha_uri);
        assert!(registry.lookup_code_payload(alpha_uri).is_some());
        assert!(registry.lookup_code_payload(beta_uri).is_none());

        registry.retain_code_payload_on_accept(alpha_uri);
        registry.clear_cache();
        let beta = registry.query(CompletionScope::PreSession, Path::new("."), "beta", 1);
        assert_eq!(beta[0].uri, beta_uri);
        assert!(registry.lookup_code_payload(alpha_uri).is_some());
        assert!(registry.lookup_code_payload(beta_uri).is_some());

        registry.retain_code_payloads_for_uris([alpha_uri]);
        assert!(registry.lookup_code_payload(alpha_uri).is_some());
        assert!(registry.lookup_code_payload(beta_uri).is_none());

        let _ = registry.query(CompletionScope::PreSession, Path::new("."), "beta", 1);
        assert!(registry.lookup_code_payload(alpha_uri).is_none());
        assert!(registry.lookup_code_payload(beta_uri).is_some());
    }

    #[test]
    #[ignore = "manual large-symbol performance benchmark"]
    fn benchmark_large_compact_code_query_core() {
        const SYMBOL_COUNT: usize = 77_000;
        const SAMPLE_COUNT: usize = 7;

        let candidates = (0..SYMBOL_COUNT)
            .map(|index| {
                compact_candidate(&format!("symbol-{index:05}"), &format!("symbol_{index:05}"))
            })
            .collect();
        let mut registry = test_registry(vec![Box::new(CompactCodeSource {
            candidates: Arc::new(candidates),
        })]);
        let mut samples = Vec::with_capacity(SAMPLE_COUNT);

        for _ in 0..SAMPLE_COUNT {
            let started = Instant::now();
            let rows = registry.query(CompletionScope::PreSession, Path::new("."), "symbol", 20);
            samples.push(started.elapsed());
            assert_eq!(rows.len(), 20);
        }

        samples.sort_unstable();
        eprintln!(
            "compact_code_query symbols={SYMBOL_COUNT} result_limit=20 median_us={}",
            samples[SAMPLE_COUNT / 2].as_micros()
        );
    }

    struct CountingSource {
        name: &'static str,
        entries: Vec<MentionEntry>,
        build_count: Arc<AtomicUsize>,
    }

    impl MentionSource for CountingSource {
        fn build(&mut self, _cwd: &Path) -> anyhow::Result<Vec<MentionEntry>> {
            self.build_count.fetch_add(1, Ordering::Relaxed);
            Ok(self.entries.clone())
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    fn mention(kind: MentionKind, id: usize, display: String) -> MentionEntry {
        let code_path = (kind == MentionKind::CodeFile).then(|| display.clone());
        MentionEntry {
            section_header: None,
            kind,
            uri: format!("test://{id}"),
            display,
            secondary: None,
            agent: None,
            model: None,
            effort: None,
            worker_kind: None,
            worker_cli_identity: None,
            code_path,
            code_scope: None,
            tag: None,
            search_text: None,
            atom_text: None,
            unconsumed_suffix: None,
            issue_preview: None,
        }
    }

    fn test_registry(sources: Vec<Box<dyn MentionSource>>) -> MentionRegistry {
        MentionRegistry::with_source_slots(
            sources.into_iter().map(MentionSourceSlot::new).collect(),
        )
    }

    fn test_session_registry(sources: Vec<Box<dyn SessionMentionSource>>) -> MentionRegistry {
        MentionRegistry::with_source_slots(
            sources
                .into_iter()
                .map(MentionSourceSlot::session)
                .collect(),
        )
    }

    fn test_slots(slots: Vec<MentionSourceSlot>) -> MentionRegistry {
        MentionRegistry::with_source_slots(slots)
    }

    #[test]
    #[ignore = "manual large-symbol performance benchmark"]
    fn benchmark_large_symbol_query_core() {
        const SYMBOL_COUNT: usize = 77_000;
        const SAMPLE_COUNT: usize = 7;

        let entries = (0..SYMBOL_COUNT)
            .map(|index| {
                mention(
                    MentionKind::CodeSymbol,
                    index,
                    format!("crate::module_{:03}::symbol_{index}", index % 241),
                )
            })
            .collect();
        let mut registry = test_registry(vec![Box::new(StaticSource {
            name: "large_symbol_benchmark",
            entries,
        })]);

        // Build the source cache outside the measured region. The benchmark
        // represents steady-state typing after the code-graph snapshot loads.
        let _ = registry.query(
            CompletionScope::PreSession,
            Path::new("."),
            "symbol_42424",
            20,
        );

        for query in ["symbol_42424", "symbol"] {
            for simulate_legacy_clone in [true, false] {
                let mut samples = Vec::with_capacity(SAMPLE_COUNT);
                for _ in 0..SAMPLE_COUNT {
                    let started = std::time::Instant::now();
                    let work = registry.prepare_query_work(
                        CompletionScope::PreSession,
                        Path::new("."),
                        query,
                        20,
                    );
                    // Before immutable source snapshots, every non-empty query
                    // cloned every entry solely to inspect worker composition.
                    let legacy_clone = simulate_legacy_clone.then(|| {
                        work.sources
                            .iter()
                            .flat_map(|entries| entries.iter().cloned())
                            .collect::<Vec<_>>()
                    });
                    let rows = registry.run_query_work(work).entries;
                    std::hint::black_box(legacy_clone);
                    samples.push(started.elapsed());
                    assert!(!rows.is_empty());
                }
                samples.sort_unstable();
                let mode = if simulate_legacy_clone {
                    "legacy_full_clone"
                } else {
                    "immutable_snapshot"
                };
                eprintln!(
                    "large_symbol_query_core symbols={SYMBOL_COUNT} query={query:?} mode={mode} median_us={}",
                    samples[SAMPLE_COUNT / 2].as_micros()
                );
            }
        }
    }

    struct ProcessEnvRestore {
        previous_cwd: PathBuf,
        previous_env: Option<std::ffi::OsString>,
    }

    impl ProcessEnvRestore {
        fn enter(cwd: &Path) -> Self {
            let restore = Self {
                previous_cwd: std::env::current_dir().unwrap(),
                previous_env: std::env::var_os(CODE_GRAPH_INDEX_ENV),
            };
            std::env::remove_var(CODE_GRAPH_INDEX_ENV);
            std::env::set_current_dir(cwd).unwrap();
            restore
        }

        fn with_env_override(self, path: &Path) -> Self {
            std::env::set_var(CODE_GRAPH_INDEX_ENV, path);
            self
        }
    }

    impl Drop for ProcessEnvRestore {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous_cwd).unwrap();
            match &self.previous_env {
                Some(value) => std::env::set_var(CODE_GRAPH_INDEX_ENV, value),
                None => std::env::remove_var(CODE_GRAPH_INDEX_ENV),
            }
        }
    }

    fn write_pointer(
        root: &Path,
        canonical_artifact_path: &Path,
        graph_content_hash: &str,
        manifest_version: &str,
        indexed_commit_oid: Option<&str>,
    ) {
        let pointer = GraphIndexPointer {
            schema: "spur-graph-pointer-v1".to_string(),
            graph_content_hash: graph_content_hash.to_string(),
            manifest_version: manifest_version.to_string(),
            source_kind: SourceKind::Git,
            indexed_commit_oid: indexed_commit_oid.map(str::to_string),
            canonical_artifact_path: canonical_artifact_path.to_path_buf(),
        };
        let path = root.join(".spur/graph-index.pointer.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_string_pretty(&pointer).unwrap()).unwrap();
    }

    fn write_graph_fixture(path: &Path, stable_file_id: &str, file_path: &str, content_hash: &str) {
        write_artifact_parquet(
            &graph_artifact(stable_file_id, file_path, content_hash),
            path,
            WriteOptions::default(),
            Vec::new(),
        )
        .unwrap();
    }

    fn graph_artifact(
        stable_file_id: &str,
        file_path: &str,
        content_hash: &str,
    ) -> GraphIndexArtifact {
        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: TEST_GRAPH_INDEX_VERSION.to_string(),
                content_hash_blake3: Some(content_hash.to_string()),
            },
            manifest_version: "registry-test-manifest".to_string(),
            graph_content_hash: content_hash.to_string(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: stable_file_id.to_string(),
                path: file_path.to_string(),
                content_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                node_ids: Vec::new(),
            }],
            files: vec![GraphFileArtifact {
                stable_file_id: stable_file_id.to_string(),
                file_path: file_path.to_string(),
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
        }
    }

    #[test]
    fn empty_query_caps_each_kind() {
        // Limitation: this test validates section caps on large candidate sets,
        // but does not instrument exact clone counts.
        let workers = (0..100)
            .map(|i| WorkerMentionDescriptor {
                name: format!("worker-{i}"),
                kind: AgentKind::Generic,
                cli_identity: format!("worker-{i}"),
                description: None,
                tier: None,
            })
            .collect::<Vec<_>>();
        let files = (0..120)
            .map(|i| mention(MentionKind::File, i, format!("src/path-{i}.rs")))
            .collect::<Vec<_>>();
        let issues = (0..80)
            .map(|i| issue(&format!("bd-{i}"), &format!("Issue {i}"), None))
            .collect::<Vec<_>>();
        let code_files = (0..60)
            .map(|i| mention(MentionKind::CodeFile, 100 + i, format!("src/code-{i}.rs")))
            .collect::<Vec<_>>();
        let code_symbols = (0..60)
            .map(|i| mention(MentionKind::CodeSymbol, 200 + i, format!("symbol_{i}")))
            .collect::<Vec<_>>();

        let mut registry = test_slots(vec![
            MentionSourceSlot::session(Box::new(WorkerMentionSource::new(workers))),
            MentionSourceSlot::new(Box::new(StaticSource {
                name: "file",
                entries: files,
            })),
            MentionSourceSlot::session(Box::new(IssueMentionSource::new(issues))),
            MentionSourceSlot::new(Box::new(StaticSource {
                name: "code",
                entries: code_files.into_iter().chain(code_symbols).collect(),
            })),
        ]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "", 128);
        let content: Vec<&MentionEntry> = results
            .iter()
            .filter(|entry| entry.section_header.is_none())
            .collect();

        assert_eq!(content.len(), 16);
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::Worker)
                .count(),
            WORKER_PIN_CAP
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::File)
                .count(),
            FILE_CAP
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::Issue)
                .count(),
            ISSUE_CAP
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| matches!(e.kind, MentionKind::CodeFile | MentionKind::CodeSymbol))
                .count(),
            CODE_CAP
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::Worker)
                .count(),
            4
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::File)
                .count(),
            6
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| e.kind == MentionKind::Issue)
                .count(),
            3
        );
        assert_eq!(
            content
                .iter()
                .filter(|e| matches!(e.kind, MentionKind::CodeFile | MentionKind::CodeSymbol))
                .count(),
            3
        );
        assert!(content[..4].iter().all(|e| e.kind == MentionKind::Worker));
        assert!(content[4..10].iter().all(|e| e.kind == MentionKind::File));
        assert!(content[10..13].iter().all(|e| e.kind == MentionKind::Issue));
        assert!(content[13..16]
            .iter()
            .all(|e| matches!(e.kind, MentionKind::CodeFile | MentionKind::CodeSymbol)));
    }

    #[test]
    fn empty_query_emits_section_headers_in_order() {
        let mut registry = test_slots(vec![
            MentionSourceSlot::session(Box::new(WorkerMentionSource::new(vec![
                WorkerMentionDescriptor {
                    name: "alpha".into(),
                    kind: AgentKind::Generic,
                    cli_identity: "alpha".into(),
                    description: None,
                    tier: None,
                },
            ]))),
            MentionSourceSlot::new(Box::new(StaticSource {
                name: "file",
                entries: vec![mention(MentionKind::File, 1, "src/main.rs".into())],
            })),
            MentionSourceSlot::session(Box::new(IssueMentionSource::new(vec![issue(
                "bd-1", "Issue", None,
            )]))),
            MentionSourceSlot::new(Box::new(StaticSource {
                name: "code",
                entries: vec![mention(MentionKind::CodeFile, 2, "src/lib.rs".into())],
            })),
        ]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "", 128);
        let headers = results
            .iter()
            .filter_map(|entry| entry.section_header)
            .collect::<Vec<_>>();
        assert_eq!(headers, vec!["Workers", "Files", "Issues", "Code"]);

        let mut workers_only =
            test_session_registry(vec![Box::new(WorkerMentionSource::new(vec![
                WorkerMentionDescriptor {
                    name: "alpha".into(),
                    kind: AgentKind::Generic,
                    cli_identity: "alpha".into(),
                    description: None,
                    tier: None,
                },
            ]))]);
        let workers_only_results =
            workers_only.query(CompletionScope::PreSession, Path::new("."), "", 128);
        let workers_only_headers = workers_only_results
            .iter()
            .filter_map(|entry| entry.section_header)
            .collect::<Vec<_>>();
        assert_eq!(workers_only_headers, vec!["Workers"]);
    }

    #[test]
    fn typed_query_prefers_code_file_when_file_has_same_path() {
        let mut registry = test_registry(vec![
            Box::new(StaticSource {
                name: "file",
                entries: vec![
                    mention(MentionKind::File, 1, "src/shared.rs".into()),
                    mention(MentionKind::File, 2, "src/file_only.rs".into()),
                ],
            }),
            Box::new(StaticSource {
                name: "code",
                entries: vec![
                    mention(MentionKind::CodeFile, 3, "src/code_only.rs".into()),
                    mention(MentionKind::CodeFile, 4, "src/shared.rs".into()),
                ],
            }),
        ]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "src", 16);
        let content: Vec<&MentionEntry> = results
            .iter()
            .filter(|entry| entry.section_header.is_none())
            .collect();

        assert_eq!(
            content
                .iter()
                .filter(|entry| entry.display == "src/shared.rs")
                .count(),
            1,
            "expected same-path File/CodeFile rows to collapse into one result: {content:?}"
        );
        assert!(
            content.iter().any(
                |entry| entry.kind == MentionKind::CodeFile && entry.display == "src/shared.rs"
            ),
            "expected CodeFile to survive for duplicate path: {content:?}"
        );
        assert!(
            content
                .iter()
                .any(|entry| entry.kind == MentionKind::File
                    && entry.display == "src/file_only.rs"),
            "expected File-only path to survive: {content:?}"
        );
        assert!(
            content
                .iter()
                .any(|entry| entry.kind == MentionKind::CodeFile
                    && entry.display == "src/code_only.rs"),
            "expected CodeFile-only path to survive: {content:?}"
        );
    }

    #[test]
    fn empty_query_file_header_disappears_when_files_are_code_duplicates() {
        let mut registry = test_registry(vec![
            Box::new(StaticSource {
                name: "file",
                entries: vec![mention(MentionKind::File, 1, "src/shared.rs".into())],
            }),
            Box::new(StaticSource {
                name: "code",
                entries: vec![mention(MentionKind::CodeFile, 2, "src/shared.rs".into())],
            }),
        ]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "", 16);
        let headers = results
            .iter()
            .filter_map(|entry| entry.section_header)
            .collect::<Vec<_>>();
        let content = results
            .iter()
            .filter(|entry| entry.section_header.is_none())
            .collect::<Vec<_>>();

        assert_eq!(headers, vec!["Code"]);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].kind, MentionKind::CodeFile);
        assert_eq!(content[0].display, "src/shared.rs");
    }

    #[test]
    fn typed_query_tier_breaks_within_window() {
        let mut registry = test_registry(vec![Box::new(StaticSource {
            name: "mixed",
            entries: vec![
                mention(MentionKind::Worker, 1, "abcd".into()),
                mention(MentionKind::File, 2, "abce".into()),
            ],
        })]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "abc", 10);
        assert_eq!(results.len(), 2);
        // For `abc` vs `abcd` / `abce`, both rows score in the same bucket;
        // tier rank is the deciding key, so File beats Worker.
        assert_eq!(results[0].kind, MentionKind::File);
    }

    #[test]
    fn typed_query_higher_bucket_beats_lower_bucket_even_if_tier_is_worse() {
        let mut registry = test_registry(vec![Box::new(StaticSource {
            name: "mixed",
            entries: vec![
                mention(MentionKind::File, 1, "needle-notes.md".into()),
                mention(MentionKind::CodeSymbol, 2, "needle".into()),
            ],
        })]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "needle", 10);
        assert_eq!(results.len(), 2);
        // Invariant under test: score bucket is the primary key. The exact
        // code-symbol hit is in a higher bucket than the weak file prefix hit.
        assert_eq!(results[0].kind, MentionKind::CodeSymbol);
    }

    #[test]
    fn code_symbol_path_ranking_uses_structured_code_path_not_secondary() {
        let mut symbol = mention(MentionKind::CodeSymbol, 1, "render_dashboard".into());
        symbol.secondary = Some("not a parseable location".into());
        symbol.code_path = Some("crates/spur-tui/src/dashboard_view.rs".into());

        let mut registry = test_registry(vec![Box::new(StaticSource {
            name: "code",
            entries: vec![symbol],
        })]);

        let results = registry.query(
            CompletionScope::PreSession,
            Path::new("."),
            "dashboard_view",
            10,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].display, "render_dashboard");
    }

    #[test]
    fn exact_bare_code_symbol_ranks_above_exact_scoped_symbol() {
        let mut scoped = mention(MentionKind::CodeSymbol, 1, "Cache".into());
        scoped.uri = "test://a-scoped-cache".into();
        scoped.code_scope = Some("CacheStore".into());

        let mut bare = mention(MentionKind::CodeSymbol, 2, "Cache".into());
        bare.uri = "test://z-bare-cache".into();

        let mut registry = test_registry(vec![Box::new(StaticSource {
            name: "code",
            entries: vec![scoped, bare],
        })]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "cache", 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].uri, "test://z-bare-cache");
        assert_eq!(results[1].uri, "test://a-scoped-cache");
    }

    #[test]
    fn compact_code_symbol_uses_canonical_uri_tie_key() {
        let mut file = mention(MentionKind::CodeFile, 1, "needle".into());
        file.uri = "graph://file/file-needle".into();
        file.code_path = Some("needle".into());
        let mut registry = test_registry(vec![
            Box::new(StaticSource {
                name: "code",
                entries: vec![file],
            }),
            Box::new(CompactCodeSource {
                candidates: Arc::new(vec![compact_candidate("e057beef", "needle")]),
            }),
        ]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "needle", 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].kind, MentionKind::CodeFile);
        assert_eq!(results[1].kind, MentionKind::CodeSymbol);
    }

    #[test]
    fn empty_query_keeps_most_recent_issues_first() {
        let mut registry = test_session_registry(vec![Box::new(IssueMentionSource::new(vec![
            issue("bd-5", "Issue 5", None),
            issue("bd-4", "Issue 4", None),
            issue("bd-3", "Issue 3", None),
            issue("bd-2", "Issue 2", None),
            issue("bd-1", "Issue 1", None),
        ]))]);

        let results = registry.query(CompletionScope::PreSession, Path::new("."), "", 128);
        let issues: Vec<&MentionEntry> = results
            .iter()
            .filter(|entry| entry.kind == MentionKind::Issue)
            .collect();

        assert_eq!(issues.len(), 3);
        assert_eq!(issue_id(issues[0]), "bd-5");
        assert_eq!(issue_id(issues[1]), "bd-4");
        assert_eq!(issue_id(issues[2]), "bd-3");
    }

    #[test]
    fn comparator_is_strict_weak_ordering() {
        let entries = [
            mention(MentionKind::Worker, 1, "worker-a".into()),
            mention(MentionKind::CodeSymbol, 2, "code-b".into()),
            mention(MentionKind::File, 3, "file-c".into()),
            mention(MentionKind::Issue, 4, "issue-d".into()),
            mention(MentionKind::File, 5, "file-e".into()),
            mention(MentionKind::Worker, 6, "worker-f".into()),
        ];
        let base = vec![
            RankedMentionRef {
                rank: 95,
                tier: 1,
                value: RankedMentionValue::Entry(&entries[0]),
            },
            RankedMentionRef {
                rank: 100,
                tier: 4,
                value: RankedMentionValue::Entry(&entries[1]),
            },
            RankedMentionRef {
                rank: 86,
                tier: 0,
                value: RankedMentionValue::Entry(&entries[2]),
            },
            RankedMentionRef {
                rank: 88,
                tier: 2,
                value: RankedMentionValue::Entry(&entries[3]),
            },
            RankedMentionRef {
                rank: 45,
                tier: 0,
                value: RankedMentionValue::Entry(&entries[4]),
            },
            RankedMentionRef {
                rank: 0,
                tier: 1,
                value: RankedMentionValue::Entry(&entries[5]),
            },
        ];
        let max_rank = base.iter().map(|mention| mention.rank).max().unwrap_or(0);

        for a in &base {
            assert_eq!(typed_query_cmp(a, a, max_rank), std::cmp::Ordering::Equal);
        }
        for a in &base {
            for b in &base {
                let ab = typed_query_cmp(a, b, max_rank);
                let ba = typed_query_cmp(b, a, max_rank);
                assert_eq!(ab, ba.reverse());
            }
        }
        for a in &base {
            for b in &base {
                for c in &base {
                    if typed_query_cmp(a, b, max_rank) == std::cmp::Ordering::Less
                        && typed_query_cmp(b, c, max_rank) == std::cmp::Ordering::Less
                    {
                        assert_eq!(typed_query_cmp(a, c, max_rank), std::cmp::Ordering::Less);
                    }
                }
            }
        }

        let mut expected = base.clone();
        expected.sort_by(|a, b| typed_query_cmp(a, b, max_rank));
        let expected_ids: Vec<String> = expected
            .iter()
            .map(|item| item.stable_key().to_owned())
            .collect();

        for shift in 0..base.len() {
            let mut shuffled = base.clone();
            shuffled.rotate_left(shift);
            shuffled.sort_by(|a, b| typed_query_cmp(a, b, max_rank));
            let ids: Vec<String> = shuffled
                .iter()
                .map(|item| item.stable_key().to_owned())
                .collect();
            assert_eq!(ids, expected_ids);
        }
    }
}
