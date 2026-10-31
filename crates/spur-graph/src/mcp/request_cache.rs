use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use super::overlay_snapshot::SnapshotIdentity;
use crate::{GraphIndexArtifact, OverlayGeneration, ParquetClient};

const PARQUET_CLIENT_CACHE_CAPACITY: usize = 8;
const OVERLAY_DELTA_CACHE_CAPACITY: usize = PARQUET_CLIENT_CACHE_CAPACITY;

static PARQUET_CLIENT_CACHE: OnceLock<Mutex<ParquetClientCache>> = OnceLock::new();
static OVERLAY_DELTA_CACHE: OnceLock<Mutex<OverlayDeltaCache>> = OnceLock::new();
static OVERLAY_DELTA_IN_FLIGHT: OnceLock<
    Mutex<HashMap<OverlayDeltaCacheKey, Arc<OverlayInFlight>>>,
> = OnceLock::new();
static OVERLAY_GENERATION_CACHE: OnceLock<Mutex<OverlayGenerationCache>> = OnceLock::new();
static OVERLAY_GENERATION_IN_FLIGHT: OnceLock<
    Mutex<HashMap<SnapshotIdentity, Arc<OverlayGenerationInFlight>>>,
> = OnceLock::new();

pub(super) fn parquet_client(path: &Path) -> anyhow::Result<Arc<ParquetClient>> {
    for _attempt in 0..=1 {
        let key = ParquetClientCacheKey::from_path(path)?;
        if let Some(client) = parquet_cache_get(&key) {
            if ParquetClientCacheKey::from_path(path)? == key {
                return Ok(client);
            }
            continue;
        }
        let client = Arc::new(ParquetClient::open(path)?);
        if ParquetClientCacheKey::from_path(path)? == key {
            parquet_cache_insert(key, Arc::clone(&client));
            return Ok(client);
        }
    }
    anyhow::bail!(
        "Parquet manifest identity for `{}` changed during two open attempts",
        path.display()
    )
}

#[derive(Clone)]
pub(super) struct CachedOverlayDelta {
    pub artifact: Arc<GraphIndexArtifact>,
    pub shadowed: HashSet<String>,
}

pub(super) fn overlay_delta(
    identity: SnapshotIdentity,
    build: impl FnOnce() -> anyhow::Result<CachedOverlayDelta>,
) -> anyhow::Result<CachedOverlayDelta> {
    let cache_key = OverlayDeltaCacheKey { identity };
    if let Some(cached) = overlay_delta_cache_get(&cache_key) {
        return Ok(cached);
    }

    let (cell, leader) = overlay_in_flight_cell(cache_key.clone())?;
    if !leader {
        return wait_for_overlay_in_flight(&cell);
    }

    let result = build().map_err(SharedOverlayError::new);
    if let Ok(built) = &result {
        overlay_delta_cache_insert(cache_key.clone(), built.clone());
    }
    finish_overlay_in_flight(&cache_key, &cell, &result);
    into_anyhow_overlay_result(result)
}

#[derive(Clone)]
pub(super) struct CachedOverlayGeneration {
    identity: SnapshotIdentity,
    pub generation: Arc<OverlayGeneration>,
}

pub(super) fn overlay_generation(
    identity: SnapshotIdentity,
    build: impl FnOnce(Option<Arc<OverlayGeneration>>) -> anyhow::Result<Arc<OverlayGeneration>>,
) -> anyhow::Result<Arc<OverlayGeneration>> {
    if let Some(generation) = overlay_generation_cache_get(&identity) {
        return Ok(generation);
    }

    let mut build = Some(build);
    loop {
        if let Some(generation) = overlay_generation_cache_get(&identity) {
            return Ok(generation);
        }

        let (cell, leader) = overlay_generation_in_flight_cell(identity.clone());
        if !leader {
            match wait_for_overlay_generation(&cell) {
                OverlayGenerationInFlightOutcome::Published(generation) => {
                    return Ok(generation);
                }
                OverlayGenerationInFlightOutcome::Retry => continue,
            }
        }

        let mut leader_guard = OverlayGenerationLeaderGuard::new(identity.clone(), cell);
        if let Some(generation) = overlay_generation_cache_get(&identity) {
            leader_guard.finish(OverlayGenerationInFlightOutcome::Published(Arc::clone(
                &generation,
            )));
            return Ok(generation);
        }

        let seed = overlay_generation_compatible_latest(&identity);
        let result = build.take().expect("generation builder consumed once")(seed);
        match result {
            Ok(generation) => {
                overlay_generation_cache_insert(CachedOverlayGeneration {
                    identity,
                    generation: Arc::clone(&generation),
                });
                leader_guard.finish(OverlayGenerationInFlightOutcome::Published(Arc::clone(
                    &generation,
                )));
                return Ok(generation);
            }
            Err(error) => {
                leader_guard.finish(OverlayGenerationInFlightOutcome::Retry);
                return Err(error);
            }
        }
    }
}

#[derive(Clone)]
enum OverlayGenerationInFlightOutcome {
    Published(Arc<OverlayGeneration>),
    Retry,
}

struct OverlayGenerationInFlight {
    done: Mutex<Option<OverlayGenerationInFlightOutcome>>,
    cv: Condvar,
}

struct OverlayGenerationLeaderGuard {
    identity: SnapshotIdentity,
    cell: Arc<OverlayGenerationInFlight>,
    finished: bool,
}

impl OverlayGenerationLeaderGuard {
    fn new(identity: SnapshotIdentity, cell: Arc<OverlayGenerationInFlight>) -> Self {
        Self {
            identity,
            cell,
            finished: false,
        }
    }

    fn finish(&mut self, outcome: OverlayGenerationInFlightOutcome) {
        finish_overlay_generation_in_flight(&self.identity, &self.cell, outcome);
        self.finished = true;
    }
}

impl Drop for OverlayGenerationLeaderGuard {
    fn drop(&mut self) {
        if !self.finished {
            finish_overlay_generation_in_flight(
                &self.identity,
                &self.cell,
                OverlayGenerationInFlightOutcome::Retry,
            );
        }
    }
}

fn overlay_generation_in_flight_map(
) -> &'static Mutex<HashMap<SnapshotIdentity, Arc<OverlayGenerationInFlight>>> {
    OVERLAY_GENERATION_IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn overlay_generation_in_flight_cell(
    identity: SnapshotIdentity,
) -> (Arc<OverlayGenerationInFlight>, bool) {
    let mut map = overlay_generation_in_flight_map()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cell) = map.get(&identity) {
        return (Arc::clone(cell), false);
    }
    let cell = Arc::new(OverlayGenerationInFlight {
        done: Mutex::new(None),
        cv: Condvar::new(),
    });
    map.insert(identity, Arc::clone(&cell));
    (cell, true)
}

