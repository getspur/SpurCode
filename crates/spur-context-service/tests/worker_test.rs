#![cfg(feature = "worker")]

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::task::{Context as TaskContext, Poll};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use serde_json::Value;
use sha2::Digest as _;
use spur_context_service::jobs::{
    CreateJobOutcome, CreateJobRequest, JobKey, JobRecord, JobStatus, JobStore, JobsError,
    QueueConfig,
};
use spur_context_service::mcp::{IndexExecutionRequest, IndexExecutionStarter, McpHandlerError};
use spur_context_service::worker::{
    acquire_catalog_lease_with_retry, build_graph, download_silver_artifact_from_manifest,
    fetch_source, fetch_source_with_bronze_services, handle_spot_interruption,
    persist_silver_graph_artifact, prepare_job_with_services, retrieve_bronze_source_by_coordinate,
    run_job_and_record_with_services, upload_with_owned_catalog_lease,
    write_verified_silver_manifest_file_stream, BronzeArchiveStore, BronzeRawSource,
    BronzeRawSourceRegistry, CatalogDownload, CatalogLease, CatalogLeaseStore,
    GraphArtifactBuilder, JobEnv, JobFromLayer, SilverArtifactStore, SilverGraphArtifact,
    SilverGraphArtifactRegistry, SilverUploadedFile, StageTracker, WorkerError,
};
use tokio::io::{AsyncRead, ReadBuf};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SIDECAR_LIB_SOURCE: &str = "pub fn answer() -> u32 { 42 }\n";
const SIDECAR_UTIL_SOURCE: &str = "pub fn double(value: u32) -> u32 { value * 2 }\n";

#[test]
fn job_env_from_env_reads_catalog_dsn() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set_all([
        ("TASK_TOKEN", "task-token"),
        ("JOB_ID", "job-123"),
        ("PACKAGE", "serde"),
        ("REVISION", "1.0.197"),
        ("SOURCE", "registry:crates-io"),
        (
            "SOURCE_URL",
            "https://crates.io/api/v1/crates/serde/1.0.197/download",
        ),
        ("SOURCE_KIND", "tarball"),
        ("SPUR_CATALOG_DSN", "sqlite:/tmp/catalog.sqlite"),
    ]);

    let env = JobEnv::from_env()?;

    assert_eq!(env.task_token, "task-token");
    assert_eq!(env.job_id, "job-123");
    assert_eq!(env.package, "serde");
    assert_eq!(env.revision, "1.0.197");
    assert_eq!(env.source, "registry:crates-io");
    assert_eq!(
        env.source_url,
        "https://crates.io/api/v1/crates/serde/1.0.197/download"
    );
    assert_eq!(env.source_kind, "tarball");
    assert_eq!(env.catalog_dsn, "sqlite:/tmp/catalog.sqlite");
    assert_eq!(env.from_layer, JobFromLayer::Source);
    Ok(())
}

#[test]
fn job_env_from_env_args_parses_reprocess_from_layer() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set_all([
        ("TASK_TOKEN", "task-token"),
        ("JOB_ID", "job-123"),
        ("PACKAGE", "serde"),
        ("REVISION", "1.0.197"),
        ("SOURCE", "registry:crates-io"),
        (
            "SOURCE_URL",
            "https://crates.io/api/v1/crates/serde/1.0.197/download",
        ),
        ("SOURCE_KIND", "tarball"),
        ("SPUR_CATALOG_DSN", "sqlite:/tmp/catalog.sqlite"),
    ]);

    let env = JobEnv::from_env_args(["worker", "--from-layer", "silver"])?;

    assert_eq!(env.from_layer, JobFromLayer::Silver);
    Ok(())
}

#[test]
fn job_env_from_env_args_rejects_unknown_reprocess_layer() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env = EnvGuard::set_all([
        ("TASK_TOKEN", "task-token"),
        ("JOB_ID", "job-123"),
        ("PACKAGE", "serde"),
        ("REVISION", "1.0.197"),
        ("SOURCE", "registry:crates-io"),
        (
            "SOURCE_URL",
            "https://crates.io/api/v1/crates/serde/1.0.197/download",
        ),
        ("SOURCE_KIND", "tarball"),
        ("SPUR_CATALOG_DSN", "sqlite:/tmp/catalog.sqlite"),
    ]);

    let error = JobEnv::from_env_args(["worker", "--from-layer", "raw"]).unwrap_err();

    assert!(error.to_string().contains("unsupported --from-layer `raw`"));
    Ok(())
}

#[test]
fn fetch_source_downloads_and_extracts_tarball() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1")]);
    let root = unique_temp_dir("worker-tarball")?;
    let fixture = root.join("fixture").join("demo-0.1.0");
    fs::create_dir_all(fixture.join("src")).context("create fixture")?;
    fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .context("write manifest")?;
    fs::write(fixture.join("src/lib.rs"), "pub fn demo() {}\n").context("write lib")?;

    let archive = root.join("demo.tar.gz");
    create_tarball(root.join("fixture").as_path(), &archive)?;
    let source_url = serve_once(fs::read(&archive).context("read archive")?);

    let fetched = fetch_source(&source_url, "tarball", "0.1.0", &root.join("fetch"))?;

    assert!(fetched.join("Cargo.toml").is_file());
    assert_eq!(
        fs::read_to_string(fetched.join("src/lib.rs")).context("read fetched lib")?,
        "pub fn demo() {}\n"
    );
    Ok(())
}

#[tokio::test]
async fn bronze_fetch_uploads_archive_and_registers_raw_source() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1")]);
    let root = unique_temp_dir("worker-bronze-upload")?;
    let archive = demo_tarball(&root)?;
    let source_url = serve_once(fs::read(&archive).context("read archive")?);
    let registry = FakeBronzeRegistry::default();
    let store = FakeBronzeArchiveStore::default();
    let env = demo_job_env(&source_url);

    let fetched =
        fetch_source_with_bronze_services(&env, &root.join("fetch"), &registry, &store).await?;

    assert!(fetched.join("Cargo.toml").is_file());
    assert_eq!(
        store.uploaded_keys(),
        ["bronze/registry:crates-io/demo/0.1.0/source.tar.gz"]
    );
    let rows = registry.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.source, "registry:crates-io");
    assert_eq!(row.package, "demo");
    assert_eq!(row.version, "0.1.0");
    assert_eq!(row.revision_kind, "semver");
    assert_eq!(row.semver_major, Some(0));
    assert_eq!(row.semver_minor, Some(1));
    assert_eq!(row.semver_patch, Some(0));
    assert_eq!(row.source_kind, "tarball");
    assert_eq!(row.source_url, source_url);
    assert_eq!(
        row.s3_uri,
        "s3://bronze-test/bronze/registry:crates-io/demo/0.1.0/source.tar.gz"
    );
    assert_eq!(row.bytes, fs::metadata(&archive)?.len());
    assert_eq!(row.fetch_status, "success");
    assert_eq!(
        store.sha256_for(&row.s3_uri),
        Some(row.content_sha256.clone())
    );
    Ok(())
}

#[tokio::test]
async fn retrieve_bronze_source_by_coordinate_extracts_registered_archive() -> Result<()> {
    let root = unique_temp_dir("worker-bronze-retrieve")?;
    let archive = demo_tarball(&root)?;
    let archive_bytes = fs::read(&archive).context("read archive")?;
    let store = FakeBronzeArchiveStore::default();
    let content_sha256 = store.seed_object(
        "bronze/registry:crates-io/demo/0.1.0/source.tar.gz",
        archive_bytes,
    );
    let registry = FakeBronzeRegistry::with_row(bronze_row(
        "http://127.0.0.1:1/unreachable.tar.gz",
        &content_sha256,
        fs::metadata(&archive)?.len(),
    ));

    let restored = retrieve_bronze_source_by_coordinate(
        "registry:crates-io",
        "demo",
        "0.1.0",
        &root.join("restored"),
        &registry,
        &store,
    )
    .await?
    .context("bronze source should be restored")?;

    assert!(restored.join("Cargo.toml").is_file());
    assert_eq!(
        fs::read_to_string(restored.join("src/lib.rs")).context("read restored lib")?,
        "pub fn demo() {}\n"
    );
    assert_eq!(store.downloads(), 1);
    Ok(())
}

#[tokio::test]
async fn bronze_dedup_skips_upstream_fetch_when_registered_hash_matches() -> Result<()> {
    let root = unique_temp_dir("worker-bronze-dedup")?;
    let archive = demo_tarball(&root)?;
    let archive_bytes = fs::read(&archive).context("read archive")?;
    let store = FakeBronzeArchiveStore::default();
    let content_sha256 = store.seed_object(
        "bronze/registry:crates-io/demo/0.1.0/source.tar.gz",
        archive_bytes,
    );
    let registry = FakeBronzeRegistry::with_row(bronze_row(
        "http://127.0.0.1:1/unreachable.tar.gz",
        &content_sha256,
        fs::metadata(&archive)?.len(),
    ));
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let fetched =
        fetch_source_with_bronze_services(&env, &root.join("fetch"), &registry, &store).await?;

    assert!(fetched.join("src/lib.rs").is_file());
    assert_eq!(store.uploads(), 0);
    assert_eq!(store.downloads(), 1);
    assert_eq!(registry.registers(), 0);
    Ok(())
}

#[tokio::test]
async fn bronze_fetch_errors_when_existing_success_row_hash_drifts() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1")]);
    let root = unique_temp_dir("worker-bronze-drift")?;
    let archive = demo_tarball(&root)?;
    let source_url = serve_once(fs::read(&archive).context("read archive")?);
    let registry = FakeBronzeRegistry::with_row(bronze_row(
        "http://127.0.0.1:1/stale.tar.gz",
        "old-sha256",
        fs::metadata(&archive)?.len(),
    ));
    let store = FakeBronzeArchiveStore::default();
    let env = demo_job_env(&source_url);

    let err = fetch_source_with_bronze_services(&env, &root.join("fetch"), &registry, &store)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("bronze content drift"));
    assert_eq!(store.uploads(), 0);
    assert_eq!(registry.registers(), 0);
    Ok(())
}

