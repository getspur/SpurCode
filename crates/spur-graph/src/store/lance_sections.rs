use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
#[cfg(feature = "embed")]
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context as _, Result};
use arrow_array::{
    Array as _, FixedSizeListArray, Float32Array, LargeStringArray, RecordBatch, StringArray,
    UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType, Field, Schema};
#[cfg(feature = "embed")]
use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
#[cfg(feature = "embed")]
use hf_hub::{api::sync::ApiBuilder as HfApiBuilder, Repo, RepoType};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;

use crate::content_hash::blake3_hex;
use crate::store::parquet::GraphArtifactSidecarRowCounts;
use crate::{
    GraphEdgeArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphSymbolArtifact,
    RelationKind,
};

pub const EMBED_MODEL_ENV: &str = "SPUR_EMBEDDING_MODEL";
pub const NOMIC_EMBED_TEXT_V15_MODEL_NAME: &str = "NomicEmbedTextV15";
pub const NOMIC_EMBED_TEXT_V15_APPROX_SIZE_MB: usize = 550;
pub const CODE_RANK_EMBED_MODEL_NAME: &str = "CodeRankEmbed";
pub const CODE_RANK_EMBED_APPROX_SIZE_MB: usize = 548;
pub const JINA_EMBEDDINGS_V2_BASE_CODE_MODEL_NAME: &str = "JinaEmbeddingsV2BaseCode";
pub const JINA_EMBEDDINGS_V2_BASE_CODE_APPROX_SIZE_MB: usize = 642;
pub const SECTIONS_DATASET_DIR: &str = "sections.lancedb";
pub const SECTIONS_TABLE: &str = "section_bodies";
pub const CODE_SYMBOLS_DATASET_DIR: &str = "code_symbols.lance";
pub const CODE_SYMBOLS_TABLE: &str = "code_symbols";
pub const SECTIONS_PARQUET: &str = "sections.parquet";
pub const CODE_SYMBOLS_PARQUET: &str = "code_symbols.parquet";
pub const EMBEDDING_VECTOR_DIMENSIONS: usize = 768;
const SECTION_EMBED_MAX_BODY_BYTES: usize = 4096;
const SECTION_EMBED_BATCH_SIZE_DEFAULT: usize = 64;
const SECTION_EMBED_BATCH_SIZE_ENV: &str = "SPUR_GRAPH_SECTION_EMBED_BATCH_SIZE";
pub const SECTION_EMBED_SKIP_ENV: &str = "SPUR_GRAPH_SKIP_SECTION_EMBEDDINGS";
pub const CODE_SYMBOL_EMBED_SKIP_ENV: &str = "SPUR_GRAPH_SKIP_CODE_SYMBOL_EMBEDDINGS";
const SECTION_WRITE_BATCH_SIZE_DEFAULT: usize = 512;
const SECTION_WRITE_BATCH_SIZE_ENV: &str = "SPUR_GRAPH_SECTION_WRITE_BATCH_SIZE";
const NOMIC_EMBED_TEXT_V15_SECTION_TEXT_VERSION: &str = "v5-nomic-embed-text-v1.5-search-document";
const NOMIC_EMBED_TEXT_V15_SYMBOL_TEXT_VERSION: &str = "v4-nomic-embed-text-v1.5-search-document";
const CODE_RANK_EMBED_SECTION_TEXT_VERSION: &str =
    "v1-coderank-embed-query-instruction-document-raw";
const CODE_RANK_EMBED_SYMBOL_TEXT_VERSION: &str =
    "v1-coderank-embed-query-instruction-document-raw";
const JINA_EMBEDDINGS_V2_BASE_CODE_SECTION_TEXT_VERSION: &str =
    "v1-jina-embeddings-v2-base-code-raw";
const JINA_EMBEDDINGS_V2_BASE_CODE_SYMBOL_TEXT_VERSION: &str =
    "v1-jina-embeddings-v2-base-code-raw";
#[cfg(feature = "embed")]
const CODE_RANK_EMBED_ONNX_REPO: &str = "jamie8johnson/CodeRankEmbed-onnx";
#[cfg(feature = "embed")]
const CODE_RANK_EMBED_ONNX_REVISION: &str = "151669b173750250e611e1be00c812f112ea6020";
#[cfg(feature = "embed")]
const CODE_RANK_EMBED_ONNX_FILE: &str = "onnx/model.onnx";
const EMBEDDING_INPUT_HASH_COLUMN: &str = "embedding_input_hash";
const EMBEDDING_MODEL_COLUMN: &str = "embedding_model";
// Integration tests spawn the debug-built CLI; keep this hook out of release builds.
#[cfg(debug_assertions)]
const SECTION_SIDECAR_TEST_FAIL_ENV: &str = "SPUR_GRAPH_TEST_FAIL_SECTION_SIDECAR";

#[cfg(feature = "embed")]
static NOMIC_EMBED_TEXT_V15_MODEL: OnceLock<Option<Mutex<TextEmbedding>>> = OnceLock::new();
#[cfg(feature = "embed")]
static CODE_RANK_EMBED_MODEL: OnceLock<Option<Mutex<TextEmbedding>>> = OnceLock::new();
#[cfg(feature = "embed")]
static JINA_EMBEDDINGS_V2_BASE_CODE_MODEL: OnceLock<Option<Mutex<TextEmbedding>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingModelSelection {
    NomicEmbedTextV15,
    CodeRankEmbed,
    JinaEmbeddingsV2BaseCode,
}

impl EmbeddingModelSelection {
    pub fn from_env() -> Self {
        std::env::var(EMBED_MODEL_ENV)
            .ok()
            .and_then(|value| Self::parse(&value))
            .or_else(|| embedding_model_from_spur_config().and_then(|value| Self::parse(&value)))
            .unwrap_or(Self::NomicEmbedTextV15)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>();
        match normalized.as_str() {
            "" | "nomic" | "nomicembedtext" | "nomicembedtextv15" | "nomicainomicembedtextv15" => {
                Some(Self::NomicEmbedTextV15)
            }
            "coderank" | "coderankembed" | "nomiccoderank" | "nomicaicoderankembed" => {
                Some(Self::CodeRankEmbed)
            }
            "jina" | "jinacode" | "jinaembeddingsv2basecode" | "jinaaijinaembeddingsv2basecode" => {
                Some(Self::JinaEmbeddingsV2BaseCode)
            }
            _ => None,
        }
    }

    pub fn model_name(self) -> &'static str {
        match self {
            Self::NomicEmbedTextV15 => NOMIC_EMBED_TEXT_V15_MODEL_NAME,
            Self::CodeRankEmbed => CODE_RANK_EMBED_MODEL_NAME,
            Self::JinaEmbeddingsV2BaseCode => JINA_EMBEDDINGS_V2_BASE_CODE_MODEL_NAME,
        }
    }

    pub fn approximate_size_mb(self) -> usize {
        match self {
            Self::NomicEmbedTextV15 => NOMIC_EMBED_TEXT_V15_APPROX_SIZE_MB,
            Self::CodeRankEmbed => CODE_RANK_EMBED_APPROX_SIZE_MB,
            Self::JinaEmbeddingsV2BaseCode => JINA_EMBEDDINGS_V2_BASE_CODE_APPROX_SIZE_MB,
        }
    }

    pub fn dimensions(self) -> usize {
        EMBEDDING_VECTOR_DIMENSIONS
    }

    pub fn max_length(self) -> usize {
        8192
    }

    #[cfg(feature = "embed")]
    pub fn fastembed_model(self) -> Option<EmbeddingModel> {
        match self {
            Self::NomicEmbedTextV15 => Some(EmbeddingModel::NomicEmbedTextV15),
            Self::CodeRankEmbed => None,
            Self::JinaEmbeddingsV2BaseCode => Some(EmbeddingModel::JinaEmbeddingsV2BaseCode),
        }
    }

    fn section_text_version(self) -> &'static str {
        match self {
            Self::NomicEmbedTextV15 => NOMIC_EMBED_TEXT_V15_SECTION_TEXT_VERSION,
            Self::CodeRankEmbed => CODE_RANK_EMBED_SECTION_TEXT_VERSION,
            Self::JinaEmbeddingsV2BaseCode => JINA_EMBEDDINGS_V2_BASE_CODE_SECTION_TEXT_VERSION,
        }
    }

    fn symbol_text_version(self) -> &'static str {
        match self {
            Self::NomicEmbedTextV15 => NOMIC_EMBED_TEXT_V15_SYMBOL_TEXT_VERSION,
            Self::CodeRankEmbed => CODE_RANK_EMBED_SYMBOL_TEXT_VERSION,
            Self::JinaEmbeddingsV2BaseCode => JINA_EMBEDDINGS_V2_BASE_CODE_SYMBOL_TEXT_VERSION,
        }
    }
}

pub fn embedding_query_text_for_model(
    query: &str,
    embedding_model: EmbeddingModelSelection,
) -> Cow<'_, str> {
    match embedding_model {
        EmbeddingModelSelection::NomicEmbedTextV15 => Cow::Owned(format!("search_query: {query}")),
        EmbeddingModelSelection::CodeRankEmbed => Cow::Owned(format!(
            "Represent this query for searching relevant code: {query}"
        )),
        EmbeddingModelSelection::JinaEmbeddingsV2BaseCode => Cow::Borrowed(query),
    }
}

fn embedding_document_title(title: &str) -> &str {
    let title = title.trim();
    if title.is_empty() {
        "none"
    } else {
        title
    }
}

fn embedding_document_text_for_model<'a>(
    title: &'a str,
    text: &'a str,
    embedding_model: EmbeddingModelSelection,
) -> Cow<'a, str> {
    match embedding_model {
        EmbeddingModelSelection::NomicEmbedTextV15 => Cow::Owned(format!(
            "search_document: title: {} | text: {text}",
            embedding_document_title(title)
        )),
        EmbeddingModelSelection::CodeRankEmbed
        | EmbeddingModelSelection::JinaEmbeddingsV2BaseCode => Cow::Borrowed(text),
    }
}

// Vector reuse is intentionally split across two scopes: in-place incremental
// upserts skip unchanged rows within the same sidecar table, while
// `previous_artifact_dir` carries vectors forward across artifact directories.
pub type SectionSidecarProgressCallback<'a> = dyn Fn(SectionSidecarProgressEvent) + Sync + 'a;

/// Identifies which phase of the sidecar write produced a progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarPhase {
    Sections,
    CodeSymbols,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarRowScope {
    Full,
    Delta,
}

