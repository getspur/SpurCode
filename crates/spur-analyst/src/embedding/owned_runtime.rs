//! A caller-owned supervised embedding session.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use super::mode::AnalystEmbedMode;
use spur_graph::EMBEDDING_VECTOR_DIMENSIONS;

#[cfg(feature = "embed")]
use {
    anyhow::{anyhow, Context},
    spur_embedding::{Cancellation, CleanupOwner},
    spur_graph::{
        embedding_query_text_for_model, load_embedding_model_cancellable_with_cleanup,
        EmbeddingModelSelection,
    },
    std::{
        thread::JoinHandle,
        time::{Duration, Instant},
    },
};

/// Clones share a single owner. Call `shutdown` before releasing the host.
#[derive(Clone)]
pub struct OwnedEmbeddingRuntime {
    owner: Arc<Owner>,
}

struct Owner {
    mode: AnalystEmbedMode,
    closed: AtomicBool,
    sidecar: SidecarTracker,
    #[cfg(feature = "embed")]
    core: Arc<Core>,
    #[cfg(feature = "embed")]
    shutdown_lock: Mutex<()>,
}

struct SidecarTracker {
    count: Mutex<usize>,
    counts: tokio::sync::watch::Sender<usize>,
    cancelled: tokio::sync::watch::Sender<bool>,
}
struct SidecarGuard<'a>(&'a SidecarTracker);
impl Drop for SidecarGuard<'_> {
    fn drop(&mut self) {
        let mut count = self.0.count.lock().unwrap_or_else(|e| e.into_inner());
        *count -= 1;
        self.0.counts.send_replace(*count);
    }
}

#[cfg(feature = "embed")]
struct Core {
    selection: EmbeddingModelSelection,
    serial: Mutex<()>,
    state: Mutex<State>,
    #[cfg(test)]
    delivery_hook: Mutex<Option<DeliveryHook>>,
}

#[cfg(all(test, feature = "embed"))]
struct DeliveryHook {
    before_send: bool,
    reached: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(feature = "embed")]
struct State {
    closed: bool,
    shutdown_result: Option<Result<(), String>>,
    cleanup_error: Option<String>,
    config: Option<Timeouts>,
    cleanup: Option<Arc<CleanupOwner>>,
    model: Option<spur_graph::store::lance_sections::EmbeddingModelHandle>,
    jobs: Vec<(Arc<Job>, JoinHandle<()>)>,
}

#[cfg(feature = "embed")]
#[derive(Clone, serde::Deserialize)]
struct Timeouts {
    queue_capacity: usize,
    startup_timeout_ms: u64,
    request_timeout_ms: u64,
    shutdown_timeout_ms: u64,
    shutdown_poll_interval_ms: u64,
}

#[cfg(feature = "embed")]
impl Timeouts {
    fn from_env() -> anyhow::Result<Self> {
        let raw = std::env::var("SPUR_EMBEDDING_HELPER_CONFIG")?;
        let config: Self = serde_json::from_str(&raw)?;
        if config.queue_capacity == 0 || config.queue_capacity.checked_add(1).is_none() {
            return Err(anyhow!("invalid embedding queue capacity"));
        }
        for ms in [
            config.startup_timeout_ms,
            config.request_timeout_ms,
            config.shutdown_timeout_ms,
            config.shutdown_poll_interval_ms,
        ] {
            if ms == 0
                || Instant::now()
                    .checked_add(Duration::from_millis(ms))
                    .is_none()
            {
                return Err(anyhow!("invalid embedding timeout"));
            }
        }
        Ok(config)
    }
    fn deadline(ms: u64) -> anyhow::Result<Instant> {
        Instant::now()
            .checked_add(Duration::from_millis(ms))
            .context("embedding deadline overflow")
    }
}

#[cfg(feature = "embed")]
struct Job {
    cancel: Cancellation,
    cancelled: AtomicBool,
}