#[tokio::test]
async fn silver_bundle_publication_uses_distinct_complete_versioned_root() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_SILVER_BUCKET", "silver-test")]);
    let root = unique_temp_dir("worker-silver-upload")?;
    let artifact_dir = root.join("artifact");
    let source_root = root.join("source");
    fs::create_dir_all(source_root.join("src"))?;
    fs::write(source_root.join("src/lib.rs"), "pub fn demo() {}\n")?;
    write_silver_artifact_fixture(&artifact_dir)?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeSilverArtifactStore::new(events.clone());
    let registry = FakeSilverRegistry::new(events.clone());
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let row = persist_silver_graph_artifact(
        &env,
        &artifact_dir,
        &source_root,
        "bronze-sha256",
        "builder-v1",
        &store,
        &registry,
    )
    .await?;

    assert_eq!(
        row.artifact_s3_prefix,
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/"
    );
    assert_eq!(
        row.manifest_uri,
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json"
    );
    assert_eq!(row.builder_version, "builder-v1");
    assert_eq!(row.graph_content_hash, "graph-hash-123");
    assert_eq!(row.node_count, 7);
    assert_eq!(row.edge_count, 11);
    assert_eq!(row.file_count, 3);
    assert_eq!(row.embedding_count, 5);
    assert_eq!(row.build_status, "success");

    let registered = registry.rows();
    assert_eq!(registered, [row]);

    let manifest = store.manifest().context("manifest should be uploaded")?;
    assert_eq!(serde_json::to_value(&manifest)?["schema_version"], 1);
    assert_eq!(manifest.files.len(), 9);
    assert!(manifest.schema_hash.starts_with("sha256:"));
    assert!(manifest
        .files
        .iter()
        .any(|file| file.path == "nodes.parquet"));
    assert!(manifest
        .files
        .iter()
        .any(|file| file.path == "code_symbols.parquet"));
    let graph_manifest_files = manifest
        .files
        .iter()
        .filter(|file| file.path == "manifest.json")
        .collect::<Vec<_>>();
    assert_eq!(graph_manifest_files.len(), 1);
    let graph_manifest_bytes = fs::read(artifact_dir.join("manifest.json"))?;
    assert_eq!(
        graph_manifest_files[0].size_bytes,
        graph_manifest_bytes.len() as u64
    );
    assert_eq!(
        graph_manifest_files[0].sha256,
        sha256_hex(&graph_manifest_bytes)
    );

    let events = events.lock().expect("events lock").clone();
    let upload_graph_manifest = event_index(
        &events,
        "upload_file:silver/registry:crates-io/demo/0.1.0/builder-v1/manifest.json",
    )
    .context("upload graph manifest")?;
    let upload_manifest = event_index(
        &events,
        "upload_manifest:silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
    )
    .context("upload Silver root manifest")?;
    let validate = event_index(&events, "validate_manifest:").context("validate manifest")?;
    let register = event_index(&events, "register").context("register silver")?;
    assert!(
        upload_graph_manifest < upload_manifest
            && upload_manifest < validate
            && validate < register,
        "events must upload graph manifest, upload Silver root, validate bytes, then register: {events:?}"
    );
    Ok(())
}

