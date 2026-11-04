use std::path::PathBuf;
use std::sync::Arc;

#[cfg(any(not(feature = "embed"), not(unix)))]
use anyhow::bail;
#[cfg(all(feature = "embed", unix))]
use anyhow::Context as _;
use anyhow::Result;
#[cfg(unix)]
use serde_json::{json, Value};
#[cfg(unix)]
use spur_graph::{
    embedding_query_text_for_model, EmbeddingModelSelection, EMBEDDING_VECTOR_DIMENSIONS,
};

use super::protocol;
#[cfg(unix)]
use super::protocol::{EmbedRequest, PROTOCOL_VERSION};
use super::sidecar_blocking::BlockingBackend;
#[cfg(all(feature = "embed", unix))]
use super::OwnedEmbeddingRuntime;

pub const SPUR_EMBED_SOCKET_ENV: &str = protocol::SPUR_EMBED_SOCKET_ENV;
pub const MAX_EMBED_TEXTS: usize = 16;

type Ready = dyn Fn() -> bool + Send + Sync;

#[derive(Clone)]
enum EmbedBackend {
    Blocking(Arc<BlockingBackend>),
    #[cfg(all(feature = "embed", unix))]
    Owned(OwnedEmbeddingRuntime, Arc<std::sync::atomic::AtomicBool>),
}

#[derive(Clone)]
// On non-unix the socket serving path is compiled out, so the fields are
// written by `new` but never read (the connection handler is unix-only).
#[cfg_attr(not(unix), allow(dead_code))]
pub struct EmbedService {
    model_name: Arc<str>,
    ready: Arc<Ready>,
    embedder: EmbedBackend,
}

impl EmbedService {
    /// The supplied blocking embedder must eventually return: shutdown joins
    /// admitted callbacks and cannot preempt an arbitrary blocking callback.
    pub fn new<M, R, E>(model_name: M, ready: R, embedder: E) -> Self
    where
        M: Into<String>,
        R: Fn() -> bool + Send + Sync + 'static,
        E: Fn(Vec<String>) -> Result<Vec<Vec<f32>>, String> + Send + Sync + 'static,
    {
        Self {
            model_name: Arc::from(model_name.into()),
            ready: Arc::new(ready),
            embedder: EmbedBackend::Blocking(Arc::new(BlockingBackend::new(embedder))),
        }
    }

    #[cfg(all(feature = "embed", unix))]
    fn owned(runtime: OwnedEmbeddingRuntime) -> Self {
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready_for_ping = Arc::clone(&ready);
        Self {
            model_name: Arc::from(EmbeddingModelSelection::from_env().model_name()),
            ready: Arc::new(move || ready_for_ping.load(std::sync::atomic::Ordering::Acquire)),
            embedder: EmbedBackend::Owned(runtime, ready),
        }
    }

    #[cfg(unix)]
    pub async fn serve_socket(self, socket_path: PathBuf) -> Result<()> {
        super::sidecar_transport::serve(self, socket_path, std::future::pending()).await
    }

    #[cfg(unix)]
    pub async fn serve_socket_until(
        self,
        socket_path: PathBuf,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> Result<()> {
        super::sidecar_transport::serve(self, socket_path, shutdown).await
    }

    #[cfg(unix)]
    pub(super) async fn close_blocking(&self) -> Result<()> {
        match &self.embedder {
            EmbedBackend::Blocking(backend) => backend.close().await.map_err(anyhow::Error::msg),
            #[cfg(feature = "embed")]
            EmbedBackend::Owned(_, _) => Ok(()),
        }
    }

    /// The sidecar transport is a unix domain socket; on other platforms an
    /// explicit serve is rejected (clients degrade to in-process embedding).
    #[cfg(not(unix))]
    pub async fn serve_socket(self, socket_path: PathBuf) -> Result<()> {
        bail!(
            "embed sidecar socket {} unsupported: unix domain sockets are unavailable on this platform",
            socket_path.display()
        )
    }

    #[cfg(unix)]
    pub(super) async fn handle_line(&self, line: &str) -> Value {
        let request: EmbedRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                return error_response(None, format!("malformed request: {error}"));
            }
        };

        if request.v != Some(PROTOCOL_VERSION) {
            return error_response(
                request.id,
                format!("unsupported protocol version: {:?}", request.v),
            );
        }