#[cfg(feature = "embed")]
struct CancelOnDrop {
    core: Arc<Core>,
    job: Arc<Job>,
}
#[cfg(feature = "embed")]
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        // Synchronize cancellation with model publication.
        let _state = self.core.state.lock().unwrap_or_else(|e| e.into_inner());
        self.job.cancelled.store(true, Ordering::Release);
        self.job.cancel.cancel();
    }
}

impl Default for OwnedEmbeddingRuntime {
    fn default() -> Self {
        Self::new()
    }
}
impl OwnedEmbeddingRuntime {
    pub fn new() -> Self {
        Self::for_mode(AnalystEmbedMode::current())
    }

    #[cfg(all(feature = "embed", unix))]
    pub(super) fn local_helper() -> Self {
        Self::for_mode(AnalystEmbedMode::InProcess)
    }

    fn for_mode(mode: AnalystEmbedMode) -> Self {
        Self {
            owner: Arc::new(Owner {
                mode,
                closed: AtomicBool::new(false),
                sidecar: SidecarTracker {
                    count: Mutex::new(0),
                    counts: tokio::sync::watch::channel(0).0,
                    cancelled: tokio::sync::watch::channel(false).0,
                },
                #[cfg(feature = "embed")]
                core: Arc::new(Core {
                    selection: EmbeddingModelSelection::from_env(),
                    serial: Mutex::new(()),
                    state: Mutex::new(State {
                        closed: false,
                        shutdown_result: None,
                        cleanup_error: None,
                        config: None,
                        cleanup: None,
                        model: None,
                        jobs: Vec::new(),
                    }),
                    #[cfg(test)]
                    delivery_hook: Mutex::new(None),
                }),
                #[cfg(feature = "embed")]
                shutdown_lock: Mutex::new(()),
            }),
        }
    }