#[test]
fn silver_bundle_legacy_root_without_current_schema_version_fails_closed() -> Result<()> {
    let manifest = worker_silver_manifest();
    let mut legacy = serde_json::to_value(&manifest)?;
    legacy
        .as_object_mut()
        .context("Silver manifest must serialize as an object")?
        .remove("schema_version");
    assert!(
        serde_json::from_value::<spur_context_service::medallion::SilverManifest>(legacy).is_err()
    );

    let mut wrong_version = serde_json::to_value(&manifest)?;
    wrong_version["schema_version"] = serde_json::json!(2);
    assert!(
        serde_json::from_value::<spur_context_service::medallion::SilverManifest>(wrong_version)
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn silver_bundle_download_rejects_duplicate_paths_before_member_io() -> Result<()> {
    let root = unique_temp_dir("worker-silver-duplicate-path")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manifest = worker_silver_manifest();
    let graph_manifest = manifest
        .files
        .iter()
        .find(|file| file.path == "manifest.json")
        .context("graph manifest fixture")?
        .clone();
    manifest.files.push(graph_manifest);
    let store = FakeSilverArtifactStore::with_manifest(events.clone(), manifest.clone());

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &root.join("artifact"),
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("duplicate"));
    assert_no_silver_member_downloads(&events);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_download_rejects_unconfined_paths_before_member_io() -> Result<()> {
    let root = unique_temp_dir("worker-silver-unconfined-path")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manifest = worker_silver_manifest();
    let unsafe_bytes = b"fixture:../escape.parquet";
    manifest
        .files
        .push(spur_context_service::medallion::SilverManifestFile {
            path: "../escape.parquet".to_owned(),
            size_bytes: unsafe_bytes.len() as u64,
            etag: "\"../escape.parquet\"".to_owned(),
            sha256: sha256_hex(unsafe_bytes),
        });
    let store = FakeSilverArtifactStore::with_manifest(events.clone(), manifest.clone());

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &root.join("artifact"),
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("escapes artifact root"));
    assert_no_silver_member_downloads(&events);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_download_rejects_missing_graph_manifest_before_member_io() -> Result<()> {
    let root = unique_temp_dir("worker-silver-missing-graph-manifest")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manifest = worker_silver_manifest();
    manifest.files.retain(|file| file.path != "manifest.json");
    let store = FakeSilverArtifactStore::with_manifest(events.clone(), manifest.clone());

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &root.join("artifact"),
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("manifest.json"));
    assert_no_silver_member_downloads(&events);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_download_rejects_corrupt_graph_manifest_bytes() -> Result<()> {
    let root = unique_temp_dir("worker-silver-corrupt-graph-manifest")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let store = FakeSilverArtifactStore::with_manifest(events, manifest.clone());
    store.set_download_bytes("manifest.json", b"corrupt graph manifest".to_vec());

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &root.join("artifact"),
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("manifest.json"));
    assert!(error.to_string().contains("mismatch"));
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_streaming_chunked_download_publishes_exact_member() -> Result<()> {
    let root = unique_temp_dir("worker-silver-stream-success")?;
    let artifact_dir = root.join("artifact");
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let store = FakeSilverArtifactStore::with_manifest(events, manifest.clone());
    let reads = store.set_download_chunks(
        "manifest.json",
        vec![
            b"fixture:".to_vec(),
            b"manifest".to_vec(),
            b".json".to_vec(),
        ],
    );

    download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &artifact_dir,
        &store,
    )
    .await?;

    assert_eq!(
        fs::read(artifact_dir.join("manifest.json"))?,
        b"fixture:manifest.json"
    );
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    assert!(silver_download_temp_siblings(&artifact_dir)?.is_empty());
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_streaming_short_member_leaves_no_final_or_temp() -> Result<()> {
    let root = unique_temp_dir("worker-silver-stream-short")?;
    let artifact_dir = root.join("artifact");
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let store = FakeSilverArtifactStore::with_manifest(events, manifest.clone());
    store.set_download_chunks(
        "manifest.json",
        vec![b"fixture:".to_vec(), b"manifest".to_vec()],
    );

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &artifact_dir,
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("size mismatch"));
    assert!(!artifact_dir.join("manifest.json").exists());
    assert!(silver_download_temp_siblings(&artifact_dir)?.is_empty());
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_streaming_long_member_rejects_early_without_final() -> Result<()> {
    let root = unique_temp_dir("worker-silver-stream-long")?;
    let artifact_dir = root.join("artifact");
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let store = FakeSilverArtifactStore::with_manifest(events, manifest.clone());
    let reads = store.set_download_chunks(
        "manifest.json",
        vec![
            b"fixture:manifest.json".to_vec(),
            b"!".to_vec(),
            b"must-not-be-read".to_vec(),
        ],
    );

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &artifact_dir,
        &store,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("size mismatch"));
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "the stream must stop as soon as the checked size exceeds the manifest"
    );
    assert!(!artifact_dir.join("manifest.json").exists());
    assert!(silver_download_temp_siblings(&artifact_dir)?.is_empty());
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn silver_bundle_streaming_corrupt_member_does_not_reuse_temp() -> Result<()> {
    let root = unique_temp_dir("worker-silver-stream-corrupt")?;
    let artifact_dir = root.join("artifact");
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let store = FakeSilverArtifactStore::with_manifest(events, manifest.clone());
    store.set_download_chunks(
        "manifest.json",
        vec![b"Fixture:".to_vec(), b"manifest.json".to_vec()],
    );
    store.seed_stale_temp_on_download("manifest.json", b"do-not-reuse".to_vec());

    let error = download_silver_artifact_from_manifest(
        "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json",
        &manifest,
        &artifact_dir,
        &store,
    )
    .await
    .unwrap_err();

    let stale_temp = artifact_dir.join(".manifest.json.spur-download-stale.tmp");
    assert!(error.to_string().contains("SHA-256 mismatch"));
    assert!(!artifact_dir.join("manifest.json").exists());
    assert_eq!(fs::read(&stale_temp)?, b"do-not-reuse");
    assert_eq!(silver_download_temp_siblings(&artifact_dir)?, [stale_temp]);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[test]
fn s3_silver_member_validation_and_download_do_not_collect_complete_bodies() -> Result<()> {
    let source = include_str!("../src/worker.rs");
    let implementation = source
        .split_once("impl SilverArtifactStore for S3SilverArtifactStore")
        .context("S3 Silver store implementation")?
        .1;
    let validate_manifest = implementation
        .split_once("async fn validate_manifest(")
        .context("S3 validate_manifest")?
        .1
        .split_once("async fn download_manifest(")
        .context("end of S3 validate_manifest")?
        .0;
    let member_validation = validate_manifest
        .split_once("for file in &manifest.files")
        .context("S3 member validation loop")?
        .1;
    let download_manifest_file = implementation
        .split_once("async fn download_manifest_file(")
        .context("S3 download_manifest_file")?
        .1
        .split_once("\n    }\n}")
        .context("end of S3 download_manifest_file")?
        .0;
    let atomic_stream_writer = source
        .split_once("pub async fn write_verified_silver_manifest_file_stream")
        .context("verified Silver stream writer")?
        .1
        .split_once("fn validate_downloaded_silver_manifest_file")
        .context("end of verified Silver stream writer")?
        .0;

    assert!(
        !member_validation.contains(".collect()"),
        "S3 Silver member validation must hash the SDK body incrementally"
    );
    assert!(
        !download_manifest_file.contains(".collect()"),
        "S3 Silver member download must not buffer the complete object"
    );
    assert!(
        download_manifest_file.contains("write_verified_silver_manifest_file_stream"),
        "S3 member download must use the verified streaming writer"
    );
    assert!(
        atomic_stream_writer.contains("rename("),
        "verified member download must publish with an atomic rename"
    );
    Ok(())
}

#[tokio::test]
async fn silver_persistence_includes_source_sidecar() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = unique_temp_dir("worker-source-sidecar")?;
    let artifact_dir = root.join("artifact");
    let artifact_dir_str = artifact_dir.display().to_string();
    let _env_guard = EnvGuard::set_all([
        (
            "SPUR_CONTEXT_WORKER_ARTIFACT_DIR",
            artifact_dir_str.as_str(),
        ),
        ("SPUR_GRAPH_BUILDER_VERSION", "builder-v1"),
        ("SPUR_CONTEXT_SILVER_BUCKET", "silver-test"),
    ]);
    let archive = source_sidecar_tarball(&root)?;
    let archive_bytes = fs::read(&archive).context("read source-sidecar archive")?;
    let bronze_store = FakeBronzeArchiveStore::default();
    let content_sha256 = bronze_store.seed_object(
        "bronze/registry:crates-io/demo/0.1.0/source.tar.gz",
        archive_bytes,
    );
    let bronze_registry = FakeBronzeRegistry::with_row(bronze_row(
        "http://127.0.0.1:1/unreachable.tar.gz",
        &content_sha256,
        fs::metadata(&archive)?.len(),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let silver_store = FakeSilverArtifactStore::new(events.clone());
    let silver_registry = FakeSilverRegistry::new(events.clone());
    let graph_builder = FakeGraphBuilder::writes_source_manifest();
    let mut env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");
    env.from_layer = JobFromLayer::Bronze;

    let prepared = prepare_job_with_services(
        &env,
        &StageTracker::new(),
        &bronze_registry,
        &bronze_store,
        &silver_registry,
        &silver_store,
        &graph_builder,
    )
    .await?;

    let sidecar = artifact_dir.join("source_files.parquet");
    assert!(
        sidecar.is_file(),
        "silver persistence must write source_files.parquet"
    );
    assert_eq!(
        read_source_sidecar(&sidecar)?,
        vec![
            SourceSidecarRow {
                file_path: "src/lib.rs".to_owned(),
                content_oid: git_blob_oid(SIDECAR_LIB_SOURCE.as_bytes())?,
                source_text: SIDECAR_LIB_SOURCE.to_owned(),
            },
            SourceSidecarRow {
                file_path: "src/util.rs".to_owned(),
                content_oid: git_blob_oid(SIDECAR_UTIL_SOURCE.as_bytes())?,
                source_text: SIDECAR_UTIL_SOURCE.to_owned(),
            },
        ]
    );

    let manifest = silver_store
        .manifest()
        .context("source sidecar manifest should be uploaded")?;
    let sidecar_manifest_file = manifest
        .files
        .iter()
        .find(|file| file.path == "source_files.parquet")
        .context("source sidecar should be listed in manifest")?;
    assert_eq!(
        sidecar_manifest_file.sha256,
        sha256_hex(&fs::read(&sidecar)?),
        "manifest must contain the SHA-256 of the generated sidecar bytes"
    );
    assert!(manifest.files.iter().all(|file| file.sha256.len() == 64));

    let events = events.lock().expect("events lock").clone();
    let upload_sidecar = event_index(
        &events,
        "upload_file:silver/registry:crates-io/demo/0.1.0/builder-v1/source_files.parquet",
    )
    .context("upload source sidecar")?;
    let upload_manifest = event_index(&events, "upload_manifest:").context("upload manifest")?;
    assert!(upload_sidecar < upload_manifest);

    drop(prepared);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn source_sidecar_rejects_missing_manifest_source_before_upload() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_SILVER_BUCKET", "silver-test")]);
    let root = unique_temp_dir("worker-source-sidecar-missing")?;
    let artifact_dir = root.join("artifact");
    let source_root = root.join("source");
    fs::create_dir_all(source_root.join("src"))?;
    write_silver_artifact_fixture(&artifact_dir)?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeSilverArtifactStore::new(events.clone());
    let registry = FakeSilverRegistry::new(events.clone());
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let error = persist_silver_graph_artifact(
        &env,
        &artifact_dir,
        &source_root,
        "bronze-sha256",
        "builder-v1",
        &store,
        &registry,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        WorkerError::Build(detail)
            if detail.contains("read referenced source") && detail.contains("src/lib.rs")
    ));
    assert!(
        events.lock().expect("events lock").is_empty(),
        "source validation must fail before any Silver file or manifest upload"
    );
    assert!(store.manifest().is_none());
    assert_eq!(registry.registers(), 0);
    assert!(!artifact_dir.join("source_files.parquet").exists());

    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn source_sidecar_rejects_non_utf8_manifest_source_before_upload() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_SILVER_BUCKET", "silver-test")]);
    let root = unique_temp_dir("worker-source-sidecar-non-utf8")?;
    let artifact_dir = root.join("artifact");
    let source_root = root.join("source");
    let source_bytes = [0xff, 0xfe, 0xfd];
    fs::create_dir_all(source_root.join("src"))?;
    fs::write(source_root.join("src/lib.rs"), source_bytes)?;
    write_silver_artifact_fixture(&artifact_dir)?;
    write_graph_file_manifest_fixture(
        &artifact_dir,
        &[("src/lib.rs", git_blob_oid(&source_bytes)?)],
    )?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeSilverArtifactStore::new(events.clone());
    let registry = FakeSilverRegistry::new(events.clone());
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let error = persist_silver_graph_artifact(
        &env,
        &artifact_dir,
        &source_root,
        "bronze-sha256",
        "builder-v1",
        &store,
        &registry,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        WorkerError::Build(detail)
            if detail.contains("read referenced source") && detail.contains("as UTF-8")
    ));
    assert!(
        events.lock().expect("events lock").is_empty(),
        "source validation must fail before any Silver file or manifest upload"
    );
    assert!(store.manifest().is_none());
    assert_eq!(registry.registers(), 0);
    assert!(!artifact_dir.join("source_files.parquet").exists());

    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn source_sidecar_rejects_mismatched_content_oid_before_upload() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_SILVER_BUCKET", "silver-test")]);
    let root = unique_temp_dir("worker-source-sidecar-content-mismatch")?;
    let artifact_dir = root.join("artifact");
    let source_root = root.join("source");
    fs::create_dir_all(source_root.join("src"))?;
    fs::write(source_root.join("src/lib.rs"), "pub fn demo() {}\n")?;
    write_silver_artifact_fixture(&artifact_dir)?;
    write_graph_file_manifest_fixture(
        &artifact_dir,
        &[(
            "src/lib.rs",
            "0000000000000000000000000000000000000000".to_owned(),
        )],
    )?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeSilverArtifactStore::new(events.clone());
    let registry = FakeSilverRegistry::new(events.clone());
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let error = persist_silver_graph_artifact(
        &env,
        &artifact_dir,
        &source_root,
        "bronze-sha256",
        "builder-v1",
        &store,
        &registry,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        WorkerError::Build(detail)
            if detail.contains("content_oid mismatch") && detail.contains("src/lib.rs")
    ));
    assert!(
        events.lock().expect("events lock").is_empty(),
        "content identity validation must fail before any Silver file or manifest upload"
    );
    assert!(store.manifest().is_none());
    assert_eq!(registry.registers(), 0);
    assert!(!artifact_dir.join("source_files.parquet").exists());

    fs::remove_dir_all(root).ok();
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn source_sidecar_rejects_symlink_escape_before_upload() -> Result<()> {
    use std::os::unix::fs::symlink;

    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_SILVER_BUCKET", "silver-test")]);
    let root = unique_temp_dir("worker-source-sidecar-symlink-escape")?;
    let artifact_dir = root.join("artifact");
    let source_root = root.join("source");
    let outside_source = root.join("outside.rs");
    let outside_bytes = b"pub fn escaped() {}\n";
    fs::create_dir_all(source_root.join("src"))?;
    fs::write(&outside_source, outside_bytes)?;
    symlink(&outside_source, source_root.join("src/lib.rs"))?;
    write_silver_artifact_fixture(&artifact_dir)?;
    write_graph_file_manifest_fixture(
        &artifact_dir,
        &[("src/lib.rs", git_blob_oid(outside_bytes)?)],
    )?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeSilverArtifactStore::new(events.clone());
    let registry = FakeSilverRegistry::new(events.clone());
    let env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");

    let error = persist_silver_graph_artifact(
        &env,
        &artifact_dir,
        &source_root,
        "bronze-sha256",
        "builder-v1",
        &store,
        &registry,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        WorkerError::Build(detail)
            if detail.contains("resolves outside source root") && detail.contains("src/lib.rs")
    ));
    assert!(
        events.lock().expect("events lock").is_empty(),
        "path confinement validation must fail before any Silver file or manifest upload"
    );
    assert!(store.manifest().is_none());
    assert_eq!(registry.registers(), 0);
    assert!(!artifact_dir.join("source_files.parquet").exists());

    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn reprocess_from_silver_downloads_registered_silver_without_fetch_or_build() -> Result<()> {
    let root = unique_temp_dir("worker-reprocess-silver")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let silver_store = FakeSilverArtifactStore::with_manifest(events.clone(), manifest.clone());
    let silver_registry = FakeSilverRegistry::with_row(events.clone(), silver_row(&manifest));
    let bronze_registry = FakeBronzeRegistry::default();
    let bronze_store = FakeBronzeArchiveStore::default();
    let graph_builder = FakeGraphBuilder::default();
    let mut env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");
    env.from_layer = JobFromLayer::Silver;

    let prepared = prepare_job_with_services(
        &env,
        &StageTracker::new(),
        &bronze_registry,
        &bronze_store,
        &silver_registry,
        &silver_store,
        &graph_builder,
    )
    .await?;

    assert_eq!(bronze_store.downloads(), 0);
    assert_eq!(bronze_store.uploads(), 0);
    assert_eq!(graph_builder.calls(), 0);
    assert_eq!(silver_registry.lookups(), 1);
    assert_eq!(silver_registry.registers(), 0);
    assert_eq!(prepared.source_path(), None);
    assert!(prepared.artifact_dir().join("nodes.parquet").is_file());
    assert_eq!(
        fs::read(prepared.artifact_dir().join("manifest.json"))?,
        b"fixture:manifest.json"
    );
    assert_eq!(prepared.artifact_manifest(), Some(&manifest));
    let lineage = prepared.lineage().context("lineage should be stamped")?;
    assert_eq!(lineage.bronze_content_sha256, "bronze-sha256");
    assert_eq!(lineage.silver_graph_content_hash, "graph-hash-123");
    assert_eq!(lineage.builder_version, "builder-v1");
    assert_eq!(lineage.translate_schema_version, "translate-v1");
    assert_eq!(
        lineage.embed_text_version,
        "v5-nomic-embed-text-v1.5-search-document"
    );

    drop(prepared);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn prepared_job_carries_serving_artifact_lineage() -> Result<()> {
    let root = unique_temp_dir("worker-serving-artifact-lineage")?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let manifest = worker_silver_manifest();
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let source_sidecar = manifest
        .files
        .iter()
        .find(|file| file.path == "source_files.parquet")
        .context("source sidecar should be present")?;
    let row = silver_row(&manifest);
    let silver_store = FakeSilverArtifactStore::with_manifest(events.clone(), manifest.clone());
    let silver_registry = FakeSilverRegistry::with_row(events, row.clone());
    let mut env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");
    env.from_layer = JobFromLayer::Silver;

    let prepared = prepare_job_with_services(
        &env,
        &StageTracker::new(),
        &FakeBronzeRegistry::default(),
        &FakeBronzeArchiveStore::default(),
        &silver_registry,
        &silver_store,
        &FakeGraphBuilder::default(),
    )
    .await?;

    let lineage = prepared.lineage().context("lineage should be stamped")?;
    assert_eq!(lineage.graph_prefix_uri, row.artifact_s3_prefix);
    assert_eq!(lineage.graph_manifest_uri, row.manifest_uri);
    assert_eq!(lineage.graph_manifest_sha256, sha256_hex(&manifest_bytes));
    assert_eq!(lineage.graph_manifest_bytes, manifest_bytes.len() as u64);
    assert_eq!(
        lineage.source_sidecar_uri,
        format!("{}{}", lineage.graph_prefix_uri, source_sidecar.path)
    );
    assert_eq!(lineage.source_sidecar_sha256, source_sidecar.sha256);
    assert_eq!(lineage.source_sidecar_bytes, source_sidecar.size_bytes);

    drop(prepared);
    fs::remove_dir_all(root).ok();
    Ok(())
}

#[tokio::test]
async fn reprocess_from_bronze_restores_bronze_and_persists_rebuilt_silver() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = unique_temp_dir("worker-reprocess-bronze")?;
    let artifact_dir = root.join("artifact");
    let artifact_dir_str = artifact_dir.display().to_string();
    let _env_guard = EnvGuard::set_all([
        (
            "SPUR_CONTEXT_WORKER_ARTIFACT_DIR",
            artifact_dir_str.as_str(),
        ),
        ("SPUR_GRAPH_BUILDER_VERSION", "builder-v1"),
        ("SPUR_CONTEXT_SILVER_BUCKET", "silver-test"),
    ]);
    let archive = demo_tarball(&root)?;
    let archive_bytes = fs::read(&archive).context("read archive")?;
    let bronze_store = FakeBronzeArchiveStore::default();
    let content_sha256 = bronze_store.seed_object(
        "bronze/registry:crates-io/demo/0.1.0/source.tar.gz",
        archive_bytes,
    );
    let bronze_registry = FakeBronzeRegistry::with_row(bronze_row(
        "http://127.0.0.1:1/unreachable.tar.gz",
        &content_sha256,
        fs::metadata(&archive)?.len(),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let silver_store = FakeSilverArtifactStore::new(events.clone());
    let silver_registry = FakeSilverRegistry::new(events.clone());
    let graph_builder = FakeGraphBuilder::writes_fixture();
    let mut env = demo_job_env("http://127.0.0.1:1/unreachable.tar.gz");
    env.from_layer = JobFromLayer::Bronze;

    let prepared = prepare_job_with_services(
        &env,
        &StageTracker::new(),
        &bronze_registry,
        &bronze_store,
        &silver_registry,
        &silver_store,
        &graph_builder,
    )
    .await?;

    assert_eq!(bronze_store.downloads(), 1);
    assert_eq!(bronze_store.uploads(), 0);
    assert_eq!(bronze_registry.registers(), 0);
    assert_eq!(graph_builder.calls(), 1);
    let rows = silver_registry.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].bronze_content_sha256, content_sha256);
    assert_eq!(rows[0].builder_version, "builder-v1");
    assert!(prepared.source_path().is_some());
    assert!(prepared.artifact_dir().join("nodes.parquet").is_file());
    assert!(prepared.artifact_manifest().is_some());
    Ok(())
}

#[test]
fn fetch_source_rejects_localhost_url_without_escape_hatch() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "0")]);
    let root = unique_temp_dir("worker-localhost-rejected")?;

    let err = fetch_source(
        "https://127.0.0.1/archive.tar.gz",
        "tarball",
        "0.1.0",
        &root.join("fetch"),
    )
    .unwrap_err();

    match err {
        WorkerError::Fetch(detail) => {
            assert!(detail.contains("source_url abuse re-validation failed"));
            assert!(detail.contains("localhost"));
        }
        other => panic!("expected fetch error, got {other:?}"),
    }
    Ok(())
}

