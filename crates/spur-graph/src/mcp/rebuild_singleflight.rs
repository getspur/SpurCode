use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};

use crate::schema::GraphIndexArtifact;
use crate::temporal::TemporalIndex;
use tokio::sync::{Mutex, OnceCell};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RebuildKey {
    head_oid: String,
    dirty_oid_set_hash: u64,
}

impl RebuildKey {
    pub fn from(head_oid: &str, dirty: &BTreeMap<PathBuf, [u8; 20]>) -> Self {
        let mut hasher = DefaultHasher::new();
        dirty.hash(&mut hasher);

        Self {
            head_oid: head_oid.to_string(),
            dirty_oid_set_hash: hasher.finish(),
        }
    }
}

type RebuildCell = OnceCell<Arc<GraphIndexArtifact>>;
const CACHE_CAPACITY: usize = 1;

struct RebuildBundle {
    key: RebuildKey,
    artifact: Arc<GraphIndexArtifact>,
    temporal_index: OnceLock<Arc<TemporalIndex>>,
}

impl RebuildBundle {
    fn new(key: RebuildKey, artifact: Arc<GraphIndexArtifact>) -> Self {
        Self {
            key,
            artifact,
            temporal_index: OnceLock::new(),
        }
    }
}

struct RebuildCacheState {
    latest_by_worktree: HashMap<PathBuf, RebuildKey>,
    incremental_failures_by_key: HashMap<RebuildKey, u32>,
    retained: VecDeque<Arc<RebuildBundle>>,
}

impl RebuildCacheState {
    fn new() -> Self {
        Self {
            latest_by_worktree: HashMap::new(),
            incremental_failures_by_key: HashMap::new(),
            retained: VecDeque::with_capacity(CACHE_CAPACITY),
        }
    }
}

pub struct RebuildCoordinator {
    cells: Mutex<HashMap<RebuildKey, Weak<RebuildCell>>>,
    cache: StdMutex<RebuildCacheState>,
    #[cfg(any(test, feature = "test-support"))]
    build_invocations: AtomicUsize,
    #[cfg(any(test, feature = "test-support"))]
    temporal_index_build_invocations: AtomicUsize,
}

impl RebuildCoordinator {
    pub fn new() -> Self {
        Self {
            cells: Mutex::new(HashMap::new()),
            cache: StdMutex::new(RebuildCacheState::new()),
            #[cfg(any(test, feature = "test-support"))]
            build_invocations: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-support"))]
            temporal_index_build_invocations: AtomicUsize::new(0),
        }
    }

    /// Returns the graph artifact for the requested rebuild key.
    ///
    /// The coordinator stores only weak cell handles. Concurrent callers share
    /// the same strong cell while awaiting the build, and dead weak entries are
    /// collected on the next access.
    pub(crate) async fn get_or_build<F, Fut>(
        &self,
        worktree: PathBuf,
        key: RebuildKey,
        build: F,
    ) -> anyhow::Result<Arc<GraphIndexArtifact>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Arc<GraphIndexArtifact>>>,
    {
        self.record_latest_key(&worktree, &key);

        if let Some(bundle) = self.retained_bundle(&key) {
            return Ok(Arc::clone(&bundle.artifact));
        }

        let cell = {
            let mut cells = self.cells.lock().await;
            cells.retain(|_, cell| cell.strong_count() > 0);

            if let Some(cell) = cells.get(&key).and_then(Weak::upgrade) {
                cell
            } else {
                let cell = Arc::new(RebuildCell::new());
                cells.insert(key.clone(), Arc::downgrade(&cell));
                cell
            }
        };

        #[cfg(any(test, feature = "test-support"))]
        let artifact = cell
            .get_or_try_init(|| {
                self.build_invocations.fetch_add(1, Ordering::SeqCst);
                build()
            })
            .await?;
        #[cfg(not(any(test, feature = "test-support")))]
        let artifact = cell.get_or_try_init(build).await?;
        if let Some(bundle) = self.retain_artifact(&worktree, key, Arc::clone(artifact)) {
            Ok(Arc::clone(&bundle.artifact))
        } else {
            Ok(Arc::clone(artifact))
        }
    }