impl SidecarRowScope {
    fn from_delta_paths(delta_paths: Option<&BTreeSet<String>>) -> Self {
        if delta_paths.is_some() {
            Self::Delta
        } else {
            Self::Full
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionSidecarProgressEvent {
    Started {
        total_rows: usize,
        markdown_files: usize,
        embeddings_enabled: bool,
        embedding_batch_size: usize,
        write_batch_size: usize,
        row_scope: SidecarRowScope,
    },
    BatchStarted {
        batch_index: usize,
        batch_rows: usize,
        embedding_eligible_rows: usize,
        embeddings_available: bool,
        processed_rows: usize,
        total_rows: usize,
    },
    EmbeddingChunkStarted {
        batch_index: usize,
        batch_rows: usize,
        chunk_index: usize,
        chunk_count: usize,
        chunk_rows: usize,
        completed_eligible_rows: usize,
        embedding_eligible_rows: usize,
        processed_rows: usize,
        total_rows: usize,
    },
    BatchWritten {
        batch_index: usize,
        written_rows: usize,
        skipped_existing_rows: usize,
        processed_rows: usize,
        total_rows: usize,
    },
    ModelDownloading {
        model_name: &'static str,
        approximate_size_mb: usize,
    },
    Indexing {
        label: &'static str,
        phase: SidecarPhase,
    },
    Finished {
        total_rows: usize,
        final_rows: usize,
        written_rows: usize,
        skipped_existing_rows: usize,
        phase: SidecarPhase,
        row_scope: SidecarRowScope,
    },
    /// Signals the start of the code-symbol sidecar write phase.
    CodeSymbolsStarted {
        total_rows: usize,
        embeddings_enabled: bool,
        row_scope: SidecarRowScope,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionEmbeddingOptions {
    pub skip_section_embeddings: bool,
    pub skip_code_symbol_embeddings: bool,
    pub batch_size: usize,
}

impl SectionEmbeddingOptions {
    pub fn from_env() -> Self {
        let skip_section_embeddings = matches!(
            std::env::var(SECTION_EMBED_SKIP_ENV),
            Ok(value) if value == "1"
        );
        let skip_code_symbol_embeddings = matches!(
            std::env::var(CODE_SYMBOL_EMBED_SKIP_ENV),
            Ok(value) if value == "1"
        );
        let batch_size = std::env::var(SECTION_EMBED_BATCH_SIZE_ENV)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(SECTION_EMBED_BATCH_SIZE_DEFAULT);

        Self {
            skip_section_embeddings,
            skip_code_symbol_embeddings,
            batch_size,
        }
    }

    pub fn from_env_with_skip_override(skip_section_embeddings_override: bool) -> Self {
        Self::from_env_with_skip_overrides(skip_section_embeddings_override, false)
    }

    pub fn from_env_with_skip_overrides(
        skip_section_embeddings_override: bool,
        skip_code_symbol_embeddings_override: bool,
    ) -> Self {
        let mut options = Self::from_env();
        options.skip_section_embeddings |= skip_section_embeddings_override;
        options.skip_code_symbol_embeddings |= skip_code_symbol_embeddings_override;
        options
    }
}

impl Default for SectionEmbeddingOptions {
    fn default() -> Self {
        Self {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: false,
            batch_size: SECTION_EMBED_BATCH_SIZE_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarDelta {
    /// Worktree-relative paths whose sidecar rows must be regenerated.
    pub changed_paths: BTreeSet<String>,
    /// Worktree-relative paths whose previous sidecar rows must not be copied.
    pub deleted_paths: BTreeSet<String>,
}

impl SidecarDelta {
    pub fn new(changed_paths: BTreeSet<String>, deleted_paths: BTreeSet<String>) -> Self {
        Self {
            changed_paths,
            deleted_paths,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSidecarOptions {
    pub embedding: SectionEmbeddingOptions,
    pub write_batch_size: usize,
    /// When set, vectors and unchanged rows are carried forward from the
    /// previous parquet sidecars in this directory. Missing previous files
    /// are silently ignored.
    pub previous_artifact_dir: Option<PathBuf>,
    /// Caller-known path delta for incremental sidecar planning.
    pub delta: Option<SidecarDelta>,
}

impl SectionSidecarOptions {
    pub fn from_env() -> Self {
        Self {
            embedding: SectionEmbeddingOptions::from_env(),
            write_batch_size: section_write_batch_size_from_env(),
            previous_artifact_dir: None,
            delta: None,
        }
    }

    pub fn from_env_with_skip_override(skip_section_embeddings_override: bool) -> Self {
        Self::from_env_with_skip_overrides(skip_section_embeddings_override, false)
    }

    pub fn from_env_with_skip_overrides(
        skip_section_embeddings_override: bool,
        skip_code_symbol_embeddings_override: bool,
    ) -> Self {
        Self {
            embedding: SectionEmbeddingOptions::from_env_with_skip_overrides(
                skip_section_embeddings_override,
                skip_code_symbol_embeddings_override,
            ),
            write_batch_size: section_write_batch_size_from_env(),
            previous_artifact_dir: None,
            delta: None,
        }
    }

    pub fn from_embedding_options(embedding: SectionEmbeddingOptions) -> Self {
        Self {
            embedding,
            write_batch_size: section_write_batch_size_from_env(),
            previous_artifact_dir: None,
            delta: None,
        }
    }

    /// Set the directory to carry vectors from.  Returns `self` for chaining.
    pub fn with_previous_artifact_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.previous_artifact_dir = dir;
        self
    }

    /// Set the caller-known path delta for incremental sidecar planning.
    pub fn with_delta(mut self, delta: Option<SidecarDelta>) -> Self {
        self.delta = delta;
        self
    }
}

impl Default for SectionSidecarOptions {
    fn default() -> Self {
        Self {
            embedding: SectionEmbeddingOptions::default(),
            write_batch_size: SECTION_WRITE_BATCH_SIZE_DEFAULT,
            previous_artifact_dir: None,
            delta: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VectorBackfillStats {
    pub sections: VectorBackfillTableStats,
    pub code_symbols: VectorBackfillTableStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VectorBackfillTableStats {
    pub total_rows: usize,
    pub null_vector_rows: usize,
    pub eligible_rows: usize,
    pub filled_rows: usize,
}

fn section_write_batch_size_from_env() -> usize {
    std::env::var(SECTION_WRITE_BATCH_SIZE_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(SECTION_WRITE_BATCH_SIZE_DEFAULT)
}

fn normalize_section_write_batch_size(write_batch_size: usize) -> usize {
    if write_batch_size == 0 {
        SECTION_WRITE_BATCH_SIZE_DEFAULT
    } else {
        write_batch_size
    }
}

#[derive(Debug, Clone)]
struct SectionRow {
    stable_symbol_id: String,
    file_path: String,
    qualified_name: String,
    heading_level: u8,
    body_text: String,
    body_byte_start: u64,
    body_byte_end: u64,
    child_count: u32,
    parent_stable_id: Option<String>,
    content_hash: String,
    embedding_input_hash: String,
    embedding_model: String,
    vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone)]
struct SymbolRow {
    stable_symbol_id: String,
    file_path: String,
    qualified_name: String,
    entity_name: String,
    symbol_kind: String,
    embed_text: String,
    vector: Option<Vec<f32>>,
    content_hash: String,
    embedding_input_hash: String,
    embedding_model: String,
}

fn write_record_batches_parquet(
    path: &Path,
    schema: Arc<Schema>,
    mut batches: Vec<RecordBatch>,
) -> Result<usize> {
    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(schema.clone()));
    }
    let file =
        fs::File::create(path).with_context(|| format!("failed to create `{}`", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, None)
        .with_context(|| format!("failed to open parquet writer `{}`", path.display()))?;
    let mut rows = 0usize;
    for batch in &batches {
        rows += batch.num_rows();
        writer
            .write(batch)
            .with_context(|| format!("failed to write parquet batch `{}`", path.display()))?;
    }
    writer
        .close()
        .with_context(|| format!("failed to close parquet `{}`", path.display()))?;
    Ok(rows)
}

pub fn write_sections_dataset(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
) -> Result<()> {
    write_sections_dataset_with_sidecar_options_and_progress(
        artifact,
        worktree_root,
        artifact_dir,
        SectionSidecarOptions::from_env(),
        None,
    )
    .map(|_| ())
}

pub fn write_sections_dataset_skipping_embeddings(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
) -> Result<()> {
    write_sections_dataset_with_sidecar_options_and_progress(
        artifact,
        worktree_root,
        artifact_dir,
        SectionSidecarOptions::from_env_with_skip_overrides(true, true),
        None,
    )
    .map(|_| ())
}

pub fn write_sections_dataset_skipping_embeddings_with_delta(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    previous_artifact_dir: &Path,
    delta: SidecarDelta,
) -> Result<()> {
    write_sections_dataset_with_sidecar_options_and_progress(
        artifact,
        worktree_root,
        artifact_dir,
        SectionSidecarOptions::from_env_with_skip_overrides(true, true)
            .with_previous_artifact_dir(Some(previous_artifact_dir.to_path_buf()))
            .with_delta(Some(delta)),
        None,
    )
    .map(|_| ())
}

pub fn write_sections_dataset_best_effort_with_options(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
) {
    if let Err(error) = write_sections_dataset_with_sidecar_options_and_progress(
        artifact,
        worktree_root,
        artifact_dir,
        SectionSidecarOptions::from_embedding_options(options),
        None,
    ) {
        tracing::warn!(
            error = %error,
            artifact_dir = %artifact_dir.display(),
            "spur-graph: section sidecar write failed; graph artifact remains usable"
        );
    }
}

pub(crate) fn write_sections_dataset_with_sidecar_options_and_progress(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: SectionSidecarOptions,
    progress: Option<&SectionSidecarProgressCallback<'_>>,
) -> Result<GraphArtifactSidecarRowCounts> {
    #[cfg(debug_assertions)]
    if matches!(
        std::env::var(SECTION_SIDECAR_TEST_FAIL_ENV),
        Ok(value) if value == "1"
    ) {
        anyhow::bail!("forced section sidecar failure for tests");
    }

    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    write_sections_dataset_without_current_runtime(
                        artifact,
                        worktree_root,
                        artifact_dir,
                        options,
                        progress,
                    )
                })
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
    }
    write_sections_dataset_without_current_runtime(
        artifact,
        worktree_root,
        artifact_dir,
        options,
        progress,
    )
}

fn write_sections_dataset_without_current_runtime(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: SectionSidecarOptions,
    progress: Option<&SectionSidecarProgressCallback<'_>>,
) -> Result<GraphArtifactSidecarRowCounts> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create sidecar runtime")?;
    runtime.block_on(write_sections_dataset_async(
        artifact,
        worktree_root,
        artifact_dir,
        options,
        progress,
    ))
}

async fn write_sections_dataset_async(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: SectionSidecarOptions,
    progress: Option<&SectionSidecarProgressCallback<'_>>,
) -> Result<GraphArtifactSidecarRowCounts> {
    let embedding_model = EmbeddingModelSelection::from_env();
    write_sections_dataset_async_with_embedding_model(
        artifact,
        worktree_root,
        artifact_dir,
        options,
        embedding_model,
        progress,
    )
    .await
}

async fn write_sections_dataset_async_with_embedding_model(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: SectionSidecarOptions,
    embedding_model: EmbeddingModelSelection,
    progress: Option<&SectionSidecarProgressCallback<'_>>,
) -> Result<GraphArtifactSidecarRowCounts> {
    fs::create_dir_all(artifact_dir)
        .with_context(|| format!("failed to create `{}`", artifact_dir.display()))?;

    let schema = sections_schema();
    let previous_compatible = options
        .previous_artifact_dir
        .as_deref()
        .is_none_or(|prev_dir| previous_sidecars_match_embedding_model(prev_dir, embedding_model));
    let delta = previous_compatible
        .then_some(options.delta.as_ref())
        .flatten();
    let changed_paths = delta.map(|delta| &delta.changed_paths);
    let mut batcher = SectionRowBatcher::new(
        artifact,
        worktree_root,
        options.write_batch_size,
        changed_paths,
        embedding_model,
    );
    let row_scope = SidecarRowScope::from_delta_paths(changed_paths);
    let total_rows = batcher.total_rows();
    let mut retained_rows = Vec::new();
    if let (Some(prev_dir), Some(delta)) = (options.previous_artifact_dir.as_deref(), delta) {
        if prev_dir != artifact_dir {
            retained_rows = retained_section_rows_from_prev_parquet(prev_dir, delta);
        }
    }
    let skipped_existing_rows = retained_rows.len();
    emit_progress(
        progress,
        SectionSidecarProgressEvent::Started {
            total_rows,
            markdown_files: batcher.markdown_file_count(),
            embeddings_enabled: !options.embedding.skip_section_embeddings,
            embedding_batch_size: options.embedding.batch_size,
            write_batch_size: batcher.write_batch_size(),
            row_scope,
        },
    );
    let mut embedder = SectionEmbedder::new(options.embedding, embedding_model);
    let mut batches = Vec::new();
    if !retained_rows.is_empty() {
        batches.push(rows_to_batch(retained_rows, schema.clone())?);
    }
    let mut batch_index = 0usize;
    let mut processed_rows = 0usize;
    let mut written_rows = 0usize;

    while let Some(mut rows) = batcher.next_batch()? {
        batch_index += 1;
        processed_rows += rows.len();
        if let Some(prev_dir) = options.previous_artifact_dir.as_deref() {
            if prev_dir != artifact_dir {
                fill_section_vectors_from_prev_parquet(&mut rows, prev_dir);
            }
        }
        let embedding_eligible_rows = rows
            .iter()
            .filter(|row| is_embedding_eligible(row) && row.vector.is_none())
            .count();
        if embedding_eligible_rows > 0 && embedder.needs_model_init() {
            emit_progress(
                progress,
                SectionSidecarProgressEvent::ModelDownloading {
                    model_name: embedding_model.model_name(),
                    approximate_size_mb: embedding_model.approximate_size_mb(),
                },
            );
        }
        let embeddings_available =
            embedder.prepare_model_for_eligible_rows(embedding_eligible_rows);
        emit_progress(
            progress,
            SectionSidecarProgressEvent::BatchStarted {
                batch_index,
                batch_rows: rows.len(),
                embedding_eligible_rows,
                embeddings_available,
                processed_rows,
                total_rows,
            },
        );
        let batch_rows = rows.len();
        embedder
            .embed_rows_with_progress(&mut rows, |chunk| {
                emit_progress(
                    progress,
                    SectionSidecarProgressEvent::EmbeddingChunkStarted {
                        batch_index,
                        batch_rows,
                        chunk_index: chunk.chunk_index,
                        chunk_count: chunk.chunk_count,
                        chunk_rows: chunk.chunk_rows,
                        completed_eligible_rows: chunk.completed_eligible_rows,
                        embedding_eligible_rows: chunk.embedding_eligible_rows,
                        processed_rows,
                        total_rows,
                    },
                );
            })
            .await;
        written_rows += rows.len();
        batches.push(rows_to_batch(rows, schema.clone())?);
        emit_progress(
            progress,
            SectionSidecarProgressEvent::BatchWritten {
                batch_index,
                written_rows,
                skipped_existing_rows,
                processed_rows,
                total_rows,
            },
        );
    }

    let section_bodies =
        write_record_batches_parquet(&artifact_dir.join(SECTIONS_PARQUET), schema, batches)?;
    emit_progress(
        progress,
        SectionSidecarProgressEvent::Finished {
            total_rows,
            final_rows: section_bodies,
            written_rows,
            skipped_existing_rows,
            phase: SidecarPhase::Sections,
            row_scope,
        },
    );
    let code_symbols = write_symbol_rows_dataset_async(
        artifact,
        worktree_root,
        artifact_dir,
        &options,
        embedding_model,
        progress,
    )
    .await?;
    Ok(GraphArtifactSidecarRowCounts {
        section_bodies,
        code_symbols,
    })
}

async fn write_symbol_rows_dataset_async(
    artifact: &GraphIndexArtifact,
    worktree_root: &Path,
    artifact_dir: &Path,
    options: &SectionSidecarOptions,
    embedding_model: EmbeddingModelSelection,
    progress: Option<&SectionSidecarProgressCallback<'_>>,
) -> Result<usize> {
    let write_batch_size = options.write_batch_size;
    let embedding_options = options.embedding;
    fs::create_dir_all(artifact_dir)
        .with_context(|| format!("failed to create `{}`", artifact_dir.display()))?;
    let schema = symbol_rows_schema();
    let previous_compatible = options
        .previous_artifact_dir
        .as_deref()
        .is_none_or(|prev_dir| previous_sidecars_match_embedding_model(prev_dir, embedding_model));
    let delta = previous_compatible
        .then_some(options.delta.as_ref())
        .flatten();
    let changed_paths = delta.map(|delta| &delta.changed_paths);
    let mut batcher = SymbolRowBatcher::new(
        artifact,
        worktree_root,
        write_batch_size,
        changed_paths,
        embedding_model,
    );
    let row_scope = SidecarRowScope::from_delta_paths(changed_paths);
    let total_rows = batcher.total_rows();
    let mut retained_rows = Vec::new();
    if let (Some(prev_dir), Some(delta)) = (options.previous_artifact_dir.as_deref(), delta) {
        if prev_dir != artifact_dir {
            retained_rows = retained_symbol_rows_from_prev_parquet(prev_dir, delta);
        }
    }
    let skipped_existing_rows = retained_rows.len();
    emit_progress(
        progress,
        SectionSidecarProgressEvent::CodeSymbolsStarted {
            total_rows,
            embeddings_enabled: !embedding_options.skip_code_symbol_embeddings,
            row_scope,
        },
    );
    let mut embedder = SymbolEmbedder::new(embedding_options, embedding_model);
    let mut batches = Vec::new();
    if !retained_rows.is_empty() {
        batches.push(symbol_rows_to_batch(retained_rows, schema.clone())?);
    }
    let mut batch_index = 0usize;
    let mut processed_rows = 0usize;
    let mut written_rows = 0usize;
    while let Some(mut rows) = batcher.next_batch()? {
        batch_index += 1;
        processed_rows += rows.len();
        if let Some(prev_dir) = options.previous_artifact_dir.as_deref() {
            if prev_dir != artifact_dir {
                fill_symbol_vectors_from_prev_parquet(&mut rows, prev_dir);
            }
        }
        let embedding_eligible_rows = rows
            .iter()
            .filter(|row| !row.embed_text.trim().is_empty() && row.vector.is_none())
            .count();
        let embeddings_available =
            embedder.prepare_model_for_eligible_rows(embedding_eligible_rows);
        emit_progress(
            progress,
            SectionSidecarProgressEvent::BatchStarted {
                batch_index,
                batch_rows: rows.len(),
                embedding_eligible_rows,
                embeddings_available,
                processed_rows,
                total_rows,
            },
        );
        let batch_rows = rows.len();
        embedder
            .embed_rows_with_progress(&mut rows, |chunk| {
                emit_progress(
                    progress,
                    SectionSidecarProgressEvent::EmbeddingChunkStarted {
                        batch_index,
                        batch_rows,
                        chunk_index: chunk.chunk_index,
                        chunk_count: chunk.chunk_count,
                        chunk_rows: chunk.chunk_rows,
                        completed_eligible_rows: chunk.completed_eligible_rows,
                        embedding_eligible_rows: chunk.embedding_eligible_rows,
                        processed_rows,
                        total_rows,
                    },
                );
            })
            .await;
        written_rows += rows.len();
        batches.push(symbol_rows_to_batch(rows, schema.clone())?);
        emit_progress(
            progress,
            SectionSidecarProgressEvent::BatchWritten {
                batch_index,
                written_rows,
                skipped_existing_rows,
                processed_rows,
                total_rows,
            },
        );
    }
    let final_rows =
        write_record_batches_parquet(&artifact_dir.join(CODE_SYMBOLS_PARQUET), schema, batches)?;
    emit_progress(
        progress,
        SectionSidecarProgressEvent::Finished {
            total_rows,
            final_rows,
            written_rows,
            skipped_existing_rows,
            phase: SidecarPhase::CodeSymbols,
            row_scope,
        },
    );
    Ok(final_rows)
}

fn emit_progress(
    progress: Option<&SectionSidecarProgressCallback<'_>>,
    event: SectionSidecarProgressEvent,
) {
    if let Some(progress) = progress {
        progress(event);
    }
}

struct SectionRowBatcher<'a> {
    worktree_root: &'a Path,
    embedding_model: EmbeddingModelSelection,
    child_count_by_parent: HashMap<&'a str, u32>,
    parent_by_child: HashMap<&'a str, String>,
    manifest_by_path: BTreeMap<&'a str, &'a GraphFileManifestEntry>,
    sections_by_path: BTreeMap<&'a str, Vec<&'a GraphSymbolArtifact>>,
    ordered_paths: Vec<&'a str>,
    next_path_index: usize,
    pending_rows: VecDeque<SectionRow>,
    write_batch_size: usize,
}

impl<'a> SectionRowBatcher<'a> {
    fn new(
        artifact: &'a GraphIndexArtifact,
        worktree_root: &'a Path,
        write_batch_size: usize,
        changed_paths: Option<&'a BTreeSet<String>>,
        embedding_model: EmbeddingModelSelection,
    ) -> Self {
        let write_batch_size = normalize_section_write_batch_size(write_batch_size);
        let section_ids: BTreeSet<&str> = artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.symbol_kind == "section")
            .map(|symbol| symbol.stable_symbol_id.as_str())
            .collect();
        let child_count_by_parent = child_count_by_parent(&artifact.edges, &section_ids);
        let parent_by_child = parent_by_child(&artifact.edges, &section_ids);
        let manifest_by_path: BTreeMap<&str, &GraphFileManifestEntry> = artifact
            .file_manifests
            .iter()
            .map(|manifest| (manifest.path.as_str(), manifest))
            .collect();

        let mut sections_by_path: BTreeMap<&str, Vec<&GraphSymbolArtifact>> = BTreeMap::new();
        for symbol in &artifact.symbols {
            if symbol.symbol_kind == "section" {
                sections_by_path
                    .entry(symbol.file_path.as_str())
                    .or_default()
                    .push(symbol);
            }
        }
        for sections in sections_by_path.values_mut() {
            sections.sort_by(|a, b| {
                a.byte_range[0]
                    .cmp(&b.byte_range[0])
                    .then(a.stable_symbol_id.cmp(&b.stable_symbol_id))
            });
        }

        let mut ordered_paths = BTreeSet::new();
        for path in sections_by_path.keys() {
            if changed_paths.is_none_or(|cp| cp.contains(*path)) {
                ordered_paths.insert(*path);
            }
        }
        for manifest in manifest_by_path.values() {
            if is_markdown_path(&manifest.path)
                && !sections_by_path.contains_key(manifest.path.as_str())
                && changed_paths.is_none_or(|cp| cp.contains(manifest.path.as_str()))
            {
                ordered_paths.insert(manifest.path.as_str());
            }
        }

        Self {
            worktree_root,
            embedding_model,
            child_count_by_parent,
            parent_by_child,
            manifest_by_path,
            sections_by_path,
            ordered_paths: ordered_paths.into_iter().collect(),
            next_path_index: 0,
            pending_rows: VecDeque::new(),
            write_batch_size,
        }
    }

    fn next_batch(&mut self) -> Result<Option<Vec<SectionRow>>> {
        let mut batch = Vec::with_capacity(self.write_batch_size);
        while batch.len() < self.write_batch_size {
            if let Some(row) = self.pending_rows.pop_front() {
                batch.push(row);
                continue;
            }

            let Some(path) = self.ordered_paths.get(self.next_path_index).copied() else {
                break;
            };
            self.next_path_index += 1;
            self.pending_rows = VecDeque::from(self.rows_for_path(path)?);
        }

        if batch.is_empty() {
            Ok(None)
        } else {
            Ok(Some(batch))
        }
    }

    fn total_rows(&self) -> usize {
        self.ordered_paths
            .iter()
            .filter_map(|path| self.sections_by_path.get(*path))
            .map(Vec::len)
            .sum::<usize>()
            + self
                .ordered_paths
                .iter()
                .filter(|path| !self.sections_by_path.contains_key(**path))
                .count()
    }

    fn markdown_file_count(&self) -> usize {
        self.ordered_paths.len()
    }

    fn write_batch_size(&self) -> usize {
        self.write_batch_size
    }

    fn rows_for_path(&self, path: &str) -> Result<Vec<SectionRow>> {
        let bytes = match read_file_bytes(self.worktree_root, path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "section_rows: skipping unreadable file");
                return Ok(Vec::new());
            }
        };
        let source_content_hash = blake3_hex(&bytes);
        let content_hash =
            section_embed_content_hash_for_model(&source_content_hash, self.embedding_model);
        let source = match std::str::from_utf8(&bytes) {
            Ok(source) => source,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "section_rows: skipping non-UTF-8 markdown");
                return Ok(Vec::new());
            }
        };

        if let Some(sections) = self.sections_by_path.get(path) {
            let mut rows = Vec::new();
            for section in sections {
                if let Some(row) = section_row(
                    section,
                    source,
                    content_hash.as_str(),
                    self.embedding_model,
                    &self.child_count_by_parent,
                    &self.parent_by_child,
                )? {
                    rows.push(row);
                }
            }
            return Ok(rows);
        }

        let Some(manifest) = self.manifest_by_path.get(path) else {
            return Ok(Vec::new());
        };
        Ok(vec![SectionRow {
            stable_symbol_id: manifest.stable_file_id.clone(),
            file_path: manifest.path.clone(),
            qualified_name: manifest.path.clone(),
            heading_level: 0,
            body_text: source.to_owned(),
            body_byte_start: 0,
            body_byte_end: bytes.len() as u64,
            child_count: 0,
            parent_stable_id: None,
            content_hash,
            embedding_input_hash: section_embedding_input_hash_for_model(
                &manifest.path,
                source,
                self.embedding_model,
            ),
            embedding_model: self.embedding_model.model_name().to_owned(),
            vector: None,
        }])
    }
}

struct SymbolRowBatcher<'a> {
    worktree_root: &'a Path,
    embedding_model: EmbeddingModelSelection,
    symbols_by_path: BTreeMap<&'a str, Vec<&'a GraphSymbolArtifact>>,
    manifest_by_path: BTreeMap<&'a str, &'a GraphFileManifestEntry>,
    ordered_paths: Vec<&'a str>,
    next_path_index: usize,
    pending_rows: VecDeque<SymbolRow>,
    write_batch_size: usize,
}

impl<'a> SymbolRowBatcher<'a> {
    fn new(
        artifact: &'a GraphIndexArtifact,
        worktree_root: &'a Path,
        write_batch_size: usize,
        changed_paths: Option<&'a BTreeSet<String>>,
        embedding_model: EmbeddingModelSelection,
    ) -> Self {
        let write_batch_size = normalize_section_write_batch_size(write_batch_size);
        let mut symbols_by_path: BTreeMap<&str, Vec<&GraphSymbolArtifact>> = BTreeMap::new();
        for symbol in &artifact.symbols {
            if is_code_symbol_kind(&symbol.symbol_kind) {
                symbols_by_path
                    .entry(symbol.file_path.as_str())
                    .or_default()
                    .push(symbol);
            }
        }
        let manifest_by_path: BTreeMap<&str, &GraphFileManifestEntry> = artifact
            .file_manifests
            .iter()
            .map(|manifest| (manifest.path.as_str(), manifest))
            .collect();
        for symbols in symbols_by_path.values_mut() {
            symbols.sort_by(|a, b| {
                a.byte_range[0]
                    .cmp(&b.byte_range[0])
                    .then(a.stable_symbol_id.cmp(&b.stable_symbol_id))
            });
        }
        let ordered_paths: Vec<&str> = match changed_paths {
            Some(cp) => symbols_by_path
                .keys()
                .copied()
                .filter(|path| cp.contains(*path))
                .collect(),
            None => symbols_by_path.keys().copied().collect(),
        };

        Self {
            worktree_root,
            embedding_model,
            symbols_by_path,
            manifest_by_path,
            ordered_paths,
            next_path_index: 0,
            pending_rows: VecDeque::new(),
            write_batch_size,
        }
    }

    fn next_batch(&mut self) -> Result<Option<Vec<SymbolRow>>> {
        let mut batch = Vec::with_capacity(self.write_batch_size);
        while batch.len() < self.write_batch_size {
            if let Some(row) = self.pending_rows.pop_front() {
                batch.push(row);
                continue;
            }

            let Some(path) = self.ordered_paths.get(self.next_path_index).copied() else {
                break;
            };
            self.next_path_index += 1;
            self.pending_rows = VecDeque::from(self.rows_for_path(path)?);
        }

        if batch.is_empty() {
            Ok(None)
        } else {
            Ok(Some(batch))
        }
    }

    /// Upper-bound estimate of the total rows that will be emitted (based on symbol count).
    fn total_rows(&self) -> usize {
        self.ordered_paths
            .iter()
            .filter_map(|path| self.symbols_by_path.get(*path))
            .map(Vec::len)
            .sum::<usize>()
    }

    fn rows_for_path(&self, path: &str) -> Result<Vec<SymbolRow>> {
        let bytes = match self.read_source_bytes(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "symbol_rows: skipping unreadable file");
                return Ok(Vec::new());
            }
        };
        let content_hash = blake3_hex(&bytes);
        let source = match std::str::from_utf8(&bytes) {
            Ok(source) => source,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "symbol_rows: skipping non-UTF-8 source");
                return Ok(Vec::new());
            }
        };
        let Some(symbols) = self.symbols_by_path.get(path) else {
            return Ok(Vec::new());
        };

        let mut rows = Vec::new();
        for symbol in symbols {
            if let Some(row) =
                symbol_row(symbol, source, content_hash.as_str(), self.embedding_model)?
            {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    fn read_source_bytes(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(manifest) = self.manifest_by_path.get(path) {
            match read_git_blob_bytes(self.worktree_root, &manifest.content_oid) {
                Ok(bytes) => return Ok(bytes),
                Err(error) => {
                    tracing::debug!(
                        path = %path,
                        content_oid = %manifest.content_oid,
                        error = %error,
                        "symbol_rows: falling back to worktree source after git blob read failed"
                    );
                }
            }
        }
        read_file_bytes(self.worktree_root, path)
    }
}

fn section_row(
    section: &GraphSymbolArtifact,
    source: &str,
    content_hash: &str,
    embedding_model: EmbeddingModelSelection,
    child_count_by_parent: &HashMap<&str, u32>,
    parent_by_child: &HashMap<&str, String>,
) -> Result<Option<SectionRow>> {
    let start = section.byte_range[0];
    let end = section.byte_range[1];
    let Some(body_text) = source.get(start..end) else {
        tracing::warn!(
            file_path = %section.file_path,
            stable_symbol_id = %section.stable_symbol_id,
            byte_start = start,
            byte_end = end,
            "section_rows: skipping non-UTF-8-boundary byte range"
        );
        return Ok(None);
    };
    let body_text = body_text.to_owned();
    let embedding_input_hash = section_embedding_input_hash_for_model(
        &section.qualified_name,
        &body_text,
        embedding_model,
    );
    Ok(Some(SectionRow {
        stable_symbol_id: section.stable_symbol_id.clone(),
        file_path: section.file_path.clone(),
        qualified_name: section.qualified_name.clone(),
        heading_level: heading_level(&body_text),
        body_text,
        body_byte_start: start as u64,
        body_byte_end: end as u64,
        child_count: child_count_by_parent
            .get(section.stable_symbol_id.as_str())
            .copied()
            .unwrap_or(0),
        parent_stable_id: parent_by_child
            .get(section.stable_symbol_id.as_str())
            .cloned(),
        content_hash: content_hash.to_owned(),
        embedding_input_hash,
        embedding_model: embedding_model.model_name().to_owned(),
        vector: None,
    }))
}

fn symbol_row(
    symbol: &GraphSymbolArtifact,
    source: &str,
    content_hash: &str,
    embedding_model: EmbeddingModelSelection,
) -> Result<Option<SymbolRow>> {
    let start = symbol.byte_range[0];
    let end = symbol.byte_range[1];
    if source.get(start..end).is_none() {
        tracing::warn!(
            file_path = %symbol.file_path,
            stable_symbol_id = %symbol.stable_symbol_id,
            byte_start = start,
            byte_end = end,
            "symbol_rows: skipping non-UTF-8-boundary byte range"
        );
        return Ok(None);
    }

    let doc_text = doc_text_for_symbol(source, start).with_context(|| {
        format!(
            "failed to derive doc text for symbol `{}` in `{}`",
            symbol.stable_symbol_id, symbol.file_path
        )
    })?;
    let has_long_doc = doc_text.chars().count() > 50;
    if !has_long_doc && !is_meaningful_entity_name(&symbol.entity_name) {
        return Ok(None);
    }

    let qualified_name = if symbol.qualified_name.is_empty() {
        symbol.entity_name.clone()
    } else {
        symbol.qualified_name.clone()
    };
    let embed_text = if has_long_doc {
        format!("{} {} {}", symbol.entity_name, qualified_name, doc_text)
    } else {
        format!(
            "{} {} {}",
            symbol.entity_name, qualified_name, symbol.symbol_kind
        )
    };
    let has_significant_body = symbol_body_line_count(symbol) > 5;
    let embed_text = if has_significant_body {
        match first_source_line_for_symbol(source, symbol)? {
            Some(first_line) => format!("{first_line} {embed_text}"),
            None => embed_text,
        }
    } else {
        embed_text
    };
    let content_hash =
        symbol_embed_content_hash_for_model(content_hash, has_significant_body, embedding_model);
    let embedding_input_hash =
        symbol_embedding_input_hash_for_model(&embed_text, has_significant_body, embedding_model);

    Ok(Some(SymbolRow {
        stable_symbol_id: symbol.stable_symbol_id.clone(),
        file_path: symbol.file_path.clone(),
        qualified_name,
        entity_name: symbol.entity_name.clone(),
        symbol_kind: symbol.symbol_kind.clone(),
        embed_text,
        vector: None,
        content_hash,
        embedding_input_hash,
        embedding_model: embedding_model.model_name().to_owned(),
    }))
}

fn symbol_body_line_count(symbol: &GraphSymbolArtifact) -> usize {
    let [start, end] = symbol.line_range;
    end.saturating_sub(start).saturating_add(1)
}

fn first_source_line_for_symbol<'a>(
    source: &'a str,
    symbol: &GraphSymbolArtifact,
) -> Result<Option<&'a str>> {
    let start = symbol.byte_range[0];
    let end = symbol.byte_range[1];
    let body = source.get(start..end).with_context(|| {
        format!(
            "symbol byte range {}..{} is not a UTF-8 boundary in `{}`",
            start, end, symbol.file_path
        )
    })?;
    Ok(body.lines().map(str::trim).find(|line| !line.is_empty()))
}

fn section_embed_content_hash_for_model(
    source_content_hash: &str,
    embedding_model: EmbeddingModelSelection,
) -> String {
    blake3_hex(
        format!(
            "{}:section\0{source_content_hash}",
            embedding_model.section_text_version()
        )
        .as_bytes(),
    )
}

fn section_embedding_input_hash_for_model(
    title: &str,
    body_text: &str,
    embedding_model: EmbeddingModelSelection,
) -> String {
    let input = embedding_document_text_for_model(title, body_text, embedding_model);
    let input_hash = blake3_hex(input.as_bytes());
    section_embed_content_hash_for_model(&input_hash, embedding_model)
}

fn symbol_embed_content_hash_for_model(
    source_content_hash: &str,
    _has_significant_body: bool,
    embedding_model: EmbeddingModelSelection,
) -> String {
    blake3_hex(
        format!(
            "{}:symbol\0{source_content_hash}",
            embedding_model.symbol_text_version()
        )
        .as_bytes(),
    )
}

fn symbol_embedding_input_hash_for_model(
    embed_text: &str,
    has_significant_body: bool,
    embedding_model: EmbeddingModelSelection,
) -> String {
    symbol_embed_content_hash_for_model(
        &blake3_hex(embed_text.as_bytes()),
        has_significant_body,
        embedding_model,
    )
}

fn doc_text_for_symbol(source: &str, byte_start: usize) -> Result<String> {
    let prefix = source
        .get(..byte_start)
        .with_context(|| format!("symbol byte start {byte_start} is not a UTF-8 boundary"))?;
    let line_doc = preceding_line_doc_text(prefix);
    if !line_doc.is_empty() {
        return Ok(line_doc);
    }
    Ok(preceding_block_doc_text(prefix).unwrap_or_default())
}

fn preceding_line_doc_text(prefix: &str) -> String {
    let prefix = prefix.trim_end_matches([' ', '\t', '\r']);
    let mut lines = Vec::new();
    for line in prefix.lines().rev() {
        let trimmed = line.trim_start();
        let text = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//!"))
            .or_else(|| {
                trimmed
                    .strip_prefix('#')
                    .filter(|_| !trimmed.starts_with("#!") && !trimmed.starts_with("#["))
            });
        let Some(text) = text else {
            break;
        };
        lines.push(text.trim().to_owned());
    }
    lines.reverse();
    normalize_doc_lines(lines)
}

fn preceding_block_doc_text(prefix: &str) -> Option<String> {
    let prefix = prefix.trim_end_matches([' ', '\t', '\r']).trim_end();
    if !prefix.ends_with("*/") {
        return None;
    }
    let start = prefix.rfind("/*")?;
    let body = &prefix[start + 2..prefix.len().saturating_sub(2)];
    if !(body.starts_with('*') || body.starts_with('!')) {
        return None;
    }
    let body = body.trim_start_matches(['*', '!']);
    let lines = body
        .lines()
        .map(|line| {
            line.trim()
                .strip_prefix('*')
                .unwrap_or(line.trim())
                .trim()
                .to_owned()
        })
        .collect();
    Some(normalize_doc_lines(lines))
}

fn normalize_doc_lines(lines: Vec<String>) -> String {
    lines
        .into_iter()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_code_symbol_kind(symbol_kind: &str) -> bool {
    !matches!(symbol_kind, "section" | "external" | "commit")
}

fn is_meaningful_entity_name(entity_name: &str) -> bool {
    let mut has_alpha = false;
    let mut meaningful_len = 0usize;
    for ch in entity_name.chars() {
        if ch.is_alphabetic() {
            has_alpha = true;
        }
        if ch.is_alphanumeric() || ch == '_' {
            meaningful_len += 1;
        }
    }
    has_alpha && meaningful_len > 2
}

#[cfg(test)]
fn embed_eligible_rows(
    rows: &[SectionRow],
    options: SectionEmbeddingOptions,
) -> Vec<Option<Vec<f32>>> {
    let mut embedder = SectionEmbedder::new(options, EmbeddingModelSelection::NomicEmbedTextV15);
    embedder.embed_row_vectors(rows)
}

struct SectionEmbedder {
    service: TextEmbeddingService,
}

struct SymbolEmbedder {
    service: TextEmbeddingService,
}

struct TextEmbeddingService {
    options: SectionEmbeddingOptions,
    skip_embeddings: bool,
    embedding_model: EmbeddingModelSelection,
    model_requested: bool,
}

#[derive(Clone)]
struct EmbeddingTextInput<'a> {
    row_index: usize,
    stable_symbol_id: &'a str,
    text: Cow<'a, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SectionEmbeddingChunkProgress {
    chunk_index: usize,
    chunk_count: usize,
    chunk_rows: usize,
    completed_eligible_rows: usize,
    embedding_eligible_rows: usize,
}

impl SectionEmbedder {
    fn new(options: SectionEmbeddingOptions, embedding_model: EmbeddingModelSelection) -> Self {
        Self {
            service: TextEmbeddingService::new(
                options,
                options.skip_section_embeddings,
                embedding_model,
            ),
        }
    }

    fn needs_model_init(&self) -> bool {
        self.service.needs_model_init()
    }

    fn prepare_model_for_eligible_rows(&mut self, eligible_rows: usize) -> bool {
        self.service
            .prepare_model_for_eligible_rows(eligible_rows, "section")
    }