#[test]
fn fetch_source_rejects_extracted_source_over_configured_cap() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([
        ("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1"),
        ("SPUR_CONTEXT_MAX_SOURCE_BYTES", "128"),
    ]);
    let root = unique_temp_dir("worker-source-size-cap")?;
    let fixture = root.join("fixture").join("demo-0.1.0");
    fs::create_dir_all(fixture.join("src")).context("create fixture")?;
    fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .context("write manifest")?;
    fs::write(fixture.join("src/lib.rs"), "x".repeat(1024)).context("write large lib")?;
    let archive = root.join("demo.tar.gz");
    create_tarball(root.join("fixture").as_path(), &archive)?;
    let source_url = serve_once(fs::read(&archive).context("read archive")?);

    let err = fetch_source(&source_url, "tarball", "0.1.0", &root.join("fetch")).unwrap_err();

    match err {
        WorkerError::Fetch(detail) => {
            assert!(detail.contains("source tree exceeded size cap"));
            assert!(detail.contains("128"));
        }
        other => panic!("expected fetch error, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn spot_interruption_handler_writes_checkpoint() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = unique_temp_dir("worker-checkpoint")?;
    let _env_guard = EnvGuard::set_all([
        ("SPUR_CONTEXT_WORKER_CHECKPOINT_DIR", root.to_str().unwrap()),
        ("SPUR_CONTEXT_WORKER_SKIP_SFN", "1"),
    ]);
    let env = JobEnv {
        task_token: "task-token".to_owned(),
        job_id: "job-spot".to_owned(),
        package: "demo".to_owned(),
        revision: "deadbeef".to_owned(),
        source: "git:custom".to_owned(),
        source_url: "https://github.com/example/demo.git".to_owned(),
        source_kind: "git".to_owned(),
        catalog_dsn: "sqlite:/tmp/catalog.sqlite".to_owned(),
        from_layer: JobFromLayer::Source,
    };

    handle_spot_interruption(&env, "translate").await?;

    let checkpoint_path = root.join("jobs").join("job-spot").join("checkpoint.json");
    let checkpoint: Value =
        serde_json::from_str(&fs::read_to_string(&checkpoint_path).context("read checkpoint")?)
            .context("parse checkpoint")?;

    assert_eq!(checkpoint["job_id"], "job-spot");
    assert_eq!(checkpoint["last_completed_stage"], "translate");
    assert_eq!(checkpoint["error"], "spot_interrupted");
    assert_eq!(checkpoint["package"], "demo");
    assert_eq!(checkpoint["revision"], "deadbeef");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_updates_job_stages_around_fetch_build_and_translate() -> Result<()> {
    let store = Arc::new(FakeJobStore::default());
    store.seed_job("job-stages");
    let tracker = StageTracker::with_job_store("job-stages", store.clone());

    tokio::task::spawn_blocking(move || {
        tracker.set("fetch_source");
        tracker.set("build_graph");
        tracker.set("translate");
    })
    .await
    .context("stage reporter task")?;

    assert_eq!(
        store.stage_updates(),
        ["fetch_source", "build_graph", "translate"]
    );
    let record = store
        .lookup_job("job-stages")
        .await?
        .context("job should exist")?;
    assert_eq!(record.status, JobStatus::Running);
    assert_eq!(record.stage.as_deref(), Some("translate"));
    Ok(())
}

#[tokio::test]
async fn lambda_worker_records_failed_job_without_sfn_callback() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1")]);
    let store = Arc::new(FakeJobStore::default());
    store.seed_job("job-lambda-fail");
    let leases = Arc::new(FakeCatalogLeaseStore::lost());
    let env = JobEnv {
        task_token: String::new(),
        job_id: "job-lambda-fail".to_owned(),
        package: "demo".to_owned(),
        revision: "deadbeef".to_owned(),
        source: "git:custom".to_owned(),
        source_url: "https://github.com/example/demo".to_owned(),
        source_kind: "unsupported".to_owned(),
        catalog_dsn: "sqlite:/tmp/catalog.sqlite".to_owned(),
        from_layer: JobFromLayer::Source,
    };

    let err = run_job_and_record_with_services(&env, store.clone(), leases)
        .await
        .unwrap_err();

    assert!(matches!(err, WorkerError::Fetch(_)));
    let record = store
        .lookup_job("job-lambda-fail")
        .await?
        .context("job should exist")?;
    assert_eq!(record.status, JobStatus::Failed);
    assert_eq!(record.error_code.as_deref(), Some("fetch"));
    assert!(record
        .error_detail
        .as_deref()
        .is_some_and(|detail| detail.contains("unsupported SOURCE_KIND")));
    Ok(())
}

#[tokio::test]
async fn catalog_lease_blocks_upload_when_token_is_lost() -> Result<()> {
    let leases = FakeCatalogLeaseStore::lost();
    let lease = CatalogLease {
        catalog_uri: "s3://bucket/catalog.ducklake".to_owned(),
        owner_job_id: "job-lost".to_owned(),
        lease_token: "token-1".to_owned(),
        expires_at_unix_secs: 1_900_000_000,
        fencing_counter: 7,
    };
    let upload_calls = AtomicUsize::new(0);

    let err = upload_with_owned_catalog_lease(&leases, &lease, || {
        upload_calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(()) }
    })
    .await
    .unwrap_err();

    assert!(err.to_string().contains("lease lost"));
    assert_eq!(upload_calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn catalog_lease_acquisition_retries_contention_until_available() -> Result<()> {
    let leases = FakeCatalogLeaseStore::contended(3);

    let lease = acquire_catalog_lease_with_retry(
        &leases,
        "postgres:catalog",
        "job-contended",
        Duration::from_secs(1),
    )
    .await?;

    assert_eq!(lease.owner_job_id, "job-contended");
    assert_eq!(leases.acquire_attempts(), 4);
    Ok(())
}

#[tokio::test]
async fn catalog_download_upload_uses_conditional_s3_write_metadata() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let server = FakeS3Server::start("catalog-before", "\"etag-before\"", "version-before")?;
    let _env = EnvGuard::set_all([
        ("AWS_ENDPOINT_URL_S3", server.endpoint.as_str()),
        ("AWS_ACCESS_KEY_ID", "test"),
        ("AWS_SECRET_ACCESS_KEY", "test"),
        ("AWS_REGION", "us-east-1"),
    ]);

    let download = CatalogDownload::fetch("s3://catalog-bucket/catalog.ducklake")
        .await?
        .context("s3 catalog should be downloaded")?;
    fs::write(download.local_path(), "catalog-after").context("mutate local catalog")?;
    download.upload().await?;

    let put = server.put_request();
    assert!(put.contains("if-match: \"etag-before\""));
    assert!(!put.contains("if-none-match"));
    assert!(put.contains("/catalog-bucket/catalog.ducklake"));
    Ok(())
}

#[test]
fn worker_image_builds_spur_cli_without_embedding_features() -> Result<()> {
    let deploy_script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/spur-context-service/deploy.sh");
    let deploy_script = fs::read_to_string(&deploy_script)
        .with_context(|| format!("read {}", deploy_script.display()))?;

    assert!(
        deploy_script.contains(
            "build -p spur-cli --release --no-default-features --features worker-no-embed,duckdb-bundled"
        ),
        "worker image spur CLI build must disable default embedding features"
    );
    Ok(())
}

#[test]
fn build_graph_invokes_spur_with_progress_visible() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = unique_temp_dir("worker-build-graph")?;
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).context("create fake bin dir")?;
    let record = root.join("spur-record.txt");
    let fake_spur = bin_dir.join("spur");
    fs::write(
        &fake_spur,
        format!(
            "#!/usr/bin/env bash\n{{\n  printf 'args=%s\\n' \"$*\"\n  printf 'skip_embeddings=%s\\n' \"${{SPUR_GRAPH_SKIP_SECTION_EMBEDDINGS-}}\"\n  printf 'skip_code_symbol_embeddings=%s\\n' \"${{SPUR_GRAPH_SKIP_CODE_SYMBOL_EMBEDDINGS-}}\"\n}} > {}\n",
            record.display()
        ),
    )
    .context("write fake spur")?;
    let mut perms = fs::metadata(&fake_spur)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&fake_spur, perms).context("chmod fake spur")?;

    let source = root.join("source");
    fs::create_dir_all(&source).context("create source")?;
    let artifact = root.join("artifact");
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _env = EnvGuard::set_all([("PATH", path.as_str())]);

    build_graph(&source, &artifact)?;

    let record = fs::read_to_string(&record).context("read fake spur record")?;
    assert!(record.contains("args=graph build"));
    assert!(record.contains("--no-analyst"));
    assert!(!record.contains("--quiet"));
    assert!(record.contains("skip_embeddings=1"));
    assert!(record.contains("skip_code_symbol_embeddings=1"));
    Ok(())
}

