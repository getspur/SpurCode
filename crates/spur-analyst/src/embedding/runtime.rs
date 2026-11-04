use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use spur_graph::EMBEDDING_VECTOR_DIMENSIONS;

use super::OwnedEmbeddingRuntime;

pub(super) const EMBED_INFERENCE_TIMEOUT: Duration = Duration::from_millis(1500);
const AUTO_SIDECAR_PING_TIMEOUT: Duration = Duration::from_millis(100);
const AUTO_SIDECAR_UNAVAILABLE_TTL: Duration = Duration::from_secs(3);

static GLOBAL_RUNTIME: EmbeddingRuntime = EmbeddingRuntime;
static AUTO_SIDECAR_PROBE_CACHE: OnceLock<Mutex<Option<AutoSidecarProbeCache>>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct AutoSidecarProbeCache {
    reachable: bool,
    checked_at: Instant,
}

/// Stateless compatibility facade. Every query owns and joins its operation scope.
pub struct EmbeddingRuntime;

impl EmbeddingRuntime {
    pub fn global() -> &'static Self {
        &GLOBAL_RUNTIME
    }

    pub async fn embed_query(&self, query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        let runtime = OwnedEmbeddingRuntime::new();
        let result = runtime.embed_query(query).await;
        if let Err(error) = runtime.shutdown().await {
            tracing::error!(%error, "embedding compatibility shutdown failed");
        }
        result
    }
}

/// Warm within an explicit host-owned lifetime.
pub async fn warm_embed_model(runtime: &OwnedEmbeddingRuntime) -> anyhow::Result<()> {
    runtime.warm().await
}

fn auto_sidecar_probe_cache() -> &'static Mutex<Option<AutoSidecarProbeCache>> {
    AUTO_SIDECAR_PROBE_CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_auto_sidecar_reachable(now: Instant) -> Option<bool> {
    let cache = auto_sidecar_probe_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cached = (*cache)?;
    if cached.reachable {
        return Some(true);
    }

    let age = now
        .checked_duration_since(cached.checked_at)
        .unwrap_or(Duration::ZERO);
    (age < AUTO_SIDECAR_UNAVAILABLE_TTL).then_some(false)
}

fn record_auto_sidecar_probe(reachable: bool) {
    let mut cache = auto_sidecar_probe_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache = Some(AutoSidecarProbeCache {
        reachable,
        checked_at: Instant::now(),
    });
}

#[cfg(test)]
pub(crate) fn reset_auto_sidecar_probe_for_test() {
    let mut cache = auto_sidecar_probe_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache = None;
}

pub(super) async fn auto_sidecar_reachable() -> bool {
    // Cache policy: a reachable sidecar is sticky for the process so serving
    // processes keep sharing it and never fall back to loading their own model.
    // Misses are cached briefly to avoid adding a socket probe to every BM25
    // fallback query while still letting a newly started sidecar be discovered.
    if let Some(reachable) = cached_auto_sidecar_reachable(Instant::now()) {
        return reachable;
    }

    let reachable = super::sidecar_client::ping(AUTO_SIDECAR_PING_TIMEOUT).await;
    record_auto_sidecar_probe(reachable);
    reachable
}