        match request.op.as_str() {
            "ping" => self.handle_ping(request.id),
            "embed" => self.handle_embed(request.id, request.texts).await,
            other => error_response(request.id, format!("unknown op: {other}")),
        }
    }

    #[cfg(unix)]
    fn handle_ping(&self, id: Option<Value>) -> Value {
        json!({
            "v": PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "model": self.model_name.as_ref(),
            "ready": (self.ready)(),
        })
    }

    #[cfg(unix)]
    async fn handle_embed(&self, id: Option<Value>, texts: Option<Vec<String>>) -> Value {
        let Some(texts) = texts else {
            return error_response(id, "embed request missing texts");
        };
        if texts.len() > MAX_EMBED_TEXTS {
            return error_response(
                id,
                format!("embed request accepts at most {MAX_EMBED_TEXTS} texts"),
            );
        }

        let embedding_model = EmbeddingModelSelection::from_env();
        let vectors = match &self.embedder {
            EmbedBackend::Blocking(embedder) => {
                let normalized = texts
                    .iter()
                    .map(|text| embedding_query_text_for_model(text, embedding_model).into_owned())
                    .collect::<Vec<_>>();
                match embedder.embed(normalized).await {
                    Ok(vectors) => vectors,
                    Err(error) => return error_response(id, error),
                }
            }
            #[cfg(feature = "embed")]
            EmbedBackend::Owned(runtime, ready) => {
                let mut vectors = Vec::with_capacity(texts.len());
                for text in &texts {
                    match runtime.embed_local_query(text).await {
                        Ok(vector) => vectors.push(vector.to_vec()),
                        Err(error) => {
                            ready.store(false, std::sync::atomic::Ordering::Release);
                            return error_response(id, format!("{error:#}"));
                        }
                    }
                }
                if !texts.is_empty() {
                    ready.store(true, std::sync::atomic::Ordering::Release);
                }
                vectors
            }
        };

        if vectors.len() != texts.len() {
            return error_response(
                id,
                format!(
                    "embedder returned {} vectors for {} texts",
                    vectors.len(),
                    texts.len()
                ),
            );
        }
        if vectors
            .iter()
            .any(|vector| vector.len() != EMBEDDING_VECTOR_DIMENSIONS)
        {
            return error_response(
                id,
                format!("embedder returned vector with dimension other than {EMBEDDING_VECTOR_DIMENSIONS}"),
            );
        }

        json!({
            "v": PROTOCOL_VERSION,
            "id": id,
            "vectors": vectors,
        })
    }
}

pub fn resolve_socket_path(socket: Option<PathBuf>) -> Result<PathBuf> {
    protocol::resolve_socket_path(socket)
}

#[cfg(feature = "embed")]
pub async fn serve(socket: Option<PathBuf>) -> Result<()> {
    serve_until(socket, shutdown_signal()).await
}

#[cfg(all(feature = "embed", unix))]
async fn shutdown_signal() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if let Err(error) = result { tracing::error!(%error, "embed service interrupt listener failed"); }
                }
                _ = terminate.recv() => {}
            }
        }
        Err(error) => {
            tracing::error!(%error, "embed service termination listener failed");
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "embed service interrupt listener failed");
            }
        }
    }
}

#[cfg(all(feature = "embed", not(unix)))]
async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "embed service interrupt listener failed");
    }
}

#[cfg(all(feature = "embed", unix))]
pub async fn serve_until(
    socket: Option<PathBuf>,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let runtime = OwnedEmbeddingRuntime::local_helper();
    let result = match resolve_socket_path(socket) {
        Ok(path) => {
            super::sidecar_transport::serve(EmbedService::owned(runtime.clone()), path, shutdown)
                .await
        }
        Err(error) => Err(error),
    };
    runtime
        .shutdown()
        .await
        .context("embed service shutdown failed")?;
    result
}

#[cfg(all(feature = "embed", not(unix)))]
pub async fn serve_until(
    _socket: Option<PathBuf>,
    _shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    bail!("embed sidecar socket unsupported: unix domain sockets are unavailable")
}

#[cfg(not(feature = "embed"))]
pub async fn serve(_socket: Option<PathBuf>) -> Result<()> {
    bail!("spur embed serve requires a binary built with the `embed` feature")
}

#[cfg(unix)]
fn error_response(id: Option<Value>, error: impl Into<String>) -> Value {
    json!({
        "v": PROTOCOL_VERSION,
        "id": id,
        "error": error.into(),
    })
}

#[cfg(all(test, feature = "embed", unix))]
mod real_acceptance {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an explicitly configured real helper and model cache"]
    async fn warm_query_and_v1_multi_text_share_owned_runtime() {
        let query = std::env::var("SPUR_S6C_QUERY").expect("explicit query");
        let output = std::env::var("SPUR_S6C_OUTPUT").expect("explicit output");
        let mut cycles = Vec::new();
        for cycle in 0..2 {
            let runtime = OwnedEmbeddingRuntime::new();
            runtime.warm().await.expect("real warm inference");
            let first = runtime.embed_query(&query).await.expect("real query");
            let service = EmbedService::owned(runtime.clone());
            let request = json!({"v": 1, "id": "batch", "op": "embed", "texts": [query, query]});
            let batch = service.handle_line(&request.to_string()).await;
            assert!(batch.get("error").is_none(), "{batch}");
            let rows = batch["vectors"].as_array().expect("v1 batch vectors");
            assert_eq!(rows.len(), 2);
            let rows = rows.clone();
            drop(service);
            runtime.shutdown().await.expect("checked owned shutdown");
            cycles.push(json!({"cycle": cycle, "query": first.to_vec(), "batch": rows}));
            std::fs::write(&output, serde_json::to_vec_pretty(&cycles).unwrap()).unwrap();
        }
    }
}