#[test]
fn build_graph_kills_spur_after_configured_timeout() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = unique_temp_dir("worker-build-graph-timeout")?;
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).context("create fake bin dir")?;
    let fake_spur = bin_dir.join("spur");
    fs::write(&fake_spur, "#!/usr/bin/env bash\nsleep 2\nexit 0\n").context("write fake spur")?;
    let mut perms = fs::metadata(&fake_spur)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&fake_spur, perms).context("chmod fake spur")?;

    let source = root.join("source");
    fs::create_dir_all(&source).context("create source")?;
    let artifact = root.join("artifact");
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _env = EnvGuard::set_all([
        ("PATH", path.as_str()),
        ("SPUR_CONTEXT_MAX_BUILD_SECONDS", "1"),
    ]);

    let err = build_graph(&source, &artifact).unwrap_err();

    match err {
        WorkerError::Build(detail) => {
            assert!(detail.contains("timed out"));
            assert!(detail.contains("1s"));
        }
        other => panic!("expected build error, got {other:?}"),
    }
    Ok(())
}

#[test]
#[ignore = "requires git on PATH; run with: scripts/spur-cargo test -p spur-context-service --features worker --test worker_test fetch_source_clones_git_repo -- --ignored"]
fn fetch_source_clones_git_repo() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_SKIP_ABUSE_REVALIDATE", "1")]);
    let root = unique_temp_dir("worker-git")?;
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).context("create repo")?;
    fs::write(repo.join("src/lib.rs"), "pub fn demo() {}\n").context("write lib")?;
    run_git(&root, ["init", "repo"])?;
    run_git(&repo, ["add", "."])?;
    run_git(&repo, ["commit", "-m", "initial"])?;
    let revision = git_output(&repo, ["rev-parse", "HEAD"])?;

    let fetched = fetch_source(
        repo.to_str().context("repo path is utf-8")?,
        "git",
        revision.trim(),
        &root.join("fetch"),
    )?;

    assert!(fetched.join("src/lib.rs").is_file());
    Ok(())
}

#[tokio::test]
#[ignore = "requires AWS credentials and SPUR_CONTEXT_WORKER_AWS_CHECKPOINT_URI=s3://bucket/prefix/checkpoint.json; run with --ignored"]
async fn spot_interruption_handler_writes_checkpoint_to_s3() -> Result<()> {
    let _guard = ENV_LOCK.lock().unwrap();
    let uri = std::env::var("SPUR_CONTEXT_WORKER_AWS_CHECKPOINT_URI")
        .context("set SPUR_CONTEXT_WORKER_AWS_CHECKPOINT_URI")?;
    let _env_guard = EnvGuard::set_all([("SPUR_CONTEXT_WORKER_CHECKPOINT_URI", uri.as_str())]);
    let env = JobEnv {
        task_token: "task-token".to_owned(),
        job_id: "job-spot-aws".to_owned(),
        package: "demo".to_owned(),
        revision: "deadbeef".to_owned(),
        source: "git:custom".to_owned(),
        source_url: "https://github.com/example/demo.git".to_owned(),
        source_kind: "git".to_owned(),
        catalog_dsn: "sqlite:/tmp/catalog.sqlite".to_owned(),
        from_layer: JobFromLayer::Source,
    };

    handle_spot_interruption(&env, "fetch").await?;
    Ok(())
}

#[tokio::test]
async fn drainer_is_noop_when_store_has_no_queued_jobs() -> Result<()> {
    // The worker-test FakeJobStore does not implement queue accounting, so the
    // default `list_queued_jobs` returns empty. The drainer must not crash and
    // must return an empty summary.
    use spur_context_service::drainer;

    let store = Arc::new(FakeJobStore::default());
    let starter = NoopStarter;
    let config = QueueConfig::default();

    let summary = drainer::drain_queued_jobs_with_services(&*store, &starter, config, 0).await;

    assert_eq!(summary.dispatched, 0);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.failed, 0);
    Ok(())
}

/// Fake `IndexExecutionStarter` that never actually starts an execution.
struct NoopStarter;

impl IndexExecutionStarter for NoopStarter {
    fn start_execution<'a>(
        &'a self,
        _request: IndexExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, McpHandlerError>> + Send + 'a>>
    {
        Box::pin(async { Ok("arn:fake".to_owned()) })
    }
}

fn create_tarball(source_dir: &Path, archive: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-czf")
        .arg(archive)
        .arg("-C")
        .arg(source_dir)
        .arg(".")
        .status()
        .context("run tar")?;
    anyhow::ensure!(status.success(), "tar exited with {status}");
    Ok(())
}

fn serve_once(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock http");
    let addr = listener.local_addr().expect("mock http addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept mock http");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/gzip\r\n\r\n",
            body.len()
        );
        stream
            .write_all(headers.as_bytes())
            .expect("write mock headers");
        stream.write_all(&body).expect("write mock body");
    });
    format!("http://{addr}/demo.tar.gz")
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .context("run git")?;
    anyhow::ensure!(status.success(), "git exited with {status}");
    Ok(())
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .context("run git")?;
    anyhow::ensure!(output.status.success(), "git exited with {}", output.status);
    String::from_utf8(output.stdout).context("git output utf-8")
}

fn git_blob_oid(bytes: &[u8]) -> Result<String> {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn git hash-object")?;
    child
        .stdin
        .as_mut()
        .context("git hash-object stdin")?
        .write_all(bytes)
        .context("write git hash-object stdin")?;
    let output = child
        .wait_with_output()
        .context("wait for git hash-object")?;
    anyhow::ensure!(
        output.status.success(),
        "git hash-object exited with {}",
        output.status
    );
    Ok(String::from_utf8(output.stdout)
        .context("git hash-object output UTF-8")?
        .trim_end()
        .to_owned())
}

