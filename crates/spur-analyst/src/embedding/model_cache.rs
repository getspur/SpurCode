#[cfg(feature = "embed")]
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "embed")]
use spur_graph::{store::lance_sections::EmbeddingModelHandle, EmbeddingModelSelection};

#[cfg(feature = "embed")]
static NOMIC_EMBED_TEXT_V15_MODEL: EmbedModelCell<EmbeddingModelHandle> = EmbedModelCell::new();
#[cfg(feature = "embed")]
static CODE_RANK_EMBED_MODEL: EmbedModelCell<EmbeddingModelHandle> = EmbedModelCell::new();
#[cfg(feature = "embed")]
static JINA_EMBEDDINGS_V2_BASE_CODE_MODEL: EmbedModelCell<EmbeddingModelHandle> =
    EmbedModelCell::new();

#[cfg(feature = "embed")]
pub(crate) struct EmbedModelCell<M> {
    model: OnceLock<Arc<Mutex<M>>>,
    loading: Mutex<bool>,
}

#[cfg(feature = "embed")]
pub(crate) struct EmbedLoadPermit<'a, M> {
    cell: &'a EmbedModelCell<M>,
    completed: bool,
}

#[cfg(feature = "embed")]
impl<M> EmbedModelCell<M> {
    pub(crate) const fn new() -> Self {
        Self {
            model: OnceLock::new(),
            loading: Mutex::new(false),
        }
    }

    pub(crate) fn ready(&self) -> Option<Arc<Mutex<M>>> {
        self.model.get().cloned()
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.model.get().is_some()
    }

    pub(crate) fn begin_load(&self) -> Option<EmbedLoadPermit<'_, M>> {
        if self.is_ready() {
            return None;
        }

        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.is_ready() || *loading {
            return None;
        }

        *loading = true;
        Some(EmbedLoadPermit {
            cell: self,
            completed: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn is_loading_for_test(&self) -> bool {
        *self
            .loading
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn load_if_idle(&self, load: impl FnOnce() -> Option<M>) -> Option<Arc<Mutex<M>>> {
        if let Some(model) = self.ready() {
            return Some(model);
        }

        let permit = self.begin_load()?;
        permit.complete(load())
    }

    fn clear_loading(&self) {
        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *loading = false;
    }
}

#[cfg(feature = "embed")]
impl<M> EmbedLoadPermit<'_, M> {
    pub(crate) fn complete(mut self, model: Option<M>) -> Option<Arc<Mutex<M>>> {
        if let Some(model) = model {
            let _ = self.cell.model.set(Arc::new(Mutex::new(model)));
        }
        self.cell.clear_loading();
        self.completed = true;
        self.cell.ready()
    }
}

#[cfg(feature = "embed")]
impl<M> Drop for EmbedLoadPermit<'_, M> {
    fn drop(&mut self) {
        if !self.completed {
            self.cell.clear_loading();
        }
    }
}

#[cfg(feature = "embed")]
pub(crate) fn embed_model_cell(
    embedding_model: EmbeddingModelSelection,
) -> &'static EmbedModelCell<EmbeddingModelHandle> {
    match embedding_model {
        EmbeddingModelSelection::NomicEmbedTextV15 => &NOMIC_EMBED_TEXT_V15_MODEL,
        EmbeddingModelSelection::CodeRankEmbed => &CODE_RANK_EMBED_MODEL,
        EmbeddingModelSelection::JinaEmbeddingsV2BaseCode => &JINA_EMBEDDINGS_V2_BASE_CODE_MODEL,
    }
}
