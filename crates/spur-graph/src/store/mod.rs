pub mod artifact_staging;
pub mod build;
pub mod cache;
pub mod canonical_hash;
pub mod commit_index;
pub mod lance_sections;
pub mod macro_invocations;
pub mod parquet;
pub mod pointer;
pub mod shard_writer;
pub mod snapshot;
pub mod temporal_shards;

pub use artifact_staging::ArtifactStagingDir;
pub use build::{
    artifact_from_facts, artifact_from_facts_incremental, current_manifest_version, BuildMode,
    EXTRACTOR_VERSION, SCHEMA_VERSION,
};
pub use canonical_hash::artifact_content_hash_blake3_hex;
pub use lance_sections::{
    embedding_query_text_for_model, fastembed_cache_dir, write_sections_dataset,
    EmbeddingModelSelection, CODE_RANK_EMBED_MODEL_NAME, CODE_SYMBOLS_DATASET_DIR,
    CODE_SYMBOLS_PARQUET, CODE_SYMBOLS_TABLE, EMBEDDING_VECTOR_DIMENSIONS, EMBED_MODEL_ENV,
    JINA_EMBEDDINGS_V2_BASE_CODE_MODEL_NAME, NOMIC_EMBED_TEXT_V15_MODEL_NAME, SECTIONS_DATASET_DIR,
    SECTIONS_PARQUET, SECTIONS_TABLE,
};
#[cfg(feature = "embed")]
pub use lance_sections::{
    load_embedding_model, load_embedding_model_cancellable,
    load_embedding_model_cancellable_with_cleanup,
};
pub use parquet::{
    load_temporal_artifact_parquet, read_artifact_header_parquet, read_artifact_parquet,
    stamp_sidecar_status, stream_temporal_artifact_parquet, write_artifact_parquet,
    write_worktree_delta, DeltaSummary, GraphArtifactManifest, GraphArtifactSidecarRowCounts,
    GraphArtifactSidecarStatus, TemporalArtifactTable, WriteOptions,
};
pub use pointer::{
    read_current_pointer, resolve_artifact_location, write_current_pointer, ArtifactCacheKey,
    ResolvedArtifact,
};
pub use shard_writer::TemporalShardSink;
pub use temporal_shards::{ShardIndexEntry, TemporalShardConfig};