fn unique_temp_dir(name: &str) -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "spur-context-service-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("clock before unix epoch")?
            .as_nanos()
    ));
    fs::create_dir_all(&path).context("create temp dir")?;
    Ok(path)
}

fn demo_tarball(root: &Path) -> Result<PathBuf> {
    let fixture = root.join("fixture").join("demo-0.1.0");
    fs::create_dir_all(fixture.join("src")).context("create fixture")?;
    fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .context("write manifest")?;
    fs::write(fixture.join("src/lib.rs"), "pub fn demo() {}\n").context("write lib")?;
    let archive = root.join("demo.tar.gz");
    create_tarball(root.join("fixture").as_path(), &archive)?;
    Ok(archive)
}

fn source_sidecar_tarball(root: &Path) -> Result<PathBuf> {
    let fixture = root.join("sidecar-fixture").join("demo-0.1.0");
    fs::create_dir_all(fixture.join("src")).context("create sidecar fixture")?;
    fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .context("write sidecar manifest")?;
    fs::write(fixture.join("src/lib.rs"), SIDECAR_LIB_SOURCE).context("write sidecar lib")?;
    fs::write(fixture.join("src/util.rs"), SIDECAR_UTIL_SOURCE).context("write sidecar util")?;
    let archive = root.join("source-sidecar-demo.tar.gz");
    create_tarball(root.join("sidecar-fixture").as_path(), &archive)?;
    Ok(archive)
}

fn demo_job_env(source_url: &str) -> JobEnv {
    JobEnv {
        task_token: "task-token".to_owned(),
        job_id: "job-bronze".to_owned(),
        package: "demo".to_owned(),
        revision: "0.1.0".to_owned(),
        source: "registry:crates-io".to_owned(),
        source_url: source_url.to_owned(),
        source_kind: "tarball".to_owned(),
        catalog_dsn: "sqlite:/tmp/catalog.sqlite".to_owned(),
        from_layer: JobFromLayer::Source,
    }
}

fn write_silver_artifact_fixture(artifact_dir: &Path) -> Result<()> {
    fs::create_dir_all(artifact_dir).context("create artifact dir")?;
    for relative in [
        "nodes.parquet",
        "edges.parquet",
        "edges_unresolved.parquet",
        "files.parquet",
        "file_manifests.parquet",
        "code_symbols.parquet",
        "sections.parquet",
    ] {
        fs::write(artifact_dir.join(relative), format!("fixture:{relative}"))
            .with_context(|| format!("write {relative}"))?;
    }
    fs::write(
        artifact_dir.join("manifest.json"),
        serde_json::json!({
            "graph_index_version": "4",
            "schema_version": "spur-graph-schema-v9",
            "manifest_version": "manifest-v1",
            "graph_content_hash": "graph-hash-123",
            "indexed_commit_oid": null,
            "extractor_version": "extractor-v1",
            "complete": true,
            "row_counts": {
                "nodes": 7,
                "edges": 11,
                "edges_by_dst": null,
                "edges_unresolved": 2,
                "files": 3,
                "file_manifests": 3,
                "tombstones": 0,
                "commits": 0,
                "symbol_snapshots": 0,
                "temporal_edges": 0,
                "diagnostics": 0
            },
            "sidecar_complete": true,
            "sidecar_row_counts": {
                "section_bodies": 4,
                "code_symbols": 5
            },
            "parquet_writer": {
                "compression": "zstd-3",
                "row_group_size": 16384
            },
            "edges_by_dst_present": false,
            "temporal_shards": []
        })
        .to_string(),
    )
    .context("write graph artifact manifest")?;
    write_graph_file_manifest_fixture(
        artifact_dir,
        &[("src/lib.rs", git_blob_oid(b"pub fn demo() {}\n")?)],
    )?;
    Ok(())
}

fn write_source_manifest_fixture(artifact_dir: &Path) -> Result<()> {
    write_graph_file_manifest_fixture(
        artifact_dir,
        &[
            ("src/lib.rs", git_blob_oid(SIDECAR_LIB_SOURCE.as_bytes())?),
            ("src/util.rs", git_blob_oid(SIDECAR_UTIL_SOURCE.as_bytes())?),
        ],
    )
}

fn write_graph_file_manifest_fixture(artifact_dir: &Path, rows: &[(&str, String)]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("content_oid", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from_iter_values(
                rows.iter().map(|(path, _)| *path),
            )),
            Arc::new(StringArray::from_iter_values(
                rows.iter().map(|(_, content_oid)| content_oid.as_str()),
            )),
        ],
    )?;
    let file = fs::File::create(artifact_dir.join("file_manifests.parquet"))?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct SourceSidecarRow {
    file_path: String,
    content_oid: String,
    source_text: String,
}

fn read_source_sidecar(path: &Path) -> Result<Vec<SourceSidecarRow>> {
    let mut rows = Vec::new();
    let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path)?)?.build()?;
    for batch in reader {
        let batch = batch?;
        let file_path = batch
            .column_by_name("file_path")
            .context("source sidecar file_path column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("source sidecar file_path must be UTF-8")?;
        let content_oid = batch
            .column_by_name("content_oid")
            .context("source sidecar content_oid column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("source sidecar content_oid must be UTF-8")?;
        let source_text = batch
            .column_by_name("source_text")
            .context("source sidecar source_text column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("source sidecar source_text must be UTF-8")?;
        for row in 0..batch.num_rows() {
            rows.push(SourceSidecarRow {
                file_path: file_path.value(row).to_owned(),
                content_oid: content_oid.value(row).to_owned(),
                source_text: source_text.value(row).to_owned(),
            });
        }
    }
    Ok(rows)
}

fn worker_silver_manifest() -> spur_context_service::medallion::SilverManifest {
    spur_context_service::medallion::SilverManifest {
        schema_hash: "sha256:test-schema".to_owned(),
        files: [
            "manifest.json",
            "nodes.parquet",
            "edges.parquet",
            "edges_unresolved.parquet",
            "files.parquet",
            "file_manifests.parquet",
            "source_files.parquet",
            "code_symbols.parquet",
            "sections.parquet",
        ]
        .into_iter()
        .map(|path| {
            let bytes = format!("fixture:{path}");
            spur_context_service::medallion::SilverManifestFile {
                path: path.to_owned(),
                size_bytes: bytes.len() as u64,
                etag: format!("\"{path}\""),
                sha256: sha256_hex(bytes.as_bytes()),
            }
        })
        .collect(),
    }
}

fn assert_no_silver_member_downloads(events: &Arc<Mutex<Vec<String>>>) {
    let events = events.lock().expect("events lock");
    assert!(
        events
            .iter()
            .all(|event| !event.starts_with("download_file:")),
        "invalid Silver roots must fail before member I/O: {events:?}"
    );
}

fn silver_download_temp_siblings(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".spur-download-") && name.ends_with(".tmp"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn event_index(events: &[String], prefix: &str) -> Option<usize> {
    events
        .iter()
        .position(|event| event == prefix || event.starts_with(prefix))
}

fn bronze_row(source_url: &str, content_sha256: &str, bytes: u64) -> BronzeRawSource {
    BronzeRawSource {
        source: "registry:crates-io".to_owned(),
        package: "demo".to_owned(),
        version: "0.1.0".to_owned(),
        revision_kind: "semver".to_owned(),
        semver_major: Some(0),
        semver_minor: Some(1),
        semver_patch: Some(0),
        source_kind: "tarball".to_owned(),
        source_url: source_url.to_owned(),
        s3_uri: "s3://bronze-test/bronze/registry:crates-io/demo/0.1.0/source.tar.gz".to_owned(),
        content_sha256: content_sha256.to_owned(),
        bytes,
        fetched_at: 1_800_000_000,
        fetch_status: "success".to_owned(),
    }
}

fn silver_row(manifest: &spur_context_service::medallion::SilverManifest) -> SilverGraphArtifact {
    SilverGraphArtifact {
        source: "registry:crates-io".to_owned(),
        package: "demo".to_owned(),
        version: "0.1.0".to_owned(),
        revision_kind: "semver".to_owned(),
        semver_major: Some(0),
        semver_minor: Some(1),
        semver_patch: Some(0),
        bronze_content_sha256: "bronze-sha256".to_owned(),
        builder_version: "builder-v1".to_owned(),
        graph_content_hash: "graph-hash-123".to_owned(),
        artifact_s3_prefix: "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/"
            .to_owned(),
        manifest_uri:
            "s3://silver-test/silver/registry:crates-io/demo/0.1.0/builder-v1/silver-manifest.json"
                .to_owned(),
        manifest_schema_hash: manifest.schema_hash.clone(),
        node_count: 7,
        edge_count: 11,
        file_count: 3,
        embedding_count: 5,
        built_at: 1_800_000_000,
        build_status: "success".to_owned(),
    }
}

#[derive(Default)]
struct FakeGraphBuilder {
    calls: AtomicUsize,
    write_fixture: bool,
    write_source_manifest: bool,
}

impl FakeGraphBuilder {
    fn writes_fixture() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            write_fixture: true,
            write_source_manifest: false,
        }
    }

    fn writes_source_manifest() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            write_fixture: true,
            write_source_manifest: true,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl GraphArtifactBuilder for FakeGraphBuilder {
    async fn build(
        &self,
        _source_path: &Path,
        artifact_base: &Path,
    ) -> Result<PathBuf, WorkerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.write_fixture {
            write_silver_artifact_fixture(artifact_base)
                .map_err(|error| WorkerError::Build(format!("write fake artifact: {error:#}")))?;
            if self.write_source_manifest {
                write_source_manifest_fixture(artifact_base).map_err(|error| {
                    WorkerError::Build(format!("write fake source manifest: {error:#}"))
                })?;
            }
        }
        Ok(artifact_base.to_path_buf())
    }
}

struct FakeSilverArtifactStore {
    events: Arc<Mutex<Vec<String>>>,
    manifest: Mutex<Option<spur_context_service::medallion::SilverManifest>>,
    download_chunks: Mutex<HashMap<String, Vec<Vec<u8>>>>,
    download_reads: Mutex<HashMap<String, Arc<AtomicUsize>>>,
    stale_temps: Mutex<HashMap<String, Vec<u8>>>,
}

struct ChunkedReader {
    chunks: VecDeque<Vec<u8>>,
    offset: usize,
    reads: Option<Arc<AtomicUsize>>,
}

impl ChunkedReader {
    fn new(chunks: Vec<Vec<u8>>, reads: Option<Arc<AtomicUsize>>) -> Self {
        Self {
            chunks: chunks.into(),
            offset: 0,
            reads,
        }
    }
}

impl AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let Some(chunk_len) = self.chunks.front().map(Vec::len) else {
            return Poll::Ready(Ok(()));
        };
        if self.offset == 0 {
            if let Some(reads) = &self.reads {
                reads.fetch_add(1, Ordering::SeqCst);
            }
        }
        let start = self.offset;
        let end = start + buf.remaining().min(chunk_len - start);
        buf.put_slice(&self.chunks.front().expect("chunk exists")[start..end]);
        self.offset = end;
        if self.offset == chunk_len {
            self.chunks.pop_front();
            self.offset = 0;
        }
        Poll::Ready(Ok(()))
    }
}

