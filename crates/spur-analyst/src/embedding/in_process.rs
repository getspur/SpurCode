use std::time::Duration;
#[cfg(feature = "embed")]
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use spur_graph::EMBEDDING_VECTOR_DIMENSIONS;
#[cfg(feature = "embed")]
use spur_graph::store::lance_sections::EmbeddingModelHandle;
#[cfg(feature = "embed")]
use spur_graph::{
    embedding_query_text_for_model, load_embedding_model, EmbeddingModelSelection, EMBED_MODEL_ENV,
};

#[cfg(feature = "embed")]
use super::{
    mode::AnalystEmbedMode,
    model_cache::{embed_model_cell, EmbedModelCell},
};

#[cfg(all(test, feature = "embed"))]
static DISABLE_EMBED_QUERY_FOR_TESTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "embed")]
pub(crate) fn load_embed_model(
    embedding_model: EmbeddingModelSelection,
) -> Result<EmbeddingModelHandle, String> {
    tracing::info!(
        model = embedding_model.model_name(),
        "Loading embedding model for knowledge_context_pack hybrid search"
    );
    load_embedding_model(embedding_model, false).map_err(|error| error.to_string())
}

#[cfg(feature = "embed")]
fn start_embed_model_load_if_needed(embedding_model: EmbeddingModelSelection) -> bool {
    if !AnalystEmbedMode::current().allows_in_process("start_embed_model_load_if_needed") {
        return false;
    }

    let Some(permit) = embed_model_cell(embedding_model).begin_load() else {
        return false;
    };

    let spawn_result = std::thread::Builder::new()
        .name("spur-analyst-embed-warm".into())
        .spawn(move || {
            tracing::info!(
                model = embedding_model.model_name(),
                "Pre-warming embedding model for knowledge_context_pack"
            );
            let load_result = load_embed_model(embedding_model);
            match load_result {
                Ok(model) => {
                    let _ = permit.complete(Some(model));
                    tracing::info!(
                        model = embedding_model.model_name(),
                        "embedding model loaded successfully"
                    );
                }
                Err(error) => {
                    let _ = permit.complete(None);
                    tracing::warn!(
                        %error,
                        model = embedding_model.model_name(),
                        "embedding model failed to load; will retry on a later warm or query"
                    );
                }
            }
        });

    match spawn_result {
        Ok(_handle) => true,
        Err(error) => {
            tracing::warn!(
                %error,
                model = embedding_model.model_name(),
                "failed to spawn embedding model warm-up thread"
            );
            false
        }
    }
}

#[cfg(feature = "embed")]
pub(crate) fn warm_embed_model_in_process() {
    warm_embed_model_in_process_for_model(EmbeddingModelSelection::from_env());
}

#[cfg(feature = "embed")]
pub(crate) fn warm_embed_model_in_process_for_model(embedding_model: EmbeddingModelSelection) {
    if !start_embed_model_load_if_needed(embedding_model) {
        tracing::debug!(
            model = embedding_model.model_name(),
            "embedding model warm-up skipped; already ready or loading"
        );
    }
}

#[cfg(feature = "embed")]
pub(crate) fn embed_model_ready(embedding_model: EmbeddingModelSelection) -> bool {
    embed_model_cell(embedding_model).ready().is_some()
}

#[cfg(feature = "embed")]
pub(crate) fn embed_texts_in_process(
    embedding_model: EmbeddingModelSelection,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    let model = ready_or_load_model(embedding_model)?;
    let refs = texts.iter().map(String::as_str).collect::<Vec<_>>();
    let mut model = model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    model.embed(&refs, None).map_err(|error| error.to_string())
}