    #[cfg(all(feature = "embed", unix))]
    pub(super) async fn embed_local_query(
        &self,
        query: &str,
    ) -> anyhow::Result<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        self.run(query.to_owned()).await
    }

    async fn sidecar_operation<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        let tracker = &self.owner.sidecar;
        {
            let mut count = tracker.count.lock().unwrap_or_else(|e| e.into_inner());
            if self.owner.closed.load(Ordering::Acquire) {
                return None;
            }
            *count = count.checked_add(1)?;
            tracker.counts.send_replace(*count);
        }
        let _guard = SidecarGuard(tracker);
        let mut cancel = tracker.cancelled.subscribe();
        if *cancel.borrow() {
            return None;
        }
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => None,
            result = future => Some(result),
        };
        // Completion and shutdown closure use the same gate. A private
        // transport result is not yet a completed public operation.
        let open = {
            let _gate = tracker.count.lock().unwrap_or_else(|e| e.into_inner());
            !self.owner.closed.load(Ordering::Acquire)
        };
        if open {
            result
        } else {
            None
        }
    }

    /// Warm a local helper with validated inference when local mode is selected.
    /// Off and external modes skip local warm and do not warm a local model.
    pub async fn warm(&self) -> anyhow::Result<()> {
        if self.owner.closed.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!("embedding runtime is shut down"));
        }
        match self.owner.mode {
            AnalystEmbedMode::Off | AnalystEmbedMode::Sidecar => Ok(()),
            AnalystEmbedMode::Auto => match self
                .sidecar_operation(super::runtime::auto_sidecar_reachable())
                .await
            {
                Some(true) => Ok(()),
                Some(false) => self.warm_helper().await,
                None => Err(anyhow::anyhow!("embedding runtime is shut down")),
            },
            AnalystEmbedMode::InProcess => self.warm_helper().await,
        }
    }
    async fn warm_helper(&self) -> anyhow::Result<()> {
        #[cfg(feature = "embed")]
        {
            self.run("warm".to_owned()).await.map(|_| ())
        }
        #[cfg(not(feature = "embed"))]
        {
            Ok(())
        }
    }
    pub async fn embed_query(&self, query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        if self.owner.closed.load(Ordering::Acquire) {
            return None;
        }
        match self.owner.mode {
            AnalystEmbedMode::Off => None,
            AnalystEmbedMode::Sidecar => self.embed_sidecar(query).await,
            AnalystEmbedMode::Auto => match self
                .sidecar_operation(super::runtime::auto_sidecar_reachable())
                .await?
            {
                true => self.embed_sidecar(query).await,
                false => self.embed_helper(query).await,
            },
            AnalystEmbedMode::InProcess => self.embed_helper(query).await,
        }
    }
    async fn embed_sidecar(&self, query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        self.sidecar_operation(super::sidecar_client::embed_query(
            query,
            super::runtime::EMBED_INFERENCE_TIMEOUT,
        ))
        .await
        .flatten()
    }
    async fn embed_helper(&self, query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        #[cfg(feature = "embed")]
        {
            self.run(query.to_owned()).await.ok()
        }
        #[cfg(not(feature = "embed"))]
        {
            let _ = query;
            None
        }
    }

    #[cfg(feature = "embed")]
    async fn run(&self, text: String) -> anyhow::Result<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        let core = Arc::clone(&self.owner.core);
        let job = Arc::new(Job {
            cancel: Cancellation::new(),
            cancelled: AtomicBool::new(false),
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut state = core.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(anyhow!("embedding runtime is shut down"));
            }
            // Reap completed jobs at admission, keeping the registry bounded by active work.
            let mut index = 0;
            while index < state.jobs.len() {
                if state.jobs[index].1.is_finished() {
                    let (_, handle) = state.jobs.swap_remove(index);
                    if handle.join().is_err() {
                        state
                            .cleanup_error
                            .get_or_insert_with(|| "embedding job panicked".into());
                    }
                } else {
                    index += 1;
                }
            }
            let config = match &state.config {
                Some(config) => config.clone(),
                None => Timeouts::from_env()?,
            };
            if state.jobs.len() >= config.queue_capacity + 1 {
                return Err(anyhow!("embedding queue is full"));
            }
            state.config = Some(config);
            let worker_core = Arc::clone(&core);
            let worker_job = Arc::clone(&job);
            let handle = std::thread::Builder::new()
                .name("spur-owned-embedding".into())
                .spawn(move || {
                    let result = worker_core.infer(&worker_job, &text);
                    #[cfg(test)]
                    let hook = worker_core.delivery_hook.lock().unwrap().take();
                    #[cfg(test)]
                    if let Some(hook) = hook.as_ref().filter(|h| h.before_send) {
                        hook.reached.send(()).unwrap();
                        hook.release.recv().unwrap();
                    }
                    let _ = tx.send(result);
                    #[cfg(test)]
                    if let Some(hook) = hook.as_ref().filter(|h| !h.before_send) {
                        hook.reached.send(()).unwrap();
                        hook.release.recv().unwrap();
                    }
                })?;
            state.jobs.push((Arc::clone(&job), handle));
        }
        let cancel = CancelOnDrop { core, job };
        let result = rx.await.context("embedding job terminated")?;
        let open = {
            let _gate = self
                .owner
                .sidecar
                .count
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            !self.owner.closed.load(Ordering::Acquire)
                && !cancel.job.cancelled.load(Ordering::Acquire)
        };
        if open {
            result
        } else {
            Err(anyhow!("cancelled"))
        }
    }

    /// Stop admission, cancel and join work, then close and join helper cleanup.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        {
            // Serialize closure with sidecar admission and its active count.
            let _count = self
                .owner
                .sidecar
                .count
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            self.owner.closed.store(true, Ordering::Release);
            self.owner.sidecar.cancelled.send_replace(true);
        }
        let mut counts = self.owner.sidecar.counts.subscribe();
        #[cfg(feature = "embed")]
        {
            let jobs = {
                let mut state = self
                    .owner
                    .core
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                state.closed = true;
                state
                    .jobs
                    .iter()
                    .map(|(job, _)| {
                        job.cancelled.store(true, Ordering::Release);
                        Arc::clone(job)
                    })
                    .collect::<Vec<_>>()
            };
            for job in jobs {
                job.cancel.cancel();
            }
            let owner = Arc::clone(&self.owner);
            let closing = tokio::task::spawn_blocking(move || owner.close());
            while *counts.borrow_and_update() != 0 {
                counts
                    .changed()
                    .await
                    .map_err(|_| anyhow::anyhow!("sidecar tracker closed"))?;
            }
            closing.await.context("embedding shutdown task failed")?
        }
        #[cfg(not(feature = "embed"))]
        {
            while *counts.borrow_and_update() != 0 {
                counts
                    .changed()
                    .await
                    .map_err(|_| anyhow::anyhow!("sidecar tracker closed"))?;
            }
            Ok(())
        }
    }
}