impl FakeSilverArtifactStore {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            manifest: Mutex::new(None),
            download_chunks: Mutex::new(HashMap::new()),
            download_reads: Mutex::new(HashMap::new()),
            stale_temps: Mutex::new(HashMap::new()),
        }
    }

    fn with_manifest(
        events: Arc<Mutex<Vec<String>>>,
        manifest: spur_context_service::medallion::SilverManifest,
    ) -> Self {
        let download_chunks = manifest
            .files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    vec![format!("fixture:{}", file.path).into_bytes()],
                )
            })
            .collect();
        Self {
            events,
            manifest: Mutex::new(Some(manifest)),
            download_chunks: Mutex::new(download_chunks),
            download_reads: Mutex::new(HashMap::new()),
            stale_temps: Mutex::new(HashMap::new()),
        }
    }

    fn manifest(&self) -> Option<spur_context_service::medallion::SilverManifest> {
        self.manifest.lock().expect("silver manifest lock").clone()
    }

    fn set_download_bytes(&self, relative_path: &str, bytes: Vec<u8>) {
        self.set_download_chunks(relative_path, vec![bytes]);
    }

    fn set_download_chunks(&self, relative_path: &str, chunks: Vec<Vec<u8>>) -> Arc<AtomicUsize> {
        self.download_chunks
            .lock()
            .expect("silver download chunks lock")
            .insert(relative_path.to_owned(), chunks);
        let reads = Arc::new(AtomicUsize::new(0));
        self.download_reads
            .lock()
            .expect("silver download reads lock")
            .insert(relative_path.to_owned(), reads.clone());
        reads
    }

    fn seed_stale_temp_on_download(&self, relative_path: &str, bytes: Vec<u8>) {
        self.stale_temps
            .lock()
            .expect("silver stale temps lock")
            .insert(relative_path.to_owned(), bytes);
    }
}

#[async_trait]
impl SilverArtifactStore for FakeSilverArtifactStore {
    async fn upload_file(&self, key: &str, path: &Path) -> Result<SilverUploadedFile, WorkerError> {
        let bytes = fs::read(path)
            .map_err(|error| WorkerError::Build(format!("fake silver read: {error}")))?;
        let size_bytes = bytes.len() as u64;
        let relative_path = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| WorkerError::Build("fake silver path has no filename".to_owned()))?;
        self.download_chunks
            .lock()
            .expect("silver download chunks lock")
            .insert(relative_path.to_owned(), vec![bytes]);
        self.events
            .lock()
            .expect("events lock")
            .push(format!("upload_file:{key}"));
        Ok(SilverUploadedFile {
            s3_uri: format!("s3://silver-test/{key}"),
            etag: format!("\"etag:{key}\""),
            size_bytes,
        })
    }

    async fn upload_manifest(
        &self,
        key: &str,
        manifest: &spur_context_service::medallion::SilverManifest,
    ) -> Result<String, WorkerError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("upload_manifest:{key}"));
        *self.manifest.lock().expect("silver manifest lock") = Some(manifest.clone());
        Ok(format!("s3://silver-test/{key}"))
    }

    async fn validate_manifest(
        &self,
        manifest_uri: &str,
        manifest: &spur_context_service::medallion::SilverManifest,
    ) -> Result<(), WorkerError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("validate_manifest:{manifest_uri}"));
        let uploaded = self.manifest.lock().expect("silver manifest lock").clone();
        if uploaded.as_ref() != Some(manifest) {
            return Err(WorkerError::Build("manifest not uploaded".to_owned()));
        }
        Ok(())
    }

    async fn download_manifest(
        &self,
        manifest_uri: &str,
    ) -> Result<spur_context_service::medallion::SilverManifest, WorkerError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("download_manifest:{manifest_uri}"));
        self.manifest()
            .ok_or_else(|| WorkerError::Build("missing fake silver manifest".to_owned()))
    }

    async fn download_manifest_file(
        &self,
        manifest_uri: &str,
        file: &spur_context_service::medallion::SilverManifestFile,
        dest: &Path,
    ) -> Result<(), WorkerError> {
        let relative_path = &file.path;
        self.events
            .lock()
            .expect("events lock")
            .push(format!("download_file:{manifest_uri}:{relative_path}"));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| WorkerError::Build(format!("fake mkdir: {error}")))?;
        }
        if let Some(bytes) = self
            .stale_temps
            .lock()
            .expect("silver stale temps lock")
            .get(relative_path)
            .cloned()
        {
            let file_name = dest
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| WorkerError::Build("fake destination has no filename".to_owned()))?;
            let stale_temp = dest.with_file_name(format!(".{file_name}.spur-download-stale.tmp"));
            fs::write(stale_temp, bytes)
                .map_err(|error| WorkerError::Build(format!("fake stale temp: {error}")))?;
        }
        let chunks = self
            .download_chunks
            .lock()
            .expect("silver download chunks lock")
            .get(relative_path)
            .cloned()
            .ok_or_else(|| {
                WorkerError::Build(format!("missing fake silver file `{relative_path}`"))
            })?;
        let reads = self
            .download_reads
            .lock()
            .expect("silver download reads lock")
            .get(relative_path)
            .cloned();
        write_verified_silver_manifest_file_stream(ChunkedReader::new(chunks, reads), file, dest)
            .await
    }
}

struct FakeSilverRegistry {
    events: Arc<Mutex<Vec<String>>>,
    rows: Mutex<Vec<SilverGraphArtifact>>,
    lookups: AtomicUsize,
    registers: AtomicUsize,
}