    async fn embed_rows_with_progress<F>(&mut self, rows: &mut [SectionRow], on_chunk_started: F)
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        if rows.iter().all(|row| row.vector.is_some()) || self.service.skip_embeddings {
            return;
        }
        let vectors = self
            .embed_row_vectors_with_progress(rows, on_chunk_started)
            .await;
        for (row, vector) in rows.iter_mut().zip(vectors) {
            if row.vector.is_none() {
                row.vector = vector;
            }
        }
    }

    #[cfg(test)]
    fn embed_row_vectors(&mut self, rows: &[SectionRow]) -> Vec<Option<Vec<f32>>> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.block_on(self.embed_row_vectors_with_progress(rows, |_| {}))
        } else {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(self.embed_row_vectors_with_progress(rows, |_| {}))
        }
    }

    async fn embed_row_vectors_with_progress<F>(
        &mut self,
        rows: &[SectionRow],
        on_chunk_started: F,
    ) -> Vec<Option<Vec<f32>>>
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        let result = vec![None; rows.len()];
        if self.service.skip_embeddings || !rows.iter().any(is_embedding_eligible) {
            return result;
        }

        self.service
            .embed_inputs_with_progress(
                rows.len(),
                section_embedding_inputs(rows, self.service.embedding_model),
                on_chunk_started,
                "section",
            )
            .await
    }
}

impl SymbolEmbedder {
    fn new(options: SectionEmbeddingOptions, embedding_model: EmbeddingModelSelection) -> Self {
        Self {
            service: TextEmbeddingService::new(
                options,
                options.skip_code_symbol_embeddings,
                embedding_model,
            ),
        }
    }

    fn prepare_model_for_eligible_rows(&mut self, eligible_rows: usize) -> bool {
        self.service
            .prepare_model_for_eligible_rows(eligible_rows, "code symbol")
    }

    // Kept for future callers or tests; production path uses embed_rows_with_progress.
    #[allow(dead_code)]
    async fn embed_rows(&mut self, rows: &mut [SymbolRow]) {
        if rows.iter().all(|row| row.vector.is_some()) {
            return;
        }
        let vectors = self.embed_row_vectors(rows).await;
        for (row, vector) in rows.iter_mut().zip(vectors) {
            if row.vector.is_none() {
                row.vector = vector;
            }
        }
    }

    async fn embed_rows_with_progress<F>(&mut self, rows: &mut [SymbolRow], on_chunk_started: F)
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        if rows.iter().all(|row| row.vector.is_some()) || self.service.skip_embeddings {
            return;
        }
        let vectors = self
            .embed_row_vectors_with_progress(rows, on_chunk_started)
            .await;
        for (row, vector) in rows.iter_mut().zip(vectors) {
            if row.vector.is_none() {
                row.vector = vector;
            }
        }
    }

    // Kept for future callers or tests; production path uses embed_row_vectors_with_progress.
    #[allow(dead_code)]
    async fn embed_row_vectors(&mut self, rows: &[SymbolRow]) -> Vec<Option<Vec<f32>>> {
        self.embed_row_vectors_with_progress(rows, |_| {}).await
    }

    async fn embed_row_vectors_with_progress<F>(
        &mut self,
        rows: &[SymbolRow],
        on_chunk_started: F,
    ) -> Vec<Option<Vec<f32>>>
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        self.service
            .embed_inputs_with_progress(
                rows.len(),
                symbol_embedding_inputs(rows, self.service.embedding_model),
                on_chunk_started,
                "code symbol",
            )
            .await
    }
}

impl TextEmbeddingService {
    fn new(
        options: SectionEmbeddingOptions,
        skip_embeddings: bool,
        embedding_model: EmbeddingModelSelection,
    ) -> Self {
        Self {
            options,
            skip_embeddings,
            embedding_model,
            model_requested: false,
        }
    }

    fn needs_model_init(&self) -> bool {
        if self.skip_embeddings || self.model_requested {
            return false;
        }
        #[cfg(feature = "embed")]
        {
            embed_model_cell(self.embedding_model).get().is_none()
        }
        #[cfg(not(feature = "embed"))]
        {
            false
        }
    }

    fn prepare_model(&mut self, embedding_kind: &'static str) -> bool {
        if self.skip_embeddings {
            return false;
        }
        #[cfg(feature = "embed")]
        {
            self.model(embedding_kind).is_some()
        }
        #[cfg(not(feature = "embed"))]
        {
            let _ = embedding_kind;
            false
        }
    }

    fn prepare_model_for_eligible_rows(
        &mut self,
        eligible_rows: usize,
        embedding_kind: &'static str,
    ) -> bool {
        eligible_rows > 0 && self.prepare_model(embedding_kind)
    }

    async fn embed_inputs_with_progress<F>(
        &mut self,
        row_count: usize,
        inputs: Vec<EmbeddingTextInput<'_>>,
        on_chunk_started: F,
        embedding_kind: &'static str,
    ) -> Vec<Option<Vec<f32>>>
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        let result = vec![None; row_count];
        if self.skip_embeddings || inputs.is_empty() {
            return result;
        }
        let options = self.options;
        let batch_size = if options.batch_size == 0 {
            SECTION_EMBED_BATCH_SIZE_DEFAULT
        } else {
            options.batch_size
        };

        let chunks: Vec<_> = inputs.chunks(batch_size).collect();
        let chunk_count = chunks.len();

        self.embed_inputs_locally_with_progress(
            result,
            chunks,
            chunk_count,
            inputs.len(),
            on_chunk_started,
            embedding_kind,
        )
    }

    fn embed_inputs_locally_with_progress<F>(
        &mut self,
        mut result: Vec<Option<Vec<f32>>>,
        chunks: Vec<&[EmbeddingTextInput<'_>]>,
        chunk_count: usize,
        embedding_eligible_rows: usize,
        mut on_chunk_started: F,
        embedding_kind: &'static str,
    ) -> Vec<Option<Vec<f32>>>
    where
        F: FnMut(SectionEmbeddingChunkProgress),
    {
        let mut completed_eligible_rows = 0usize;
        for (chunk_offset, chunk) in chunks.into_iter().enumerate() {
            let texts: Vec<&str> = chunk.iter().map(|input| input.text.as_ref()).collect();
            on_chunk_started(SectionEmbeddingChunkProgress {
                chunk_index: chunk_offset + 1,
                chunk_count,
                chunk_rows: chunk.len(),
                completed_eligible_rows,
                embedding_eligible_rows,
            });
            let embeddings = match self.embed_texts_locally(&texts, embedding_kind) {
                Ok(embeddings) => embeddings,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        embedding_kind,
                        "embedding batch failed; skipping remaining embeddings"
                    );
                    return result;
                }
            };

            if !apply_embeddings_to_inputs(&mut result, chunk, embeddings, embedding_kind) {
                return result;
            }
            completed_eligible_rows += chunk.len();
        }

        result
    }

    #[cfg(feature = "embed")]
    fn model(&mut self, embedding_kind: &'static str) -> Option<&'static Mutex<TextEmbedding>> {
        self.model_requested = true;
        shared_embed_model(self.embedding_model, embedding_kind)
    }

    #[cfg(feature = "embed")]
    fn embed_texts_locally(
        &mut self,
        texts: &[&str],
        embedding_kind: &'static str,
    ) -> Result<Vec<Vec<f32>>> {
        tracing::info!(
            embedding_kind,
            model = self.embedding_model.model_name(),
            "Using local fastembed"
        );
        let Some(model) = self.model(embedding_kind) else {
            return Ok(Vec::new());
        };
        let mut model = model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        model.embed(texts, None)
    }

    #[cfg(not(feature = "embed"))]
    fn embed_texts_locally(
        &mut self,
        _texts: &[&str],
        embedding_kind: &'static str,
    ) -> Result<Vec<Vec<f32>>> {
        tracing::debug!(
            embedding_kind,
            model = self.embedding_model.model_name(),
            "embedding support disabled at compile time; skipping embeddings"
        );
        Ok(Vec::new())
    }
}

fn apply_embeddings_to_inputs(
    result: &mut [Option<Vec<f32>>],
    chunk: &[EmbeddingTextInput<'_>],
    embeddings: Vec<Vec<f32>>,
    embedding_kind: &'static str,
) -> bool {
    if embeddings.len() != chunk.len() {
        tracing::warn!(
            expected = chunk.len(),
            actual = embeddings.len(),
            embedding_kind,
            "embedder returned unexpected embedding count"
        );
        return false;
    }

    for (input, embedding) in chunk.iter().zip(embeddings) {
        if embedding.len() == EMBEDDING_VECTOR_DIMENSIONS {
            result[input.row_index] = Some(embedding);
        } else {
            tracing::warn!(
                stable_symbol_id = %input.stable_symbol_id,
                dimensions = embedding.len(),
                embedding_kind,
                "embedder returned unexpected embedding dimensions"
            );
        }
    }

    true
}

#[cfg(test)]
fn embed_eligible_rows_with<F>(
    rows: &[SectionRow],
    options: SectionEmbeddingOptions,
    on_chunk_started: impl FnMut(SectionEmbeddingChunkProgress),
    embed_batch: F,
) -> Vec<Option<Vec<f32>>>
where
    F: FnMut(&[&str]) -> Result<Vec<Vec<f32>>>,
{
    embed_text_inputs_with(
        rows.len(),
        section_embedding_inputs(rows, EmbeddingModelSelection::NomicEmbedTextV15),
        options,
        options.skip_section_embeddings,
        on_chunk_started,
        "section",
        embed_batch,
    )
}

#[cfg(test)]
fn embed_symbol_rows_with<F>(
    rows: &[SymbolRow],
    options: SectionEmbeddingOptions,
    on_chunk_started: impl FnMut(SectionEmbeddingChunkProgress),
    embed_batch: F,
) -> Vec<Option<Vec<f32>>>
where
    F: FnMut(&[&str]) -> Result<Vec<Vec<f32>>>,
{
    embed_text_inputs_with(
        rows.len(),
        symbol_embedding_inputs(rows, EmbeddingModelSelection::NomicEmbedTextV15),
        options,
        options.skip_code_symbol_embeddings,
        on_chunk_started,
        "code symbol",
        embed_batch,
    )
}

fn section_embedding_inputs(
    rows: &[SectionRow],
    embedding_model: EmbeddingModelSelection,
) -> Vec<EmbeddingTextInput<'_>> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| is_embedding_eligible(row) && row.vector.is_none())
        .map(|(row_index, row)| EmbeddingTextInput {
            row_index,
            stable_symbol_id: row.stable_symbol_id.as_str(),
            text: embedding_document_text_for_model(
                row.qualified_name.as_str(),
                row.body_text.as_str(),
                embedding_model,
            ),
        })
        .collect()
}

fn symbol_embedding_inputs(
    rows: &[SymbolRow],
    embedding_model: EmbeddingModelSelection,
) -> Vec<EmbeddingTextInput<'_>> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| !row.embed_text.trim().is_empty() && row.vector.is_none())
        .map(|(row_index, row)| EmbeddingTextInput {
            row_index,
            stable_symbol_id: row.stable_symbol_id.as_str(),
            text: embedding_document_text_for_model("", row.embed_text.as_str(), embedding_model),
        })
        .collect()
}

#[cfg(test)]
fn embed_text_inputs_with<F>(
    row_count: usize,
    eligible: Vec<EmbeddingTextInput<'_>>,
    options: SectionEmbeddingOptions,
    skip_embeddings: bool,
    mut on_chunk_started: impl FnMut(SectionEmbeddingChunkProgress),
    embedding_kind: &'static str,
    mut embed_batch: F,
) -> Vec<Option<Vec<f32>>>
where
    F: FnMut(&[&str]) -> Result<Vec<Vec<f32>>>,
{
    let mut result = vec![None; row_count];
    if skip_embeddings {
        return result;
    }
    if eligible.is_empty() {
        return result;
    }

    let batch_size = if options.batch_size == 0 {
        SECTION_EMBED_BATCH_SIZE_DEFAULT
    } else {
        options.batch_size
    };

    let chunks: Vec<_> = eligible.chunks(batch_size).collect();
    let chunk_count = chunks.len();
    let mut completed_eligible_rows = 0usize;
    for (chunk_offset, chunk) in chunks.into_iter().enumerate() {
        let texts: Vec<&str> = chunk.iter().map(|input| input.text.as_ref()).collect();
        on_chunk_started(SectionEmbeddingChunkProgress {
            chunk_index: chunk_offset + 1,
            chunk_count,
            chunk_rows: chunk.len(),
            completed_eligible_rows,
            embedding_eligible_rows: eligible.len(),
        });
        let embeddings = match embed_batch(&texts) {
            Ok(embeddings) => embeddings,
            Err(error) => {
                tracing::warn!(error = %error, embedding_kind, "fastembed encode failed for embedding batch; skipping remaining embeddings");
                return result;
            }
        };

        if embeddings.len() != chunk.len() {
            tracing::warn!(
                expected = chunk.len(),
                actual = embeddings.len(),
                embedding_kind,
                "fastembed returned unexpected embedding count"
            );
            return result;
        }

        for (input, embedding) in chunk.iter().zip(embeddings) {
            if embedding.len() == EMBEDDING_VECTOR_DIMENSIONS {
                result[input.row_index] = Some(embedding);
            } else {
                tracing::warn!(
                    stable_symbol_id = %input.stable_symbol_id,
                    dimensions = embedding.len(),
                    embedding_kind,
                    "fastembed returned unexpected embedding dimensions"
                );
            }
        }
        completed_eligible_rows += chunk.len();
    }

    result
}

#[cfg(feature = "embed")]
fn embed_model_cell(
    embedding_model: EmbeddingModelSelection,
) -> &'static OnceLock<Option<Mutex<TextEmbedding>>> {
    match embedding_model {
        EmbeddingModelSelection::NomicEmbedTextV15 => &NOMIC_EMBED_TEXT_V15_MODEL,
        EmbeddingModelSelection::CodeRankEmbed => &CODE_RANK_EMBED_MODEL,
        EmbeddingModelSelection::JinaEmbeddingsV2BaseCode => &JINA_EMBEDDINGS_V2_BASE_CODE_MODEL,
    }
}

#[cfg(feature = "embed")]
pub fn load_embedding_model(
    embedding_model: EmbeddingModelSelection,
    show_download_progress: bool,
) -> Result<TextEmbedding> {
    if embedding_model == EmbeddingModelSelection::CodeRankEmbed {
        return load_code_rank_embed_model(show_download_progress);
    }

    let registered_model = embedding_model
        .fastembed_model()
        .context("registered FastEmbed model missing")?;
    let mut init_options = InitOptions::new(registered_model)
        .with_max_length(embedding_model.max_length())
        .with_show_download_progress(show_download_progress);

    if let Some(cache_dir) = fastembed_cache_dir() {
        init_options = init_options.with_cache_dir(cache_dir);
    }

    TextEmbedding::try_new(init_options)
}

#[cfg(feature = "embed")]
fn load_code_rank_embed_model(show_download_progress: bool) -> Result<TextEmbedding> {
    let mut api_builder = HfApiBuilder::from_env()
        .with_progress(show_download_progress)
        .with_retries(3);
    if let Some(cache_dir) = fastembed_cache_dir() {
        api_builder = api_builder.with_cache_dir(cache_dir);
    }
    let model_repo = api_builder
        .build()
        .context("build Hugging Face client for CodeRankEmbed")?
        .repo(Repo::with_revision(
            CODE_RANK_EMBED_ONNX_REPO.to_owned(),
            RepoType::Model,
            CODE_RANK_EMBED_ONNX_REVISION.to_owned(),
        ));

    let read_model_file = |file_name: &str| -> Result<Vec<u8>> {
        let file_path = model_repo
            .get(file_name)
            .with_context(|| format!("download CodeRankEmbed file `{file_name}`"))?;
        fs::read(&file_path).with_context(|| {
            format!(
                "read CodeRankEmbed file `{}` from `{}`",
                file_name,
                file_path.display()
            )
        })
    };
    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read_model_file("tokenizer.json")?,
        config_file: read_model_file("config.json")?,
        special_tokens_map_file: read_model_file("special_tokens_map.json")?,
        tokenizer_config_file: read_model_file("tokenizer_config.json")?,
    };
    let model = UserDefinedEmbeddingModel::new(
        read_model_file(CODE_RANK_EMBED_ONNX_FILE)?,
        tokenizer_files,
    )
    .with_pooling(Pooling::Mean);

    TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::new()
            .with_max_length(EmbeddingModelSelection::CodeRankEmbed.max_length()),
    )
    .context("initialize revision-pinned CodeRankEmbed FP32 ONNX model")
}

#[cfg(feature = "embed")]
fn shared_embed_model(
    embedding_model: EmbeddingModelSelection,
    embedding_kind: &'static str,
) -> Option<&'static Mutex<TextEmbedding>> {
    embed_model_cell(embedding_model)
        .get_or_init(|| match load_embedding_model(embedding_model, true) {
            Ok(model) => Some(Mutex::new(model)),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    embedding_kind,
                    model = embedding_model.model_name(),
                    "fastembed model unavailable; skipping embeddings"
                );
                None
            }
        })
        .as_ref()
}

fn is_embedding_eligible(row: &SectionRow) -> bool {
    row.heading_level >= 2 && row.body_text.len() <= SECTION_EMBED_MAX_BODY_BYTES
}

fn child_count_by_parent<'a>(
    edges: &'a [GraphEdgeArtifact],
    section_ids: &BTreeSet<&'a str>,
) -> HashMap<&'a str, u32> {
    let mut counts = HashMap::new();
    for edge in edges {
        if edge.relation != RelationKind::Contains {
            continue;
        }
        if !section_ids.contains(edge.source_stable_symbol_id.as_str()) {
            continue;
        }
        if edge
            .target_stable_symbol_id
            .as_deref()
            .is_some_and(|target| section_ids.contains(target))
        {
            *counts
                .entry(edge.source_stable_symbol_id.as_str())
                .or_insert(0) += 1;
        }
    }
    counts
}

fn parent_by_child<'a>(
    edges: &'a [GraphEdgeArtifact],
    section_ids: &BTreeSet<&'a str>,
) -> HashMap<&'a str, String> {
    let mut parents = HashMap::new();
    for edge in edges {
        if edge.relation != RelationKind::Contains
            || !section_ids.contains(edge.source_stable_symbol_id.as_str())
        {
            continue;
        }
        if let Some(target) = edge.target_stable_symbol_id.as_deref() {
            if section_ids.contains(target) {
                parents.insert(target, edge.source_stable_symbol_id.clone());
            }
        }
    }
    parents
}

pub fn backfill_missing_vectors(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
) -> Result<VectorBackfillStats> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| backfill_missing_vectors_without_current_runtime(artifact_dir, options))
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
    }
    backfill_missing_vectors_without_current_runtime(artifact_dir, options)
}

fn backfill_missing_vectors_without_current_runtime(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
) -> Result<VectorBackfillStats> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create vector backfill runtime")?;
    runtime.block_on(backfill_missing_vectors_async(artifact_dir, options))
}

async fn backfill_missing_vectors_async(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
) -> Result<VectorBackfillStats> {
    let embedding_model = EmbeddingModelSelection::from_env();
    let mut section_service =
        TextEmbeddingService::new(options, options.skip_section_embeddings, embedding_model);
    let mut symbol_service = TextEmbeddingService::new(
        options,
        options.skip_code_symbol_embeddings,
        embedding_model,
    );
    backfill_missing_vectors_async_with_embedding_model_and_embedder(
        artifact_dir,
        options,
        embedding_model,
        |phase, texts| match phase {
            SidecarPhase::Sections => section_service.embed_texts_locally(texts, "section"),
            SidecarPhase::CodeSymbols => symbol_service.embed_texts_locally(texts, "code symbol"),
        },
    )
    .await
}

async fn backfill_missing_vectors_async_with_embedding_model_and_embedder<F>(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
    embedding_model: EmbeddingModelSelection,
    mut embed_batch: F,
) -> Result<VectorBackfillStats>
where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    let code_symbols =
        backfill_symbol_vectors(artifact_dir, options, embedding_model, &mut embed_batch).await?;
    let sections =
        backfill_section_vectors(artifact_dir, options, embedding_model, &mut embed_batch).await?;
    Ok(VectorBackfillStats {
        sections,
        code_symbols,
    })
}

#[cfg(test)]
async fn backfill_missing_vectors_with<F>(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
    embed_batch: F,
) -> Result<VectorBackfillStats>
where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    backfill_missing_vectors_async_with_embedding_model_and_embedder(
        artifact_dir,
        options,
        EmbeddingModelSelection::NomicEmbedTextV15,
        embed_batch,
    )
    .await
}

async fn backfill_section_vectors<F>(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
    embedding_model: EmbeddingModelSelection,
    embed_batch: &mut F,
) -> Result<VectorBackfillTableStats>
where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    if options.skip_section_embeddings {
        return Ok(VectorBackfillTableStats::default());
    }
    let path = artifact_dir.join(SECTIONS_PARQUET);
    if !path.is_file() {
        return Ok(VectorBackfillTableStats::default());
    }
    let mut all_rows = load_all_section_rows_from_parquet(&path)?;
    let total_rows = all_rows.len();
    let null_vector_rows = all_rows.iter().filter(|row| row.vector.is_none()).count();
    let mut eligible: Vec<SectionRow> = all_rows
        .iter()
        .filter(|row| {
            row.vector.is_none()
                && section_row_matches_current_embedding_contract(row, embedding_model)
        })
        .cloned()
        .collect();
    sort_section_rows_by_vector_identity(&mut eligible);
    let eligible_rows = eligible.len();
    apply_backfill_embeddings_to_section_rows(&mut eligible, options, embedding_model, embed_batch);
    let filled_rows = merge_filled_section_vectors(&mut all_rows, eligible);
    write_record_batches_parquet(
        &path,
        sections_schema(),
        vec![rows_to_batch(all_rows, sections_schema())?],
    )?;
    Ok(VectorBackfillTableStats {
        total_rows,
        null_vector_rows,
        eligible_rows,
        filled_rows,
    })
}

async fn backfill_symbol_vectors<F>(
    artifact_dir: &Path,
    options: SectionEmbeddingOptions,
    embedding_model: EmbeddingModelSelection,
    embed_batch: &mut F,
) -> Result<VectorBackfillTableStats>
where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    if options.skip_code_symbol_embeddings {
        return Ok(VectorBackfillTableStats::default());
    }
    let path = artifact_dir.join(CODE_SYMBOLS_PARQUET);
    if !path.is_file() {
        return Ok(VectorBackfillTableStats::default());
    }
    let mut all_rows = load_all_symbol_rows_from_parquet(&path)?;
    let total_rows = all_rows.len();
    let null_vector_rows = all_rows.iter().filter(|row| row.vector.is_none()).count();
    let mut eligible: Vec<SymbolRow> = all_rows
        .iter()
        .filter(|row| {
            row.vector.is_none()
                && symbol_row_matches_current_embedding_contract(row, embedding_model)
        })
        .cloned()
        .collect();
    sort_symbol_rows_by_vector_identity(&mut eligible);
    let eligible_rows = eligible.len();
    apply_backfill_embeddings_to_symbol_rows(&mut eligible, options, embedding_model, embed_batch);
    let filled_rows = merge_filled_symbol_vectors(&mut all_rows, eligible);
    write_record_batches_parquet(
        &path,
        symbol_rows_schema(),
        vec![symbol_rows_to_batch(all_rows, symbol_rows_schema())?],
    )?;
    Ok(VectorBackfillTableStats {
        total_rows,
        null_vector_rows,
        eligible_rows,
        filled_rows,
    })
}

fn load_all_section_rows_from_parquet(path: &Path) -> Result<Vec<SectionRow>> {
    let mut rows = Vec::new();
    for batch in read_parquet_batches(path)? {
        rows.extend(section_rows_from_record_batch(&batch)?);
    }
    Ok(rows)
}

fn load_all_symbol_rows_from_parquet(path: &Path) -> Result<Vec<SymbolRow>> {
    let mut rows = Vec::new();
    for batch in read_parquet_batches(path)? {
        rows.extend(symbol_rows_from_record_batch(&batch)?);
    }
    Ok(rows)
}

fn read_parquet_batches(path: &Path) -> Result<Vec<RecordBatch>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open parquet `{}`", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("failed to read parquet `{}`", path.display()))?
        .build()
        .with_context(|| format!("failed to build parquet reader `{}`", path.display()))?;
    reader
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to collect parquet batches `{}`", path.display()))
}

