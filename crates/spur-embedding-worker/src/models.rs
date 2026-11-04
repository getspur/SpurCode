use crate::model_artifacts::ResolvedArtifacts;
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};

pub struct ModelCache {
    artifacts: Option<ResolvedArtifacts>,
    model: Option<TextEmbedding>,
}

impl ModelCache {
    pub fn new(artifacts: ResolvedArtifacts) -> Self {
        Self {
            artifacts: Some(artifacts),
            model: None,
        }
    }

    pub fn embed(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if self.model.is_none() {
            let bytes = self
                .artifacts
                .take()
                .ok_or_else(|| anyhow::anyhow!("model load already failed"))?
                .verified_bytes()?;
            let [onnx_file, config_file, tokenizer_file, tokenizer_config_file, special_tokens_map_file]: [Vec<u8>; 5] = bytes.try_into().map_err(|_| anyhow::anyhow!("invalid artifact count"))?;
            let files = TokenizerFiles {
                tokenizer_file,
                config_file,
                special_tokens_map_file,
                tokenizer_config_file,
            };
            let model =
                UserDefinedEmbeddingModel::new(onnx_file, files).with_pooling(Pooling::Mean);
            self.model = Some(TextEmbedding::try_new_from_user_defined(
                model,
                InitOptionsUserDefined::new().with_max_length(8192),
            )?);
            #[cfg(feature = "test-support")]
            eprintln!("phase: model_loaded");
        } else {
            #[cfg(feature = "test-support")]
            eprintln!("phase: model_reused");
        }
        self.model
            .as_mut()
            .expect("model initialized")
            .embed(texts.to_vec(), None)
    }

    pub fn clear(&mut self) {
        self.model.take();
        self.artifacts.take();
        #[cfg(feature = "test-support")]
        eprintln!("phase: model_dropped");
    }
}