impl FakeSilverRegistry {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            rows: Mutex::new(Vec::new()),
            lookups: AtomicUsize::new(0),
            registers: AtomicUsize::new(0),
        }
    }

    fn with_row(events: Arc<Mutex<Vec<String>>>, row: SilverGraphArtifact) -> Self {
        Self {
            events,
            rows: Mutex::new(vec![row]),
            lookups: AtomicUsize::new(0),
            registers: AtomicUsize::new(0),
        }
    }

    fn rows(&self) -> Vec<SilverGraphArtifact> {
        self.rows.lock().expect("silver registry lock").clone()
    }

    fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }

    fn registers(&self) -> usize {
        self.registers.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SilverGraphArtifactRegistry for FakeSilverRegistry {
    async fn lookup(
        &self,
        source: &str,
        package: &str,
        version: &str,
    ) -> Result<Option<SilverGraphArtifact>, WorkerError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .rows
            .lock()
            .expect("silver registry lock")
            .iter()
            .rev()
            .find(|row| {
                row.source == source
                    && row.package == package
                    && row.version == version
                    && row.build_status == "success"
            })
            .cloned())
    }

    async fn register(&self, row: &SilverGraphArtifact) -> Result<(), WorkerError> {
        self.registers.fetch_add(1, Ordering::SeqCst);
        self.events
            .lock()
            .expect("events lock")
            .push("register".to_owned());
        self.rows
            .lock()
            .expect("silver registry lock")
            .push(row.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FakeBronzeRegistry {
    rows: Mutex<Vec<BronzeRawSource>>,
    registers: AtomicUsize,
}

impl FakeBronzeRegistry {
    fn with_row(row: BronzeRawSource) -> Self {
        Self {
            rows: Mutex::new(vec![row]),
            registers: AtomicUsize::new(0),
        }
    }

    fn rows(&self) -> Vec<BronzeRawSource> {
        self.rows.lock().expect("bronze registry lock").clone()
    }

    fn registers(&self) -> usize {
        self.registers.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BronzeRawSourceRegistry for FakeBronzeRegistry {
    async fn lookup(
        &self,
        source: &str,
        package: &str,
        version: &str,
    ) -> Result<Option<BronzeRawSource>, WorkerError> {
        Ok(self
            .rows
            .lock()
            .expect("bronze registry lock")
            .iter()
            .rev()
            .find(|row| row.source == source && row.package == package && row.version == version)
            .cloned())
    }

    async fn register(&self, row: &BronzeRawSource) -> Result<(), WorkerError> {
        self.registers.fetch_add(1, Ordering::SeqCst);
        self.rows
            .lock()
            .expect("bronze registry lock")
            .push(row.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FakeBronzeArchiveStore {
    objects: Mutex<HashMap<String, FakeBronzeObject>>,
    uploaded_keys: Mutex<Vec<String>>,
    uploads: AtomicUsize,
    downloads: AtomicUsize,
}

#[derive(Clone)]
struct FakeBronzeObject {
    content_sha256: String,
    bytes: Vec<u8>,
}

impl FakeBronzeArchiveStore {
    fn seed_object(&self, key: &str, bytes: Vec<u8>) -> String {
        let content_sha256 = sha256_hex(&bytes);
        self.objects.lock().expect("bronze store lock").insert(
            format!("s3://bronze-test/{key}"),
            FakeBronzeObject {
                content_sha256: content_sha256.clone(),
                bytes,
            },
        );
        content_sha256
    }

    fn uploaded_keys(&self) -> Vec<String> {
        self.uploaded_keys
            .lock()
            .expect("uploaded keys lock")
            .clone()
    }

    fn sha256_for(&self, uri: &str) -> Option<String> {
        self.objects
            .lock()
            .expect("bronze store lock")
            .get(uri)
            .map(|object| object.content_sha256.clone())
    }

    fn uploads(&self) -> usize {
        self.uploads.load(Ordering::SeqCst)
    }

    fn downloads(&self) -> usize {
        self.downloads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BronzeArchiveStore for FakeBronzeArchiveStore {
    async fn content_sha256(&self, s3_uri: &str) -> Result<Option<String>, WorkerError> {
        Ok(self.sha256_for(s3_uri))
    }

    async fn download_to_path(&self, s3_uri: &str, path: &Path) -> Result<(), WorkerError> {
        let object = self
            .objects
            .lock()
            .expect("bronze store lock")
            .get(s3_uri)
            .cloned()
            .ok_or_else(|| WorkerError::Fetch(format!("missing fake bronze object {s3_uri}")))?;
        fs::write(path, object.bytes)
            .map_err(|error| WorkerError::Fetch(format!("write fake bronze download: {error}")))?;
        self.downloads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn upload_path(
        &self,
        key: &str,
        content_sha256: &str,
        path: &Path,
    ) -> Result<String, WorkerError> {
        let bytes = fs::read(path)
            .map_err(|error| WorkerError::Fetch(format!("read fake bronze upload: {error}")))?;
        let uri = format!("s3://bronze-test/{key}");
        self.objects.lock().expect("bronze store lock").insert(
            uri.clone(),
            FakeBronzeObject {
                content_sha256: content_sha256.to_owned(),
                bytes,
            },
        );
        self.uploaded_keys
            .lock()
            .expect("uploaded keys lock")
            .push(key.to_owned());
        self.uploads.fetch_add(1, Ordering::SeqCst);
        Ok(uri)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    format!("{digest:x}")
}

#[derive(Default)]
struct FakeJobStore {
    next_id: AtomicU64,
    state: Mutex<FakeJobState>,
}

#[derive(Default)]
struct FakeJobState {
    jobs: std::collections::HashMap<String, JobRecord>,
    dedupe: std::collections::HashMap<JobKey, String>,
    stage_updates: Vec<String>,
}

#[async_trait]
impl JobStore for FakeJobStore {
    async fn create_or_get_active_job(
        &self,
        request: CreateJobRequest,
    ) -> spur_context_service::jobs::Result<CreateJobOutcome> {
        let key = request.key();
        let mut state = self.state.lock().expect("fake store lock");
        if let Some(job_id) = state.dedupe.get(&key) {
            if let Some(record) = state.jobs.get(job_id) {
                return Ok(CreateJobOutcome::Existing(record.clone()));
            }
        }

        let job_id = format!("job-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let record = JobRecord {
            job_id: job_id.clone(),
            status: JobStatus::Queued,
            source: request.source,
            package: request.package,
            revision: request.revision,
            source_url: request.source_url,
            source_url_hash: request.source_url_hash,
            source_kind: request.source_kind,
            caller_id: request.caller_id,
            execution_arn: None,
            attempt: 1,
            stage: None,
            snapshot_id: None,
            row_counts: None,
            error_code: None,
            error_detail: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            owner_kind: None,
            owner_id: None,
            queue_shard: None,
            queue_sort_key: None,
            next_eligible_at: None,
            dispatched_at: None,
        };
        state.dedupe.insert(key, job_id.clone());
        state.jobs.insert(job_id, record.clone());
        Ok(CreateJobOutcome::Created(record))
    }

    async fn record_execution_started(
        &self,
        job_id: &str,
        execution_arn: &str,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        self.update_job(job_id, |record| {
            record.execution_arn = Some(execution_arn.to_owned());
        })
    }

    async fn update_stage(
        &self,
        job_id: &str,
        status: JobStatus,
        stage: &str,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let mut state = self.state.lock().expect("fake store lock");
        let record = state
            .jobs
            .get_mut(job_id)
            .ok_or(spur_context_service::jobs::JobsError::NotFound)?;
        record.status = status;
        record.stage = Some(stage.to_owned());
        record.updated_at = format!("stage:{stage}");
        let updated = record.clone();
        state.stage_updates.push(stage.to_owned());
        Ok(updated)
    }

    async fn mark_complete(
        &self,
        job_id: &str,
        snapshot_id: i64,
        row_counts: Value,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let record = self.update_job(job_id, |record| {
            record.status = JobStatus::Complete;
            record.snapshot_id = Some(snapshot_id);
            record.row_counts = Some(row_counts);
            record.error_code = None;
            record.error_detail = None;
        })?;
        self.release_dedupe_if_owner(&record).await?;
        Ok(record)
    }

    async fn mark_failed(
        &self,
        job_id: &str,
        code: &str,
        detail: &str,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let record = self.update_job(job_id, |record| {
            record.status = JobStatus::Failed;
            record.error_code = Some(code.to_owned());
            record.error_detail = Some(detail.to_owned());
        })?;
        self.release_dedupe_if_owner(&record).await?;
        Ok(record)
    }

    async fn lookup_job(
        &self,
        job_id: &str,
    ) -> spur_context_service::jobs::Result<Option<JobRecord>> {
        Ok(self
            .state
            .lock()
            .expect("fake store lock")
            .jobs
            .get(job_id)
            .cloned())
    }

    async fn release_dedupe_if_owner(
        &self,
        record: &JobRecord,
    ) -> spur_context_service::jobs::Result<()> {
        let mut state = self.state.lock().expect("fake store lock");
        let key = record.key();
        if state
            .dedupe
            .get(&key)
            .is_some_and(|job_id| job_id == &record.job_id)
        {
            state.dedupe.remove(&key);
        }
        Ok(())
    }
}

impl FakeJobStore {
    fn seed_job(&self, job_id: &str) {
        let record = JobRecord {
            job_id: job_id.to_owned(),
            status: JobStatus::Queued,
            source: "git:custom".to_owned(),
            package: "demo".to_owned(),
            revision: "main".to_owned(),
            source_url: "https://github.com/example/demo".to_owned(),
            source_url_hash: "sha256:demo".to_owned(),
            source_kind: "git".to_owned(),
            caller_id: "test".to_owned(),
            execution_arn: None,
            attempt: 1,
            stage: None,
            snapshot_id: None,
            row_counts: None,
            error_code: None,
            error_detail: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            owner_kind: None,
            owner_id: None,
            queue_shard: None,
            queue_sort_key: None,
            next_eligible_at: None,
            dispatched_at: None,
        };
        let mut state = self.state.lock().expect("fake store lock");
        state.dedupe.insert(record.key(), job_id.to_owned());
        state.jobs.insert(job_id.to_owned(), record);
    }

    fn stage_updates(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("fake store lock")
            .stage_updates
            .clone()
    }

    fn update_job(
        &self,
        job_id: &str,
        update: impl FnOnce(&mut JobRecord),
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let mut state = self.state.lock().expect("fake store lock");
        let record = state.jobs.get_mut(job_id).ok_or(JobsError::NotFound)?;
        update(record);
        Ok(record.clone())
    }
}

struct FakeCatalogLeaseStore {
    lost: AtomicBool,
    acquire_failures_remaining: AtomicUsize,
    acquire_attempts: AtomicUsize,
}

impl FakeCatalogLeaseStore {
    fn lost() -> Self {
        Self {
            lost: AtomicBool::new(true),
            acquire_failures_remaining: AtomicUsize::new(0),
            acquire_attempts: AtomicUsize::new(0),
        }
    }

    fn contended(failures: usize) -> Self {
        Self {
            lost: AtomicBool::new(false),
            acquire_failures_remaining: AtomicUsize::new(failures),
            acquire_attempts: AtomicUsize::new(0),
        }
    }

    fn acquire_attempts(&self) -> usize {
        self.acquire_attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CatalogLeaseStore for FakeCatalogLeaseStore {
    async fn acquire(&self, catalog_uri: &str, owner_job_id: &str) -> Result<CatalogLease> {
        self.acquire_attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .acquire_failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            anyhow::bail!("ConditionalCheckFailedException: catalog lease is held");
        }
        Ok(CatalogLease {
            catalog_uri: catalog_uri.to_owned(),
            owner_job_id: owner_job_id.to_owned(),
            lease_token: "token".to_owned(),
            expires_at_unix_secs: 1_900_000_000,
            fencing_counter: 1,
        })
    }

    async fn renew(&self, lease: &CatalogLease) -> Result<CatalogLease> {
        Ok(lease.clone())
    }

    async fn assert_owned(&self, _lease: &CatalogLease) -> Result<()> {
        if self.lost.load(Ordering::SeqCst) {
            anyhow::bail!("lease lost");
        }
        Ok(())
    }

    async fn release(&self, _lease: &CatalogLease) -> Result<()> {
        Ok(())
    }
}

struct FakeS3Server {
    endpoint: String,
    put_request: Arc<Mutex<Option<String>>>,
}

impl FakeS3Server {
    fn start(body: &'static str, etag: &'static str, version: &'static str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").context("bind fake s3")?;
        let endpoint = format!("http://{}", listener.local_addr().context("fake s3 addr")?);
        let put_request = Arc::new(Mutex::new(None));
        let put_request_thread = put_request.clone();
        thread::spawn(move || {
            let (mut get_stream, _) = listener.accept().expect("accept fake s3 get");
            let _get = read_http_request(&mut get_stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: {etag}\r\nx-amz-version-id: {version}\r\n\r\n{body}",
                body.len()
            );
            get_stream
                .write_all(response.as_bytes())
                .expect("write fake s3 get");

            let (mut put_stream, _) = listener.accept().expect("accept fake s3 put");
            let put = read_http_request(&mut put_stream).to_ascii_lowercase();
            *put_request_thread.lock().expect("put request lock") = Some(put);
            put_stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .expect("write fake s3 put");
        });
        Ok(Self {
            endpoint,
            put_request,
        })
    }

    fn put_request(&self) -> String {
        self.put_request
            .lock()
            .expect("put request lock")
            .clone()
            .expect("put request should be captured")
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).expect("read fake s3 request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).to_string()
}

struct EnvGuard {
    previous: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set_all<const N: usize>(vars: [(&'static str, &str); N]) -> Self {
        let mut previous = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            previous.push((key, std::env::var(key).ok()));
            std::env::set_var(key, value);
        }
        Self { previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