fn wait_for_overlay_generation(
    cell: &OverlayGenerationInFlight,
) -> OverlayGenerationInFlightOutcome {
    let mut done = cell
        .done
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if let Some(outcome) = done.as_ref() {
            return outcome.clone();
        }
        done = cell
            .cv
            .wait(done)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

fn finish_overlay_generation_in_flight(
    identity: &SnapshotIdentity,
    cell: &OverlayGenerationInFlight,
    outcome: OverlayGenerationInFlightOutcome,
) {
    {
        let mut map = overlay_generation_in_flight_map()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if map
            .get(identity)
            .is_some_and(|candidate| std::ptr::eq(candidate.as_ref(), cell))
        {
            map.remove(identity);
        }
    }
    let mut done = cell
        .done
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *done = Some(outcome);
    cell.cv.notify_all();
}

type SharedOverlayResult = Result<CachedOverlayDelta, SharedOverlayError>;

#[derive(Clone)]
struct SharedOverlayError(Arc<anyhow::Error>);

impl SharedOverlayError {
    fn new(error: anyhow::Error) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Debug for SharedOverlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for SharedOverlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for SharedOverlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

fn into_anyhow_overlay_result(result: SharedOverlayResult) -> anyhow::Result<CachedOverlayDelta> {
    result.map_err(anyhow::Error::new)
}

struct OverlayInFlight {
    done: Mutex<Option<SharedOverlayResult>>,
    cv: Condvar,
}

fn overlay_in_flight_map() -> &'static Mutex<HashMap<OverlayDeltaCacheKey, Arc<OverlayInFlight>>> {
    OVERLAY_DELTA_IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn overlay_in_flight_cell(
    cache_key: OverlayDeltaCacheKey,
) -> anyhow::Result<(Arc<OverlayInFlight>, bool)> {
    let mut map = overlay_in_flight_map()
        .lock()
        .map_err(|_| anyhow::anyhow!("overlay in-flight map lock poisoned"))?;
    if let Some(cell) = map.get(&cache_key) {
        return Ok((Arc::clone(cell), false));
    }
    let cell = Arc::new(OverlayInFlight {
        done: Mutex::new(None),
        cv: Condvar::new(),
    });
    map.insert(cache_key, Arc::clone(&cell));
    Ok((cell, true))
}

fn wait_for_overlay_in_flight(cell: &OverlayInFlight) -> anyhow::Result<CachedOverlayDelta> {
    let mut done = cell
        .done
        .lock()
        .map_err(|_| anyhow::anyhow!("overlay in-flight result lock poisoned"))?;
    while done.is_none() {
        done = cell
            .cv
            .wait(done)
            .map_err(|_| anyhow::anyhow!("overlay in-flight wait poisoned"))?;
    }
    match done.as_ref() {
        Some(result) => into_anyhow_overlay_result(result.clone()),
        None => Err(anyhow::anyhow!(
            "overlay in-flight result missing after wait"
        )),
    }
}

fn finish_overlay_in_flight(
    cache_key: &OverlayDeltaCacheKey,
    cell: &OverlayInFlight,
    result: &SharedOverlayResult,
) {
    if let Ok(mut done) = cell.done.lock() {
        *done = Some(result.clone());
        cell.cv.notify_all();
    }
    if let Ok(mut map) = overlay_in_flight_map().lock() {
        map.remove(cache_key);
    }
}

fn parquet_cache() -> &'static Mutex<ParquetClientCache> {
    PARQUET_CLIENT_CACHE
        .get_or_init(|| Mutex::new(ParquetClientCache::new(PARQUET_CLIENT_CACHE_CAPACITY)))
}

fn overlay_delta_cache() -> &'static Mutex<OverlayDeltaCache> {
    OVERLAY_DELTA_CACHE
        .get_or_init(|| Mutex::new(OverlayDeltaCache::new(OVERLAY_DELTA_CACHE_CAPACITY)))
}

fn overlay_generation_cache() -> &'static Mutex<OverlayGenerationCache> {
    OVERLAY_GENERATION_CACHE
        .get_or_init(|| Mutex::new(OverlayGenerationCache::new(OVERLAY_DELTA_CACHE_CAPACITY)))
}

fn parquet_cache_get(key: &ParquetClientCacheKey) -> Option<Arc<ParquetClient>> {
    let Ok(mut cache) = parquet_cache().lock() else {
        return None;
    };
    cache.get(key)
}

fn parquet_cache_insert(key: ParquetClientCacheKey, client: Arc<ParquetClient>) {
    let Ok(mut cache) = parquet_cache().lock() else {
        return;
    };
    cache.insert(key, client);
}

fn overlay_delta_cache_get(key: &OverlayDeltaCacheKey) -> Option<CachedOverlayDelta> {
    let Ok(mut cache) = overlay_delta_cache().lock() else {
        return None;
    };
    cache.get(key)
}

fn overlay_delta_cache_insert(key: OverlayDeltaCacheKey, value: CachedOverlayDelta) {
    let Ok(mut cache) = overlay_delta_cache().lock() else {
        return;
    };
    cache.insert(key, value);
}

fn overlay_generation_cache_get(identity: &SnapshotIdentity) -> Option<Arc<OverlayGeneration>> {
    let mut cache = overlay_generation_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.get(identity)
}

fn overlay_generation_compatible_latest(
    identity: &SnapshotIdentity,
) -> Option<Arc<OverlayGeneration>> {
    let cache = overlay_generation_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.compatible_latest(identity)
}