fn section_rows_from_record_batch(batch: &RecordBatch) -> Result<Vec<SectionRow>> {
    let stable_symbol_ids = string_column(batch, "stable_symbol_id", "section")?;
    let file_paths = string_column(batch, "file_path", "section")?;
    let qualified_names = string_column(batch, "qualified_name", "section")?;
    let heading_levels = batch
        .column_by_name("heading_level")
        .context("section rows missing heading_level column")?
        .as_any()
        .downcast_ref::<UInt8Array>()
        .context("section heading_level column was not UInt8")?;
    let body_texts = batch
        .column_by_name("body_text")
        .context("section rows missing body_text column")?
        .as_any()
        .downcast_ref::<LargeStringArray>()
        .context("section body_text column was not LargeUtf8")?;
    let body_byte_starts = batch
        .column_by_name("body_byte_start")
        .context("section rows missing body_byte_start column")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .context("section body_byte_start column was not UInt64")?;
    let body_byte_ends = batch
        .column_by_name("body_byte_end")
        .context("section rows missing body_byte_end column")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .context("section body_byte_end column was not UInt64")?;
    let child_counts = batch
        .column_by_name("child_count")
        .context("section rows missing child_count column")?
        .as_any()
        .downcast_ref::<UInt32Array>()
        .context("section child_count column was not UInt32")?;
    let parent_stable_ids = string_column(batch, "parent_stable_id", "section")?;
    let content_hashes = string_column(batch, "content_hash", "section")?;
    let embedding_input_hashes = string_column(batch, EMBEDDING_INPUT_HASH_COLUMN, "section")?;
    let embedding_models = string_column(batch, EMBEDDING_MODEL_COLUMN, "section")?;
    let vectors = batch
        .column_by_name("vector")
        .and_then(|col| col.as_any().downcast_ref::<FixedSizeListArray>());
    let mut rows = Vec::with_capacity(batch.num_rows());
    for index in 0..batch.num_rows() {
        rows.push(SectionRow {
            stable_symbol_id: stable_symbol_ids.value(index).to_owned(),
            file_path: file_paths.value(index).to_owned(),
            qualified_name: qualified_names.value(index).to_owned(),
            heading_level: heading_levels.value(index),
            body_text: body_texts.value(index).to_owned(),
            body_byte_start: body_byte_starts.value(index),
            body_byte_end: body_byte_ends.value(index),
            child_count: child_counts.value(index),
            parent_stable_id: if parent_stable_ids.is_null(index) {
                None
            } else {
                Some(parent_stable_ids.value(index).to_owned())
            },
            content_hash: content_hashes.value(index).to_owned(),
            embedding_input_hash: embedding_input_hashes.value(index).to_owned(),
            embedding_model: embedding_models.value(index).to_owned(),
            vector: vector_at(vectors, index),
        });
    }
    Ok(rows)
}

fn symbol_rows_from_record_batch(batch: &RecordBatch) -> Result<Vec<SymbolRow>> {
    let stable_symbol_ids = string_column(batch, "stable_symbol_id", "code symbol")?;
    let file_paths = string_column(batch, "file_path", "code symbol")?;
    let qualified_names = string_column(batch, "qualified_name", "code symbol")?;
    let entity_names = string_column(batch, "entity_name", "code symbol")?;
    let symbol_kinds = string_column(batch, "symbol_kind", "code symbol")?;
    let embed_texts = batch
        .column_by_name("embed_text")
        .context("code symbol rows missing embed_text column")?
        .as_any()
        .downcast_ref::<LargeStringArray>()
        .context("code symbol embed_text column was not LargeUtf8")?;
    let content_hashes = string_column(batch, "content_hash", "code symbol")?;
    let embedding_input_hashes = string_column(batch, EMBEDDING_INPUT_HASH_COLUMN, "code symbol")?;
    let embedding_models = string_column(batch, EMBEDDING_MODEL_COLUMN, "code symbol")?;
    let vectors = batch
        .column_by_name("vector")
        .and_then(|col| col.as_any().downcast_ref::<FixedSizeListArray>());
    let mut rows = Vec::with_capacity(batch.num_rows());
    for index in 0..batch.num_rows() {
        rows.push(SymbolRow {
            stable_symbol_id: stable_symbol_ids.value(index).to_owned(),
            file_path: file_paths.value(index).to_owned(),
            qualified_name: qualified_names.value(index).to_owned(),
            entity_name: entity_names.value(index).to_owned(),
            symbol_kind: symbol_kinds.value(index).to_owned(),
            embed_text: embed_texts.value(index).to_owned(),
            vector: vector_at(vectors, index),
            content_hash: content_hashes.value(index).to_owned(),
            embedding_input_hash: embedding_input_hashes.value(index).to_owned(),
            embedding_model: embedding_models.value(index).to_owned(),
        });
    }
    Ok(rows)
}

fn vector_at(vectors: Option<&FixedSizeListArray>, index: usize) -> Option<Vec<f32>> {
    let vectors = vectors?;
    if vectors.is_null(index) {
        return None;
    }
    vectors
        .value(index)
        .as_any()
        .downcast_ref::<Float32Array>()
        .map(|array| array.values().to_vec())
}

fn merge_filled_section_vectors(all_rows: &mut [SectionRow], filled: Vec<SectionRow>) -> usize {
    let filled_by_key: HashMap<(String, String, String, String), Vec<f32>> = filled
        .into_iter()
        .filter_map(|row| {
            row.vector.map(|vector| {
                (
                    (
                        row.file_path,
                        row.embedding_model,
                        row.embedding_input_hash,
                        row.stable_symbol_id,
                    ),
                    vector,
                )
            })
        })
        .collect();
    let mut filled_rows = 0usize;
    for row in all_rows {
        if row.vector.is_some() {
            continue;
        }
        let key = (
            row.file_path.clone(),
            row.embedding_model.clone(),
            row.embedding_input_hash.clone(),
            row.stable_symbol_id.clone(),
        );
        if let Some(vector) = filled_by_key.get(&key) {
            row.vector = Some(vector.clone());
            filled_rows += 1;
        }
    }
    filled_rows
}

fn merge_filled_symbol_vectors(all_rows: &mut [SymbolRow], filled: Vec<SymbolRow>) -> usize {
    let filled_by_key: HashMap<(String, String, String, String), Vec<f32>> = filled
        .into_iter()
        .filter_map(|row| {
            row.vector.map(|vector| {
                (
                    (
                        row.file_path,
                        row.embedding_model,
                        row.embedding_input_hash,
                        row.stable_symbol_id,
                    ),
                    vector,
                )
            })
        })
        .collect();
    let mut filled_rows = 0usize;
    for row in all_rows {
        if row.vector.is_some() {
            continue;
        }
        let key = (
            row.file_path.clone(),
            row.embedding_model.clone(),
            row.embedding_input_hash.clone(),
            row.stable_symbol_id.clone(),
        );
        if let Some(vector) = filled_by_key.get(&key) {
            row.vector = Some(vector.clone());
            filled_rows += 1;
        }
    }
    filled_rows
}

fn string_column<'a>(
    batch: &'a RecordBatch,
    column: &str,
    table_label: &str,
) -> Result<&'a StringArray> {
    batch
        .column_by_name(column)
        .with_context(|| format!("{table_label} backfill rows missing {column} column"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("{table_label} {column} column was not Utf8"))
}

fn section_row_matches_current_embedding_contract(
    row: &SectionRow,
    embedding_model: EmbeddingModelSelection,
) -> bool {
    row.embedding_model == embedding_model.model_name()
        && row.embedding_input_hash
            == section_embedding_input_hash_for_model(
                row.qualified_name.as_str(),
                row.body_text.as_str(),
                embedding_model,
            )
        && is_embedding_eligible(row)
}

fn symbol_row_matches_current_embedding_contract(
    row: &SymbolRow,
    embedding_model: EmbeddingModelSelection,
) -> bool {
    row.embedding_model == embedding_model.model_name()
        && row.embedding_input_hash
            == symbol_embedding_input_hash_for_model(
                row.embed_text.as_str(),
                false,
                embedding_model,
            )
        && !row.embed_text.trim().is_empty()
}

fn apply_backfill_embeddings_to_section_rows<F>(
    rows: &mut [SectionRow],
    options: SectionEmbeddingOptions,
    embedding_model: EmbeddingModelSelection,
    embed_batch: &mut F,
) where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    let vectors = embed_backfill_inputs_with(
        rows.len(),
        section_embedding_inputs(rows, embedding_model),
        options.batch_size,
        SidecarPhase::Sections,
        "section",
        embed_batch,
    );
    for (row, vector) in rows.iter_mut().zip(vectors) {
        row.vector = vector;
    }
}

fn apply_backfill_embeddings_to_symbol_rows<F>(
    rows: &mut [SymbolRow],
    options: SectionEmbeddingOptions,
    embedding_model: EmbeddingModelSelection,
    embed_batch: &mut F,
) where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    let vectors = embed_backfill_inputs_with(
        rows.len(),
        symbol_embedding_inputs(rows, embedding_model),
        options.batch_size,
        SidecarPhase::CodeSymbols,
        "code symbol",
        embed_batch,
    );
    for (row, vector) in rows.iter_mut().zip(vectors) {
        row.vector = vector;
    }
}

fn embed_backfill_inputs_with<F>(
    row_count: usize,
    eligible: Vec<EmbeddingTextInput<'_>>,
    batch_size: usize,
    phase: SidecarPhase,
    embedding_kind: &'static str,
    embed_batch: &mut F,
) -> Vec<Option<Vec<f32>>>
where
    F: FnMut(SidecarPhase, &[&str]) -> Result<Vec<Vec<f32>>>,
{
    let mut result = vec![None; row_count];
    if eligible.is_empty() {
        return result;
    }
    let batch_size = if batch_size == 0 {
        SECTION_EMBED_BATCH_SIZE_DEFAULT
    } else {
        batch_size
    };
    for chunk in eligible.chunks(batch_size) {
        let texts: Vec<&str> = chunk.iter().map(|input| input.text.as_ref()).collect();
        let embeddings = match embed_batch(phase, &texts) {
            Ok(embeddings) => embeddings,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    embedding_kind,
                    "vector backfill embedding batch failed; stopping this table"
                );
                return result;
            }
        };
        if !apply_embeddings_to_inputs(&mut result, chunk, embeddings, embedding_kind) {
            return result;
        }
    }
    result
}

fn sort_section_rows_by_vector_identity(rows: &mut [SectionRow]) {
    rows.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.stable_symbol_id.cmp(&b.stable_symbol_id))
            .then(a.embedding_model.cmp(&b.embedding_model))
            .then(a.embedding_input_hash.cmp(&b.embedding_input_hash))
    });
}

fn sort_symbol_rows_by_vector_identity(rows: &mut [SymbolRow]) {
    rows.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.stable_symbol_id.cmp(&b.stable_symbol_id))
            .then(a.embedding_model.cmp(&b.embedding_model))
            .then(a.embedding_input_hash.cmp(&b.embedding_input_hash))
    });
}

fn previous_sidecars_match_embedding_model(
    prev_dir: &Path,
    embedding_model: EmbeddingModelSelection,
) -> bool {
    let expected = embedding_model.model_name();
    if let Ok(rows) = load_all_section_rows_from_parquet(&prev_dir.join(SECTIONS_PARQUET)) {
        if rows
            .iter()
            .any(|row| row.vector.is_some() && row.embedding_model != expected)
        {
            return false;
        }
    }
    if let Ok(rows) = load_all_symbol_rows_from_parquet(&prev_dir.join(CODE_SYMBOLS_PARQUET)) {
        if rows
            .iter()
            .any(|row| row.vector.is_some() && row.embedding_model != expected)
        {
            return false;
        }
    }
    true
}

fn retained_section_rows_from_prev_parquet(
    prev_dir: &Path,
    delta: &SidecarDelta,
) -> Vec<SectionRow> {
    let Ok(rows) = load_all_section_rows_from_parquet(&prev_dir.join(SECTIONS_PARQUET)) else {
        return Vec::new();
    };
    let excluded = excluded_delta_paths(delta);
    rows.into_iter()
        .filter(|row| !excluded.contains(row.file_path.as_str()))
        .collect()
}

fn retained_symbol_rows_from_prev_parquet(prev_dir: &Path, delta: &SidecarDelta) -> Vec<SymbolRow> {
    let Ok(rows) = load_all_symbol_rows_from_parquet(&prev_dir.join(CODE_SYMBOLS_PARQUET)) else {
        return Vec::new();
    };
    let excluded = excluded_delta_paths(delta);
    rows.into_iter()
        .filter(|row| !excluded.contains(row.file_path.as_str()))
        .collect()
}

fn excluded_delta_paths(delta: &SidecarDelta) -> BTreeSet<&str> {
    delta
        .changed_paths
        .iter()
        .chain(delta.deleted_paths.iter())
        .map(String::as_str)
        .collect()
}

fn fill_section_vectors_from_prev_parquet(rows: &mut [SectionRow], prev_dir: &Path) {
    fill_row_vectors_from_parquet(
        prev_dir.join(SECTIONS_PARQUET),
        rows,
        |row| {
            (
                row.file_path.clone(),
                row.embedding_model.clone(),
                row.embedding_input_hash.clone(),
                row.stable_symbol_id.clone(),
            )
        },
        |row, vector| row.vector = Some(vector),
    );
}

fn fill_symbol_vectors_from_prev_parquet(rows: &mut [SymbolRow], prev_dir: &Path) {
    fill_row_vectors_from_parquet(
        prev_dir.join(CODE_SYMBOLS_PARQUET),
        rows,
        |row| {
            (
                row.file_path.clone(),
                row.embedding_model.clone(),
                row.embedding_input_hash.clone(),
                row.stable_symbol_id.clone(),
            )
        },
        |row, vector| row.vector = Some(vector),
    );
}

fn fill_row_vectors_from_parquet<T>(
    path: PathBuf,
    rows: &mut [T],
    key_of: impl Fn(&T) -> (String, String, String, String),
    set_vector: impl Fn(&mut T, Vec<f32>),
) {
    if rows.is_empty() || !path.is_file() {
        return;
    }
    let Ok(file) = fs::File::open(&path) else {
        return;
    };
    let Ok(builder) = ParquetRecordBatchReaderBuilder::try_new(file) else {
        return;
    };
    let Ok(reader) = builder.build() else {
        return;
    };
    let mut previous = HashMap::<(String, String, String, String), Vec<f32>>::new();
    for batch in reader.flatten() {
        let Some(ids) = batch
            .column_by_name("stable_symbol_id")
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
        else {
            continue;
        };
        let Some(file_paths) = batch
            .column_by_name("file_path")
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
        else {
            continue;
        };
        let Some(models) = batch
            .column_by_name(EMBEDDING_MODEL_COLUMN)
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
        else {
            continue;
        };
        let Some(hashes) = batch
            .column_by_name(EMBEDDING_INPUT_HASH_COLUMN)
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
        else {
            continue;
        };
        let Some(vectors) = batch
            .column_by_name("vector")
            .and_then(|col| col.as_any().downcast_ref::<FixedSizeListArray>())
        else {
            continue;
        };
        let values = vectors
            .values()
            .as_any()
            .downcast_ref::<Float32Array>()
            .expect("vector items are f32");
        for row in 0..batch.num_rows() {
            if vectors.is_null(row) {
                continue;
            }
            let start = row * EMBEDDING_VECTOR_DIMENSIONS;
            let end = start + EMBEDDING_VECTOR_DIMENSIONS;
            if end > values.len() {
                continue;
            }
            previous.insert(
                (
                    file_paths.value(row).to_owned(),
                    models.value(row).to_owned(),
                    hashes.value(row).to_owned(),
                    ids.value(row).to_owned(),
                ),
                values.values()[start..end].to_vec(),
            );
        }
    }
    for row in rows {
        if let Some(vector) = previous.get(&key_of(row)) {
            set_vector(row, vector.clone());
        }
    }
}

/// Test helper: calls `fill_section_vectors_from_prev` in a blocking context.
#[cfg(test)]
async fn carry_forward_section_vectors(
    mut rows: Vec<SectionRow>,
    prev_dir: &Path,
) -> Vec<SectionRow> {
    fill_section_vectors_from_prev_parquet(&mut rows, prev_dir);
    rows
}

/// Test helper: calls `fill_symbol_vectors_from_prev` in a blocking context.
#[cfg(test)]
async fn carry_forward_symbol_vectors(mut rows: Vec<SymbolRow>, prev_dir: &Path) -> Vec<SymbolRow> {
    fill_symbol_vectors_from_prev_parquet(&mut rows, prev_dir);
    rows
}

fn rows_to_batch(rows: Vec<SectionRow>, schema: Arc<Schema>) -> Result<RecordBatch> {
    let mut stable_symbol_ids = Vec::with_capacity(rows.len());
    let mut file_paths = Vec::with_capacity(rows.len());
    let mut qualified_names = Vec::with_capacity(rows.len());
    let mut heading_levels = Vec::with_capacity(rows.len());
    let mut body_texts = Vec::with_capacity(rows.len());
    let mut body_byte_starts = Vec::with_capacity(rows.len());
    let mut body_byte_ends = Vec::with_capacity(rows.len());
    let mut child_counts = Vec::with_capacity(rows.len());
    let mut parent_stable_ids = Vec::with_capacity(rows.len());
    let mut content_hashes = Vec::with_capacity(rows.len());
    let mut embedding_input_hashes = Vec::with_capacity(rows.len());
    let mut embedding_models = Vec::with_capacity(rows.len());
    let mut flat_vectors = Vec::with_capacity(rows.len() * EMBEDDING_VECTOR_DIMENSIONS);
    let mut vector_validity = Vec::with_capacity(rows.len());

    for row in rows {
        stable_symbol_ids.push(row.stable_symbol_id);
        file_paths.push(row.file_path);
        qualified_names.push(row.qualified_name);
        heading_levels.push(row.heading_level);
        body_texts.push(row.body_text);
        body_byte_starts.push(row.body_byte_start);
        body_byte_ends.push(row.body_byte_end);
        child_counts.push(row.child_count);
        parent_stable_ids.push(row.parent_stable_id);
        content_hashes.push(row.content_hash);
        embedding_input_hashes.push(row.embedding_input_hash);
        embedding_models.push(row.embedding_model);
        if let Some(vector) = row
            .vector
            .filter(|vector| vector.len() == EMBEDDING_VECTOR_DIMENSIONS)
        {
            flat_vectors.extend(vector);
            vector_validity.push(true);
        } else {
            flat_vectors.extend(std::iter::repeat_n(0.0f32, EMBEDDING_VECTOR_DIMENSIONS));
            vector_validity.push(false);
        }
    }

    let vector_array = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        EMBEDDING_VECTOR_DIMENSIONS as i32,
        Arc::new(Float32Array::from(flat_vectors)),
        Some(NullBuffer::from(vector_validity)),
    )
    .context("failed to build LanceDB section vector array")?;

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(stable_symbol_ids)),
            Arc::new(StringArray::from(file_paths)),
            Arc::new(StringArray::from(qualified_names)),
            Arc::new(UInt8Array::from(heading_levels)),
            Arc::new(LargeStringArray::from(body_texts)),
            Arc::new(UInt64Array::from(body_byte_starts)),
            Arc::new(UInt64Array::from(body_byte_ends)),
            Arc::new(UInt32Array::from(child_counts)),
            Arc::new(StringArray::from(parent_stable_ids)),
            Arc::new(StringArray::from(content_hashes)),
            Arc::new(vector_array),
            Arc::new(StringArray::from(embedding_input_hashes)),
            Arc::new(StringArray::from(embedding_models)),
        ],
    )
    .context("failed to build LanceDB sections batch")
}

fn symbol_rows_to_batch(rows: Vec<SymbolRow>, schema: Arc<Schema>) -> Result<RecordBatch> {
    let mut stable_symbol_ids = Vec::with_capacity(rows.len());
    let mut file_paths = Vec::with_capacity(rows.len());
    let mut qualified_names = Vec::with_capacity(rows.len());
    let mut entity_names = Vec::with_capacity(rows.len());
    let mut symbol_kinds = Vec::with_capacity(rows.len());
    let mut embed_texts = Vec::with_capacity(rows.len());
    let mut flat_vectors = Vec::with_capacity(rows.len() * EMBEDDING_VECTOR_DIMENSIONS);
    let mut vector_validity = Vec::with_capacity(rows.len());
    let mut content_hashes = Vec::with_capacity(rows.len());
    let mut embedding_input_hashes = Vec::with_capacity(rows.len());
    let mut embedding_models = Vec::with_capacity(rows.len());

    for row in rows {
        stable_symbol_ids.push(row.stable_symbol_id);
        file_paths.push(row.file_path);
        qualified_names.push(row.qualified_name);
        entity_names.push(row.entity_name);
        symbol_kinds.push(row.symbol_kind);
        embed_texts.push(row.embed_text);
        if let Some(vector) = row
            .vector
            .filter(|vector| vector.len() == EMBEDDING_VECTOR_DIMENSIONS)
        {
            flat_vectors.extend(vector);
            vector_validity.push(true);
        } else {
            flat_vectors.extend(std::iter::repeat_n(0.0f32, EMBEDDING_VECTOR_DIMENSIONS));
            vector_validity.push(false);
        }
        content_hashes.push(row.content_hash);
        embedding_input_hashes.push(row.embedding_input_hash);
        embedding_models.push(row.embedding_model);
    }

    let vector_array = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        EMBEDDING_VECTOR_DIMENSIONS as i32,
        Arc::new(Float32Array::from(flat_vectors)),
        Some(NullBuffer::from(vector_validity)),
    )
    .context("failed to build LanceDB code symbol vector array")?;

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(stable_symbol_ids)),
            Arc::new(StringArray::from(file_paths)),
            Arc::new(StringArray::from(qualified_names)),
            Arc::new(StringArray::from(entity_names)),
            Arc::new(StringArray::from(symbol_kinds)),
            Arc::new(LargeStringArray::from(embed_texts)),
            Arc::new(vector_array),
            Arc::new(StringArray::from(content_hashes)),
            Arc::new(StringArray::from(embedding_input_hashes)),
            Arc::new(StringArray::from(embedding_models)),
        ],
    )
    .context("failed to build LanceDB code symbols batch")
}

fn sections_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("stable_symbol_id", DataType::Utf8, false),
        Field::new("file_path", DataType::Utf8, false),
        Field::new("qualified_name", DataType::Utf8, false),
        Field::new("heading_level", DataType::UInt8, false),
        Field::new("body_text", DataType::LargeUtf8, false),
        Field::new("body_byte_start", DataType::UInt64, false),
        Field::new("body_byte_end", DataType::UInt64, false),
        Field::new("child_count", DataType::UInt32, false),
        Field::new("parent_stable_id", DataType::Utf8, true),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_VECTOR_DIMENSIONS as i32,
            ),
            true,
        ),
        Field::new(EMBEDDING_INPUT_HASH_COLUMN, DataType::Utf8, false),
        Field::new(EMBEDDING_MODEL_COLUMN, DataType::Utf8, false),
    ]))
}

fn symbol_rows_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("stable_symbol_id", DataType::Utf8, false),
        Field::new("file_path", DataType::Utf8, false),
        Field::new("qualified_name", DataType::Utf8, false),
        Field::new("entity_name", DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("embed_text", DataType::LargeUtf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_VECTOR_DIMENSIONS as i32,
            ),
            true,
        ),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new(EMBEDDING_INPUT_HASH_COLUMN, DataType::Utf8, false),
        Field::new(EMBEDDING_MODEL_COLUMN, DataType::Utf8, false),
    ]))
}

fn read_file_bytes(worktree_root: &Path, path: &str) -> Result<Vec<u8>> {
    fs::read(worktree_root.join(path))
        .with_context(|| format!("failed to read `{}`", worktree_root.join(path).display()))
}

fn read_git_blob_bytes(worktree_root: &Path, oid: &str) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(["cat-file", "blob", oid])
        .current_dir(worktree_root)
        .output()
        .with_context(|| format!("failed to spawn git cat-file blob `{oid}`"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "git cat-file blob `{oid}` failed in `{}`: {}",
        worktree_root.display(),
        stderr.trim()
    )
}

fn heading_level(body_text: &str) -> u8 {
    body_text
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b'#')
        .count()
        .try_into()
        .unwrap_or(u8::MAX)
}

fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

fn embedding_model_from_spur_config() -> Option<String> {
    if let Some(model) = std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_config_toml(&cwd))
        .and_then(|path| graph_embedding_model_in_toml(&path))
    {
        return Some(model);
    }

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".spur").join("config.toml"))
        .and_then(|path| graph_embedding_model_in_toml(&path))
}

fn find_project_config_toml(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        let candidate = current.join(".spur").join("config.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        current = current.parent()?;
    }
}

fn graph_embedding_model_in_toml(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value = raw.parse::<toml::Value>().ok()?;
    value
        .get("graph")?
        .get("embedding_model")?
        .as_str()
        .map(str::to_owned)
}

