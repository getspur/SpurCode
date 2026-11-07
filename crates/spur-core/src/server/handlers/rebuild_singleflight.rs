use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};

use spur_graph::schema::GraphIndexArtifact;
use spur_graph::temporal::TemporalIndex;
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

pub(crate) struct RebuildCoordinator {
    cells: Mutex<HashMap<RebuildKey, Weak<RebuildCell>>>,
    retained: StdMutex<VecDeque<Arc<RebuildBundle>>>,
    #[cfg(any(test, feature = "test-support"))]
    build_invocations: AtomicUsize,
    #[cfg(any(test, feature = "test-support"))]
    temporal_index_build_invocations: AtomicUsize,
}

impl RebuildCoordinator {
    pub fn new() -> Self {
        Self {
            cells: Mutex::new(HashMap::new()),
            retained: StdMutex::new(VecDeque::with_capacity(CACHE_CAPACITY)),
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
    pub async fn get_or_build<F, Fut>(
        &self,
        key: RebuildKey,
        build: F,
    ) -> anyhow::Result<Arc<GraphIndexArtifact>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Arc<GraphIndexArtifact>>>,
    {
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
        let bundle = self.retain_artifact(key, Arc::clone(artifact));
        Ok(Arc::clone(&bundle.artifact))
    }

    pub(crate) fn temporal_index_for_artifact(
        &self,
        key: RebuildKey,
        artifact: Arc<GraphIndexArtifact>,
    ) -> Arc<TemporalIndex> {
        let bundle = self
            .retained_bundle(&key)
            .unwrap_or_else(|| self.retain_artifact(key, artifact));
        self.temporal_index_for_bundle(&bundle)
    }

    pub(crate) fn temporal_index_for_retained_artifact(
        &self,
        key: &RebuildKey,
    ) -> Option<Arc<TemporalIndex>> {
        let bundle = self.retained_bundle(key)?;
        Some(self.temporal_index_for_bundle(&bundle))
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
        let Ok(mut retained) = self.retained.lock() else {
            return None;
        };
        let position = retained.iter().position(|bundle| bundle.key == *key)?;
        let bundle = retained.remove(position)?;
        retained.push_back(Arc::clone(&bundle));
        Some(bundle)
    }

    fn retain_artifact(
        &self,
        key: RebuildKey,
        artifact: Arc<GraphIndexArtifact>,
    ) -> Arc<RebuildBundle> {
        let Ok(mut retained) = self.retained.lock() else {
            return Arc::new(RebuildBundle::new(key, artifact));
        };

        if let Some(position) = retained.iter().position(|bundle| bundle.key == key) {
            let bundle = retained.remove(position).expect("position came from iter");
            if Arc::ptr_eq(&bundle.artifact, &artifact) {
                retained.push_back(Arc::clone(&bundle));
                return bundle;
            }
        }

        let bundle = Arc::new(RebuildBundle::new(key, artifact));
        retained.push_back(Arc::clone(&bundle));
        while retained.len() > CACHE_CAPACITY {
            retained.pop_front();
        }
        bundle
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn build_invocation_count(&self) -> usize {
        self.build_invocations.load(Ordering::SeqCst)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn temporal_index_build_invocation_count(&self) -> usize {
        self.temporal_index_build_invocations.load(Ordering::SeqCst)
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

    use spur_graph::schema::{GraphIndexArtifact, GraphIndexHeader};

    use super::{RebuildCoordinator, RebuildKey};

    fn key() -> RebuildKey {
        let mut dirty = BTreeMap::new();
        dirty.insert(PathBuf::from("src/lib.rs"), [7; 20]);
        RebuildKey::from("head-a", &dirty)
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

        let first = coordinator.get_or_build(key(), {
            let builds = Arc::clone(&builds);
            move || async move {
                builds.fetch_add(1, Ordering::SeqCst);
                wait_for_release.await.expect("release build");
                Ok(artifact("first"))
            }
        });
        tokio::pin!(first);
        assert_pending(first.as_mut()).await;

        let second = coordinator.get_or_build(key(), {
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
            .get_or_build(key(), {
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
            .get_or_build(key(), {
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
    async fn temporal_index_is_built_once_for_retained_artifact() {
        let coordinator = RebuildCoordinator::new();
        let retained_artifact = coordinator
            .get_or_build(key(), || async { Ok(artifact("first")) })
            .await
            .expect("build succeeds");

        let first_index =
            coordinator.temporal_index_for_artifact(key(), Arc::clone(&retained_artifact));
        let second_index = coordinator.temporal_index_for_artifact(key(), retained_artifact);

        assert!(Arc::ptr_eq(&first_index, &second_index));
        assert_eq!(coordinator.temporal_index_build_invocation_count(), 1);
    }
}