#[cfg(feature = "embed")]
impl Core {
    fn infer(&self, job: &Job, text: &str) -> anyhow::Result<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        let (config, cleanup, existing) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || job.cancelled.load(Ordering::Acquire) {
                return Err(anyhow!("cancelled"));
            }
            let config = match &state.config {
                Some(c) => c.clone(),
                None => Timeouts::from_env()?,
            };
            let cleanup = match &state.cleanup {
                Some(c) => Arc::clone(c),
                None => Arc::new(CleanupOwner::new(Duration::from_millis(
                    config.shutdown_poll_interval_ms,
                ))?),
            };
            state.config = Some(config.clone());
            state.cleanup = Some(Arc::clone(&cleanup));
            (config, cleanup, state.model.take())
        };
        let mut model = match existing {
            Some(model) => model,
            None => load_embedding_model_cancellable_with_cleanup(
                self.selection,
                false,
                job.cancel.clone(),
                Timeouts::deadline(config.startup_timeout_ms)?,
                cleanup,
            )?,
        };
        let input = embedding_query_text_for_model(text, self.selection);
        let result = model
            .embed_cancellable(
                &[input.as_ref()],
                Timeouts::deadline(config.request_timeout_ms)?,
                job.cancel.clone(),
            )
            .and_then(|rows| rows.into_iter().next().context("missing embedding"))
            .and_then(|row| {
                row.try_into()
                    .map_err(|_| anyhow!("unexpected embedding dimensions"))
            });
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || job.cancelled.load(Ordering::Acquire) || result.is_err() {
            drop(state);
            if let Err(error) = model.close() {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .cleanup_error
                    .get_or_insert_with(|| format!("{error:#}"));
            }
            return if job.cancelled.load(Ordering::Acquire) {
                Err(anyhow!("cancelled"))
            } else {
                result
            };
        }
        state.model = Some(model);
        result
    }
}

#[cfg(feature = "embed")]
impl Owner {
    fn close(&self) -> anyhow::Result<()> {
        let _shutdown = self.shutdown_lock.lock().unwrap_or_else(|e| e.into_inner());
        let jobs = {
            let mut state = self.core.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(result) = &state.shutdown_result {
                return result.clone().map_err(|e| anyhow!(e));
            }
            state.closed = true;
            for (job, _) in &state.jobs {
                job.cancelled.store(true, Ordering::Release);
                job.cancel.cancel();
            }
            std::mem::take(&mut state.jobs)
        };
        let mut error = None;
        for (_, job) in jobs {
            if job.join().is_err() {
                error.get_or_insert_with(|| "embedding job panicked".to_owned());
            }
        }
        let (mut model, cleanup, config, previous_error) = {
            let mut state = self.core.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.model.take(),
                state.cleanup.clone(),
                state.config.clone(),
                state.cleanup_error.clone(),
            )
        };
        if let Some(model) = &mut model {
            if let Err(e) = model.close() {
                error.get_or_insert_with(|| format!("{e:#}"));
            }
        }
        drop(model);
        if let (Some(cleanup), Some(config)) = (cleanup, config) {
            match Timeouts::deadline(config.shutdown_timeout_ms)
                .and_then(|deadline| cleanup.wait_for_cleanup(deadline).map_err(Into::into))
            {
                Ok(()) => {}
                Err(e) => {
                    error.get_or_insert_with(|| format!("{e:#}"));
                }
            }
        }
        if let Some(e) = previous_error {
            error.get_or_insert(e);
        }
        let result = match error {
            Some(e) => Err(e),
            None => Ok(()),
        };
        self.core
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutdown_result = Some(result.clone());
        result.map_err(|e| anyhow!(e))
    }
}
#[cfg(feature = "embed")]
impl Drop for Owner {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::warn!(%error, "owned embedding shutdown failed");
        }
    }
}