pub fn fastembed_cache_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(
            PathBuf::from(home)
                .join(".spur")
                .join("cache")
                .join("fastembed"),
        );
    }

    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .map(|xdg_cache| xdg_cache.join("spur").join("fastembed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn section_row_fixture(heading_level: u8, body_text: String) -> SectionRow {
        let qualified_name = "docs/example.md::Section".to_owned();
        let embedding_input_hash = section_embedding_input_hash_for_model(
            &qualified_name,
            &body_text,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        SectionRow {
            stable_symbol_id: "symbol".to_owned(),
            file_path: "docs/example.md".to_owned(),
            qualified_name,
            heading_level,
            body_text,
            body_byte_start: 0,
            body_byte_end: 0,
            child_count: 0,
            parent_stable_id: None,
            content_hash: "hash".to_owned(),
            embedding_input_hash,
            embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
            vector: None,
        }
    }

    fn versioned_section_row(
        stable_symbol_id: &str,
        file_path: &str,
        content_hash: &str,
    ) -> SectionRow {
        let body_text = format!("## {stable_symbol_id}\n\nBody.");
        let qualified_name = stable_symbol_id.to_owned();
        let embedding_input_hash = section_embedding_input_hash_for_model(
            &qualified_name,
            &body_text,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        SectionRow {
            stable_symbol_id: stable_symbol_id.to_owned(),
            file_path: file_path.to_owned(),
            qualified_name,
            heading_level: 2,
            body_text,
            body_byte_start: 0,
            body_byte_end: 0,
            child_count: 0,
            parent_stable_id: None,
            content_hash: content_hash.to_owned(),
            embedding_input_hash,
            embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
            vector: None,
        }
    }

    fn symbol_row_fixture(stable_symbol_id: &str, embed_text: &str) -> SymbolRow {
        let embedding_input_hash = symbol_embedding_input_hash_for_model(
            embed_text,
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        SymbolRow {
            stable_symbol_id: stable_symbol_id.to_owned(),
            file_path: "src/lib.rs".to_owned(),
            qualified_name: stable_symbol_id.to_owned(),
            entity_name: stable_symbol_id.to_owned(),
            symbol_kind: "function".to_owned(),
            embed_text: embed_text.to_owned(),
            vector: None,
            content_hash: "hash".to_owned(),
            embedding_input_hash,
            embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
        }
    }

    fn fake_vector(seed: f32) -> Vec<f32> {
        vec![seed; EMBEDDING_VECTOR_DIMENSIONS]
    }

    fn write_source(root: &Path, path: &str, source: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("source parent")).expect("create source parent");
        fs::write(path, source).expect("write source");
    }

    fn sidecar_delta(changed_paths: &[&str], deleted_paths: &[&str]) -> SidecarDelta {
        SidecarDelta::new(
            changed_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            deleted_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
        )
    }

    fn incremental_skip_sidecar_options(
        previous_artifact_dir: &Path,
        changed_paths: &[&str],
        deleted_paths: &[&str],
    ) -> SectionSidecarOptions {
        SectionSidecarOptions {
            embedding: SectionEmbeddingOptions {
                skip_section_embeddings: true,
                skip_code_symbol_embeddings: true,
                batch_size: SECTION_EMBED_BATCH_SIZE_DEFAULT,
            },
            write_batch_size: SECTION_WRITE_BATCH_SIZE_DEFAULT,
            previous_artifact_dir: Some(previous_artifact_dir.to_path_buf()),
            delta: Some(sidecar_delta(changed_paths, deleted_paths)),
        }
    }

    fn many_functions_source(prefix: &str, count: usize) -> String {
        let mut source = String::new();
        for index in 0..count {
            let name = format!("{prefix}_{index:03}");
            source.push_str(&format!(
                "/// Stable documentation for {name} that is deliberately long enough to form embedding text.\n",
            ));
            source.push_str(&format!("pub fn {name}() {{}}\n\n"));
        }
        source
    }

    fn code_symbol(
        path: &str,
        source: &str,
        stable_symbol_id: &str,
        entity_name: &str,
    ) -> GraphSymbolArtifact {
        let needle = format!("pub fn {entity_name}");
        let start = source
            .find(&needle)
            .unwrap_or_else(|| panic!("missing function `{entity_name}` in `{path}`"));
        let line_len = source[start..].lines().next().expect("function line").len();
        let line = source[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        GraphSymbolArtifact {
            stable_symbol_id: stable_symbol_id.to_owned(),
            file_path: path.to_owned(),
            byte_range: [start, start + line_len],
            line_range: [line, line],
            entity_name: entity_name.to_owned(),
            qualified_name: entity_name.to_owned(),
            symbol_kind: "function".to_owned(),
            anchor_hash: format!("anchor:{stable_symbol_id}"),
            enclosing_scope: None,
        }
    }

    fn code_symbols_for_functions(
        path: &str,
        source: &str,
        ids_and_names: &[(&str, String)],
    ) -> Vec<GraphSymbolArtifact> {
        ids_and_names
            .iter()
            .map(|(stable_symbol_id, entity_name)| {
                code_symbol(path, source, stable_symbol_id, entity_name)
            })
            .collect()
    }

    fn markdown_section_symbols(
        path: &str,
        source: &str,
        ids_and_headings: &[(&str, &str)],
    ) -> Vec<GraphSymbolArtifact> {
        let headings = ids_and_headings
            .iter()
            .map(|(_, heading)| *heading)
            .collect::<Vec<_>>();
        let ranges = section_ranges(source, &headings);
        ids_and_headings
            .iter()
            .zip(ranges)
            .map(|((stable_symbol_id, heading), [start, end])| {
                let line = source[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                let end_line = line
                    + source[start..end]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                let entity_name = heading.trim_start_matches('#').trim().to_owned();
                GraphSymbolArtifact {
                    stable_symbol_id: (*stable_symbol_id).to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [start, end],
                    line_range: [line, end_line],
                    entity_name: entity_name.clone(),
                    qualified_name: format!("{path}::{entity_name}"),
                    symbol_kind: "section".to_owned(),
                    anchor_hash: format!("anchor:{stable_symbol_id}"),
                    enclosing_scope: None,
                }
            })
            .collect()
    }

    fn graph_artifact_for_code_files(
        graph_content_hash: &str,
        files: Vec<(&str, &str, Vec<GraphSymbolArtifact>)>,
    ) -> GraphIndexArtifact {
        let mut file_manifests = Vec::new();
        let mut graph_files = Vec::new();
        let mut file_node_ids = Vec::new();
        let mut symbols = Vec::new();
        let mut symbol_node_ids = Vec::new();
        let mut next_node_id = 1_u64;

        for (path, source, mut file_symbols) in files {
            let stable_file_id = format!("file:{path}");
            let file_node_id = crate::NodeId(next_node_id);
            next_node_id += 1;
            file_manifests.push(GraphFileManifestEntry {
                stable_file_id: stable_file_id.clone(),
                path: path.to_owned(),
                content_oid: blake3_hex(source.as_bytes()),
                node_ids: vec![file_node_id],
            });
            graph_files.push(crate::GraphFileArtifact {
                stable_file_id,
                file_path: path.to_owned(),
            });
            file_node_ids.push(file_node_id);
            for symbol in file_symbols.drain(..) {
                symbols.push(symbol);
                symbol_node_ids.push(crate::NodeId(next_node_id));
                next_node_id += 1;
            }
        }

        GraphIndexArtifact {
            macro_invocations: None,
            header: crate::GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: graph_content_hash.to_owned(),
            file_manifests,
            files: graph_files,
            file_node_ids,
            symbols,
            symbol_node_ids,
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn symbol_rows_from_artifact(artifact: &GraphIndexArtifact, root: &Path) -> Vec<SymbolRow> {
        let mut batcher = SymbolRowBatcher::new(
            artifact,
            root,
            4096,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let mut rows = Vec::new();
        while let Some(batch) = batcher.next_batch().expect("symbol row batch") {
            rows.extend(batch);
        }
        rows
    }

    fn section_rows_from_artifact(artifact: &GraphIndexArtifact, root: &Path) -> Vec<SectionRow> {
        let mut batcher = SectionRowBatcher::new(
            artifact,
            root,
            4096,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let mut rows = Vec::new();
        while let Some(batch) = batcher.next_batch().expect("section row batch") {
            rows.extend(batch);
        }
        rows
    }

    async fn write_previous_section_sidecar_rows(dir: &Path, rows: Vec<SectionRow>) {
        fs::create_dir_all(dir).expect("create previous section sidecar dir");
        write_record_batches_parquet(
            &dir.join(SECTIONS_PARQUET),
            sections_schema(),
            vec![rows_to_batch(rows, sections_schema()).expect("previous section batch")],
        )
        .expect("write previous sections parquet");
    }

    async fn write_previous_symbol_sidecar_rows(dir: &Path, rows: Vec<SymbolRow>) {
        fs::create_dir_all(dir).expect("create previous sidecar dir");
        write_record_batches_parquet(
            &dir.join(CODE_SYMBOLS_PARQUET),
            symbol_rows_schema(),
            vec![symbol_rows_to_batch(rows, symbol_rows_schema()).expect("previous symbol batch")],
        )
        .expect("write previous symbols parquet");
    }

    #[derive(Debug)]
    struct StoredSymbolRow {
        stable_symbol_id: String,
        file_path: String,
        embedding_model: String,
        has_vector: bool,
        vector: Option<Vec<f32>>,
    }

    #[derive(Debug)]
    struct StoredSectionRow {
        stable_symbol_id: String,
        file_path: String,
        embedding_model: String,
        has_vector: bool,
        vector: Option<Vec<f32>>,
    }

    async fn read_stored_section_rows(dir: &Path) -> Vec<StoredSectionRow> {
        let mut rows = Vec::new();
        for batch in
            read_parquet_batches(&dir.join(SECTIONS_PARQUET)).expect("read sections parquet")
        {
            let ids = batch
                .column_by_name("stable_symbol_id")
                .expect("stable_symbol_id column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("stable_symbol_id utf8");
            let file_paths = batch
                .column_by_name("file_path")
                .expect("file_path column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("file_path utf8");
            let embedding_models = batch
                .column_by_name(EMBEDDING_MODEL_COLUMN)
                .expect("embedding_model column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("embedding_model utf8");
            let vectors = batch
                .column_by_name("vector")
                .expect("vector column")
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .expect("vector fixed-size list");
            for index in 0..batch.num_rows() {
                let vector = if vectors.is_null(index) {
                    None
                } else {
                    vectors
                        .value(index)
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .map(|array| array.values().to_vec())
                };
                rows.push(StoredSectionRow {
                    stable_symbol_id: ids.value(index).to_owned(),
                    file_path: file_paths.value(index).to_owned(),
                    embedding_model: embedding_models.value(index).to_owned(),
                    has_vector: !vectors.is_null(index),
                    vector,
                });
            }
        }
        rows
    }

    async fn section_fts_hit_count(dir: &Path, query: &str) -> usize {
        let mut hits = 0usize;
        for batch in
            read_parquet_batches(&dir.join(SECTIONS_PARQUET)).expect("read sections parquet")
        {
            let body_texts = batch
                .column_by_name("body_text")
                .expect("body_text column")
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .expect("body_text large utf8");
            for index in 0..batch.num_rows() {
                if body_texts
                    .value(index)
                    .to_lowercase()
                    .contains(&query.to_lowercase())
                {
                    hits += 1;
                }
            }
        }
        hits
    }

    async fn read_stored_symbol_rows(dir: &Path) -> Vec<StoredSymbolRow> {
        let mut rows = Vec::new();
        for batch in
            read_parquet_batches(&dir.join(CODE_SYMBOLS_PARQUET)).expect("read symbols parquet")
        {
            let ids = batch
                .column_by_name("stable_symbol_id")
                .expect("stable_symbol_id column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("stable_symbol_id utf8");
            let file_paths = batch
                .column_by_name("file_path")
                .expect("file_path column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("file_path utf8");
            let embedding_models = batch
                .column_by_name(EMBEDDING_MODEL_COLUMN)
                .expect("embedding_model column")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("embedding_model utf8");
            let vectors = batch
                .column_by_name("vector")
                .expect("vector column")
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .expect("vector fixed-size list");
            for index in 0..batch.num_rows() {
                let vector = if vectors.is_null(index) {
                    None
                } else {
                    vectors
                        .value(index)
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .map(|array| array.values().to_vec())
                };
                rows.push(StoredSymbolRow {
                    stable_symbol_id: ids.value(index).to_owned(),
                    file_path: file_paths.value(index).to_owned(),
                    embedding_model: embedding_models.value(index).to_owned(),
                    has_vector: !vectors.is_null(index),
                    vector,
                });
            }
        }
        rows
    }

    fn code_symbol_batches(events: &[SectionSidecarProgressEvent]) -> Vec<(usize, usize)> {
        let mut in_code_symbols = false;
        let mut batches = Vec::new();
        for event in events {
            match event {
                SectionSidecarProgressEvent::CodeSymbolsStarted { .. } => {
                    in_code_symbols = true;
                }
                SectionSidecarProgressEvent::Finished {
                    phase: SidecarPhase::CodeSymbols,
                    ..
                } => {
                    in_code_symbols = false;
                }
                SectionSidecarProgressEvent::BatchStarted {
                    batch_rows,
                    embedding_eligible_rows,
                    ..
                } if in_code_symbols => {
                    batches.push((*batch_rows, *embedding_eligible_rows));
                }
                _ => {}
            }
        }
        batches
    }

    fn section_batches(events: &[SectionSidecarProgressEvent]) -> Vec<(usize, usize)> {
        let mut in_sections = true;
        let mut batches = Vec::new();
        for event in events {
            match event {
                SectionSidecarProgressEvent::CodeSymbolsStarted { .. }
                | SectionSidecarProgressEvent::Finished {
                    phase: SidecarPhase::Sections,
                    ..
                } => {
                    in_sections = false;
                }
                SectionSidecarProgressEvent::BatchStarted {
                    batch_rows,
                    embedding_eligible_rows,
                    ..
                } if in_sections => {
                    batches.push((*batch_rows, *embedding_eligible_rows));
                }
                _ => {}
            }
        }
        batches
    }

    fn code_symbol_finished(
        events: &[SectionSidecarProgressEvent],
    ) -> (usize, usize, usize, usize) {
        events
            .iter()
            .find_map(|event| match event {
                SectionSidecarProgressEvent::Finished {
                    total_rows,
                    final_rows,
                    written_rows,
                    skipped_existing_rows,
                    phase: SidecarPhase::CodeSymbols,
                    ..
                } => Some((
                    *total_rows,
                    *final_rows,
                    *written_rows,
                    *skipped_existing_rows,
                )),
                _ => None,
            })
            .expect("code symbol finished event")
    }

    fn section_finished(events: &[SectionSidecarProgressEvent]) -> (usize, usize, usize, usize) {
        events
            .iter()
            .find_map(|event| match event {
                SectionSidecarProgressEvent::Finished {
                    total_rows,
                    final_rows,
                    written_rows,
                    skipped_existing_rows,
                    phase: SidecarPhase::Sections,
                    ..
                } => Some((
                    *total_rows,
                    *final_rows,
                    *written_rows,
                    *skipped_existing_rows,
                )),
                _ => None,
            })
            .expect("section finished event")
    }

    fn code_symbol_indexing_events(events: &[SectionSidecarProgressEvent]) -> Vec<&'static str> {
        events
            .iter()
            .filter_map(|event| match event {
                SectionSidecarProgressEvent::Indexing {
                    label,
                    phase: SidecarPhase::CodeSymbols,
                } => Some(*label),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn sidecar_indexing_progress_reports_only_fts_indexes() {
        let root = tempfile::tempdir().expect("root");
        let sidecar_dir = tempfile::tempdir().expect("sidecar");
        let markdown_source = "## Topic\n\nSearchable markdown body.\n";
        let rust_source = "/// Searchable symbol docs.\npub fn searchable_symbol() {}\n";
        write_source(root.path(), "docs/topic.md", markdown_source);
        write_source(root.path(), "src/lib.rs", rust_source);
        let artifact = graph_artifact_for_code_files(
            "fts-indexing-only",
            vec![
                (
                    "docs/topic.md",
                    markdown_source,
                    markdown_section_symbols(
                        "docs/topic.md",
                        markdown_source,
                        &[("section:topic", "## Topic")],
                    ),
                ),
                (
                    "src/lib.rs",
                    rust_source,
                    vec![code_symbol(
                        "src/lib.rs",
                        rust_source,
                        "sym:searchable",
                        "searchable_symbol",
                    )],
                ),
            ],
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let events = Arc::clone(&events);
            move |event| events.lock().expect("events").push(event)
        };

        let row_counts = write_sections_dataset_with_sidecar_options_and_progress(
            &artifact,
            root.path(),
            sidecar_dir.path(),
            SectionSidecarOptions {
                embedding: SectionEmbeddingOptions {
                    skip_section_embeddings: true,
                    skip_code_symbol_embeddings: true,
                    batch_size: SECTION_EMBED_BATCH_SIZE_DEFAULT,
                },
                write_batch_size: SECTION_WRITE_BATCH_SIZE_DEFAULT,
                previous_artifact_dir: None,
                delta: None,
            },
            Some(&progress),
        )
        .expect("write sidecar");

        assert_eq!(row_counts.section_bodies, 1);
        assert_eq!(row_counts.code_symbols, 1);
        let section_rows = read_stored_section_rows(sidecar_dir.path()).await;
        assert_eq!(section_rows.len(), 1);
        assert!(
            !section_rows[0].has_vector,
            "section skip should write a searchable row with a null vector"
        );
        let symbol_rows = read_stored_symbol_rows(sidecar_dir.path()).await;
        assert_eq!(symbol_rows.len(), 1);
        assert!(
            !symbol_rows[0].has_vector,
            "code-symbol skip should write a searchable row with a null vector"
        );
        let events = events.lock().expect("events").clone();
        let indexing_events = events
            .iter()
            .filter_map(|event| match event {
                SectionSidecarProgressEvent::Indexing { label, phase } => Some((*phase, *label)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            indexing_events,
            Vec::<(SidecarPhase, &str)>::new(),
            "parquet sidecars do not build Lance FTS indexes; DuckDB owns BM25"
        );
    }

    fn graph_artifact_for_path(
        stable_file_id: &str,
        path: &str,
        symbols: Vec<GraphSymbolArtifact>,
    ) -> GraphIndexArtifact {
        GraphIndexArtifact {
            macro_invocations: None,
            header: crate::GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: stable_file_id.to_owned(),
                path: path.to_owned(),
                content_oid: "blob-oid".to_owned(),
                node_ids: Vec::new(),
            }],
            files: vec![crate::GraphFileArtifact {
                stable_file_id: stable_file_id.to_owned(),
                file_path: path.to_owned(),
            }],
            file_node_ids: Vec::new(),
            symbols,
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("env lock")
    }

    #[test]
    fn sections_schema_includes_nullable_vector_column() {
        let schema = sections_schema();
        let field = schema.field_with_name("vector").expect("vector field");

        assert!(field.is_nullable());
        match field.data_type() {
            DataType::FixedSizeList(item, dimensions)
                if *dimensions == EMBEDDING_VECTOR_DIMENSIONS as i32 =>
            {
                assert_eq!(item.name(), "item");
                assert_eq!(item.data_type(), &DataType::Float32);
                assert!(item.is_nullable());
            }
            data_type => panic!(
                "expected FixedSizeList<Float32, {}>, got {data_type:?}",
                EMBEDDING_VECTOR_DIMENSIONS
            ),
        }
    }

    #[test]
    fn embedding_migration_uses_nomic_v15_contract() {
        assert_eq!(NOMIC_EMBED_TEXT_V15_MODEL_NAME, "NomicEmbedTextV15");
        assert_eq!(EMBEDDING_VECTOR_DIMENSIONS, 768);
        assert_eq!(
            NOMIC_EMBED_TEXT_V15_SECTION_TEXT_VERSION,
            "v5-nomic-embed-text-v1.5-search-document"
        );
        assert_eq!(
            NOMIC_EMBED_TEXT_V15_SYMBOL_TEXT_VERSION,
            "v4-nomic-embed-text-v1.5-search-document"
        );
    }

    #[test]
    fn coderank_loader_is_revision_pinned_to_fp32_onnx() {
        assert_eq!(
            CODE_RANK_EMBED_ONNX_REPO,
            "jamie8johnson/CodeRankEmbed-onnx"
        );
        assert_eq!(
            CODE_RANK_EMBED_ONNX_REVISION,
            "151669b173750250e611e1be00c812f112ea6020"
        );
        assert_eq!(CODE_RANK_EMBED_ONNX_FILE, "onnx/model.onnx");
        assert_eq!(EmbeddingModelSelection::CodeRankEmbed.max_length(), 8192);
    }

    #[test]
    fn fastembed_cache_dir_prefers_user_spur_over_xdg() {
        let actual = {
            let _guard = env_lock();
            let _xdg = EnvGuard::set("XDG_CACHE_HOME", "/tmp/spur-xdg-cache");
            let _home = EnvGuard::set("HOME", "/tmp/spur-home");
            fastembed_cache_dir()
        };

        assert_eq!(
            actual,
            Some(std::path::PathBuf::from(
                "/tmp/spur-home/.spur/cache/fastembed"
            ))
        );
    }

    #[test]
    fn fastembed_cache_dir_uses_user_spur_when_xdg_unset() {
        let actual = {
            let _guard = env_lock();
            let _xdg = EnvGuard::remove("XDG_CACHE_HOME");
            let _home = EnvGuard::set("HOME", "/tmp/spur-home");
            fastembed_cache_dir()
        };

        assert_eq!(
            actual,
            Some(std::path::PathBuf::from(
                "/tmp/spur-home/.spur/cache/fastembed"
            ))
        );
    }

    #[test]
    fn fastembed_cache_dir_uses_xdg_when_home_is_unset() {
        let actual = {
            let _guard = env_lock();
            let _xdg = EnvGuard::set("XDG_CACHE_HOME", "/tmp/spur-xdg-cache");
            let _home = EnvGuard::remove("HOME");
            fastembed_cache_dir()
        };

        assert_eq!(
            actual,
            Some(std::path::PathBuf::from(
                "/tmp/spur-xdg-cache/spur/fastembed"
            ))
        );
    }

    #[test]
    fn fastembed_cache_dir_returns_none_without_cache_or_home() {
        let actual = {
            let _guard = env_lock();
            let _xdg = EnvGuard::remove("XDG_CACHE_HOME");
            let _home = EnvGuard::remove("HOME");
            fastembed_cache_dir()
        };

        assert_eq!(actual, None);
    }

    #[test]
    fn embedding_model_defaults_to_nomic_v15_and_accepts_aliases() {
        let _guard = env_lock();
        let _env = EnvGuard::remove(EMBED_MODEL_ENV);

        assert_eq!(
            EmbeddingModelSelection::from_env(),
            EmbeddingModelSelection::NomicEmbedTextV15
        );

        drop(_env);
        let _env = EnvGuard::set(EMBED_MODEL_ENV, "nomic-ai/nomic-embed-text-v1.5");

        assert_eq!(
            EmbeddingModelSelection::from_env(),
            EmbeddingModelSelection::NomicEmbedTextV15
        );
        assert_eq!(
            EmbeddingModelSelection::from_env().model_name(),
            NOMIC_EMBED_TEXT_V15_MODEL_NAME
        );
        assert_eq!(EmbeddingModelSelection::NomicEmbedTextV15.dimensions(), 768);
        assert_eq!(
            EmbeddingModelSelection::NomicEmbedTextV15.fastembed_model(),
            Some(EmbeddingModel::NomicEmbedTextV15)
        );
    }

    struct CwdGuard {
        previous: PathBuf,
    }

    impl CwdGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self { previous }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    #[test]
    fn embedding_model_from_env_uses_project_config_toml_when_env_unset() {
        let _guard = env_lock();
        let _env = EnvGuard::remove(EMBED_MODEL_ENV);
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".spur")).unwrap();
        std::fs::write(
            repo.path().join(".spur/config.toml"),
            "[graph]\nembedding_model = \"coderank\"\n",
        )
        .unwrap();
        let _home = EnvGuard::set("HOME", home.path().to_str().expect("utf8 home"));
        let _cwd = CwdGuard::set(repo.path());

        assert_eq!(
            EmbeddingModelSelection::from_env(),
            EmbeddingModelSelection::CodeRankEmbed
        );
    }

    #[test]
    fn embedding_model_from_env_prefers_env_over_config_toml() {
        let _guard = env_lock();
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".spur")).unwrap();
        std::fs::write(
            repo.path().join(".spur/config.toml"),
            "[graph]\nembedding_model = \"coderank\"\n",
        )
        .unwrap();
        let _home = EnvGuard::set("HOME", home.path().to_str().expect("utf8 home"));
        let _env = EnvGuard::set(EMBED_MODEL_ENV, "jina-code");
        let _cwd = CwdGuard::set(repo.path());

        assert_eq!(
            EmbeddingModelSelection::from_env(),
            EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
        );
    }

    #[test]
    fn optional_embedding_models_accept_aliases_and_use_distinct_contracts() {
        for alias in ["coderank", "nomic-coderank", "nomic-ai/CodeRankEmbed"] {
            assert_eq!(
                EmbeddingModelSelection::parse(alias),
                Some(EmbeddingModelSelection::CodeRankEmbed)
            );
        }
        for alias in [
            "jina-code",
            "jina-embeddings-v2-base-code",
            "jinaai/jina-embeddings-v2-base-code",
        ] {
            assert_eq!(
                EmbeddingModelSelection::parse(alias),
                Some(EmbeddingModelSelection::JinaEmbeddingsV2BaseCode)
            );
        }

        assert_eq!(
            EmbeddingModelSelection::NomicEmbedTextV15.fastembed_model(),
            Some(EmbeddingModel::NomicEmbedTextV15)
        );
        assert_eq!(
            EmbeddingModelSelection::CodeRankEmbed.fastembed_model(),
            None,
            "CodeRankEmbed uses the revision-pinned user-defined ONNX path"
        );
        assert_eq!(
            EmbeddingModelSelection::JinaEmbeddingsV2BaseCode.fastembed_model(),
            Some(EmbeddingModel::JinaEmbeddingsV2BaseCode)
        );
    }

    #[test]
    #[ignore = "downloads and loads the NomicEmbedTextV15 FastEmbed model"]
    fn fastembed_smoke_constructs_model_and_returns_expected_dimensions() {
        assert_embedding_model_smoke(EmbeddingModelSelection::NomicEmbedTextV15);
    }

    #[test]
    #[ignore = "downloads and loads the CodeRankEmbed FP32 ONNX model"]
    fn coderank_embed_smoke_constructs_model_and_returns_expected_dimensions() {
        assert_embedding_model_smoke(EmbeddingModelSelection::CodeRankEmbed);
    }

    #[test]
    #[ignore = "downloads and loads the JinaEmbeddingsV2BaseCode FP32 ONNX model"]
    fn jina_code_smoke_constructs_model_and_returns_expected_dimensions() {
        assert_embedding_model_smoke(EmbeddingModelSelection::JinaEmbeddingsV2BaseCode);
    }

    fn assert_embedding_model_smoke(embedding_model: EmbeddingModelSelection) {
        let mut service = TextEmbeddingService::new(
            SectionEmbeddingOptions {
                batch_size: 1,
                ..SectionEmbeddingOptions::default()
            },
            false,
            embedding_model,
        );
        let text = embedding_query_text_for_model("default logger smoke", embedding_model);

        let embeddings = service
            .embed_texts_locally(&[text.as_ref()], "fastembed smoke")
            .expect("embedding smoke should run");

        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].len(), EMBEDDING_VECTOR_DIMENSIONS);
    }

    #[test]
    fn text_embedding_service_requires_local_nomic_model_initialization() {
        let service = TextEmbeddingService::new(
            SectionEmbeddingOptions::default(),
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );

        assert!(service.needs_model_init());
    }

    #[test]
    fn embedding_model_from_env_selects_jina_code_alias() {
        let _guard = env_lock();
        let _model = EnvGuard::set(EMBED_MODEL_ENV, "jina-code");

        assert_eq!(
            EmbeddingModelSelection::from_env(),
            EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
        );
        assert_eq!(
            EmbeddingModelSelection::from_env().model_name(),
            JINA_EMBEDDINGS_V2_BASE_CODE_MODEL_NAME
        );
        assert_eq!(
            EmbeddingModelSelection::JinaEmbeddingsV2BaseCode.dimensions(),
            768
        );
    }

    #[test]
    fn nomic_embedding_text_version_is_part_of_vector_content_hash() {
        let source_hash = "source-content";

        assert_ne!(
            section_embed_content_hash_for_model(
                source_hash,
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            source_hash
        );
        assert_ne!(
            symbol_embed_content_hash_for_model(
                source_hash,
                false,
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            source_hash
        );
        assert_ne!(
            section_embedding_input_hash_for_model(
                "same title",
                "same body",
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            blake3_hex("same body".as_bytes())
        );
        assert_ne!(
            symbol_embedding_input_hash_for_model(
                "same embed text",
                false,
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            blake3_hex("same embed text".as_bytes())
        );
        let expected_symbol_input_hash = blake3_hex(
            format!(
                "v4-nomic-embed-text-v1.5-search-document:symbol\0{}",
                blake3_hex("same embed text".as_bytes())
            )
            .as_bytes(),
        );
        assert_eq!(
            symbol_embedding_input_hash_for_model(
                "same embed text",
                false,
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            expected_symbol_input_hash,
            "symbol embedding hashes must include the Nomic prompt contract version"
        );
    }

    #[test]
    fn nomic_formats_query_and_document_inputs() {
        let section_rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let symbol_rows = vec![symbol_row_fixture("symbol-one", "one embed text")];

        assert_eq!(
            embedding_query_text_for_model(
                "find task spawner",
                EmbeddingModelSelection::NomicEmbedTextV15
            )
            .as_ref(),
            "search_query: find task spawner"
        );
        assert_eq!(
            section_embedding_inputs(&section_rows, EmbeddingModelSelection::NomicEmbedTextV15)[0]
                .text
                .as_ref(),
            "search_document: title: docs/example.md::Section | text: ## Install\n\nInstall body."
        );
        assert_eq!(
            symbol_embedding_inputs(&symbol_rows, EmbeddingModelSelection::NomicEmbedTextV15)[0]
                .text
                .as_ref(),
            "search_document: title: none | text: one embed text"
        );
    }

    #[test]
    fn code_models_apply_their_model_specific_query_and_document_contracts() {
        let section_rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let symbol_rows = vec![symbol_row_fixture("symbol-one", "one embed text")];

        assert_eq!(
            embedding_query_text_for_model(
                "find task spawner",
                EmbeddingModelSelection::CodeRankEmbed
            ),
            "Represent this query for searching relevant code: find task spawner"
        );
        assert_eq!(
            section_embedding_inputs(&section_rows, EmbeddingModelSelection::CodeRankEmbed)[0].text,
            "## Install\n\nInstall body."
        );
        assert_eq!(
            symbol_embedding_inputs(&symbol_rows, EmbeddingModelSelection::CodeRankEmbed)[0].text,
            "one embed text"
        );

        assert_eq!(
            embedding_query_text_for_model(
                "find task spawner",
                EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
            ),
            "find task spawner"
        );
        assert_eq!(
            section_embedding_inputs(
                &section_rows,
                EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
            )[0]
            .text,
            "## Install\n\nInstall body."
        );
        assert_eq!(
            symbol_embedding_inputs(
                &symbol_rows,
                EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
            )[0]
            .text,
            "one embed text"
        );

        let source_hash = "same-source";
        assert_ne!(
            section_embed_content_hash_for_model(
                source_hash,
                EmbeddingModelSelection::NomicEmbedTextV15
            ),
            section_embed_content_hash_for_model(
                source_hash,
                EmbeddingModelSelection::CodeRankEmbed
            )
        );
        assert_ne!(
            section_embed_content_hash_for_model(
                source_hash,
                EmbeddingModelSelection::CodeRankEmbed
            ),
            section_embed_content_hash_for_model(
                source_hash,
                EmbeddingModelSelection::JinaEmbeddingsV2BaseCode
            )
        );
    }

    #[test]
    fn section_embedding_input_hash_includes_qualified_name() {
        let source = "## Shared\n\nSame body.";
        let section = |stable_symbol_id: &str, qualified_name: &str| GraphSymbolArtifact {
            stable_symbol_id: stable_symbol_id.to_owned(),
            file_path: "docs/example.md".to_owned(),
            byte_range: [0, source.len()],
            line_range: [1, 3],
            entity_name: qualified_name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            symbol_kind: "section".to_owned(),
            anchor_hash: format!("anchor:{stable_symbol_id}"),
            enclosing_scope: None,
        };

        let first = section_row(
            &section("section-one", "Guide::Install"),
            source,
            "content-hash",
            EmbeddingModelSelection::NomicEmbedTextV15,
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("first section row")
        .expect("first section row present");
        let second = section_row(
            &section("section-two", "Guide::Usage"),
            source,
            "content-hash",
            EmbeddingModelSelection::NomicEmbedTextV15,
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("second section row")
        .expect("second section row present");

        assert_ne!(
            first.embedding_input_hash, second.embedding_input_hash,
            "same section body under different headings must not reuse vectors"
        );
    }

    #[test]
    fn embed_eligible_rows_returns_none_for_h1_and_oversized() {
        let rows = vec![
            section_row_fixture(1, "# Title\n\nBody".to_owned()),
            section_row_fixture(2, format!("## Heading\n\n{}", "x".repeat(4097))),
        ];

        assert_eq!(
            embed_eligible_rows(&rows, SectionEmbeddingOptions::default()),
            vec![None, None]
        );
    }

    #[test]
    fn section_embedding_options_from_env_uses_defaults_for_missing_invalid_and_zero() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let batch = EnvGuard::remove(SECTION_EMBED_BATCH_SIZE_ENV);

        assert_eq!(
            SectionEmbeddingOptions::from_env(),
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: false,
                batch_size: SECTION_EMBED_BATCH_SIZE_DEFAULT,
            }
        );

        std::env::set_var(SECTION_EMBED_BATCH_SIZE_ENV, "not-a-number");
        assert_eq!(
            SectionEmbeddingOptions::from_env().batch_size,
            SECTION_EMBED_BATCH_SIZE_DEFAULT
        );

        std::env::set_var(SECTION_EMBED_BATCH_SIZE_ENV, "0");
        assert_eq!(
            SectionEmbeddingOptions::from_env().batch_size,
            SECTION_EMBED_BATCH_SIZE_DEFAULT
        );

        drop(batch);
    }

    #[test]
    fn section_embedding_options_from_env_accepts_skip_and_valid_batch_size() {
        let _lock = env_lock();
        let _skip = EnvGuard::set(SECTION_EMBED_SKIP_ENV, "1");
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "7");

        assert_eq!(
            SectionEmbeddingOptions::from_env(),
            SectionEmbeddingOptions {
                skip_section_embeddings: true,
                skip_code_symbol_embeddings: false,
                batch_size: 7,
            }
        );
    }

    #[test]
    fn section_embedding_options_from_env_accepts_code_symbol_skip() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::set(CODE_SYMBOL_EMBED_SKIP_ENV, "1");
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "7");

        assert_eq!(
            SectionEmbeddingOptions::from_env(),
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: true,
                batch_size: 7,
            }
        );
    }

    #[test]
    fn section_embedding_options_from_env_with_skip_override_matches_env_skip() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "7");

        assert_eq!(
            SectionEmbeddingOptions::from_env_with_skip_override(true),
            SectionEmbeddingOptions {
                skip_section_embeddings: true,
                skip_code_symbol_embeddings: false,
                batch_size: 7,
            }
        );
    }

    #[test]
    fn section_embedding_options_from_env_with_skip_overrides_splits_flags() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "7");

        assert_eq!(
            SectionEmbeddingOptions::from_env_with_skip_overrides(false, true),
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: true,
                batch_size: 7,
            }
        );
    }

    #[test]
    fn section_sidecar_options_from_env_uses_default_write_batch_for_missing_invalid_and_zero() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let _embed_batch = EnvGuard::remove(SECTION_EMBED_BATCH_SIZE_ENV);
        let write_batch = EnvGuard::remove(SECTION_WRITE_BATCH_SIZE_ENV);

        assert_eq!(
            SectionSidecarOptions::from_env().write_batch_size,
            SECTION_WRITE_BATCH_SIZE_DEFAULT
        );

        std::env::set_var(SECTION_WRITE_BATCH_SIZE_ENV, "not-a-number");
        assert_eq!(
            SectionSidecarOptions::from_env().write_batch_size,
            SECTION_WRITE_BATCH_SIZE_DEFAULT
        );

        std::env::set_var(SECTION_WRITE_BATCH_SIZE_ENV, "0");
        assert_eq!(
            SectionSidecarOptions::from_env().write_batch_size,
            SECTION_WRITE_BATCH_SIZE_DEFAULT
        );

        drop(write_batch);
    }

    #[test]
    fn section_sidecar_options_from_embedding_options_reads_write_batch_env() {
        let _lock = env_lock();
        let _write_batch = EnvGuard::set(SECTION_WRITE_BATCH_SIZE_ENV, "2");
        let embedding = SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 13,
        };

        let options = SectionSidecarOptions::from_embedding_options(embedding);

        assert_eq!(options.embedding, embedding);
        assert_eq!(options.write_batch_size, 2);
    }

    #[test]
    fn section_row_batcher_yields_configured_batch_lengths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let path = "docs/sections.md";
        let source = "## One\n\nBody one.\n## Two\n\nBody two.\n## Three\n\nBody three.\n## Four\n\nBody four.\n## Five\n\nBody five.\n";
        std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
        std::fs::write(root.join(path), source).expect("write markdown");

        let ranges = section_ranges(
            source,
            &["## One", "## Two", "## Three", "## Four", "## Five"],
        );
        let symbols = ranges
            .iter()
            .enumerate()
            .rev()
            .map(|(index, [start, end])| GraphSymbolArtifact {
                stable_symbol_id: format!("section-{index}"),
                file_path: path.to_owned(),
                byte_range: [*start, *end],
                line_range: [1, 1],
                entity_name: format!("Section {index}"),
                qualified_name: format!("docs/sections.md::Section{index}"),
                symbol_kind: "section".to_owned(),
                anchor_hash: format!("anchor-{index}"),
                enclosing_scope: None,
            })
            .collect();
        let artifact = GraphIndexArtifact {
            macro_invocations: None,
            header: crate::GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: "file-sections".to_owned(),
                path: path.to_owned(),
                content_oid: "sections".to_owned(),
                node_ids: Vec::new(),
            }],
            files: vec![crate::GraphFileArtifact {
                stable_file_id: "file-sections".to_owned(),
                file_path: path.to_owned(),
            }],
            file_node_ids: Vec::new(),
            symbols,
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };
        let mut batcher = SectionRowBatcher::new(
            &artifact,
            root,
            2,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let mut lengths = Vec::new();

        while let Some(batch) = batcher.next_batch().expect("section batch") {
            assert!(batch.len() <= 2);
            lengths.push(batch.len());
        }

        assert_eq!(lengths, vec![2, 2, 1]);
    }

    #[test]
    fn section_row_batcher_skips_non_utf8_boundary_ranges() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let path = "docs/sections.md";
        let source = "## Bad\n\ncaf\u{e9}\n\n## Good\n\nBody.\n";
        std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
        std::fs::write(root.join(path), source).expect("write markdown");

        let mid_char_end = source.find('\u{e9}').expect("accent") + 1;
        assert!(source.get(0..mid_char_end).is_none());
        let good_start = source.find("## Good").expect("good start");
        let artifact = graph_artifact_for_path(
            "file-sections",
            path,
            vec![
                GraphSymbolArtifact {
                    stable_symbol_id: "bad-section".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [0, mid_char_end],
                    line_range: [1, 3],
                    entity_name: "Bad".to_owned(),
                    qualified_name: "docs/sections.md::Bad".to_owned(),
                    symbol_kind: "section".to_owned(),
                    anchor_hash: "bad-anchor".to_owned(),
                    enclosing_scope: None,
                },
                GraphSymbolArtifact {
                    stable_symbol_id: "good-section".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [good_start, source.len()],
                    line_range: [5, 7],
                    entity_name: "Good".to_owned(),
                    qualified_name: "docs/sections.md::Good".to_owned(),
                    symbol_kind: "section".to_owned(),
                    anchor_hash: "good-anchor".to_owned(),
                    enclosing_scope: None,
                },
            ],
        );

        let mut batcher = SectionRowBatcher::new(
            &artifact,
            root,
            16,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let rows = batcher
            .next_batch()
            .expect("section batch")
            .expect("section rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stable_symbol_id, "good-section");
    }

    #[test]
    fn symbol_row_batcher_prepends_first_source_line_for_long_bodies_only() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let path = "src/lib.rs";
        let source = concat!(
            "fn tiny() {\n",
            "    ready();\n",
            "}\n",
            "\n",
            "fn handle_error(delegation: Delegation) {\n",
            "    let status = delegation.status();\n",
            "    if status.is_retryable() {\n",
            "        delegation.retry();\n",
            "    }\n",
            "    delegation.finish();\n",
            "}\n",
        );
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::write(root.join(path), source).expect("write source");

        let tiny_start = source.find("fn tiny").expect("tiny start");
        let tiny_end = source.find("\n\nfn handle_error").expect("tiny end");
        let long_start = source.find("fn handle_error").expect("long start");
        let artifact = GraphIndexArtifact {
            macro_invocations: None,
            header: crate::GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: "file-lib".to_owned(),
                path: path.to_owned(),
                content_oid: "blob-oid".to_owned(),
                node_ids: Vec::new(),
            }],
            files: vec![crate::GraphFileArtifact {
                stable_file_id: "file-lib".to_owned(),
                file_path: path.to_owned(),
            }],
            file_node_ids: Vec::new(),
            symbols: vec![
                GraphSymbolArtifact {
                    stable_symbol_id: "tiny".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [tiny_start, tiny_end],
                    line_range: [1, 3],
                    entity_name: "tiny".to_owned(),
                    qualified_name: "crate::tiny".to_owned(),
                    symbol_kind: "function".to_owned(),
                    anchor_hash: "tiny-anchor".to_owned(),
                    enclosing_scope: None,
                },
                GraphSymbolArtifact {
                    stable_symbol_id: "handle-error".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [long_start, source.len()],
                    line_range: [5, 11],
                    entity_name: "handle_error".to_owned(),
                    qualified_name: "crate::handle_error".to_owned(),
                    symbol_kind: "function".to_owned(),
                    anchor_hash: "handle-error-anchor".to_owned(),
                    enclosing_scope: None,
                },
            ],
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let mut batcher = SymbolRowBatcher::new(
            &artifact,
            root,
            16,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let rows = batcher
            .next_batch()
            .expect("symbol batch")
            .expect("symbol rows");
        let embed_text_by_id: HashMap<_, _> = rows
            .iter()
            .map(|row| (row.stable_symbol_id.as_str(), row.embed_text.as_str()))
            .collect();

        assert_eq!(
            embed_text_by_id.get("tiny").copied(),
            Some("tiny crate::tiny function")
        );
        assert_eq!(
            embed_text_by_id.get("handle-error").copied(),
            Some(
                "fn handle_error(delegation: Delegation) { handle_error crate::handle_error function"
            )
        );
    }

    #[test]
    fn symbol_row_batcher_skips_non_utf8_boundary_ranges() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let path = "src/lib.rs";
        let source = concat!(
            "fn bad_symbol() {\n",
            "    let word = \"caf\u{e9}\";\n",
            "    consume(word);\n",
            "    consume(word);\n",
            "    consume(word);\n",
            "    consume(word);\n",
            "}\n",
            "\n",
            "fn good_symbol() {\n",
            "    ready();\n",
            "}\n",
        );
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::write(root.join(path), source).expect("write source");

        let mid_char_end = source.find('\u{e9}').expect("accent") + 1;
        assert!(source.get(0..mid_char_end).is_none());
        let good_start = source.find("fn good_symbol").expect("good start");
        let artifact = graph_artifact_for_path(
            "file-lib",
            path,
            vec![
                GraphSymbolArtifact {
                    stable_symbol_id: "bad-symbol".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [0, mid_char_end],
                    line_range: [1, 7],
                    entity_name: "bad_symbol".to_owned(),
                    qualified_name: "crate::bad_symbol".to_owned(),
                    symbol_kind: "function".to_owned(),
                    anchor_hash: "bad-anchor".to_owned(),
                    enclosing_scope: None,
                },
                GraphSymbolArtifact {
                    stable_symbol_id: "good-symbol".to_owned(),
                    file_path: path.to_owned(),
                    byte_range: [good_start, source.len()],
                    line_range: [9, 11],
                    entity_name: "good_symbol".to_owned(),
                    qualified_name: "crate::good_symbol".to_owned(),
                    symbol_kind: "function".to_owned(),
                    anchor_hash: "good-anchor".to_owned(),
                    enclosing_scope: None,
                },
            ],
        );

        let mut batcher = SymbolRowBatcher::new(
            &artifact,
            root,
            16,
            None,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let rows = batcher
            .next_batch()
            .expect("symbol batch")
            .expect("symbol rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stable_symbol_id, "good-symbol");
    }

    fn section_ranges(source: &str, headings: &[&str]) -> Vec<[usize; 2]> {
        headings
            .iter()
            .enumerate()
            .map(|(index, heading)| {
                let start = source.find(heading).expect("heading");
                let end = headings
                    .get(index + 1)
                    .and_then(|next| source.find(next))
                    .unwrap_or(source.len());
                [start, end]
            })
            .collect()
    }

    #[test]
    fn embed_eligible_rows_returns_none_without_calling_embedder_when_skipped() {
        let rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 1,
        };

        assert_eq!(
            embed_eligible_rows_with(
                &rows,
                options,
                |_| panic!("progress should not be called"),
                |_| panic!("embedder should not be called"),
            ),
            vec![None]
        );
    }

    #[test]
    fn section_skip_leaves_code_symbol_embedding_eligible() {
        let section_rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let symbol_rows = vec![symbol_row_fixture("sym-a", "symbol embed text")];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 1,
        };

        assert_eq!(
            embed_eligible_rows_with(
                &section_rows,
                options,
                |_| panic!("section progress should not be called"),
                |_| panic!("section embedder should not be called"),
            ),
            vec![None]
        );

        let symbol_vectors = embed_symbol_rows_with(
            &symbol_rows,
            options,
            |_| {},
            |texts: &[&str]| Ok(vec![vec![0.5; EMBEDDING_VECTOR_DIMENSIONS]; texts.len()]),
        );
        assert!(
            symbol_vectors[0].is_some(),
            "section skip must not suppress code-symbol vectors"
        );
    }

    #[test]
    fn code_symbol_skip_leaves_section_embedding_eligible() {
        let section_rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let symbol_rows = vec![symbol_row_fixture("sym-a", "symbol embed text")];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: true,
            batch_size: 1,
        };

        let section_vectors = embed_eligible_rows_with(
            &section_rows,
            options,
            |_| {},
            |texts: &[&str]| Ok(vec![vec![0.5; EMBEDDING_VECTOR_DIMENSIONS]; texts.len()]),
        );
        assert!(
            section_vectors[0].is_some(),
            "code-symbol skip must not suppress section vectors"
        );

        assert_eq!(
            embed_symbol_rows_with(
                &symbol_rows,
                options,
                |_| panic!("code-symbol progress should not be called"),
                |_| panic!("code-symbol embedder should not be called"),
            ),
            vec![None]
        );
    }

    #[test]
    fn env_skipped_embedding_options_do_not_request_fastembed_model_for_either_phase() {
        let _lock = env_lock();
        let _skip = EnvGuard::set(SECTION_EMBED_SKIP_ENV, "1");
        let _code_symbol_skip = EnvGuard::set(CODE_SYMBOL_EMBED_SKIP_ENV, "1");
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "1");

        assert_skipped_options_do_not_request_fastembed_model(SectionEmbeddingOptions::from_env());
    }

    #[test]
    fn override_skipped_embedding_options_do_not_request_fastembed_model_for_either_phase() {
        let _lock = env_lock();
        let _skip = EnvGuard::remove(SECTION_EMBED_SKIP_ENV);
        let _code_symbol_skip = EnvGuard::remove(CODE_SYMBOL_EMBED_SKIP_ENV);
        let _batch = EnvGuard::set(SECTION_EMBED_BATCH_SIZE_ENV, "1");

        assert_skipped_options_do_not_request_fastembed_model(
            SectionEmbeddingOptions::from_env_with_skip_overrides(true, true),
        );
    }

    fn assert_skipped_options_do_not_request_fastembed_model(options: SectionEmbeddingOptions) {
        let mut section_embedder =
            SectionEmbedder::new(options, EmbeddingModelSelection::NomicEmbedTextV15);
        let mut symbol_embedder =
            SymbolEmbedder::new(options, EmbeddingModelSelection::NomicEmbedTextV15);

        assert!(!section_embedder.prepare_model_for_eligible_rows(1));
        assert!(!symbol_embedder.prepare_model_for_eligible_rows(1));
        assert!(!section_embedder.service.model_requested);
        assert!(!symbol_embedder.service.model_requested);
        assert!(
            embed_model_cell(EmbeddingModelSelection::NomicEmbedTextV15)
                .get()
                .is_none(),
            "skipped embeddings must not initialize the shared FastEmbed model"
        );
    }

    #[test]
    fn section_embedder_does_not_initialize_model_for_skipped_or_ineligible_rows() {
        let rows = vec![section_row_fixture(1, "# Title\n\nSkipped.".to_owned())];
        let mut embedder = SectionEmbedder::new(
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: false,
                batch_size: 1,
            },
            EmbeddingModelSelection::NomicEmbedTextV15,
        );

        assert_eq!(embedder.embed_row_vectors(&rows), vec![None]);
        assert!(!embedder.service.model_requested);

        let rows = vec![section_row_fixture(
            2,
            "## Install\n\nInstall body.".to_owned(),
        )];
        let mut embedder = SectionEmbedder::new(
            SectionEmbeddingOptions {
                skip_section_embeddings: true,
                skip_code_symbol_embeddings: false,
                batch_size: 1,
            },
            EmbeddingModelSelection::NomicEmbedTextV15,
        );

        assert_eq!(embedder.embed_row_vectors(&rows), vec![None]);
        assert!(!embedder.service.model_requested);
    }

    #[test]
    fn embed_eligible_rows_uses_configured_batch_size() {
        let rows = vec![
            section_row_fixture(2, "## One\n\nBody one.".to_owned()),
            section_row_fixture(1, "# Title\n\nSkipped.".to_owned()),
            section_row_fixture(2, "## Two\n\nBody two.".to_owned()),
            section_row_fixture(2, "## Three\n\nBody three.".to_owned()),
        ];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: false,
            batch_size: 2,
        };
        let mut batch_sizes = Vec::new();

        let vectors = embed_eligible_rows_with(
            &rows,
            options,
            |_| {},
            |texts| {
                batch_sizes.push(texts.len());
                Ok(vec![vec![0.25; EMBEDDING_VECTOR_DIMENSIONS]; texts.len()])
            },
        );

        assert_eq!(batch_sizes, vec![2, 1]);
        assert!(vectors[0].is_some());
        assert!(vectors[1].is_none());
        assert!(vectors[2].is_some());
        assert!(vectors[3].is_some());
    }

    #[test]
    fn embed_symbol_rows_uses_embed_text_and_configured_batch_size() {
        let rows = vec![
            symbol_row_fixture("symbol-one", "one embed text"),
            symbol_row_fixture("symbol-two", "two embed text"),
            symbol_row_fixture("symbol-three", "three embed text"),
        ];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: false,
            batch_size: 2,
        };
        let mut batch_texts = Vec::new();

        let vectors = embed_symbol_rows_with(
            &rows,
            options,
            |_| {},
            |texts: &[&str]| {
                batch_texts.push(
                    texts
                        .iter()
                        .map(|text| (*text).to_owned())
                        .collect::<Vec<_>>(),
                );
                Ok(vec![vec![0.5; EMBEDDING_VECTOR_DIMENSIONS]; texts.len()])
            },
        );

        assert_eq!(
            batch_texts,
            vec![
                vec![
                    "search_document: title: none | text: one embed text".to_owned(),
                    "search_document: title: none | text: two embed text".to_owned(),
                ],
                vec!["search_document: title: none | text: three embed text".to_owned()],
            ]
        );
        assert!(vectors.iter().all(Option::is_some));
    }

    // ---- carry-forward tests (TDD: written before implementation) ----

    /// Write section rows with known fake vectors into dir A (v1), then write
    /// into fresh dir B with `previous_artifact_dir = Some(A)` and embeddings
    /// disabled.  Unchanged embedding inputs must carry their vectors; a
    /// changed embedding input must not.
    #[tokio::test]
    async fn carry_forward_fills_section_vectors_from_previous_dir() {
        let dir_a = tempfile::tempdir().expect("dir_a");

        let fake_vec: Vec<f32> = (0..EMBEDDING_VECTOR_DIMENSIONS)
            .map(|i| i as f32 * 0.001)
            .collect();

        // ---- write v1 into dir_a with known vectors ----
        let old_changed_body = "## Changed\n\nOld body.";
        let new_changed_body = "## Changed\n\nNew body.";
        let row_unchanged = SectionRow {
            vector: Some(fake_vec.clone()),
            heading_level: 2,
            ..versioned_section_row("unchanged", "docs/a.md", "hash-unchanged")
        };
        let row_changed = SectionRow {
            vector: Some(fake_vec.clone()),
            heading_level: 2,
            body_text: old_changed_body.to_owned(),
            embedding_input_hash: section_embedding_input_hash_for_model(
                "changed",
                old_changed_body,
                EmbeddingModelSelection::NomicEmbedTextV15,
            ),
            ..versioned_section_row("changed", "docs/b.md", "hash-old")
        };
        write_previous_section_sidecar_rows(dir_a.path(), vec![row_unchanged, row_changed]).await;

        // ---- carry forward to dir_b ----
        let rows_v2 = vec![
            // unchanged: same (file_path, embedding_input_hash, stable_symbol_id)
            versioned_section_row("unchanged", "docs/a.md", "hash-unchanged"),
            // changed: same stable id, but different embedding input
            SectionRow {
                body_text: new_changed_body.to_owned(),
                embedding_input_hash: section_embedding_input_hash_for_model(
                    "changed",
                    new_changed_body,
                    EmbeddingModelSelection::NomicEmbedTextV15,
                ),
                ..versioned_section_row("changed", "docs/b.md", "hash-new")
            },
        ];
        let carried = carry_forward_section_vectors(rows_v2, dir_a.path()).await;

        let unchanged = carried
            .iter()
            .find(|r| r.stable_symbol_id == "unchanged")
            .expect("unchanged row");
        let changed = carried
            .iter()
            .find(|r| r.stable_symbol_id == "changed")
            .expect("changed row");

        assert_eq!(
            unchanged.vector.as_ref().map(Vec::len),
            Some(EMBEDDING_VECTOR_DIMENSIONS),
            "unchanged row should carry vector from previous dir"
        );
        assert_eq!(
            unchanged.vector.as_ref(),
            Some(&fake_vec),
            "carried vector must match original"
        );
        assert!(
            changed.vector.is_none(),
            "changed embedding input must NOT carry vector"
        );
    }

    /// Verifying that the dimension guard prevents carrying a vector with wrong
    /// length.
    #[tokio::test]
    async fn carry_forward_ignores_wrong_dimension_vectors() {
        let dir_a = tempfile::tempdir().expect("dir_a");
        // wrong_vec is intentionally not used as a write target because the
        // Lance schema enforces EMBEDDING_VECTOR_DIMENSIONS; instead we test
        // that a row absent from the previous table yields None.

        // Write a row with correct dimensions first so the sidecar exists, then
        // check that a row NOT in the prev sidecar returns None.
        let correct_vec: Vec<f32> = vec![0.5; EMBEDDING_VECTOR_DIMENSIONS];
        let mut row = versioned_section_row("sym-x", "docs/x.md", "hash-x");
        row.vector = Some(correct_vec.clone());
        row.heading_level = 2;
        write_previous_section_sidecar_rows(dir_a.path(), vec![row]).await;

        // Row with same identity but missing from prev still gets None
        let rows_v2 = vec![versioned_section_row("sym-y", "docs/y.md", "hash-y")];
        let carried = carry_forward_section_vectors(rows_v2, dir_a.path()).await;

        assert!(
            carried[0].vector.is_none(),
            "row not present in prev table must not carry a vector"
        );

        // Row with same identity carries the correct vector
        let rows_v2 = vec![versioned_section_row("sym-x", "docs/x.md", "hash-x")];
        let carried = carry_forward_section_vectors(rows_v2, dir_a.path()).await;
        assert_eq!(
            carried[0].vector.as_ref().map(Vec::len),
            Some(EMBEDDING_VECTOR_DIMENSIONS),
        );
    }

    /// Carry-forward for code symbols must be keyed by embedding input, so a
    /// large changed file does not re-embed every unchanged symbol in the
    /// default 512-row write batches split into 64-row embedding chunks.
    #[tokio::test]
    async fn carry_forward_symbol_vectors_reuses_unchanged_embed_text_in_changed_file() {
        let dir_a = tempfile::tempdir().expect("dir_a");

        let fake_vec: Vec<f32> = (0..EMBEDDING_VECTOR_DIMENSIONS)
            .map(|i| i as f32 * 0.002)
            .collect();

        let unchanged_input_hash = symbol_embedding_input_hash_for_model(
            "unchanged embed text",
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let old_changed_input_hash = symbol_embedding_input_hash_for_model(
            "old changed embed text",
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let new_changed_input_hash = symbol_embedding_input_hash_for_model(
            "new changed embed text",
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );

        let row_unchanged = SymbolRow {
            stable_symbol_id: "sym-unchanged".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            qualified_name: "sym-unchanged".to_owned(),
            entity_name: "sym_unchanged".to_owned(),
            symbol_kind: "function".to_owned(),
            embed_text: "unchanged embed text".to_owned(),
            vector: Some(fake_vec.clone()),
            content_hash: "hash-old".to_owned(),
            embedding_input_hash: unchanged_input_hash.clone(),
            embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
        };
        let row_changed = SymbolRow {
            stable_symbol_id: "sym-changed".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            qualified_name: "sym-changed".to_owned(),
            entity_name: "sym_changed".to_owned(),
            symbol_kind: "function".to_owned(),
            embed_text: "old changed embed text".to_owned(),
            vector: Some(fake_vec.clone()),
            content_hash: "hash-old".to_owned(),
            embedding_input_hash: old_changed_input_hash,
            embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
        };
        write_previous_symbol_sidecar_rows(dir_a.path(), vec![row_unchanged, row_changed]).await;

        let rows_v2 = vec![
            SymbolRow {
                stable_symbol_id: "sym-unchanged".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                qualified_name: "sym-unchanged".to_owned(),
                entity_name: "sym_unchanged".to_owned(),
                symbol_kind: "function".to_owned(),
                embed_text: "unchanged embed text".to_owned(),
                vector: None,
                content_hash: "hash-new".to_owned(),
                embedding_input_hash: unchanged_input_hash,
                embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
            },
            SymbolRow {
                stable_symbol_id: "sym-changed".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                qualified_name: "sym-changed".to_owned(),
                entity_name: "sym_changed".to_owned(),
                symbol_kind: "function".to_owned(),
                embed_text: "new changed embed text".to_owned(),
                vector: None,
                content_hash: "hash-new".to_owned(),
                embedding_input_hash: new_changed_input_hash,
                embedding_model: NOMIC_EMBED_TEXT_V15_MODEL_NAME.to_owned(),
            },
        ];
        let carried = carry_forward_symbol_vectors(rows_v2, dir_a.path()).await;

        let unchanged = carried
            .iter()
            .find(|r| r.stable_symbol_id == "sym-unchanged")
            .expect("unchanged");
        let changed = carried
            .iter()
            .find(|r| r.stable_symbol_id == "sym-changed")
            .expect("changed");

        assert_eq!(
            unchanged.vector.as_ref().map(Vec::len),
            Some(EMBEDDING_VECTOR_DIMENSIONS),
            "unchanged embed_text must carry even when the file content_hash changed"
        );
        assert_eq!(unchanged.vector.as_ref(), Some(&fake_vec));
        assert!(
            changed.vector.is_none(),
            "changed embed_text must not carry a stale vector"
        );
    }

    #[tokio::test]
    async fn carry_forward_section_vectors_reuses_unchanged_body_in_changed_file() {
        let dir_a = tempfile::tempdir().expect("dir_a");

        let fake_vec: Vec<f32> = (0..EMBEDDING_VECTOR_DIMENSIONS)
            .map(|i| i as f32 * 0.003)
            .collect();

        let unchanged_input_hash = section_embedding_input_hash_for_model(
            "section-unchanged",
            "## Stable\n\nUnchanged body.",
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let old_changed_input_hash = section_embedding_input_hash_for_model(
            "section-changed",
            "## Changed\n\nOld body.",
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let new_changed_input_hash = section_embedding_input_hash_for_model(
            "section-changed",
            "## Changed\n\nNew body.",
            EmbeddingModelSelection::NomicEmbedTextV15,
        );

        let row_unchanged = SectionRow {
            vector: Some(fake_vec.clone()),
            body_text: "## Stable\n\nUnchanged body.".to_owned(),
            content_hash: "hash-old".to_owned(),
            embedding_input_hash: unchanged_input_hash.clone(),
            ..versioned_section_row("section-unchanged", "docs/a.md", "hash-old")
        };
        let row_changed = SectionRow {
            vector: Some(fake_vec.clone()),
            body_text: "## Changed\n\nOld body.".to_owned(),
            content_hash: "hash-old".to_owned(),
            embedding_input_hash: old_changed_input_hash,
            ..versioned_section_row("section-changed", "docs/a.md", "hash-old")
        };
        write_previous_section_sidecar_rows(dir_a.path(), vec![row_unchanged, row_changed]).await;

        let rows_v2 = vec![
            SectionRow {
                body_text: "## Stable\n\nUnchanged body.".to_owned(),
                content_hash: "hash-new".to_owned(),
                embedding_input_hash: unchanged_input_hash,
                ..versioned_section_row("section-unchanged", "docs/a.md", "hash-new")
            },
            SectionRow {
                body_text: "## Changed\n\nNew body.".to_owned(),
                content_hash: "hash-new".to_owned(),
                embedding_input_hash: new_changed_input_hash,
                ..versioned_section_row("section-changed", "docs/a.md", "hash-new")
            },
        ];
        let carried = carry_forward_section_vectors(rows_v2, dir_a.path()).await;

        let unchanged = carried
            .iter()
            .find(|r| r.stable_symbol_id == "section-unchanged")
            .expect("unchanged");
        let changed = carried
            .iter()
            .find(|r| r.stable_symbol_id == "section-changed")
            .expect("changed");

        assert_eq!(
            unchanged.vector.as_ref().map(Vec::len),
            Some(EMBEDDING_VECTOR_DIMENSIONS),
            "unchanged section body must carry even when file content_hash changed"
        );
        assert_eq!(unchanged.vector.as_ref(), Some(&fake_vec));
        assert!(
            changed.vector.is_none(),
            "changed section body must not carry a stale vector"
        );
    }

    #[tokio::test]
    async fn sidecar_planning_limits_512_row_batches_to_touched_paths_before_64_row_chunks() {
        let root = tempfile::tempdir().expect("root");
        let prev_dir = tempfile::tempdir().expect("prev sidecar");
        let next_dir = tempfile::tempdir().expect("next sidecar");
        let unchanged_source = many_functions_source("unchanged_symbol", 513);
        let changed_old_source = concat!(
            "/// Stable keep docs that should continue to use the previous embedding input.\n",
            "pub fn keep_symbol() {}\n\n",
            "/// Old docs for the symbol whose embedding input changed.\n",
            "pub fn reembed_symbol() {}\n",
        );
        let deleted_source =
            "/// Removed docs with a previous sidecar row.\npub fn removed_symbol() {}\n";
        write_source(root.path(), "src/unchanged.rs", &unchanged_source);
        write_source(root.path(), "src/changed.rs", changed_old_source);
        write_source(root.path(), "src/deleted.rs", deleted_source);

        let unchanged_symbols = (0..513)
            .map(|index| {
                (
                    format!("sym:unchanged:{index:03}"),
                    format!("unchanged_symbol_{index:03}"),
                )
            })
            .collect::<Vec<_>>();
        let prev_artifact = graph_artifact_for_code_files(
            "prev-hash",
            vec![
                (
                    "src/unchanged.rs",
                    &unchanged_source,
                    code_symbols_for_functions(
                        "src/unchanged.rs",
                        &unchanged_source,
                        &unchanged_symbols
                            .iter()
                            .map(|(id, name)| (id.as_str(), name.clone()))
                            .collect::<Vec<_>>(),
                    ),
                ),
                (
                    "src/changed.rs",
                    changed_old_source,
                    vec![
                        code_symbol(
                            "src/changed.rs",
                            changed_old_source,
                            "sym:keep",
                            "keep_symbol",
                        ),
                        code_symbol(
                            "src/changed.rs",
                            changed_old_source,
                            "sym:reembed",
                            "reembed_symbol",
                        ),
                    ],
                ),
                (
                    "src/deleted.rs",
                    deleted_source,
                    vec![code_symbol(
                        "src/deleted.rs",
                        deleted_source,
                        "sym:removed",
                        "removed_symbol",
                    )],
                ),
            ],
        );
        write_previous_symbol_sidecar_rows(
            prev_dir.path(),
            symbol_rows_from_artifact(&prev_artifact, root.path()),
        )
        .await;

        let changed_new_source = concat!(
            "/// Stable keep docs that should continue to use the previous embedding input.\n",
            "pub fn keep_symbol() {}\n\n",
            "/// New docs for the symbol whose embedding input changed.\n",
            "pub fn reembed_symbol() {}\n",
        );
        write_source(root.path(), "src/changed.rs", changed_new_source);
        fs::remove_file(root.path().join("src/deleted.rs")).expect("remove deleted source");
        let next_artifact = graph_artifact_for_code_files(
            "next-hash",
            vec![
                (
                    "src/unchanged.rs",
                    &unchanged_source,
                    code_symbols_for_functions(
                        "src/unchanged.rs",
                        &unchanged_source,
                        &unchanged_symbols
                            .iter()
                            .map(|(id, name)| (id.as_str(), name.clone()))
                            .collect::<Vec<_>>(),
                    ),
                ),
                (
                    "src/changed.rs",
                    changed_new_source,
                    vec![
                        code_symbol(
                            "src/changed.rs",
                            changed_new_source,
                            "sym:keep",
                            "keep_symbol",
                        ),
                        code_symbol(
                            "src/changed.rs",
                            changed_new_source,
                            "sym:reembed",
                            "reembed_symbol",
                        ),
                    ],
                ),
            ],
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let events = Arc::clone(&events);
            move |event| events.lock().expect("events").push(event)
        };

        let row_counts = write_sections_dataset_with_sidecar_options_and_progress(
            &next_artifact,
            root.path(),
            next_dir.path(),
            incremental_skip_sidecar_options(
                prev_dir.path(),
                &["src/changed.rs"],
                &["src/deleted.rs"],
            ),
            Some(&progress),
        )
        .expect("write incremental sidecar");

        let events = events.lock().expect("events").clone();
        let code_batches = code_symbol_batches(&events);
        let (total_rows, final_rows, written_rows, skipped_existing_rows) =
            code_symbol_finished(&events);
        assert_eq!(total_rows, 2, "progress should count only delta rows");
        assert_eq!(
            final_rows, 515,
            "progress final_rows should report the complete code-symbol table"
        );
        assert_eq!(
            row_counts.code_symbols, 515,
            "final sidecar should keep 513 copied rows plus 2 touched rows"
        );
        assert_eq!(
            written_rows, 2,
            "incremental sidecar planning should write only the touched path rows, avoiding a full 512-row batch that becomes 64-row embedding chunks"
        );
        assert_eq!(
            skipped_existing_rows, 513,
            "unchanged-path rows should be copied/seeded from the previous sidecar"
        );
        assert!(
            code_batches.iter().all(|(batch_rows, _)| *batch_rows <= 2),
            "code-symbol generation should be proportional to touched rows; got batches {code_batches:?}"
        );
        assert_eq!(
            code_symbol_indexing_events(&events),
            Vec::<&'static str>::new(),
            "small seeded code-symbol deltas should not rebuild or optimize indexes"
        );
        let stored_rows = read_stored_symbol_rows(next_dir.path()).await;
        assert_eq!(stored_rows.len(), 515);
        assert!(
            !stored_rows
                .iter()
                .any(|row| row.file_path == "src/deleted.rs"),
            "deleted-path rows from the previous sidecar must not survive seeding"
        );
    }

    #[tokio::test]
    async fn section_sidecar_planning_limits_markdown_delta_to_touched_paths() {
        let root = tempfile::tempdir().expect("root");
        let prev_dir = tempfile::tempdir().expect("prev sidecar");
        let next_dir = tempfile::tempdir().expect("next sidecar");
        let unchanged_source = concat!(
            "## Stable One\n\nUnchanged section body one.\n\n",
            "## Stable Two\n\nUnchanged section body two.\n\n",
            "## Stable Three\n\nUnchanged section body three.\n",
        );
        let changed_old_source = concat!(
            "## Keep Section\n\nThis section body stays the same across the markdown edit.\n\n",
            "## Reembed Section\n\nOld markdown body that should no longer provide a vector.\n",
        );
        let deleted_source =
            "## Removed Section\n\nPrevious markdown row that should be dropped.\n";
        write_source(root.path(), "docs/unchanged.md", unchanged_source);
        write_source(root.path(), "docs/changed.md", changed_old_source);
        write_source(root.path(), "docs/deleted.md", deleted_source);

        let prev_artifact = graph_artifact_for_code_files(
            "prev-section-hash",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_source,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_source,
                        &[
                            ("section:stable-one", "## Stable One"),
                            ("section:stable-two", "## Stable Two"),
                            ("section:stable-three", "## Stable Three"),
                        ],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_old_source,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_old_source,
                        &[
                            ("section:keep", "## Keep Section"),
                            ("section:reembed", "## Reembed Section"),
                        ],
                    ),
                ),
                (
                    "docs/deleted.md",
                    deleted_source,
                    markdown_section_symbols(
                        "docs/deleted.md",
                        deleted_source,
                        &[("section:removed", "## Removed Section")],
                    ),
                ),
            ],
        );
        let mut prev_rows = section_rows_from_artifact(&prev_artifact, root.path());
        for (index, row) in prev_rows.iter_mut().enumerate() {
            row.vector = Some(fake_vector(index as f32));
        }
        write_previous_section_sidecar_rows(prev_dir.path(), prev_rows).await;

        let changed_new_source = concat!(
            "## Keep Section\n\nThis section body stays the same across the markdown edit.\n\n",
            "## Reembed Section\n\nNew markdown body that should be embedded later.\n",
        );
        write_source(root.path(), "docs/changed.md", changed_new_source);
        fs::remove_file(root.path().join("docs/deleted.md")).expect("remove deleted markdown");
        let next_artifact = graph_artifact_for_code_files(
            "next-section-hash",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_source,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_source,
                        &[
                            ("section:stable-one", "## Stable One"),
                            ("section:stable-two", "## Stable Two"),
                            ("section:stable-three", "## Stable Three"),
                        ],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_new_source,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_new_source,
                        &[
                            ("section:keep", "## Keep Section"),
                            ("section:reembed", "## Reembed Section"),
                        ],
                    ),
                ),
            ],
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let events = Arc::clone(&events);
            move |event| events.lock().expect("events").push(event)
        };

        let row_counts = write_sections_dataset_with_sidecar_options_and_progress(
            &next_artifact,
            root.path(),
            next_dir.path(),
            incremental_skip_sidecar_options(
                prev_dir.path(),
                &["docs/changed.md"],
                &["docs/deleted.md"],
            ),
            Some(&progress),
        )
        .expect("write incremental section sidecar");

        let events = events.lock().expect("events").clone();
        let section_batches = section_batches(&events);
        let (total_rows, final_rows, written_rows, skipped_existing_rows) =
            section_finished(&events);
        assert_eq!(total_rows, 2, "progress should count only delta rows");
        assert_eq!(
            final_rows, 5,
            "progress final_rows should report the complete section table"
        );
        assert_eq!(
            row_counts.section_bodies, 5,
            "final section sidecar should keep 3 copied rows plus 2 touched rows"
        );
        assert_eq!(
            section_batches,
            vec![(2, 1)],
            "section generation should be limited to the changed markdown path, not a default 512-row batch before 64-row embedding chunks"
        );
        assert_eq!(
            written_rows, 2,
            "incremental section planning should write only touched markdown rows"
        );
        assert_eq!(
            skipped_existing_rows, 3,
            "unchanged markdown rows should be copied/seeded from the previous sidecar"
        );

        let stored_rows = read_stored_section_rows(next_dir.path()).await;
        assert_eq!(stored_rows.len(), 5);
        assert!(
            !stored_rows
                .iter()
                .any(|row| row.file_path == "docs/deleted.md"),
            "deleted markdown rows from the previous sidecar must not survive seeding"
        );
        for stable_symbol_id in [
            "section:stable-one",
            "section:stable-two",
            "section:stable-three",
            "section:keep",
        ] {
            assert!(
                stored_rows
                    .iter()
                    .any(|row| row.stable_symbol_id == stable_symbol_id && row.has_vector),
                "{stable_symbol_id} should retain its previous section vector"
            );
        }
        assert!(
            stored_rows
                .iter()
                .any(|row| row.stable_symbol_id == "section:reembed" && !row.has_vector),
            "changed markdown body should not retain a stale previous vector"
        );
    }

    #[tokio::test]
    async fn public_skip_delta_sidecar_carries_vectors_and_refreshes_markdown_fts() {
        let root = tempfile::tempdir().expect("root");
        let prev_dir = tempfile::tempdir().expect("prev sidecar");
        let next_dir = tempfile::tempdir().expect("next sidecar");
        let unchanged_source = "## Stable Section\n\nstableoverlaytoken unchanged body.\n";
        let changed_old_source = concat!(
            "## Keep Section\n\nkeepoverlaytoken unchanged body.\n\n",
            "## Reembed Section\n\noldoverlaytoken body that should disappear.\n",
        );
        let deleted_source = "## Deleted Section\n\ndeletedoverlaytoken body.\n";
        write_source(root.path(), "docs/unchanged.md", unchanged_source);
        write_source(root.path(), "docs/changed.md", changed_old_source);
        write_source(root.path(), "docs/deleted.md", deleted_source);

        let prev_artifact = graph_artifact_for_code_files(
            "prev-public-delta",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_source,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_source,
                        &[("section:stable", "## Stable Section")],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_old_source,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_old_source,
                        &[
                            ("section:keep", "## Keep Section"),
                            ("section:reembed", "## Reembed Section"),
                        ],
                    ),
                ),
                (
                    "docs/deleted.md",
                    deleted_source,
                    markdown_section_symbols(
                        "docs/deleted.md",
                        deleted_source,
                        &[("section:deleted", "## Deleted Section")],
                    ),
                ),
            ],
        );
        let mut prev_rows = section_rows_from_artifact(&prev_artifact, root.path());
        for (index, row) in prev_rows.iter_mut().enumerate() {
            row.vector = Some(fake_vector(index as f32));
        }
        write_previous_section_sidecar_rows(prev_dir.path(), prev_rows).await;
        assert_eq!(
            section_fts_hit_count(prev_dir.path(), "oldoverlaytoken").await,
            1
        );

        let changed_new_source = concat!(
            "## Keep Section\n\nkeepoverlaytoken unchanged body.\n\n",
            "## Reembed Section\n\nnewoverlaytoken body should be searchable.\n",
        );
        write_source(root.path(), "docs/changed.md", changed_new_source);
        fs::remove_file(root.path().join("docs/deleted.md")).expect("remove deleted markdown");
        let next_artifact = graph_artifact_for_code_files(
            "next-public-delta",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_source,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_source,
                        &[("section:stable", "## Stable Section")],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_new_source,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_new_source,
                        &[
                            ("section:keep", "## Keep Section"),
                            ("section:reembed", "## Reembed Section"),
                        ],
                    ),
                ),
            ],
        );

        write_sections_dataset_skipping_embeddings_with_delta(
            &next_artifact,
            root.path(),
            next_dir.path(),
            prev_dir.path(),
            sidecar_delta(&["docs/changed.md"], &["docs/deleted.md"]),
        )
        .expect("write public delta sidecar");

        let stored_rows = read_stored_section_rows(next_dir.path()).await;
        assert_eq!(stored_rows.len(), 3);
        assert!(
            !stored_rows
                .iter()
                .any(|row| row.file_path == "docs/deleted.md"),
            "deleted markdown rows must not be copied into the delta sidecar"
        );
        for stable_symbol_id in ["section:stable", "section:keep"] {
            assert!(
                stored_rows
                    .iter()
                    .any(|row| row.stable_symbol_id == stable_symbol_id && row.has_vector),
                "{stable_symbol_id} should retain its carried-forward vector"
            );
        }
        assert!(
            stored_rows
                .iter()
                .any(|row| row.stable_symbol_id == "section:reembed" && !row.has_vector),
            "changed markdown body must not carry a stale vector"
        );
        assert_eq!(
            section_fts_hit_count(next_dir.path(), "newoverlaytoken").await,
            1
        );
        assert_eq!(
            section_fts_hit_count(next_dir.path(), "oldoverlaytoken").await,
            0
        );
        assert_eq!(
            section_fts_hit_count(next_dir.path(), "deletedoverlaytoken").await,
            0
        );
    }

    #[tokio::test]
    async fn fresh_sidecar_seed_reuses_vectors_without_full_512_by_64_reembedding() {
        let root = tempfile::tempdir().expect("root");
        let prev_dir = tempfile::tempdir().expect("prev sidecar");
        let next_dir = tempfile::tempdir().expect("next sidecar");
        let unchanged_source = concat!(
            "/// One unchanged function with a previous vector.\n",
            "pub fn stable_one() {}\n\n",
            "/// Two unchanged function with a previous vector.\n",
            "pub fn stable_two() {}\n",
        );
        let changed_old_source = concat!(
            "/// Keep docs are unchanged even though this file changes.\n",
            "pub fn keep_symbol() {}\n\n",
            "/// Old docs for the changed embedding input with enough detail to be embedded.\n",
            "pub fn reembed_symbol() {}\n",
        );
        let deleted_source =
            "/// Removed docs with a previous vector.\npub fn removed_symbol() {}\n";
        write_source(root.path(), "src/unchanged.rs", unchanged_source);
        write_source(root.path(), "src/changed.rs", changed_old_source);
        write_source(root.path(), "src/deleted.rs", deleted_source);

        let prev_artifact = graph_artifact_for_code_files(
            "prev-hash-small",
            vec![
                (
                    "src/unchanged.rs",
                    unchanged_source,
                    vec![
                        code_symbol(
                            "src/unchanged.rs",
                            unchanged_source,
                            "sym:stable-one",
                            "stable_one",
                        ),
                        code_symbol(
                            "src/unchanged.rs",
                            unchanged_source,
                            "sym:stable-two",
                            "stable_two",
                        ),
                    ],
                ),
                (
                    "src/changed.rs",
                    changed_old_source,
                    vec![
                        code_symbol(
                            "src/changed.rs",
                            changed_old_source,
                            "sym:keep",
                            "keep_symbol",
                        ),
                        code_symbol(
                            "src/changed.rs",
                            changed_old_source,
                            "sym:reembed",
                            "reembed_symbol",
                        ),
                    ],
                ),
                (
                    "src/deleted.rs",
                    deleted_source,
                    vec![code_symbol(
                        "src/deleted.rs",
                        deleted_source,
                        "sym:removed",
                        "removed_symbol",
                    )],
                ),
            ],
        );
        let mut prev_rows = symbol_rows_from_artifact(&prev_artifact, root.path());
        for (index, row) in prev_rows.iter_mut().enumerate() {
            row.vector = Some(fake_vector(index as f32));
        }
        write_previous_symbol_sidecar_rows(prev_dir.path(), prev_rows).await;

        let changed_new_source = concat!(
            "/// Keep docs are unchanged even though this file changes.\n",
            "pub fn keep_symbol() {}\n\n",
            "/// New docs for the changed embedding input with enough detail to be embedded.\n",
            "pub fn reembed_symbol() {}\n",
        );
        write_source(root.path(), "src/changed.rs", changed_new_source);
        fs::remove_file(root.path().join("src/deleted.rs")).expect("remove deleted source");
        let next_artifact = graph_artifact_for_code_files(
            "next-hash-small",
            vec![
                (
                    "src/unchanged.rs",
                    unchanged_source,
                    vec![
                        code_symbol(
                            "src/unchanged.rs",
                            unchanged_source,
                            "sym:stable-one",
                            "stable_one",
                        ),
                        code_symbol(
                            "src/unchanged.rs",
                            unchanged_source,
                            "sym:stable-two",
                            "stable_two",
                        ),
                    ],
                ),
                (
                    "src/changed.rs",
                    changed_new_source,
                    vec![
                        code_symbol(
                            "src/changed.rs",
                            changed_new_source,
                            "sym:keep",
                            "keep_symbol",
                        ),
                        code_symbol(
                            "src/changed.rs",
                            changed_new_source,
                            "sym:reembed",
                            "reembed_symbol",
                        ),
                    ],
                ),
            ],
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let events = Arc::clone(&events);
            move |event| events.lock().expect("events").push(event)
        };

        let row_counts = write_sections_dataset_async_with_embedding_model(
            &next_artifact,
            root.path(),
            next_dir.path(),
            incremental_skip_sidecar_options(
                prev_dir.path(),
                &["src/changed.rs"],
                &["src/deleted.rs"],
            ),
            EmbeddingModelSelection::NomicEmbedTextV15,
            Some(&progress),
        )
        .await
        .expect("write fresh seeded sidecar");

        let events = events.lock().expect("events").clone();
        let code_batches = code_symbol_batches(&events);
        let (total_rows, final_rows, written_rows, skipped_existing_rows) =
            code_symbol_finished(&events);
        assert_eq!(total_rows, 2, "progress should count only delta rows");
        assert_eq!(
            final_rows, 4,
            "progress final_rows should report the complete code-symbol table"
        );
        assert_eq!(row_counts.code_symbols, 4);
        assert_eq!(
            code_batches,
            vec![(2, 1)],
            "fresh staging should generate the touched file only and re-embed only the changed embedding input"
        );
        assert_eq!(
            written_rows, 2,
            "fresh staging should be seeded from the previous sidecar instead of rewriting all rows"
        );
        assert_eq!(
            skipped_existing_rows, 2,
            "unchanged rows should be present from the seed before touched rows are written"
        );

        let stored_rows = read_stored_symbol_rows(next_dir.path()).await;
        assert_eq!(stored_rows.len(), 4);
        for stable_symbol_id in ["sym:stable-one", "sym:stable-two", "sym:keep"] {
            assert!(
                stored_rows
                    .iter()
                    .any(|row| row.stable_symbol_id == stable_symbol_id && row.has_vector),
                "{stable_symbol_id} should retain its previous vector in the fresh sidecar"
            );
        }
        assert!(
            !stored_rows
                .iter()
                .any(|row| row.stable_symbol_id == "sym:removed"),
            "deleted-path vector rows must not be copied into fresh staging"
        );
    }

    #[tokio::test]
    async fn fresh_delta_sidecar_rebuilds_when_previous_embedding_model_differs() {
        const LEGACY_EMBED_MODEL_NAME: &str = "LegacyEmbeddingModel";

        let root = tempfile::tempdir().expect("root");
        let prev_dir = tempfile::tempdir().expect("prev sidecar");
        let next_dir = tempfile::tempdir().expect("next sidecar");
        let unchanged_markdown = "## Stable Section\n\nUnchanged markdown body.\n";
        let changed_markdown_old = "## Changed Section\n\nOld markdown body.\n";
        let unchanged_source = concat!(
            "/// Unchanged function documentation long enough to be embedded and retained in the old sidecar.\n",
            "pub fn stable_symbol() {}\n",
        );
        let changed_source_old = concat!(
            "/// Old changed function documentation long enough to be embedded in the old sidecar.\n",
            "pub fn changed_symbol() {}\n",
        );
        write_source(root.path(), "docs/unchanged.md", unchanged_markdown);
        write_source(root.path(), "docs/changed.md", changed_markdown_old);
        write_source(root.path(), "src/unchanged.rs", unchanged_source);
        write_source(root.path(), "src/changed.rs", changed_source_old);

        let prev_artifact = graph_artifact_for_code_files(
            "prev-model-hash",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_markdown,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_markdown,
                        &[("section:stable", "## Stable Section")],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_markdown_old,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_markdown_old,
                        &[("section:changed", "## Changed Section")],
                    ),
                ),
                (
                    "src/unchanged.rs",
                    unchanged_source,
                    vec![code_symbol(
                        "src/unchanged.rs",
                        unchanged_source,
                        "sym:stable",
                        "stable_symbol",
                    )],
                ),
                (
                    "src/changed.rs",
                    changed_source_old,
                    vec![code_symbol(
                        "src/changed.rs",
                        changed_source_old,
                        "sym:changed",
                        "changed_symbol",
                    )],
                ),
            ],
        );
        let mut prev_section_rows = section_rows_from_artifact(&prev_artifact, root.path());
        for (index, row) in prev_section_rows.iter_mut().enumerate() {
            row.vector = Some(fake_vector(index as f32));
            row.embedding_model = LEGACY_EMBED_MODEL_NAME.to_owned();
        }
        let mut prev_symbol_rows = symbol_rows_from_artifact(&prev_artifact, root.path());
        for (index, row) in prev_symbol_rows.iter_mut().enumerate() {
            row.vector = Some(fake_vector(index as f32));
            row.embedding_model = LEGACY_EMBED_MODEL_NAME.to_owned();
        }
        write_previous_section_sidecar_rows(prev_dir.path(), prev_section_rows).await;
        write_previous_symbol_sidecar_rows(prev_dir.path(), prev_symbol_rows).await;

        let changed_markdown_new = "## Changed Section\n\nNew markdown body.\n";
        let changed_source_new = concat!(
            "/// New changed function documentation long enough to be embedded with the Nomic sidecar model.\n",
            "pub fn changed_symbol() {}\n",
        );
        write_source(root.path(), "docs/changed.md", changed_markdown_new);
        write_source(root.path(), "src/changed.rs", changed_source_new);
        let next_artifact = graph_artifact_for_code_files(
            "next-model-hash",
            vec![
                (
                    "docs/unchanged.md",
                    unchanged_markdown,
                    markdown_section_symbols(
                        "docs/unchanged.md",
                        unchanged_markdown,
                        &[("section:stable", "## Stable Section")],
                    ),
                ),
                (
                    "docs/changed.md",
                    changed_markdown_new,
                    markdown_section_symbols(
                        "docs/changed.md",
                        changed_markdown_new,
                        &[("section:changed", "## Changed Section")],
                    ),
                ),
                (
                    "src/unchanged.rs",
                    unchanged_source,
                    vec![code_symbol(
                        "src/unchanged.rs",
                        unchanged_source,
                        "sym:stable",
                        "stable_symbol",
                    )],
                ),
                (
                    "src/changed.rs",
                    changed_source_new,
                    vec![code_symbol(
                        "src/changed.rs",
                        changed_source_new,
                        "sym:changed",
                        "changed_symbol",
                    )],
                ),
            ],
        );

        let row_counts = write_sections_dataset_async_with_embedding_model(
            &next_artifact,
            root.path(),
            next_dir.path(),
            incremental_skip_sidecar_options(
                prev_dir.path(),
                &["docs/changed.md", "src/changed.rs"],
                &[],
            ),
            EmbeddingModelSelection::NomicEmbedTextV15,
            None,
        )
        .await
        .expect("write Nomic delta from incompatible sidecar seed");

        assert_eq!(row_counts.section_bodies, 2);
        assert_eq!(row_counts.code_symbols, 2);
        let section_rows = read_stored_section_rows(next_dir.path()).await;
        assert_eq!(section_rows.len(), 2);
        assert!(
            section_rows
                .iter()
                .all(|row| row.embedding_model == NOMIC_EMBED_TEXT_V15_MODEL_NAME),
            "section sidecar should fall back to a full Nomic rewrite instead of retaining legacy rows: {section_rows:?}"
        );
        let symbol_rows = read_stored_symbol_rows(next_dir.path()).await;
        assert_eq!(symbol_rows.len(), 2);
        assert!(
            symbol_rows
                .iter()
                .all(|row| row.embedding_model == NOMIC_EMBED_TEXT_V15_MODEL_NAME),
            "code symbol sidecar should fall back to a full Nomic rewrite instead of retaining legacy rows: {symbol_rows:?}"
        );
    }

    #[tokio::test]
    async fn existing_code_symbol_sidecar_delta_deletes_removed_paths_without_batches() {
        let root = tempfile::tempdir().expect("root");
        let sidecar_dir = tempfile::tempdir().expect("sidecar");
        let deleted_source =
            "/// Removed docs with a previous sidecar row.\npub fn removed_symbol() {}\n";
        write_source(root.path(), "src/deleted.rs", deleted_source);
        let prev_artifact = graph_artifact_for_code_files(
            "prev-delete-only",
            vec![(
                "src/deleted.rs",
                deleted_source,
                vec![code_symbol(
                    "src/deleted.rs",
                    deleted_source,
                    "sym:removed",
                    "removed_symbol",
                )],
            )],
        );
        write_previous_symbol_sidecar_rows(
            sidecar_dir.path(),
            symbol_rows_from_artifact(&prev_artifact, root.path()),
        )
        .await;
        fs::remove_file(root.path().join("src/deleted.rs")).expect("remove deleted source");
        let next_artifact = graph_artifact_for_code_files("next-delete-only", Vec::new());

        let row_counts = write_sections_dataset_with_sidecar_options_and_progress(
            &next_artifact,
            root.path(),
            sidecar_dir.path(),
            SectionSidecarOptions {
                embedding: SectionEmbeddingOptions {
                    skip_section_embeddings: true,
                    skip_code_symbol_embeddings: true,
                    batch_size: SECTION_EMBED_BATCH_SIZE_DEFAULT,
                },
                write_batch_size: SECTION_WRITE_BATCH_SIZE_DEFAULT,
                previous_artifact_dir: None,
                delta: Some(sidecar_delta(&[], &["src/deleted.rs"])),
            },
            None,
        )
        .expect("write delete-only delta");

        assert_eq!(
            row_counts.code_symbols, 0,
            "deleted path rows should be removed even when no changed path emits a batch"
        );
        assert!(
            read_stored_symbol_rows(sidecar_dir.path()).await.is_empty(),
            "existing sidecar table must not retain deleted path rows"
        );
    }

    #[tokio::test]
    async fn vector_backfill_fills_missing_section_vectors_without_recomputing_existing_or_stale_hashes(
    ) {
        let sidecar_dir = tempfile::tempdir().expect("sidecar");
        let mut filled = versioned_section_row("section:filled", "docs/filled.md", "hash-filled");
        filled.vector = Some(fake_vector(7.0));
        let missing = versioned_section_row("section:missing", "docs/missing.md", "hash-missing");
        let expected_missing_input = embedding_document_text_for_model(
            missing.qualified_name.as_str(),
            missing.body_text.as_str(),
            EmbeddingModelSelection::NomicEmbedTextV15,
        )
        .into_owned();
        let mut stale_hash = versioned_section_row("section:stale", "docs/stale.md", "hash-stale");
        stale_hash.body_text = "## Current\n\nCurrent body.".to_owned();
        stale_hash.embedding_input_hash = section_embedding_input_hash_for_model(
            stale_hash.qualified_name.as_str(),
            "## Old\n\nOld body.",
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        let h1 = SectionRow {
            heading_level: 1,
            ..versioned_section_row("section:h1", "docs/h1.md", "hash-h1")
        };
        write_previous_section_sidecar_rows(
            sidecar_dir.path(),
            vec![filled, missing, stale_hash, h1],
        )
        .await;

        let mut calls = Vec::new();
        let stats = backfill_missing_vectors_with(
            sidecar_dir.path(),
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: true,
                batch_size: 8,
            },
            |phase, texts| {
                calls.push((
                    phase,
                    texts
                        .iter()
                        .map(|text| (*text).to_owned())
                        .collect::<Vec<_>>(),
                ));
                Ok(texts.iter().map(|_| fake_vector(42.0)).collect())
            },
        )
        .await
        .expect("backfill section vectors");

        assert_eq!(stats.sections.filled_rows, 1);
        assert_eq!(stats.code_symbols.filled_rows, 0);
        assert_eq!(
            calls,
            vec![(SidecarPhase::Sections, vec![expected_missing_input])]
        );

        let stored_rows = read_stored_section_rows(sidecar_dir.path()).await;
        let stored = |stable_symbol_id: &str| {
            stored_rows
                .iter()
                .find(|row| row.stable_symbol_id == stable_symbol_id)
                .unwrap_or_else(|| panic!("missing stored row {stable_symbol_id}"))
        };
        assert_eq!(stored("section:filled").vector, Some(fake_vector(7.0)));
        assert_eq!(stored("section:missing").vector, Some(fake_vector(42.0)));
        assert!(!stored("section:stale").has_vector);
        assert!(!stored("section:h1").has_vector);
    }

    #[tokio::test]
    async fn vector_backfill_prioritizes_code_symbol_vectors_before_sections_by_default() {
        let sidecar_dir = tempfile::tempdir().expect("sidecar");
        let section = versioned_section_row("section:missing", "docs/missing.md", "hash-section");
        let symbol = symbol_row_fixture("sym-missing", "symbol text");
        write_previous_section_sidecar_rows(sidecar_dir.path(), vec![section]).await;
        write_previous_symbol_sidecar_rows(sidecar_dir.path(), vec![symbol]).await;

        let mut calls = Vec::new();
        let stats = backfill_missing_vectors_with(
            sidecar_dir.path(),
            SectionEmbeddingOptions {
                skip_section_embeddings: false,
                skip_code_symbol_embeddings: false,
                batch_size: 8,
            },
            |phase, texts| {
                calls.push(phase);
                let value = match phase {
                    SidecarPhase::CodeSymbols => 31.0,
                    SidecarPhase::Sections => 42.0,
                };
                Ok(texts.iter().map(|_| fake_vector(value)).collect())
            },
        )
        .await
        .expect("backfill all vectors");

        assert_eq!(
            calls,
            vec![SidecarPhase::CodeSymbols, SidecarPhase::Sections]
        );
        assert_eq!(stats.code_symbols.total_rows, 1);
        assert_eq!(stats.code_symbols.filled_rows, 1);
        assert_eq!(stats.sections.total_rows, 1);
        assert_eq!(stats.sections.filled_rows, 1);

        let symbol_rows = read_stored_symbol_rows(sidecar_dir.path()).await;
        assert_eq!(symbol_rows[0].vector, Some(fake_vector(31.0)));
        let section_rows = read_stored_section_rows(sidecar_dir.path()).await;
        assert_eq!(section_rows[0].vector, Some(fake_vector(42.0)));
    }

    #[tokio::test]
    async fn vector_backfill_resumes_symbol_vectors_and_skips_changed_input_hashes() {
        let sidecar_dir = tempfile::tempdir().expect("sidecar");
        let mut filled = symbol_row_fixture("sym-filled", "filled embed text");
        filled.vector = Some(fake_vector(3.0));
        let missing_one = symbol_row_fixture("sym-missing-one", "first missing text");
        let missing_two = symbol_row_fixture("sym-missing-two", "second missing text");
        let mut stale_hash = symbol_row_fixture("sym-stale", "current stale text");
        stale_hash.embedding_input_hash = symbol_embedding_input_hash_for_model(
            "old stale text",
            false,
            EmbeddingModelSelection::NomicEmbedTextV15,
        );
        write_previous_symbol_sidecar_rows(
            sidecar_dir.path(),
            vec![filled, missing_one, missing_two, stale_hash],
        )
        .await;

        let options = SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 1,
        };
        let mut first_run_calls = Vec::new();
        let mut chunk_index = 0usize;
        let first_stats =
            backfill_missing_vectors_with(sidecar_dir.path(), options, |phase, texts| {
                chunk_index += 1;
                first_run_calls.push((
                    phase,
                    texts
                        .iter()
                        .map(|text| (*text).to_owned())
                        .collect::<Vec<_>>(),
                ));
                if chunk_index == 2 {
                    anyhow::bail!("simulated interruption after first symbol chunk");
                }
                Ok(texts.iter().map(|_| fake_vector(11.0)).collect())
            })
            .await
            .expect("first partial symbol backfill");

        assert_eq!(first_stats.code_symbols.filled_rows, 1);
        assert_eq!(
            first_run_calls,
            vec![
                (
                    SidecarPhase::CodeSymbols,
                    vec!["search_document: title: none | text: first missing text".to_owned()]
                ),
                (
                    SidecarPhase::CodeSymbols,
                    vec!["search_document: title: none | text: second missing text".to_owned()]
                ),
            ]
        );

        let mut second_run_calls = Vec::new();
        let second_stats =
            backfill_missing_vectors_with(sidecar_dir.path(), options, |phase, texts| {
                second_run_calls.push((
                    phase,
                    texts
                        .iter()
                        .map(|text| (*text).to_owned())
                        .collect::<Vec<_>>(),
                ));
                Ok(texts.iter().map(|_| fake_vector(22.0)).collect())
            })
            .await
            .expect("resume symbol backfill");

        assert_eq!(second_stats.code_symbols.filled_rows, 1);
        assert_eq!(
            second_run_calls,
            vec![(
                SidecarPhase::CodeSymbols,
                vec!["search_document: title: none | text: second missing text".to_owned()]
            )]
        );

        let stored_rows = read_stored_symbol_rows(sidecar_dir.path()).await;
        let stored = |stable_symbol_id: &str| {
            stored_rows
                .iter()
                .find(|row| row.stable_symbol_id == stable_symbol_id)
                .unwrap_or_else(|| panic!("missing stored row {stable_symbol_id}"))
        };
        assert_eq!(stored("sym-filled").vector, Some(fake_vector(3.0)));
        assert_eq!(stored("sym-missing-one").vector, Some(fake_vector(11.0)));
        assert_eq!(stored("sym-missing-two").vector, Some(fake_vector(22.0)));
        assert!(!stored("sym-stale").has_vector);
    }

    /// Pre-filled vectors must not be passed to the embedder (section).
    #[test]
    fn section_embedding_inputs_skips_pre_filled_vectors() {
        let mut row_with_vector = section_row_fixture(2, "## Filled\n\nBody.".to_owned());
        row_with_vector.vector = Some(vec![1.0; EMBEDDING_VECTOR_DIMENSIONS]);
        let row_without = section_row_fixture(2, "## Empty\n\nBody.".to_owned());

        let rows = [row_with_vector, row_without];
        let inputs = section_embedding_inputs(&rows, EmbeddingModelSelection::NomicEmbedTextV15);
        assert_eq!(
            inputs.len(),
            1,
            "only the row without a vector should be in inputs"
        );
        assert_eq!(inputs[0].stable_symbol_id, "symbol");
    }

    /// Pre-filled vectors must not be passed to the embedder (symbols).
    #[test]
    fn symbol_embedding_inputs_skips_pre_filled_vectors() {
        let mut row_with_vector = symbol_row_fixture("sym-a", "some embed text");
        row_with_vector.vector = Some(vec![0.5; EMBEDDING_VECTOR_DIMENSIONS]);
        let row_without = symbol_row_fixture("sym-b", "other embed text");

        let rows = [row_with_vector, row_without];
        let inputs = symbol_embedding_inputs(&rows, EmbeddingModelSelection::NomicEmbedTextV15);
        assert_eq!(
            inputs.len(),
            1,
            "only the row without a vector should be in inputs"
        );
        assert_eq!(inputs[0].stable_symbol_id, "sym-b");
    }

    // ---- end carry-forward tests ----

    #[test]
    fn embed_eligible_rows_reports_chunk_progress() {
        let rows = vec![
            section_row_fixture(2, "## One\n\nBody one.".to_owned()),
            section_row_fixture(1, "# Title\n\nSkipped.".to_owned()),
            section_row_fixture(2, "## Two\n\nBody two.".to_owned()),
            section_row_fixture(2, "## Three\n\nBody three.".to_owned()),
        ];
        let options = SectionEmbeddingOptions {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: false,
            batch_size: 2,
        };
        let mut progress = Vec::new();

        let vectors = embed_eligible_rows_with(
            &rows,
            options,
            |chunk| progress.push(chunk),
            |texts| Ok(vec![vec![0.25; EMBEDDING_VECTOR_DIMENSIONS]; texts.len()]),
        );

        assert_eq!(
            progress,
            vec![
                SectionEmbeddingChunkProgress {
                    chunk_index: 1,
                    chunk_count: 2,
                    chunk_rows: 2,
                    completed_eligible_rows: 0,
                    embedding_eligible_rows: 3,
                },
                SectionEmbeddingChunkProgress {
                    chunk_index: 2,
                    chunk_count: 2,
                    chunk_rows: 1,
                    completed_eligible_rows: 2,
                    embedding_eligible_rows: 3,
                },
            ]
        );
        assert!(vectors[0].is_some());
        assert!(vectors[1].is_none());
        assert!(vectors[2].is_some());
        assert!(vectors[3].is_some());
    }
}