fn overlay_generation_cache_insert(value: CachedOverlayGeneration) {
    let mut cache = overlay_generation_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(value);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParquetClientCacheKey {
    path: PathBuf,
    manifest_mtime_ns: i128,
    manifest_digest: [u8; 32],
}

impl ParquetClientCacheKey {
    fn from_path(path: &Path) -> anyhow::Result<Self> {
        let path = path.canonicalize().map_err(|error| {
            anyhow::anyhow!(
                "failed to canonicalize Parquet artifact `{}`: {error}",
                path.display()
            )
        })?;
        let manifest = path.join("manifest.json");
        let manifest_bytes = fs::read(&manifest).map_err(|error| {
            anyhow::anyhow!(
                "failed to read Parquet manifest identity `{}`: {error}",
                manifest.display()
            )
        })?;
        let manifest_mtime_ns = path_mtime_ns(&manifest);
        Ok(Self {
            path,
            manifest_mtime_ns,
            manifest_digest: *blake3::hash(&manifest_bytes).as_bytes(),
        })
    }
}

struct ParquetClientCache {
    entries: HashMap<ParquetClientCacheKey, Arc<ParquetClient>>,
    lru: VecDeque<ParquetClientCacheKey>,
    capacity: usize,
}

impl ParquetClientCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            capacity,
        }
    }

    fn get(&mut self, key: &ParquetClientCacheKey) -> Option<Arc<ParquetClient>> {
        let client = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(client)
    }

    fn insert(&mut self, key: ParquetClientCacheKey, client: Arc<ParquetClient>) {
        self.entries.insert(key.clone(), client);
        self.touch(&key);
        while self.entries.len() > self.capacity {
            let Some(expired) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
    }

    fn touch(&mut self, key: &ParquetClientCacheKey) {
        self.lru.retain(|candidate| candidate != key);
        self.lru.push_back(key.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OverlayDeltaCacheKey {
    identity: SnapshotIdentity,
}

struct OverlayDeltaCache {
    entries: HashMap<OverlayDeltaCacheKey, CachedOverlayDelta>,
    lru: VecDeque<OverlayDeltaCacheKey>,
    capacity: usize,
}

impl OverlayDeltaCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            capacity,
        }
    }

    fn get(&mut self, key: &OverlayDeltaCacheKey) -> Option<CachedOverlayDelta> {
        let value = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    fn insert(&mut self, key: OverlayDeltaCacheKey, value: CachedOverlayDelta) {
        self.entries.insert(key.clone(), value);
        self.touch(&key);
        while self.entries.len() > self.capacity {
            let Some(expired) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
    }

    fn touch(&mut self, key: &OverlayDeltaCacheKey) {
        self.lru.retain(|candidate| candidate != key);
        self.lru.push_back(key.clone());
    }
}

struct OverlayGenerationCache {
    entries: HashMap<SnapshotIdentity, CachedOverlayGeneration>,
    lru: VecDeque<SnapshotIdentity>,
    compatible: HashMap<OverlayGenerationCompatibilityKey, VecDeque<SnapshotIdentity>>,
    capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OverlayGenerationCompatibilityKey {
    canonical_worktree: PathBuf,
    indexed_graph_content_hash: String,
}

impl From<&SnapshotIdentity> for OverlayGenerationCompatibilityKey {
    fn from(identity: &SnapshotIdentity) -> Self {
        Self {
            canonical_worktree: identity.canonical_worktree.clone(),
            indexed_graph_content_hash: identity.indexed_graph_content_hash.clone(),
        }
    }
}

impl OverlayGenerationCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            compatible: HashMap::new(),
            capacity,
        }
    }

    fn get(&mut self, identity: &SnapshotIdentity) -> Option<Arc<OverlayGeneration>> {
        let generation = Arc::clone(&self.entries.get(identity)?.generation);
        self.touch(identity);
        Some(generation)
    }

    fn compatible_latest(&self, identity: &SnapshotIdentity) -> Option<Arc<OverlayGeneration>> {
        let key = OverlayGenerationCompatibilityKey::from(identity);
        let latest_identity = self.compatible.get(&key)?.back()?;
        self.entries
            .get(latest_identity)
            .map(|latest| Arc::clone(&latest.generation))
    }

    fn insert(&mut self, value: CachedOverlayGeneration) {
        let identity = value.identity.clone();
        if self.entries.insert(identity.clone(), value).is_some() {
            self.remove_compatible_identity(&identity);
        }
        self.compatible
            .entry(OverlayGenerationCompatibilityKey::from(&identity))
            .or_default()
            .push_back(identity.clone());
        self.touch(&identity);
        while self.entries.len() > self.capacity {
            let Some(expired) = self.lru.pop_front() else {
                break;
            };
            if self.entries.remove(&expired).is_some() {
                self.remove_compatible_identity(&expired);
            }
        }
        debug_assert_eq!(
            self.compatible.values().map(VecDeque::len).sum::<usize>(),
            self.entries.len()
        );
        debug_assert!(self.entries.len() <= self.capacity);
    }

    fn remove_compatible_identity(&mut self, identity: &SnapshotIdentity) {
        let key = OverlayGenerationCompatibilityKey::from(identity);
        let remove_key = self.compatible.get_mut(&key).is_some_and(|identities| {
            identities.retain(|candidate| candidate != identity);
            identities.is_empty()
        });
        if remove_key {
            self.compatible.remove(&key);
        }
    }

    fn touch(&mut self, identity: &SnapshotIdentity) {
        self.lru.retain(|candidate| candidate != identity);
        self.lru.push_back(identity.clone());
    }
}

