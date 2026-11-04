use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(feature = "embed")]
use spur_graph::EmbeddingModelSelection;
use spur_graph::EMBEDDING_VECTOR_DIMENSIONS;

use super::{in_process, mode::AnalystEmbedMode};

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

pub struct EmbeddingRuntime;

impl EmbeddingRuntime {
    pub fn global() -> &'static Self {
        &GLOBAL_RUNTIME
    }

    #[cfg(feature = "embed")]
    pub fn warm(&self) {
        match AnalystEmbedMode::current() {
            AnalystEmbedMode::Off => {}
            AnalystEmbedMode::Sidecar => {
                tracing::debug!(
                    mode = "sidecar",
                    "embedding model warm-up skipped; sidecar mode never loads in-process"
                );
            }
            AnalystEmbedMode::InProcess => in_process::warm_embed_model_in_process(),
            AnalystEmbedMode::Auto => warm_embed_model_auto(),
        }
    }

    #[cfg(not(feature = "embed"))]
    pub fn warm(&self) {}

    pub async fn embed_query(&self, query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
        match AnalystEmbedMode::current() {
            AnalystEmbedMode::Off => None,
            AnalystEmbedMode::Sidecar => embed_query_with_sidecar(query).await,
            AnalystEmbedMode::Auto => {
                if auto_sidecar_reachable().await {
                    return embed_query_with_sidecar(query).await;
                }
                in_process::embed_query_in_process(query, EMBED_INFERENCE_TIMEOUT).await
            }
            AnalystEmbedMode::InProcess => {
                in_process::embed_query_in_process(query, EMBED_INFERENCE_TIMEOUT).await
            }
        }
    }

    #[cfg(feature = "embed")]
    #[expect(
        clippy::unused_self,
        reason = "EmbeddingRuntime owns the sidecar service boundary"
    )]
    pub(crate) fn sidecar_model_name(&self) -> &'static str {
        EmbeddingModelSelection::from_env().model_name()
    }

    #[cfg(feature = "embed")]
    #[expect(
        clippy::unused_self,
        reason = "EmbeddingRuntime owns the sidecar service boundary"
    )]
    pub(crate) fn sidecar_ready(&self) -> bool {
        in_process::embed_model_ready(EmbeddingModelSelection::from_env())
    }

    #[cfg(feature = "embed")]
    #[expect(
        clippy::unused_self,
        reason = "EmbeddingRuntime owns the sidecar service boundary"
    )]
    pub(crate) fn embed_sidecar_texts(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        in_process::embed_texts_in_process(EmbeddingModelSelection::from_env(), texts)
    }
}

pub fn warm_embed_model() {
    EmbeddingRuntime::global().warm();
}

#[cfg(feature = "embed")]
fn warm_embed_model_auto() {
    match cached_auto_sidecar_reachable(Instant::now()) {
        Some(true) => {
            tracing::debug!("embedding model warm-up skipped; cached sidecar probe is reachable");
        }
        Some(false) => in_process::warm_embed_model_in_process(),
        None => {
            let embedding_model = EmbeddingModelSelection::from_env();
            let spawn_result = std::thread::Builder::new()
                .name("spur-analyst-embed-sidecar-probe".into())
                .spawn(move || {
                    let reachable = ping_sidecar_blocking(AUTO_SIDECAR_PING_TIMEOUT);
                    record_auto_sidecar_probe(reachable);
                    if reachable {
                        tracing::debug!(
                            "embedding model warm-up skipped; sidecar probe is reachable"
                        );
                    } else {
                        in_process::warm_embed_model_in_process_for_model(embedding_model);
                    }
                });

            if let Err(error) = spawn_result {
                tracing::warn!(
                    %error,
                    "failed to spawn embedding sidecar probe thread; falling back to in-process warm-up"
                );
                in_process::warm_embed_model_in_process_for_model(embedding_model);
            }
        }
    }
}

#[cfg(feature = "embed")]
fn ping_sidecar_blocking(timeout_duration: Duration) -> bool {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::debug!(
                %error,
                "failed to create runtime for embedding sidecar ping"
            );
            return false;
        }
    };
    runtime.block_on(super::sidecar_client::ping(timeout_duration))
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

async fn embed_query_with_sidecar(query: &str) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
    super::sidecar_client::embed_query(query, EMBED_INFERENCE_TIMEOUT).await
}