#[cfg(feature = "embed")]
fn ready_or_load_model(
    embedding_model: EmbeddingModelSelection,
) -> Result<Arc<Mutex<EmbeddingModelHandle>>, String> {
    let cell = embed_model_cell(embedding_model);
    loop {
        if let Some(model) = cell.ready() {
            return Ok(model);
        }

        if let Some(permit) = cell.begin_load() {
            let loaded = load_embed_model(embedding_model);
            return match loaded {
                Ok(model) => permit
                    .complete(Some(model))
                    .ok_or_else(|| "embedding model loaded but was not available".to_owned()),
                Err(error) => {
                    let _ = permit.complete(None);
                    Err(error)
                }
            };
        }

        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(feature = "embed")]
pub(crate) async fn embed_query_in_process(
    query: &str,
    timeout_duration: Duration,
) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
    if !AnalystEmbedMode::current().allows_in_process("embed_query") {
        return None;
    }

    #[cfg(test)]
    if DISABLE_EMBED_QUERY_FOR_TESTS.load(std::sync::atomic::Ordering::SeqCst) {
        return None;
    }

    let embedding_model = EmbeddingModelSelection::from_env();
    let model_cell = embed_model_cell(embedding_model);
    if !model_cell.is_ready() {
        let load_started = start_embed_model_load_if_needed(embedding_model);
        if model_cell.is_ready() {
            return embed_query_with_ready_model(query, embedding_model, timeout_duration).await;
        }
        tracing::debug!(
            load_started,
            model = embedding_model.model_name(),
            env = EMBED_MODEL_ENV,
            "embedding model not ready; degrading to BM25-only search"
        );
        return None;
    }

    embed_query_with_ready_model(query, embedding_model, timeout_duration).await
}

#[cfg(not(feature = "embed"))]
#[expect(
    clippy::unused_async,
    reason = "the disabled stub matches the embed-enabled async signature"
)]
pub(crate) async fn embed_query_in_process(
    _query: &str,
    _timeout_duration: Duration,
) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
    tracing::debug!(
        "in-process embedding model unavailable in builds without the `embed` feature; degrading to BM25-only search"
    );
    None
}

#[cfg(feature = "embed")]
async fn embed_query_with_ready_model(
    query: &str,
    embedding_model: EmbeddingModelSelection,
    timeout_duration: Duration,
) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> {
    embed_with_ready_model(
        embed_model_cell(embedding_model),
        query,
        timeout_duration,
        move |model, query| {
            let query = embedding_query_text_for_model(query.as_str(), embedding_model);
            let mut model = model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let embeddings = model.embed(&[query.as_ref()], None).ok()?;
            let embedding = embeddings.into_iter().next()?;
            embedding.try_into().ok()
        },
    )
    .await
}

#[cfg(feature = "embed")]
pub(crate) async fn embed_with_ready_model<M, F>(
    cell: &EmbedModelCell<M>,
    query: &str,
    timeout_duration: Duration,
    inference: F,
) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]>
where
    M: Send + 'static,
    F: FnOnce(Arc<Mutex<M>>, String) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> + Send + 'static,
{
    let model = cell.ready()?;
    let query = query.to_owned();
    run_embed_inference_with_timeout(timeout_duration, move || inference(model, query)).await
}

#[cfg(feature = "embed")]
async fn run_embed_inference_with_timeout<F>(
    timeout_duration: Duration,
    inference: F,
) -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]>
where
    F: FnOnce() -> Option<[f32; EMBEDDING_VECTOR_DIMENSIONS]> + Send + 'static,
{
    let started = Instant::now();
    let result =
        tokio::time::timeout(timeout_duration, tokio::task::spawn_blocking(inference)).await;
    let elapsed = started.elapsed();
    let elapsed_ms = duration_millis(elapsed);
    let timeout_ms = duration_millis(timeout_duration);

    match result {
        Ok(Ok(Some(embedding))) => {
            tracing::debug!(
                elapsed_ms,
                timeout_ms,
                "knowledge_context_pack embed inference completed"
            );
            Some(embedding)
        }
        Ok(Ok(None)) => {
            tracing::warn!(
                elapsed_ms,
                timeout_ms,
                "knowledge_context_pack embed inference failed; degrading to BM25-only search"
            );
            None
        }
        Ok(Err(error)) => {
            tracing::warn!(
                %error,
                elapsed_ms,
                timeout_ms,
                "knowledge_context_pack embed inference task failed; degrading to BM25-only search"
            );
            None
        }
        Err(_timeout) => {
            tracing::warn!(
                elapsed_ms,
                timeout_ms,
                "knowledge_context_pack embed inference timed out; degrading to BM25-only search"
            );
            None
        }
    }
}

#[cfg(feature = "embed")]
fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(all(test, feature = "embed"))]
pub(crate) fn set_embed_query_disabled_for_test(disabled: bool) -> bool {
    DISABLE_EMBED_QUERY_FOR_TESTS.swap(disabled, std::sync::atomic::Ordering::SeqCst)
}