fn path_mtime_ns(path: &Path) -> i128 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i128)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::super::overlay_snapshot::{
        self, normalized_changed_set_fingerprint, OverlayPathState, SnapshotBase, SnapshotIdentity,
    };
    use super::*;
    use anyhow::Context as _;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use crate::schema::{GraphIndexHeader, GRAPH_INDEX_VERSION_TEMPORAL};
    use crate::{write_artifact_parquet, GraphIndexArtifact, WriteOptions};

    fn write_empty_parquet(dir: &Path) -> PathBuf {
        let artifact = GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test-manifest".to_owned(),
            graph_content_hash: "cache-test-hash".to_owned(),
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
        };
        write_artifact_parquet(&artifact, dir, WriteOptions::default(), Vec::new())
            .expect("write empty parquet artifact")
    }

    #[test]
    fn parquet_client_cache_reuses_open_for_same_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = write_empty_parquet(tempdir.path());
        let first = parquet_client(&dir).expect("first open");
        let second = parquet_client(&dir).expect("second open");
        assert!(
            Arc::ptr_eq(&first, &second),
            "same artifact path should reuse the cached ParquetClient"
        );
    }

    fn empty_artifact(hash: &str) -> GraphIndexArtifact {
        GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test-manifest".to_owned(),
            graph_content_hash: hash.to_owned(),
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

    fn dummy_overlay_delta(tag: &str) -> CachedOverlayDelta {
        CachedOverlayDelta {
            artifact: Arc::new(empty_artifact(tag)),
            shadowed: HashSet::from([tag.to_owned()]),
        }
    }

    fn cache_identity(worktree: &Path, tag: u8) -> SnapshotIdentity {
        overlay_snapshot::test_snapshot_identity(worktree, tag)
    }

    fn overlay_delta_for_identity_test(
        identity: &SnapshotIdentity,
        build: impl FnOnce() -> anyhow::Result<CachedOverlayDelta>,
    ) -> anyhow::Result<CachedOverlayDelta> {
        overlay_delta(identity.clone(), build)
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn snapshot_fixture(root: &Path) -> SnapshotBase {
        run_git(root, &["init", "-q"]);
        run_git(root, &["config", "user.email", "cache@example.com"]);
        run_git(root, &["config", "user.name", "Cache Test"]);
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join("src/lib.rs"), "pub fn cached() {}\n").expect("source");
        run_git(root, &["add", "src/lib.rs"]);
        run_git(root, &["commit", "-qm", "base"]);
        let file_oids = crate::git::ls_files_with_oids(root)
            .expect("tracked files")
            .into_iter()
            .map(|entry| (entry.path, entry.content_oid))
            .collect::<BTreeMap<_, _>>();
        SnapshotBase {
            indexed_graph_content_hash: "no-ttl-graph".to_owned(),
            indexed_head_oid: Some(crate::git::rev_parse_head(root).expect("HEAD")),
            file_oids,
        }
    }

    static OVERLAY_DELTA_TEST_LOCK: Mutex<()> = Mutex::new(());
    static OVERLAY_GENERATION_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn overlay_delta_test_lock() -> std::sync::MutexGuard<'static, ()> {
        OVERLAY_DELTA_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn overlay_generation_test_lock() -> std::sync::MutexGuard<'static, ()> {
        OVERLAY_GENERATION_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn dummy_overlay_generation(tag: &str) -> Arc<crate::OverlayGeneration> {
        Arc::new(
            crate::OverlayGeneration::seed(Arc::new(empty_artifact(tag)))
                .expect("seed overlay generation"),
        )
    }

    #[test]
    fn overlay_generation_cache_exact_identity_hit_reuses_generation() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 10);
        let builds = AtomicUsize::new(0);

        let first = overlay_generation(identity.clone(), |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("exact-first"))
        })
        .expect("first exact generation");
        let second = overlay_generation(identity, |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("exact-wrong-rebuild"))
        })
        .expect("cached exact generation");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn overlay_generation_cache_invalidates_every_snapshot_identity_component() {
        let _guard = overlay_generation_test_lock();
        let first_parent = tempfile::tempdir().expect("first parent");
        let second_parent = tempfile::tempdir().expect("second parent");
        let first_worktree = first_parent.path().join("same-relative-name");
        let second_worktree = second_parent.path().join("same-relative-name");
        fs::create_dir_all(&first_worktree).expect("first worktree");
        fs::create_dir_all(&second_worktree).expect("second worktree");
        let baseline = cache_identity(&first_worktree, 11);
        let other_identity = cache_identity(&second_worktree, 11);
        let other_index_identity = cache_identity(&first_worktree, 12).index_identity;
        let mut variants = Vec::new();

        let mut worktree = baseline.clone();
        worktree.canonical_worktree = other_identity.canonical_worktree;
        variants.push(("canonical-worktree", worktree));
        let mut graph = baseline.clone();
        graph.indexed_graph_content_hash = "generation-graph-changed".to_owned();
        variants.push(("graph", graph));
        let mut indexed_head = baseline.clone();
        indexed_head.indexed_head_oid = Some("generation-indexed-head-changed".to_owned());
        variants.push(("indexed-head", indexed_head));
        let mut current_head = baseline.clone();
        current_head.current_head_oid = "generation-current-head-changed".to_owned();
        variants.push(("current-head", current_head));
        let mut index = baseline.clone();
        index.index_identity = other_index_identity;
        variants.push(("index", index));
        let mut fingerprint = baseline.clone();
        fingerprint.normalized_changed_set_fingerprint = [91; 32];
        variants.push(("changed-set", fingerprint));

        let builds = AtomicUsize::new(0);
        overlay_generation(baseline, |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("baseline"))
        })
        .expect("baseline generation");
        for (label, identity) in &variants {
            let actual = overlay_generation(identity.clone(), |_| {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_generation(label))
            })
            .unwrap_or_else(|error| panic!("{label} generation failed: {error:#}"));
            assert_eq!(actual.base_artifact().graph_content_hash, *label);
        }

        assert_eq!(builds.load(Ordering::SeqCst), 1 + variants.len());
    }

    #[test]
    fn overlay_generation_cache_isolates_worktrees() {
        let _guard = overlay_generation_test_lock();
        let first = tempfile::tempdir().expect("first worktree");
        let second = tempfile::tempdir().expect("second worktree");
        let first_identity = cache_identity(first.path(), 13);
        let second_identity = cache_identity(second.path(), 13);
        let builds = AtomicUsize::new(0);

        let first_generation = overlay_generation(first_identity.clone(), |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("first-worktree"))
        })
        .expect("first worktree generation");
        let second_generation = overlay_generation(second_identity, |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("second-worktree"))
        })
        .expect("second worktree generation");
        let first_again = overlay_generation(first_identity, |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("first-worktree-rebuilt"))
        })
        .expect("first worktree exact reuse");

        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert!(Arc::ptr_eq(&first_generation, &first_again));
        assert!(!Arc::ptr_eq(&first_generation, &second_generation));
    }

    #[test]
    fn overlay_generation_singleflight_builds_identical_identity_once() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 14);
        let builds = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let spawn = |builds: Arc<AtomicUsize>,
                     barrier: Arc<std::sync::Barrier>,
                     identity: SnapshotIdentity| {
            std::thread::spawn(move || {
                barrier.wait();
                overlay_generation(identity, |_| {
                    builds.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    Ok(dummy_overlay_generation("singleflight"))
                })
            })
        };
        let first = spawn(Arc::clone(&builds), Arc::clone(&barrier), identity.clone());
        let second = spawn(Arc::clone(&builds), barrier, identity);
        let first = first.join().expect("first thread").expect("first result");
        let second = second
            .join()
            .expect("second thread")
            .expect("second result");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn overlay_generation_publication_never_exposes_compatible_partial_value() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let prior_identity = cache_identity(tempdir.path(), 15);
        let prior = overlay_generation(prior_identity.clone(), |_| {
            Ok(dummy_overlay_generation("prior-complete"))
        })
        .expect("prior generation");
        let mut next_identity = prior_identity;
        next_identity.current_head_oid = "publication-next-head".to_owned();
        let (builder_started_tx, builder_started_rx) = std::sync::mpsc::sync_channel(0);
        let (release_builder_tx, release_builder_rx) = std::sync::mpsc::sync_channel(0);
        let leader_identity = next_identity.clone();
        let prior_for_leader = Arc::clone(&prior);
        let leader = std::thread::spawn(move || {
            overlay_generation(leader_identity, |seed| {
                let seed = seed.expect("compatible prior seed");
                assert!(Arc::ptr_eq(&seed, &prior_for_leader));
                builder_started_tx.send(()).expect("signal builder start");
                release_builder_rx.recv().expect("release builder");
                Ok(dummy_overlay_generation("next-complete"))
            })
        });
        builder_started_rx.recv().expect("builder started");

        let follower_builds = Arc::new(AtomicUsize::new(0));
        let follower_builds_thread = Arc::clone(&follower_builds);
        let (follower_tx, follower_rx) = std::sync::mpsc::channel();
        let follower = std::thread::spawn(move || {
            let result = overlay_generation(next_identity, |_| {
                follower_builds_thread.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_generation("partial-wrong"))
            });
            follower_tx.send(result).expect("send follower result");
        });
        assert!(
            follower_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "an exact follower must wait instead of observing the compatible prior generation"
        );

        release_builder_tx.send(()).expect("release leader");
        let leader = leader
            .join()
            .expect("leader thread")
            .expect("leader generation");
        let follower_result = follower_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("follower completed")
            .expect("follower generation");
        follower.join().expect("follower thread");
        assert_eq!(follower_builds.load(Ordering::SeqCst), 0);
        assert!(Arc::ptr_eq(&leader, &follower_result));
        assert!(!Arc::ptr_eq(&prior, &follower_result));
    }

    #[test]
    fn overlay_generation_lru_eviction_never_reuses_wrong_identity() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identities = (0..=OVERLAY_DELTA_CACHE_CAPACITY)
            .map(|index| {
                let mut identity = cache_identity(tempdir.path(), 16);
                identity.current_head_oid = format!("generation-head-{index}");
                identity
            })
            .collect::<Vec<_>>();
        let builds = AtomicUsize::new(0);

        for (index, identity) in identities.iter().enumerate() {
            let tag = format!("generation-entry-{index}");
            let generation = overlay_generation(identity.clone(), |_| {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_generation(&tag))
            })
            .expect("insert generation");
            assert_eq!(generation.base_artifact().graph_content_hash, tag);
        }
        let rebuilt = overlay_generation(identities[0].clone(), |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("generation-entry-0-rebuilt"))
        })
        .expect("rebuild evicted generation");

        assert_eq!(OVERLAY_DELTA_CACHE_CAPACITY, PARQUET_CLIENT_CACHE_CAPACITY);
        assert_eq!(
            rebuilt.base_artifact().graph_content_hash,
            "generation-entry-0-rebuilt"
        );
        assert_eq!(builds.load(Ordering::SeqCst), identities.len() + 1);
    }

    #[test]
    fn overlay_generation_exact_hit_has_no_ttl_expiry() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 17);
        let builds = AtomicUsize::new(0);
        let first = overlay_generation(identity.clone(), |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("no-ttl-generation"))
        })
        .expect("first generation");

        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(6_001));
        eprintln!(
            "generation no-TTL retention elapsed={:?}",
            started.elapsed()
        );
        let second = overlay_generation(identity, |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_generation("expired-wrong"))
        })
        .expect("generation after old TTL window");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn overlay_generation_builder_receives_latest_compatible_seed_only() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let first_identity = cache_identity(tempdir.path(), 18);
        let first = overlay_generation(first_identity.clone(), |seed| {
            assert!(seed.is_none());
            Ok(dummy_overlay_generation("compatible-first"))
        })
        .expect("first generation");
        let mut second_identity = first_identity.clone();
        second_identity.current_head_oid = "compatible-second-head".to_owned();
        let first_for_seed = Arc::clone(&first);
        let second = overlay_generation(second_identity.clone(), |seed| {
            let seed = seed.expect("latest compatible seed");
            assert!(Arc::ptr_eq(&seed, &first_for_seed));
            Ok(dummy_overlay_generation("compatible-second"))
        })
        .expect("second generation");

        let exact = overlay_generation(second_identity, |_| {
            panic!("compatible-latest must not replace an exact cache hit")
        })
        .expect("second exact generation");
        assert!(Arc::ptr_eq(&second, &exact));
    }

    #[test]
    fn overlay_generation_builder_reuses_seed_after_interleaved_worktree() {
        let _guard = overlay_generation_test_lock();
        let first_worktree = tempfile::tempdir().expect("first worktree");
        let second_worktree = tempfile::tempdir().expect("second worktree");
        let first_identity = cache_identity(first_worktree.path(), 28);
        let first = overlay_generation(first_identity.clone(), |seed| {
            assert!(seed.is_none());
            Ok(dummy_overlay_generation("interleaved-worktree-a1"))
        })
        .expect("first worktree generation");

        let intervening_identity = cache_identity(second_worktree.path(), 28);
        let intervening = overlay_generation(intervening_identity, |seed| {
            assert!(seed.is_none());
            Ok(dummy_overlay_generation("interleaved-worktree-b1"))
        })
        .expect("intervening worktree generation");

        let mut next_identity = first_identity;
        next_identity.current_head_oid = "interleaved-worktree-a2".to_owned();
        let first_for_seed = Arc::clone(&first);
        let intervening_for_rejection = Arc::clone(&intervening);
        overlay_generation(next_identity, |seed| {
            let seed = seed.expect("retained compatible worktree seed");
            assert!(Arc::ptr_eq(&seed, &first_for_seed));
            assert!(!Arc::ptr_eq(&seed, &intervening_for_rejection));
            Ok(dummy_overlay_generation("interleaved-worktree-a2"))
        })
        .expect("next first-worktree generation");
    }

    #[test]
    fn overlay_generation_builder_reuses_seed_after_interleaved_base_graph() {
        let _guard = overlay_generation_test_lock();
        let worktree = tempfile::tempdir().expect("worktree");
        let first_identity = cache_identity(worktree.path(), 29);
        let first = overlay_generation(first_identity.clone(), |seed| {
            assert!(seed.is_none());
            Ok(dummy_overlay_generation("interleaved-graph-a1"))
        })
        .expect("first base-graph generation");

        let mut intervening_identity = first_identity.clone();
        intervening_identity.indexed_graph_content_hash =
            "interleaved-incompatible-base-graph".to_owned();
        let intervening = overlay_generation(intervening_identity, |seed| {
            assert!(seed.is_none());
            Ok(dummy_overlay_generation("interleaved-graph-b1"))
        })
        .expect("intervening base-graph generation");

        let mut next_identity = first_identity;
        next_identity.current_head_oid = "interleaved-graph-a2".to_owned();
        let first_for_seed = Arc::clone(&first);
        let intervening_for_rejection = Arc::clone(&intervening);
        overlay_generation(next_identity, |seed| {
            let seed = seed.expect("retained compatible base-graph seed");
            assert!(Arc::ptr_eq(&seed, &first_for_seed));
            assert!(!Arc::ptr_eq(&seed, &intervening_for_rejection));
            Ok(dummy_overlay_generation("interleaved-graph-a2"))
        })
        .expect("next compatible base-graph generation");
    }

    #[test]
    fn overlay_generation_compatible_seed_falls_back_after_lru_eviction() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let first_identity = cache_identity(tempdir.path(), 30);
        let mut second_identity = first_identity.clone();
        second_identity.current_head_oid = "lru-compatible-second".to_owned();
        let first = dummy_overlay_generation("lru-compatible-first");
        let second = dummy_overlay_generation("lru-compatible-second");
        let mut cache = OverlayGenerationCache::new(2);
        cache.insert(CachedOverlayGeneration {
            identity: first_identity.clone(),
            generation: Arc::clone(&first),
        });
        cache.insert(CachedOverlayGeneration {
            identity: second_identity.clone(),
            generation: Arc::clone(&second),
        });

        assert!(cache.get(&first_identity).is_some());
        let mut incompatible_identity = first_identity.clone();
        incompatible_identity.indexed_graph_content_hash = "lru-incompatible-graph".to_owned();
        cache.insert(CachedOverlayGeneration {
            identity: incompatible_identity,
            generation: dummy_overlay_generation("lru-incompatible"),
        });
        assert!(cache.get(&second_identity).is_none());

        let mut next_identity = first_identity;
        next_identity.current_head_oid = "lru-compatible-third".to_owned();
        let seed = cache
            .compatible_latest(&next_identity)
            .expect("older retained compatible seed after latest eviction");
        assert!(Arc::ptr_eq(&seed, &first));
        assert!(!Arc::ptr_eq(&seed, &second));
    }

    #[test]
    fn overlay_generation_builder_rejects_incompatible_seed() {
        let _guard = overlay_generation_test_lock();
        let first = tempfile::tempdir().expect("first worktree");
        let second = tempfile::tempdir().expect("second worktree");
        let baseline_identity = cache_identity(first.path(), 19);
        overlay_generation(baseline_identity.clone(), |_| {
            Ok(dummy_overlay_generation("incompatible-baseline"))
        })
        .expect("baseline generation");

        let mut other_graph = baseline_identity;
        other_graph.indexed_graph_content_hash = "different-base-graph".to_owned();
        overlay_generation(other_graph, |seed| {
            assert!(seed.is_none(), "different base graph must reject the seed");
            Ok(dummy_overlay_generation("other-graph"))
        })
        .expect("other graph generation");

        let other_worktree = cache_identity(second.path(), 19);
        overlay_generation(other_worktree, |seed| {
            assert!(seed.is_none(), "different worktree must reject the seed");
            Ok(dummy_overlay_generation("other-worktree"))
        })
        .expect("other worktree generation");
    }

    #[test]
    fn overlay_generation_failure_preserves_prior_and_allows_exact_retry() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let prior_identity = cache_identity(tempdir.path(), 20);
        let prior = overlay_generation(prior_identity.clone(), |_| {
            Ok(dummy_overlay_generation("failure-prior"))
        })
        .expect("prior generation");
        let mut failed_identity = prior_identity.clone();
        failed_identity.current_head_oid = "failed-next-head".to_owned();
        let prior_for_failure = Arc::clone(&prior);
        let error = overlay_generation(failed_identity.clone(), |seed| {
            let seed = seed.expect("prior seed for failed build");
            assert!(Arc::ptr_eq(&seed, &prior_for_failure));
            anyhow::bail!("intentional generation failure")
        })
        .expect_err("failed generation build");
        assert_eq!(error.to_string(), "intentional generation failure");

        let prior_again = overlay_generation(prior_identity, |_| {
            panic!("failed successor must not invalidate its prior generation")
        })
        .expect("prior remains exact");
        assert!(Arc::ptr_eq(&prior, &prior_again));

        let prior_for_retry = Arc::clone(&prior);
        let retried = overlay_generation(failed_identity, |seed| {
            let seed = seed.expect("prior seed for retry");
            assert!(Arc::ptr_eq(&seed, &prior_for_retry));
            Ok(dummy_overlay_generation("failure-retried"))
        })
        .expect("retry generation");
        assert_eq!(
            retried.base_artifact().graph_content_hash,
            "failure-retried"
        );
    }

    #[test]
    fn overlay_generation_failed_leader_wakes_follower_for_exact_retry() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 21);
        let (leader_started_tx, leader_started_rx) = std::sync::mpsc::sync_channel(0);
        let (release_leader_tx, release_leader_rx) = std::sync::mpsc::sync_channel(0);
        let leader_identity = identity.clone();
        let leader = std::thread::spawn(move || {
            overlay_generation(leader_identity, |_| {
                leader_started_tx.send(()).expect("signal leader start");
                release_leader_rx.recv().expect("release leader");
                anyhow::bail!("leader failed")
            })
        });
        leader_started_rx.recv().expect("leader started");

        let follower_builds = Arc::new(AtomicUsize::new(0));
        let follower_builds_thread = Arc::clone(&follower_builds);
        let follower = std::thread::spawn(move || {
            overlay_generation(identity, |_| {
                follower_builds_thread.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_generation("follower-retry"))
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        release_leader_tx.send(()).expect("release failed leader");

        let leader_error = leader
            .join()
            .expect("leader thread")
            .expect_err("leader error");
        let follower = follower
            .join()
            .expect("follower thread")
            .expect("follower retry");
        assert_eq!(leader_error.to_string(), "leader failed");
        assert_eq!(follower_builds.load(Ordering::SeqCst), 1);
        assert_eq!(
            follower.base_artifact().graph_content_hash,
            "follower-retry"
        );
    }

    #[test]
    fn overlay_generation_cancelled_leader_wakes_follower_for_exact_retry() {
        let _guard = overlay_generation_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 22);
        let (leader_started_tx, leader_started_rx) = std::sync::mpsc::sync_channel(0);
        let (release_leader_tx, release_leader_rx) = std::sync::mpsc::sync_channel(0);
        let leader_identity = identity.clone();
        let leader = std::thread::spawn(move || {
            let _ = overlay_generation(leader_identity, |_| {
                leader_started_tx.send(()).expect("signal leader start");
                release_leader_rx.recv().expect("release leader");
                panic!("cancel generation leader")
            });
        });
        leader_started_rx.recv().expect("leader started");

        let follower_builds = Arc::new(AtomicUsize::new(0));
        let follower_builds_thread = Arc::clone(&follower_builds);
        let (follower_tx, follower_rx) = std::sync::mpsc::channel();
        let follower = std::thread::spawn(move || {
            let result = overlay_generation(identity, |_| {
                follower_builds_thread.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_generation("cancel-retry"))
            });
            follower_tx.send(result).expect("send follower result");
        });
        std::thread::sleep(Duration::from_millis(50));
        release_leader_tx
            .send(())
            .expect("release cancelled leader");

        assert!(leader.join().is_err(), "leader closure must unwind");
        let follower_result = follower_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("cancelled leader woke follower")
            .expect("follower exact retry");
        follower.join().expect("follower thread");
        assert_eq!(follower_builds.load(Ordering::SeqCst), 1);
        assert_eq!(
            follower_result.base_artifact().graph_content_hash,
            "cancel-retry"
        );
    }

    #[test]
    fn overlay_delta_cache_reuses_build_for_same_changed_files_fingerprint() {
        let _guard = overlay_delta_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let worktree = tempdir.path();
        let identity = cache_identity(worktree, 1);
        let builds = AtomicUsize::new(0);

        let first = overlay_delta_for_identity_test(&identity, || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("one"))
        })
        .expect("first overlay delta");
        let second = overlay_delta_for_identity_test(&identity, || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("two"))
        })
        .expect("second overlay delta");

        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "same worktree and changed-files fingerprint should reuse the cached overlay extract"
        );
        assert!(Arc::ptr_eq(&first.artifact, &second.artifact));
        assert_eq!(first.shadowed, second.shadowed);
    }

    #[test]
    fn overlay_delta_cache_rebuilds_when_changed_files_fingerprint_changes() {
        let _guard = overlay_delta_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let worktree = tempdir.path();
        let builds = AtomicUsize::new(0);

        overlay_delta_for_identity_test(&cache_identity(worktree, 1), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("one"))
        })
        .expect("first overlay delta");
        overlay_delta_for_identity_test(&cache_identity(worktree, 2), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("two"))
        })
        .expect("second overlay delta");

        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "changed file contents should extract a new overlay delta"
        );
    }

    #[test]
    fn overlay_delta_cache_keeps_per_worktree_entries_when_switching_projects() {
        let _guard = overlay_delta_test_lock();
        let spur = tempfile::tempdir().expect("spur worktree");
        let notebook = tempfile::tempdir().expect("notebook worktree");
        let otobank = tempfile::tempdir().expect("otobank worktree");
        let builds = AtomicUsize::new(0);

        overlay_delta_for_identity_test(&cache_identity(spur.path(), 1), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("spur"))
        })
        .expect("spur overlay");
        overlay_delta_for_identity_test(&cache_identity(notebook.path(), 1), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("notebook"))
        })
        .expect("notebook overlay");
        overlay_delta_for_identity_test(&cache_identity(otobank.path(), 1), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("otobank"))
        })
        .expect("otobank overlay");
        overlay_delta_for_identity_test(&cache_identity(spur.path(), 1), || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("spur-again"))
        })
        .expect("spur overlay reuse after switching projects");

        assert_eq!(
            OVERLAY_DELTA_CACHE_CAPACITY, PARQUET_CLIENT_CACHE_CAPACITY,
            "overlay cache must keep one live delta per cached parquet project"
        );
        assert_eq!(
            builds.load(Ordering::SeqCst),
            3,
            "switching projects must reuse each worktree overlay instead of rebuilding after eviction"
        );
    }

    #[test]
    fn overlay_delta_singleflight_shares_in_flight_build_for_same_key() {
        let _guard = overlay_delta_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let worktree = tempdir.path().to_path_buf();
        let identity = cache_identity(&worktree, 7);
        let builds = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let spawn = |builds: Arc<AtomicUsize>,
                     barrier: Arc<std::sync::Barrier>,
                     identity: SnapshotIdentity| {
            std::thread::spawn(move || {
                barrier.wait();
                overlay_delta_for_identity_test(&identity, || {
                    std::thread::sleep(Duration::from_millis(50));
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(dummy_overlay_delta("shared"))
                })
            })
        };

        let first = spawn(Arc::clone(&builds), Arc::clone(&barrier), identity.clone());
        let second = spawn(builds.clone(), barrier, identity);
        let first = first
            .join()
            .expect("first overlay thread")
            .expect("first overlay");
        let second = second
            .join()
            .expect("second overlay thread")
            .expect("second overlay");

        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "concurrent overlay extracts for the same worktree fingerprint must share one in-flight build"
        );
        assert!(Arc::ptr_eq(&first.artifact, &second.artifact));
        assert_eq!(first.shadowed, second.shadowed);
    }

    #[test]
    fn oid_addressed_layers_reuse_after_more_than_five_seconds() {
        let _guard = overlay_delta_test_lock();
        let parquet_tempdir = tempfile::tempdir().expect("parquet tempdir");
        let parquet_dir = write_empty_parquet(parquet_tempdir.path());
        let parquet = ParquetClient::open(&parquet_dir).expect("open parquet client");
        let first_file_oids = parquet.file_oids().expect("first file OID read");
        fs::remove_file(parquet_dir.join("file_manifests.parquet"))
            .expect("remove backing file after first read");

        let worktree_tempdir = tempfile::tempdir().expect("worktree tempdir");
        let worktree = worktree_tempdir.path();
        let base = snapshot_fixture(worktree);
        let extensions = crate::extract::languages::all_supported_extensions();
        let first_snapshot = overlay_snapshot::snapshot(worktree, base.clone(), &extensions)
            .expect("first validated snapshot");
        let identity = first_snapshot.identity.clone().expect("complete identity");
        let builds = AtomicUsize::new(0);
        let first_delta = overlay_delta_for_identity_test(&identity, || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("retained"))
        })
        .expect("first delta");

        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(6_001));
        eprintln!("no-TTL retention elapsed={:?}", started.elapsed());

        let second_file_oids = parquet
            .file_oids()
            .expect("same opened manifest reuses file OIDs after old TTL");
        let second_snapshot = overlay_snapshot::snapshot(worktree, base, &extensions)
            .expect("same validated snapshot after old TTL");
        let second_delta = overlay_delta_for_identity_test(&identity, || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("rebuilt"))
        })
        .expect("same cached delta after old TTL");

        assert_eq!(second_file_oids, first_file_oids);
        assert!(second_snapshot.measurements.snapshot_reused);
        assert_eq!(second_snapshot.path_state, first_snapshot.path_state);
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first_delta.artifact, &second_delta.artifact));
    }

    #[test]
    fn overlay_delta_invalidates_every_complete_identity_component() {
        let _guard = overlay_delta_test_lock();
        let first_parent = tempfile::tempdir().expect("first parent");
        let second_parent = tempfile::tempdir().expect("second parent");
        let first_worktree = first_parent.path().join("same-relative-name");
        let second_worktree = second_parent.path().join("same-relative-name");
        fs::create_dir_all(&first_worktree).expect("first worktree");
        fs::create_dir_all(&second_worktree).expect("second worktree");
        let baseline = cache_identity(&first_worktree, 1);
        let other_identity = cache_identity(&second_worktree, 1);
        let other_index_identity = cache_identity(&first_worktree, 2).index_identity;

        let fingerprint = |entries: Vec<(String, OverlayPathState)>| {
            normalized_changed_set_fingerprint(entries.iter().map(|(path, state)| (path, state)))
        };
        let mut variants = Vec::new();
        let mut graph = baseline.clone();
        graph.indexed_graph_content_hash = "graph-changed".to_owned();
        variants.push(("graph", graph));
        let mut indexed_head = baseline.clone();
        indexed_head.indexed_head_oid = Some("indexed-head-changed".to_owned());
        variants.push(("indexed-head", indexed_head));
        let mut current_head = baseline.clone();
        current_head.current_head_oid = "current-head-changed".to_owned();
        variants.push(("current-head", current_head));
        let mut index = baseline.clone();
        index.index_identity = other_index_identity;
        variants.push(("index", index));
        let mut changed_oid = baseline.clone();
        changed_oid.normalized_changed_set_fingerprint = fingerprint(vec![(
            "src/lib.rs".to_owned(),
            OverlayPathState::Tracked("oid-v2".to_owned()),
        )]);
        variants.push(("changed-oid", changed_oid));
        let mut deletion = baseline.clone();
        deletion.normalized_changed_set_fingerprint =
            fingerprint(vec![("src/lib.rs".to_owned(), OverlayPathState::Deleted)]);
        variants.push(("deletion", deletion));
        let mut rename = baseline.clone();
        rename.normalized_changed_set_fingerprint = fingerprint(vec![
            ("src/lib.rs".to_owned(), OverlayPathState::Deleted),
            (
                "src/renamed.rs".to_owned(),
                OverlayPathState::Tracked("oid-v1".to_owned()),
            ),
        ]);
        variants.push(("rename", rename));
        let mut other_worktree = baseline.clone();
        other_worktree.canonical_worktree = other_identity.canonical_worktree;
        variants.push(("canonical-worktree", other_worktree));

        let builds = AtomicUsize::new(0);
        overlay_delta_for_identity_test(&baseline, || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("baseline"))
        })
        .expect("baseline");
        for (label, identity) in &variants {
            let actual = overlay_delta_for_identity_test(identity, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_delta(label))
            })
            .unwrap_or_else(|error| panic!("{label} rebuild failed: {error:#}"));
            assert_eq!(
                actual.artifact.graph_content_hash, *label,
                "{label} must never reuse a mismatched complete identity"
            );
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1 + variants.len());
    }

    #[test]
    fn overlay_delta_capacity_evicts_without_returning_mismatched_entries() {
        let _guard = overlay_delta_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let builds = AtomicUsize::new(0);
        let identities = (0..=OVERLAY_DELTA_CACHE_CAPACITY)
            .map(|index| {
                let mut identity = cache_identity(tempdir.path(), 42);
                identity.current_head_oid = format!("head-{index}");
                identity
            })
            .collect::<Vec<_>>();

        for (index, identity) in identities.iter().enumerate() {
            let tag = format!("entry-{index}");
            let actual = overlay_delta_for_identity_test(identity, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(dummy_overlay_delta(&tag))
            })
            .expect("insert cache entry");
            assert_eq!(actual.artifact.graph_content_hash, tag);
        }
        assert_eq!(builds.load(Ordering::SeqCst), identities.len());

        let rebuilt = overlay_delta_for_identity_test(&identities[0], || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(dummy_overlay_delta("entry-0-rebuilt"))
        })
        .expect("oldest entry rebuild");
        assert_eq!(rebuilt.artifact.graph_content_hash, "entry-0-rebuilt");
        assert_eq!(builds.load(Ordering::SeqCst), identities.len() + 1);
    }

    #[test]
    fn overlay_delta_singleflight_followers_receive_exact_leader_error() {
        let _guard = overlay_delta_test_lock();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let identity = cache_identity(tempdir.path(), 99);
        let builds = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let spawn = |builds: Arc<AtomicUsize>,
                     barrier: Arc<std::sync::Barrier>,
                     identity: SnapshotIdentity| {
            std::thread::spawn(move || {
                barrier.wait();
                let result = overlay_delta_for_identity_test(&identity, || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    Err::<CachedOverlayDelta, _>(anyhow::anyhow!("inner exact error"))
                        .context("outer exact error")
                });
                result.err().expect("shared error")
            })
        };
        let first = spawn(Arc::clone(&builds), Arc::clone(&barrier), identity.clone());
        let second = spawn(builds.clone(), barrier, identity);
        let errors = [
            first.join().expect("first error thread"),
            second.join().expect("second error thread"),
        ];

        eprintln!(
            "singleflight exact errors: {:?}",
            errors
                .iter()
                .map(|error| format!("{error:#}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(errors
            .iter()
            .all(|error| format!("{error:#}") == "outer exact error: inner exact error"));
    }

    #[test]
    fn parquet_client_cache_opens_again_when_manifest_mtime_changes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = write_empty_parquet(tempdir.path());
        let first = parquet_client(&dir).expect("first open");
        let manifest = dir.join("manifest.json");
        let file = fs::File::open(&manifest).expect("open manifest");
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(2))
            .expect("bump manifest mtime");
        let second = parquet_client(&dir).expect("second open");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "a newer manifest should open a fresh ParquetClient"
        );
    }

    #[test]
    fn parquet_client_cache_opens_again_when_manifest_bytes_change_at_same_mtime() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = write_empty_parquet(tempdir.path());
        let first = parquet_client(&dir).expect("first open");
        let manifest = dir.join("manifest.json");
        let original_modified = fs::metadata(&manifest)
            .expect("manifest metadata")
            .modified()
            .expect("manifest modified time");
        let mut bytes = fs::read(&manifest).expect("manifest bytes");
        bytes.push(b'\n');
        fs::write(&manifest, bytes).expect("rewrite manifest bytes");
        let file = fs::File::open(&manifest).expect("open manifest");
        file.set_modified(original_modified)
            .expect("restore manifest mtime");

        let second = parquet_client(&dir).expect("second open");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "manifest content identity must invalidate even when mtime aliases"
        );
    }
}
