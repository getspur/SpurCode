use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use hf_hub::{api::sync::ApiBuilder, Repo, RepoType};
use std::{env, fs, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = env::args().collect();
    let cache = PathBuf::from(&args[2]);
    assert!(ort::init_from(env!("SPUR_BUNDLED_ORT_PATH")).commit()?);
    let mut model = match args[1].as_str() {
        "NomicEmbedTextV15" | "JinaEmbeddingsV2BaseCode" => {
            let model = if args[1] == "NomicEmbedTextV15" {
                EmbeddingModel::NomicEmbedTextV15
            } else {
                EmbeddingModel::JinaEmbeddingsV2BaseCode
            };
            TextEmbedding::try_new(
                InitOptions::new(model)
                    .with_max_length(8192)
                    .with_show_download_progress(false)
                    .with_cache_dir(cache),
            )?
        }
        "CodeRankEmbed" => {
            let repo = ApiBuilder::from_env()
                .with_cache_dir(cache)
                .with_progress(false)
                .with_retries(3)
                .build()?
                .repo(Repo::with_revision(
                    "jamie8johnson/CodeRankEmbed-onnx".into(),
                    RepoType::Model,
                    "151669b173750250e611e1be00c812f112ea6020".into(),
                ));
            let read = |name| -> anyhow::Result<Vec<u8>> { Ok(fs::read(repo.get(name)?)?) };
            let files = TokenizerFiles {
                tokenizer_file: read("tokenizer.json")?,
                config_file: read("config.json")?,
                special_tokens_map_file: read("special_tokens_map.json")?,
                tokenizer_config_file: read("tokenizer_config.json")?,
            };
            let model = UserDefinedEmbeddingModel::new(read("onnx/model.onnx")?, files)
                .with_pooling(Pooling::Mean);
            TextEmbedding::try_new_from_user_defined(
                model,
                InitOptionsUserDefined::new().with_max_length(8192),
            )?
        }
        _ => anyhow::bail!("unknown model"),
    };
    let inputs: Vec<String> = serde_json::from_slice(&fs::read(&args[3])?)?;
    let vectors = model.embed(inputs, None)?;
    drop(model);
    ort::environment::try_shutdown().map_err(|e| anyhow::anyhow!("ORT shutdown: {e:?}"))?;
    fs::write(&args[4], serde_json::to_vec(&vectors)?)?;
    Ok(())
}