    pub(crate) fn temporal_index_for_artifact(
        &self,
        worktree: &Path,
        key: RebuildKey,
        artifact: Arc<GraphIndexArtifact>,
    ) -> Arc<TemporalIndex> {
        if let Some(bundle) = self.retained_bundle(&key) {
            return self.temporal_index_for_bundle(&bundle);
        }

        if let Some(bundle) = self.retain_artifact(worktree, key, Arc::clone(&artifact)) {
            return self.temporal_index_for_bundle(&bundle);
        }

        Arc::new(TemporalIndex::new(artifact))
    }

    pub(crate) fn temporal_index_for_retained_artifact(
        &self,
        key: &RebuildKey,
    ) -> Option<Arc<TemporalIndex>> {
        let bundle = self.retained_bundle(key)?;
        Some(self.temporal_index_for_bundle(&bundle))
    }

    pub(crate) fn record_incremental_rebuild_failure(&self, key: &RebuildKey) -> u32 {
        let Ok(mut cache) = self.cache.lock() else {
            return 0;
        };
        let failures = cache
            .incremental_failures_by_key
            .entry(key.clone())
            .or_default();
        *failures = failures.saturating_add(1);
        *failures
    }

    pub(crate) fn reset_incremental_rebuild_failures(&self, key: &RebuildKey) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        cache.incremental_failures_by_key.remove(key);
    }

    fn temporal_index_for_bundle(&self, bundle: &Arc<RebuildBundle>) -> Arc<TemporalIndex> {
        let temporal_index = bundle.temporal_index.get_or_init(|| {
            #[cfg(any(test, feature = "test-support"))]
            self.temporal_index_build_invocations
                .fetch_add(1, Ordering::SeqCst);
            Arc::new(TemporalIndex::new(Arc::clone(&bundle.artifact)))
        });
        Arc::clone(temporal_index)
    }

    fn retained_bundle(&self, key: &RebuildKey) -> Option<Arc<RebuildBundle>> {
        let Ok(mut cache) = self.cache.lock() else {
            return None;
        };
        let position = cache
            .retained
            .iter()
            .position(|bundle| bundle.key == *key)?;
        let bundle = cache.retained.remove(position)?;
        cache.retained.push_back(Arc::clone(&bundle));
        Some(bundle)
    }

    fn record_latest_key(&self, worktree: &Path, key: &RebuildKey) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        cache
            .latest_by_worktree
            .insert(worktree.to_path_buf(), key.clone());
    }

    fn retain_artifact(
        &self,
        worktree: &Path,
        key: RebuildKey,
        artifact: Arc<GraphIndexArtifact>,
    ) -> Option<Arc<RebuildBundle>> {
        let Ok(mut cache) = self.cache.lock() else {
            return Some(Arc::new(RebuildBundle::new(key, artifact)));
        };

        if cache
            .latest_by_worktree
            .get(worktree)
            .is_some_and(|latest_key| latest_key != &key)
        {
            return None;
        };

        if let Some(position) = cache.retained.iter().position(|bundle| bundle.key == key) {
            let bundle = cache
                .retained
                .remove(position)
                .expect("position came from iter");
            if Arc::ptr_eq(&bundle.artifact, &artifact) {
                cache.retained.push_back(Arc::clone(&bundle));
                return Some(bundle);
            }
        }

        let bundle = Arc::new(RebuildBundle::new(key, artifact));
        cache.retained.push_back(Arc::clone(&bundle));
        while cache.retained.len() > CACHE_CAPACITY {
            cache.retained.pop_front();
        }
        Some(bundle)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn build_invocation_count(&self) -> usize {
        self.build_invocations.load(Ordering::SeqCst)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn temporal_index_build_invocation_count(&self) -> usize {
        self.temporal_index_build_invocations.load(Ordering::SeqCst)
    }
}

impl Default for RebuildCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::Poll;

    use crate::schema::{GraphIndexArtifact, GraphIndexHeader};

    use super::{RebuildCoordinator, RebuildKey};

    fn key_for(head_oid: &str, dirty_byte: u8) -> RebuildKey {
        let mut dirty = BTreeMap::new();
        dirty.insert(PathBuf::from("src/lib.rs"), [dirty_byte; 20]);
        RebuildKey::from(head_oid, &dirty)
    }

    fn key() -> RebuildKey {
        key_for("head-a", 7)
    }

    fn worktree() -> PathBuf {
        PathBuf::from("/tmp/spur-worktree-a")
    }

    fn artifact(hash: &str) -> Arc<GraphIndexArtifact> {
        Arc::new(GraphIndexArtifact {
            header: GraphIndexHeader {
                graph_index_version: "test".to_string(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_string(),
            graph_content_hash: hash.to_string(),
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
            ..GraphIndexArtifact::empty()
        })
    }

    async fn assert_pending<F>(mut future: Pin<&mut F>)
    where
        F: Future,
    {
        futures::future::poll_fn(move |cx| match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("future completed before test released the build"),
        })
        .await;
    }

    #[tokio::test]
    async fn same_key_concurrent_calls_invoke_build_once() {
        let coordinator = RebuildCoordinator::new();
        let builds = Arc::new(AtomicUsize::new(0));
        let (release_build, wait_for_release) = tokio::sync::oneshot::channel();

        let first = coordinator.get_or_build(worktree(), key(), {
            let builds = Arc::clone(&builds);
            move || async move {
                builds.fetch_add(1, Ordering::SeqCst);
                wait_for_release.await.expect("release build");
                Ok(artifact("first"))
            }
        });
        tokio::pin!(first);
        assert_pending(first.as_mut()).await;

        let second = coordinator.get_or_build(worktree(), key(), {
            let builds = Arc::clone(&builds);
            move || async move {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(artifact("second"))
            }
        });
        tokio::pin!(second);
        assert_pending(second.as_mut()).await;

        release_build.send(()).expect("send release");
        let first_artifact = first.await.expect("first build succeeds");
        let second_artifact = second.await.expect("second build succeeds");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first_artifact, &second_artifact));
        assert_eq!(first_artifact.graph_content_hash, "first");
    }

    #[tokio::test]
    async fn completed_entry_is_retained_for_next_access() {
        let coordinator = RebuildCoordinator::new();
        let builds = Arc::new(AtomicUsize::new(0));

        let first_artifact = coordinator
            .get_or_build(worktree(), key(), {
                let builds = Arc::clone(&builds);
                move || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(artifact("first"))
                }
            })
            .await
            .expect("first build succeeds");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(first_artifact.graph_content_hash, "first");
        drop(first_artifact);

        let second_artifact = coordinator
            .get_or_build(worktree(), key(), {
                let builds = Arc::clone(&builds);
                move || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(artifact("second"))
                }
            })
            .await
            .expect("second build succeeds");

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(second_artifact.graph_content_hash, "first");
    }

    #[tokio::test]
    async fn stale_completion_does_not_evict_newer_retained_artifact() {
        let coordinator = RebuildCoordinator::new();
        let worktree = worktree();
        let stale_key = key_for("head-a", 7);
        let fresh_key = key_for("head-b", 7);
        let (release_stale, wait_for_release) = tokio::sync::oneshot::channel();

        let stale = coordinator.get_or_build(worktree.clone(), stale_key, move || async move {
            wait_for_release.await.expect("release stale build");
            Ok(artifact("stale"))
        });
        tokio::pin!(stale);
        assert_pending(stale.as_mut()).await;

        let fresh_artifact = coordinator
            .get_or_build(worktree.clone(), fresh_key.clone(), || async {
                Ok(artifact("fresh"))
            })
            .await
            .expect("fresh build succeeds");
        assert_eq!(fresh_artifact.graph_content_hash, "fresh");

        release_stale.send(()).expect("send stale release");
        let stale_artifact = stale.await.expect("stale build succeeds");
        assert_eq!(stale_artifact.graph_content_hash, "stale");

        let retained_fresh = coordinator
            .get_or_build(worktree, fresh_key, || async { Ok(artifact("rebuilt")) })
            .await
            .expect("fresh cache lookup succeeds");

        assert_eq!(retained_fresh.graph_content_hash, "fresh");
    }

    #[tokio::test]
    async fn temporal_index_is_built_once_for_retained_artifact() {
        let coordinator = RebuildCoordinator::new();
        let worktree = worktree();
        let retained_artifact = coordinator
            .get_or_build(worktree.clone(), key(), || async { Ok(artifact("first")) })
            .await
            .expect("build succeeds");

        let first_index = coordinator.temporal_index_for_artifact(
            &worktree,
            key(),
            Arc::clone(&retained_artifact),
        );
        let second_index =
            coordinator.temporal_index_for_artifact(&worktree, key(), retained_artifact);

        assert!(Arc::ptr_eq(&first_index, &second_index));
        assert_eq!(coordinator.temporal_index_build_invocation_count(), 1);
    }
}