#[cfg(all(test, feature = "embed"))]
mod completion_tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll};

    #[cfg(unix)]
    #[test]
    fn off_mode_never_launches_configured_helper() {
        const CHILD: &str = "SPUR_OFF_NO_LAUNCH_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let owner = OwnedEmbeddingRuntime::for_mode(AnalystEmbedMode::Off);
                owner.warm().await.unwrap();
                assert!(owner.embed_query("unused query").await.is_none());
                owner.shutdown().await.unwrap();
            });
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("helper with spaces");
        let marker = dir.path().join("launched");
        std::fs::write(&helper, "#!/bin/sh\ntouch \"$SPUR_OFF_LAUNCH_MARKER\"\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = serde_json::json!({
            "executable": helper, "args": [], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "embedding::owned_runtime::completion_tests::off_mode_never_launches_configured_helper", "--nocapture"])
            .env(CHILD, "1")
            .env("SPUR_OFF_LAUNCH_MARKER", &marker)
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists(), "disabled embeddings launched a helper");
    }

    #[test]
    fn off_mode_ignores_invalid_helper_configuration() {
        const CHILD: &str = "SPUR_OFF_EMBED_POLICY_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let owner = OwnedEmbeddingRuntime::for_mode(AnalystEmbedMode::Off);
                owner.warm().await.unwrap();
                assert!(owner.embed_query("unused query").await.is_none());
                owner.shutdown().await.unwrap();
            });
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "embedding::owned_runtime::completion_tests::off_mode_ignores_invalid_helper_configuration",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("SPUR_EMBEDDING_HELPER_CONFIG", "invalid JSON")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn default_local_policy_does_not_require_json_configuration() {
        const CHILD: &str = "SPUR_DEFAULT_EMBED_POLICY_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            Timeouts::from_env().expect("default local helper policy must be available");
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "embedding::owned_runtime::completion_tests::default_local_policy_does_not_require_json_configuration",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("SPUR_EMBEDDING_HELPER_CONFIG")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "default policy child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn local_completion_observes_shutdown_at_both_delivery_boundaries() {
        const CHILD: &str = "SPUR_S6_COMPLETION_CHILD";
        if let Ok(boundary) = std::env::var(CHILD) {
            let runtime = OwnedEmbeddingRuntime::new();
            runtime.warm().await.unwrap();
            let (reached_tx, reached_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            *runtime.owner.core.delivery_hook.lock().unwrap() = Some(DeliveryHook {
                before_send: boundary == "before",
                reached: reached_tx,
                release: release_rx,
            });
            let mut query = Box::pin(runtime.embed_query("boundary"));
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            assert!(matches!(query.as_mut().poll(&mut cx), Poll::Pending));
            reached_rx.recv().unwrap();
            let mut close = Box::pin(runtime.shutdown());
            assert!(matches!(close.as_mut().poll(&mut cx), Poll::Pending));
            release_tx.send(()).unwrap();
            close.await.unwrap();
            assert!(
                query.await.is_none(),
                "{boundary}: result delivered after closure"
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spur-graph/tests/fixtures/embedding_fault_worker.py");
        let config = serde_json::json!({
            "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let mut failures = Vec::new();
        for boundary in ["before", "after"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "embedding::owned_runtime::completion_tests::local_completion_observes_shutdown_at_both_delivery_boundaries", "--nocapture"])
                .env(CHILD, boundary)
                .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
                .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join(boundary))
                .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
                .output().unwrap();
            if !output.status.success() {
                failures.push(format!(
                    "{boundary}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn sidecar_ready_during_closure_is_not_published() {
        let runtime = OwnedEmbeddingRuntime::new();
        let mut close = Box::pin(runtime.shutdown());
        let future = std::future::poll_fn(|cx| {
            assert!(matches!(close.as_mut().poll(cx), Poll::Pending));
            Poll::Ready(42)
        });
        let result = runtime.sidecar_operation(future).await;
        close.await.unwrap();
        assert_eq!(result, None, "ready result crossed closure");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sidecar_transport_reply_ready_during_closure_is_not_published() {
        const CHILD: &str = "SPUR_S6_SIDECAR_BOUNDARY_CHILD";
        if std::env::var_os(CHILD).is_some() {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener =
                tokio::net::UnixListener::bind(std::env::var_os("SPUR_EMBED_SOCKET").unwrap())
                    .unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = stream.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..n]).contains("embed"));
                let response = serde_json::json!({"v": 1, "vectors": [vec![1.0; EMBEDDING_VECTOR_DIMENSIONS]]});
                stream
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .unwrap();
            });
            let runtime = OwnedEmbeddingRuntime::new();
            let mut close = Box::pin(runtime.shutdown());
            let mut transport = Box::pin(super::super::sidecar_client::embed_query(
                "query",
                super::super::runtime::EMBED_INFERENCE_TIMEOUT,
            ));
            let result = runtime
                .sidecar_operation(std::future::poll_fn(|cx| {
                    match transport.as_mut().poll(cx) {
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(result) => {
                            assert!(result.is_some(), "real sidecar reply must be valid");
                            assert!(matches!(close.as_mut().poll(cx), Poll::Pending));
                            Poll::Ready(result)
                        }
                    }
                }))
                .await;
            close.await.unwrap();
            server.await.unwrap();
            assert!(result.is_none(), "sidecar result crossed closure");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "embedding::owned_runtime::completion_tests::sidecar_transport_reply_ready_during_closure_is_not_published", "--nocapture"])
            .env(CHILD, "1")
            .env("SPUR_EMBED_SOCKET", dir.path().join("sidecar.sock"))
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn result_completed_before_shutdown_is_preserved() {
        let runtime = OwnedEmbeddingRuntime::new();
        let completed = runtime.sidecar_operation(async { 42 }).await;
        runtime.shutdown().await.unwrap();
        assert_eq!(completed, Some(42));
    }

    #[tokio::test]
    async fn owner_retains_injected_cleanup_reap_report_error() {
        const CHILD: &str = "SPUR_S6_REAP_REPORT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let runtime = OwnedEmbeddingRuntime::new();
            runtime.warm().await.unwrap();
            runtime
                .owner
                .core
                .state
                .lock()
                .unwrap()
                .model
                .as_ref()
                .unwrap()
                .test_inject_reap_failure();
            let clone = runtime.clone();
            let query = tokio::spawn(async move { clone.embed_query("pending").await });
            let log = std::env::var("SPUR_GRAPH_TEST_WORKER_LOG").unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let events = std::fs::read_to_string(&log).unwrap_or_default();
                if events
                    .lines()
                    .filter(|line| line.starts_with("embed "))
                    .count()
                    == 2
                {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "query not received: {events}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            query.abort();
            assert!(query.await.unwrap_err().is_cancelled());
            let first = format!(
                "{:#}",
                runtime
                    .shutdown()
                    .await
                    .expect_err("injected cleanup error")
            );
            let second = format!(
                "{:#}",
                runtime
                    .shutdown()
                    .await
                    .expect_err("retained cleanup error")
            );
            assert!(
                first.contains("injected embedding child wait failure"),
                "{first}"
            );
            assert_eq!(first, second);
            println!("first={first}; repeated={second}");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/embedding/fixtures/owned_fault_worker.py");
        let config = serde_json::json!({
            "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
            "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
            "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
            "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
        });
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "embedding::owned_runtime::completion_tests::owner_retains_injected_cleanup_reap_report_error", "--nocapture"])
            .env(CHILD, "1")
            .env("SPUR_ANALYST_EMBED_MODE", "inprocess")
            .env("SPUR_GRAPH_TEST_WORKER_LOG", dir.path().join("worker.log"))
            .env("SPUR_GRAPH_TEST_WORKER_FAULT", "query_hang_after_warm")
            .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
            .output().unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
