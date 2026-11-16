use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::io::Cursor;
use std::ops::Range;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use duckdb::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spur_context_service::artifact_cache::{
    ArtifactCache, ArtifactFetchError, ArtifactFetcher, ArtifactStream,
};
use spur_context_service::catalog::{
    compact_gold_and_export_snapshot, connect_frozen_snapshot, CatalogResolver,
    FrozenSnapshotManifest, SnapshotCleanupOptions,
};
use spur_context_service::code_backend::CodeBackend;
use spur_context_service::jobs::{
    BacklogOwner, BacklogOwnerKind, CreateJobOutcome, CreateJobRequest, EnqueueOutcome, JobKey,
    JobRecord, JobStatus, JobStore, JobsError, QueueConfig,
};
use spur_context_service::mcp::{
    handle_tool, handle_tool_with_code_backend, handle_tool_without_catalog, index_drainer_limits,
    index_queue_config, route_index, route_index_status, route_index_status_for_caller,
    route_index_warm_lookup, route_index_without_catalog, tool_definitions, ExecutionOutcome,
    ExecutionOutcomeStatus, ExecutionStatusChecker, IndexExecutionRequest, IndexExecutionStarter,
    McpHandlerError,
};
use spur_context_service::medallion::{
    SilverManifest, SilverManifestFile, SILVER_MANIFEST_FILENAME,
};
use spur_context_service::serving_registry::{
    ArtifactRef, ServingPackage, ServingRegistry, SERVING_REGISTRY_SCHEMA_VERSION,
};

const PACKAGE: &str = "demo";
const REVISION: &str = "1.0.0";
const PARQUET_SOURCE: &str = concat!(
    "pub fn alpha() {\n",
    "    beta();\n",
    "    external::Thing::new();\n",
    "}\n",
    "\n",
    "pub fn beta() {\n",
    "}\n",
);
const PARQUET_SOURCE_OID: &str = "398807d495e9b32290c673280be0312ba07fc1dc";
const SOURCE_URL: &str = "https://1.1.1.1/example/demo";
const CRATES_IO_SOURCE_URL: &str = "https://crates.io/api/v1/crates/demo/1.0.0/download";
const GIT_SOURCE: &str = "git:github.com/example/demo";
const DIMENSIONS: usize = 768;
const EMBEDDING_MODEL: &str = "NomicEmbedTextV15";
const EMBED_TEXT_VERSION: &str = "v5-nomic-embed-text-v1.5-search-document";
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn queue_config_uses_dedicated_per_owner_running_cap() {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_RUNNING_JOBS_PER_OWNER", "7");

    assert_eq!(index_queue_config().max_running_per_owner, 7);
}

#[test]
fn drainer_limits_use_runtime_environment() {
    let _env = EnvVarGuard::set("SPUR_INDEX_DRAINER_BATCH_LIMIT", "5");

    assert_eq!(index_drainer_limits().max_dispatches_per_run, 5);
}

#[test]
fn drainer_scan_limit_uses_runtime_environment() {
    let _env = EnvVarGuard::set("SPUR_INDEX_DRAINER_SCAN_LIMIT_PER_SHARD", "11");

    assert_eq!(index_drainer_limits().scan_limit_per_shard, 11);
}

#[test]
fn queue_config_bounds_global_running_token_probes() {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_RUNNING_JOBS_GLOBAL", "100");

    assert_eq!(index_queue_config().max_running_global, 32);
}

struct EnvVarGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let previous = std::env::var_os(name);
        std::env::set_var(name, value);
        Self {
            name,
            previous,
            _guard: guard,
        }
    }

    fn remove(name: &'static str) -> Self {
        let guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let previous = std::env::var_os(name);
        std::env::remove_var(name);
        Self {
            name,
            previous,
            _guard: guard,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

#[test]
fn tool_definitions_match_external_context_surface() {
    let definitions = tool_definitions();
    let names = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "external_catalog",
            "external_code_search",
            "external_code_read",
            "external_code_callers",
            "external_code_callees",
            "external_knowledge_context",
            "external_index",
            "external_index_status",
        ]
    );

    let catalog_schema = schema_for(&definitions, "external_catalog");
    assert!(catalog_schema.get("required").is_none());
    assert_eq!(
        catalog_schema["properties"]["source"]["default"],
        "registry:crates-io"
    );
    assert_eq!(catalog_schema["properties"]["limit"]["default"], 50);
    assert_eq!(catalog_schema["properties"]["limit"]["maximum"], 200);
    assert_eq!(catalog_schema["additionalProperties"], false);

    let search_schema = schema_for(&definitions, "external_code_search");
    assert_eq!(required(search_schema), ["query", "package"]);
    assert!(search_schema["properties"]["source"]["default"].is_string());
    assert_eq!(search_schema["properties"]["limit"]["maximum"], 200);

    let read_schema = schema_for(&definitions, "external_code_read");
    assert_eq!(required(read_schema), ["selector"]);
    assert!(read_schema["properties"]["source"]["default"].is_string());

    let callers_schema = schema_for(&definitions, "external_code_callers");
    assert_eq!(required(callers_schema), ["selector"]);
    assert!(callers_schema["properties"]["source"]["default"].is_string());
    assert_eq!(
        callers_schema["properties"]["include_unresolved"]["default"],
        false
    );

    let callees_schema = schema_for(&definitions, "external_code_callees");
    assert_eq!(required(callees_schema), ["selector"]);
    assert!(callees_schema["properties"]["source"]["default"].is_string());

    let knowledge_schema = schema_for(&definitions, "external_knowledge_context");
    assert_eq!(required(knowledge_schema), ["query", "package"]);
    assert_eq!(
        knowledge_schema["properties"]["scope"]["enum"],
        json!(["code", "docs", "all"])
    );
    assert_eq!(knowledge_schema["properties"]["limit"]["default"], 8);

    let index_schema = schema_for(&definitions, "external_index");
    assert_eq!(
        required(index_schema),
        ["package", "revision", "source_url"]
    );
    assert!(index_schema["properties"]["source"]
        .get("default")
        .is_none());
    assert!(index_schema["properties"]["package"]["description"]
        .as_str()
        .unwrap()
        .contains("crates.io"));
    assert!(index_schema["properties"]["source_url"]["description"]
        .as_str()
        .unwrap()
        .contains("canonical"));
    assert!(index_schema["properties"]["source"]["description"]
        .as_str()
        .unwrap()
        .contains("registry:crates-io"));
    assert_eq!(
        index_schema["properties"]["source_kind"]["enum"],
        json!(["git", "tarball"])
    );
    assert_eq!(index_schema["properties"]["force"]["default"], false);

    let status_schema = schema_for(&definitions, "external_index_status");
    assert_eq!(required(status_schema), ["job_id"]);
}

#[tokio::test]
async fn external_code_search_resolves_latest_and_returns_candidates() -> Result<()> {
    let fixture = McpFixture::new("search")?;

    let response = handle_tool(
        "external_code_search",
        &json!({
            "query": "bet",
            "package": PACKAGE,
            "symbol_kind": "function",
            "limit": 20
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(response["total_matches"], 1);
    assert_eq!(response["truncated"], false);
    assert_eq!(
        response["candidates"][0]["stable_symbol_id"],
        "bbbbbbbbbbbbbbbb"
    );
    assert_eq!(response["candidates"][0]["revision"], REVISION);
    Ok(())
}

#[tokio::test]
async fn catalog_package_search_parquet_matches_names_keywords_and_typos() -> Result<()> {
    let mut fixture = ParquetBackendFixture::new("catalog-package-search")?;
    for (source, package) in [
        ("registry:crates-io", "duckdb"),
        ("registry:crates-io", "duckdb-loadable-macros"),
        ("registry:crates-io", "libduckdb-sys"),
        ("registry:crates-io", "axum"),
        ("registry:crates-io", "café"),
        ("git:github.com/example/duck", "duck-private"),
    ] {
        let mut entry = fixture.registry.packages[0].clone();
        entry.source = source.to_owned();
        entry.package = package.to_owned();
        fixture.registry.packages.push(entry);
    }
    fixture.expand_cache_capacity(fixture.registry.packages.len() as u64)?;
    for (filter, expected) in [
        (
            "DUCK",
            vec!["duckdb", "duckdb-loadable-macros", "libduckdb-sys"],
        ),
        ("sys duck", vec!["libduckdb-sys"]),
        ("DUCK_sys", vec!["libduckdb-sys"]),
        ("dukdb", vec!["duckdb", "duckdb-loadable-macros"]),
        ("duckbd", vec!["duckdb", "duckdb-loadable-macros"]),
        ("axm", vec!["axum"]),
        ("dk", vec![]),
        ("dukxx", vec![]),
        ("duckzz", vec![]),
        ("unrelated", vec![]),
        ("CAFÉ", vec!["café"]),
        ("cafe", vec!["café"]),
        ("---", vec![]),
        ("' OR 1=1 --", vec![]),
    ] {
        let result =
            call_parquet_backend(&fixture, "external_catalog", &json!({"name_filter":filter}))
                .await?;
        let actual = result["rows"]
            .as_array()
            .context("catalog rows")?
            .iter()
            .map(|row| row["package"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "filter {filter:?}");
        assert_eq!(result["total_matches"], expected.len(), "filter {filter:?}");
    }
    let all = call_parquet_backend(&fixture, "external_catalog", &json!({})).await?;
    for filter in ["", "   "] {
        let result =
            call_parquet_backend(&fixture, "external_catalog", &json!({"name_filter":filter}))
                .await?;
        assert_eq!(result["rows"], all["rows"], "filter {filter:?}");
    }
    let mut cursor = Value::Null;
    let mut packages = Vec::new();
    loop {
        let page = call_parquet_backend(
            &fixture,
            "external_catalog",
            &json!({"name_filter":"DUCK", "limit":1, "cursor":cursor}),
        )
        .await?;
        assert_eq!(page["total_matches"], 3);
        assert_eq!(
            page["catalog_generation"],
            ParquetBackendFixture::GENERATION
        );
        packages.extend(
            page["rows"]
                .as_array()
                .context("catalog rows")?
                .iter()
                .map(|row| row["package"].as_str().unwrap().to_owned()),
        );
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(
        packages,
        ["duckdb", "duckdb-loadable-macros", "libduckdb-sys"]
    );
    let other = call_parquet_backend(
        &fixture,
        "external_catalog",
        &json!({"source":"git:github.com/example/duck", "name_filter":"DUCK"}),
    )
    .await?;
    assert_eq!(other["total_matches"], 1);
    assert_eq!(other["rows"][0]["package"], "duck-private");
    let revisions =
        call_parquet_backend(&fixture, "external_catalog", &json!({"package":"duckdb"})).await?;
    assert_eq!(revisions["level"], "revisions");
    assert_eq!(revisions["total_matches"], 1);
    let missing = call_parquet_backend(&fixture, "external_catalog", &json!({"package":"dukdb"}))
        .await
        .expect_err("package coordinates must remain exact rather than fuzzy");
    assert!(matches!(missing, McpHandlerError::NotFound(_)));
    Ok(())
}

#[tokio::test]
async fn parquet_backend_matches_catalog_and_search_contracts() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-backend-contracts")?;

    let catalog = call_parquet_backend(&fixture, "external_catalog", &json!({})).await?;
    assert_eq!(
        catalog,
        json!({
            "level": "packages",
            "rows": [{
                "source": "registry:crates-io",
                "package": PACKAGE,
                "latest_revision": REVISION,
                "revision_count": 1,
                "indexed_at": ""
            }],
            "total_matches": 1,
            "truncated": false,
            "next_cursor": null,
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let search = call_parquet_backend(
        &fixture,
        "external_code_search",
        &json!({
            "query": "bet",
            "package": PACKAGE,
            "revision": REVISION,
            "symbol_kind": "function",
            "limit": 20
        }),
    )
    .await?;
    assert_eq!(
        search,
        json!({
            "candidates": [{
                "selector": "pkg:demo@1.0.0::demo::beta",
                "uri": "pkg-symbol://registry:crates-io/demo/1.0.0/bbbbbbbbbbbbbbbb",
                "id": "bbbbbbbbbbbbbbbb",
                "stable_symbol_id": "bbbbbbbbbbbbbbbb",
                "source": "registry:crates-io",
                "package": PACKAGE,
                "revision": REVISION,
                "entity_name": "beta",
                "qualified_name": "demo::beta",
                "file_path": "src/lib.rs",
                "line_range": [6, 7],
                "symbol_kind": "function",
                "enclosing_scope": null
            }],
            "total_matches": 1,
            "truncated": false,
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let empty = call_parquet_backend(
        &fixture,
        "external_code_search",
        &json!({
            "query": "does_not_exist",
            "package": PACKAGE,
            "revision": REVISION
        }),
    )
    .await?;
    assert_eq!(
        empty,
        json!({
            "candidates": [],
            "total_matches": 0,
            "truncated": false,
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let selector_args = json!({
        "query": "bet",
        "package": PACKAGE,
        "revision": REVISION,
        "ref": "latest"
    });
    let expected_selector_error =
        handle_tool_without_catalog("external_code_search", &selector_args)
            .expect_err("revision and ref together must remain invalid");
    let actual_selector_error =
        call_parquet_backend(&fixture, "external_code_search", &selector_args)
            .await
            .expect_err("Parquet backend must preserve selector validation");
    assert_eq!(
        actual_selector_error.json_rpc_code(),
        expected_selector_error.json_rpc_code()
    );
    assert_eq!(
        actual_selector_error.to_string(),
        expected_selector_error.to_string()
    );

    let warm = call_parquet_backend(
        &fixture,
        "external_index",
        &json!({
            "package": PACKAGE,
            "revision": REVISION,
            "source_url": SOURCE_URL,
            "source": "registry:crates-io"
        }),
    )
    .await?;
    assert_eq!(
        warm,
        json!({
            "status": "complete",
            "snapshot_id": ParquetBackendFixture::GENERATION,
            "revision": REVISION
        })
    );

    let status = call_parquet_backend(
        &fixture,
        "external_index_status",
        &json!({ "job_id": "pkg:demo@1.0.0" }),
    )
    .await?;
    assert_eq!(status["status"], "complete");
    assert_eq!(status["snapshot_id"], ParquetBackendFixture::GENERATION);
    assert_eq!(status["revision"], REVISION);

    let missing_root = ParquetBackendFixture::new("parquet-missing-root")?;
    missing_root.remove_object(&missing_root.root_uri());
    assert_retryable_backend_error(
        call_parquet_backend(&missing_root, "external_catalog", &json!({})).await,
        "artifact_unavailable",
    );

    let mut invalid_root = ParquetBackendFixture::new("parquet-invalid-root-version")?;
    invalid_root.replace_silver_root(serde_json::to_vec(&json!({
        "schema_version": 2,
        "schema_hash": "sha256:test-schema",
        "files": []
    }))?);
    assert_retryable_backend_error(
        call_parquet_backend(&invalid_root, "external_catalog", &json!({})).await,
        "invalid_artifact_identity",
    );

    let mut missing_graph_manifest = ParquetBackendFixture::new("parquet-missing-graph-manifest")?;
    missing_graph_manifest.remove_declared_member("manifest.json")?;
    assert_retryable_backend_error(
        call_parquet_backend(&missing_graph_manifest, "external_catalog", &json!({})).await,
        "invalid_artifact_identity",
    );

    let missing_member = ParquetBackendFixture::new("parquet-missing-member")?;
    missing_member.remove_object(&missing_member.member_uri("nodes.parquet"));
    assert_retryable_backend_error(
        call_parquet_backend(
            &missing_member,
            "external_code_search",
            &json!({
                "query": "bet",
                "package": PACKAGE,
                "revision": REVISION
            }),
        )
        .await,
        "artifact_unavailable",
    );

    let mut missing_etag = ParquetBackendFixture::new("parquet-missing-member-etag")?;
    let mut missing_etag_root: SilverManifest =
        serde_json::from_slice(&silver_root_for_members(&missing_etag.declared_members)?)?;
    missing_etag_root
        .files
        .iter_mut()
        .find(|file| file.path == "nodes.parquet")
        .context("nodes member in Silver manifest")?
        .etag
        .clear();
    missing_etag.replace_silver_root(serde_json::to_vec(&missing_etag_root)?);
    assert_retryable_backend_error(
        call_parquet_backend(
            &missing_etag,
            "external_code_search",
            &json!({
                "query": "bet",
                "package": PACKAGE,
                "revision": REVISION
            }),
        )
        .await,
        "invalid_artifact_identity",
    );

    let mut mismatched_etag = ParquetBackendFixture::new("parquet-mismatched-member-etag")?;
    let mut mismatched_etag_root: SilverManifest =
        serde_json::from_slice(&silver_root_for_members(&mismatched_etag.declared_members)?)?;
    mismatched_etag_root
        .files
        .iter_mut()
        .find(|file| file.path == "nodes.parquet")
        .context("nodes member in Silver manifest")?
        .etag = "etag-from-another-object".to_owned();
    mismatched_etag.replace_silver_root(serde_json::to_vec(&mismatched_etag_root)?);
    assert_retryable_backend_error(
        call_parquet_backend(
            &mismatched_etag,
            "external_code_search",
            &json!({
                "query": "bet",
                "package": PACKAGE,
                "revision": REVISION
            }),
        )
        .await,
        "artifact_integrity_mismatch",
    );

    let corrupt_graph_manifest = ParquetBackendFixture::new("parquet-corrupt-graph-manifest")?;
    corrupt_graph_manifest.replace_object(
        &corrupt_graph_manifest.member_uri("manifest.json"),
        b"corrupt graph manifest".to_vec(),
    );
    assert_retryable_backend_error(
        call_parquet_backend(&corrupt_graph_manifest, "external_catalog", &json!({})).await,
        "artifact_integrity_mismatch",
    );

    let corrupt_member = ParquetBackendFixture::new("parquet-corrupt-member")?;
    corrupt_member.replace_object(
        &corrupt_member.member_uri("nodes.parquet"),
        b"corrupt parquet member".to_vec(),
    );
    assert_retryable_backend_error(
        call_parquet_backend(
            &corrupt_member,
            "external_code_search",
            &json!({
                "query": "bet",
                "package": PACKAGE,
                "revision": REVISION
            }),
        )
        .await,
        "artifact_integrity_mismatch",
    );

    let mut stale = ParquetBackendFixture::new("parquet-stale-generation")?;
    stale.registry.packages[0].generation -= 1;
    assert_retryable_backend_error(
        call_parquet_backend(&stale, "external_catalog", &json!({})).await,
        "invalid_serving_registry",
    );

    Ok(())
}

#[tokio::test]
async fn parquet_backend_reuses_warm_client_across_distinct_queries() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-warm-client-reuse")?;
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;

    let first = handle_tool_with_code_backend(
        "external_code_search",
        &json!({
            "query": "alpha",
            "package": PACKAGE,
            "revision": REVISION
        }),
        &backend,
    )
    .await?;
    assert_eq!(first["candidates"][0]["entity_name"], "alpha");

    assert!(
        find_cached_file(&fixture.root.join("cache"), "nodes.parquet").is_err(),
        "search should retain a decoded index without materializing nodes.parquet"
    );
    fixture.remove_object(&fixture.member_uri("nodes.parquet"));

    let second = handle_tool_with_code_backend(
        "external_code_search",
        &json!({
            "query": "beta",
            "package": PACKAGE,
            "revision": REVISION
        }),
        &backend,
    )
    .await?;
    assert_eq!(second["candidates"][0]["entity_name"], "beta");

    Ok(())
}

#[tokio::test]
async fn parquet_backend_coalesces_concurrent_cold_search_loads() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-concurrent-search")?;
    *fixture
        .fetcher
        .range_delay
        .lock()
        .expect("Parquet range delay mutex should not be poisoned") =
        Some(Duration::from_millis(10));
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;
    let first_args = json!({
        "query": "alpha",
        "package": PACKAGE,
        "revision": REVISION
    });
    let second_args = json!({
        "query": "beta",
        "package": PACKAGE,
        "revision": REVISION
    });
    let (first, second) = tokio::join!(
        handle_tool_with_code_backend("external_code_search", &first_args, &backend),
        handle_tool_with_code_backend("external_code_search", &second_args, &backend)
    );
    assert_eq!(first?["candidates"][0]["entity_name"], "alpha");
    assert_eq!(second?["candidates"][0]["entity_name"], "beta");
    let nodes_uri = fixture.member_uri("nodes.parquet");
    let concurrent_range_count = fixture
        .fetcher
        .ranges
        .lock()
        .expect("Parquet range log mutex should not be poisoned")
        .iter()
        .filter(|(uri, _, _)| uri == &nodes_uri)
        .count();

    let baseline = ParquetBackendFixture::new("parquet-single-search")?;
    let baseline_backend = CodeBackend::new(baseline.registry.clone(), baseline.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;
    handle_tool_with_code_backend("external_code_search", &first_args, &baseline_backend).await?;
    let baseline_nodes_uri = baseline.member_uri("nodes.parquet");
    let baseline_range_count = baseline
        .fetcher
        .ranges
        .lock()
        .expect("Parquet range log mutex should not be poisoned")
        .iter()
        .filter(|(uri, _, _)| uri == &baseline_nodes_uri)
        .count();

    assert_eq!(concurrent_range_count, baseline_range_count);
    Ok(())
}

#[tokio::test]
async fn parquet_backend_search_does_not_materialize_the_bundle() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-remote-search")?;
    for path in [
        "manifest.json",
        "edges.parquet",
        "edges_unresolved.parquet",
        "files.parquet",
        "file_manifests.parquet",
        "source_files.parquet",
    ] {
        fixture.remove_object(&fixture.member_uri(path));
    }
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;

    let response = handle_tool_with_code_backend(
        "external_code_search",
        &json!({
            "query": "beta",
            "package": PACKAGE,
            "revision": REVISION
        }),
        &backend,
    )
    .await?;

    assert_eq!(response["candidates"][0]["entity_name"], "beta");
    let nodes_uri = fixture.member_uri("nodes.parquet");
    let nodes_len = fixture
        .declared_members
        .get("nodes.parquet")
        .context("nodes fixture member")?
        .len() as u64;
    let ranges = fixture
        .fetcher
        .ranges
        .lock()
        .expect("Parquet range log mutex should not be poisoned");
    let node_ranges = ranges
        .iter()
        .filter(|(uri, _, _)| uri == &nodes_uri)
        .collect::<Vec<_>>();
    assert!(!node_ranges.is_empty(), "search must issue Parquet ranges");
    assert!(node_ranges
        .iter()
        .all(|(_, _, etag)| etag.as_deref() == Some("etag-nodes.parquet")));
    let covered = covered_range_bytes(
        &node_ranges
            .iter()
            .map(|(_, range, _)| range.clone())
            .collect::<Vec<_>>(),
    );
    assert!(covered < nodes_len, "range reads must skip object bytes");
    assert!(
        find_cached_file(&fixture.root.join("cache"), "nodes.parquet").is_err(),
        "remote search must not materialize nodes.parquet in /tmp"
    );
    Ok(())
}

#[tokio::test]
async fn parquet_backend_reuses_warm_client_across_distinct_edge_queries() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-warm-adjacency-reuse")?;
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;

    let callers = handle_tool_with_code_backend(
        "external_code_callers",
        &json!({
            "selector": "pkg:demo@1.0.0::demo::beta",
            "include_unresolved": true
        }),
        &backend,
    )
    .await?;
    assert_eq!(callers["counts_by_kind"]["calls"], 1);

    let cached_edges = find_cached_file(&fixture.root.join("cache"), "edges.parquet")?;
    fixture.remove_object(&fixture.member_uri("edges.parquet"));
    fs::remove_file(cached_edges).context("remove materialized edges after warming client")?;

    let callees = handle_tool_with_code_backend(
        "external_code_callees",
        &json!({
            "selector": "pkg:demo@1.0.0::demo::alpha",
            "include_unresolved": true
        }),
        &backend,
    )
    .await?;
    assert_eq!(callees["counts_by_kind"]["calls"], 2);
    assert_eq!(callees["counts_by_kind"]["unresolved"], 1);

    Ok(())
}

#[tokio::test]
async fn parquet_backend_replaces_warm_client_for_different_package_identity() -> Result<()> {
    let mut fixture = ParquetBackendFixture::new("parquet-warm-client-replacement")?;
    fixture.expand_cache_capacity(2)?;
    let mut revision_1 = fixture.registry.packages[0].clone();
    revision_1.refs.clear();
    let mut revision_2 = revision_1.clone();
    revision_2.revision = "2.0.0".to_owned();
    revision_2.refs = vec!["latest".to_owned()];
    fixture.registry.packages = vec![revision_1, revision_2];
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| anyhow::anyhow!(error))?;

    let first = handle_tool_with_code_backend(
        "external_code_search",
        &json!({
            "query": "alpha",
            "package": PACKAGE,
            "revision": REVISION
        }),
        &backend,
    )
    .await?;
    assert_eq!(first["candidates"][0]["revision"], REVISION);

    let second = tokio::time::timeout(
        Duration::from_secs(2),
        handle_tool_with_code_backend(
            "external_code_search",
            &json!({
                "query": "beta",
                "package": PACKAGE,
                "revision": "2.0.0"
            }),
            &backend,
        ),
    )
    .await
    .context("switching package identity must not deadlock generation activation")??;
    assert_eq!(second["candidates"][0]["revision"], "2.0.0");

    Ok(())
}

#[tokio::test]
async fn parquet_backend_resolves_semantic_latest_and_named_refs_from_registry() -> Result<()> {
    let mut fixture = ParquetBackendFixture::new("parquet-registry-ref-contracts")?;
    fixture.expand_cache_capacity(2)?;
    let mut revision_9 = fixture.registry.packages[0].clone();
    revision_9.revision = "9.0.0".to_owned();
    revision_9.refs.clear();
    let mut revision_10 = fixture.registry.packages[0].clone();
    revision_10.revision = "10.0.0".to_owned();
    revision_10.refs = vec!["latest".to_owned(), "stable".to_owned()];
    fixture.registry.packages = vec![revision_9, revision_10];

    let packages = call_parquet_backend(&fixture, "external_catalog", &json!({})).await?;
    assert_eq!(packages["rows"][0]["latest_revision"], "10.0.0");

    let revisions =
        call_parquet_backend(&fixture, "external_catalog", &json!({ "package": PACKAGE })).await?;
    let latest_row = revisions["rows"]
        .as_array()
        .context("revision rows")?
        .iter()
        .find(|row| row["revision"] == "10.0.0")
        .context("latest semantic revision")?;
    assert_eq!(latest_row["revision_kind"], "semver");
    assert_eq!(latest_row["refs"], json!(["latest", "stable"]));

    for reference in ["latest", "stable"] {
        let search = call_parquet_backend(
            &fixture,
            "external_code_search",
            &json!({
                "query": "bet",
                "package": PACKAGE,
                "ref": reference
            }),
        )
        .await?;
        assert_eq!(search["candidates"][0]["revision"], "10.0.0");
    }
    Ok(())
}

#[tokio::test]
async fn parquet_backend_matches_read_and_edge_contracts() -> Result<()> {
    let fixture = ParquetBackendFixture::new("parquet-read-edge-contracts")?;
    let beta_byte_range = byte_range_for_lines(PARQUET_SOURCE, 6, 7)?;

    let read = call_parquet_backend(
        &fixture,
        "external_code_read",
        &json!({
            "selector": "pkg:demo@latest::demo::beta",
            "context_lines": 0
        }),
    )
    .await?;
    assert_eq!(
        read,
        json!({
            "selector": "pkg:demo@1.0.0::demo::beta",
            "stable_symbol_id": "bbbbbbbbbbbbbbbb",
            "package_source": "registry:crates-io",
            "package": PACKAGE,
            "revision": REVISION,
            "file_path": "src/lib.rs",
            "byte_range": beta_byte_range,
            "line_range": [6, 7],
            "source": "pub fn beta() {\n}\n",
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let callers = call_parquet_backend(
        &fixture,
        "external_code_callers",
        &json!({
            "selector": "pkg:demo@1.0.0::demo::beta",
            "include_unresolved": true
        }),
    )
    .await?;
    assert_eq!(
        callers,
        json!({
            "callers": [
                resolved_edge_row("caller", "calls", "calls", "syntax_exact"),
                resolved_edge_row("caller", "calls", "calls_dyn", "heuristic"),
                resolved_edge_row("caller", "references", "references_hof", "syntax_exact"),
                resolved_edge_row("caller", "references", "references_other", "syntax_exact")
            ],
            "counts_by_kind": {
                "calls": 1,
                "calls_dyn": 1,
                "references_hof": 1,
                "references_other": 1,
                "unresolved": 0
            },
            "unresolved_sample": [],
            "stable_symbol_id": "bbbbbbbbbbbbbbbb",
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let callees = call_parquet_backend(
        &fixture,
        "external_code_callees",
        &json!({
            "selector": "pkg:demo@1.0.0::demo::alpha",
            "include_unresolved": true
        }),
    )
    .await?;
    assert_eq!(
        callees,
        json!({
            "callees": [
                resolved_edge_row("callee", "calls", "calls", "syntax_exact"),
                resolved_edge_row("callee", "calls", "calls_dyn", "heuristic"),
                resolved_edge_row("callee", "references", "references_hof", "syntax_exact"),
                resolved_edge_row("callee", "references", "references_other", "syntax_exact"),
                unresolved_callee_row()
            ],
            "counts_by_kind": {
                "calls": 2,
                "calls_dyn": 1,
                "references_hof": 1,
                "references_other": 1,
                "unresolved": 1
            },
            "unresolved_sample": ["external::Thing"],
            "stable_symbol_id": "aaaaaaaaaaaaaaaa",
            "catalog_generation": ParquetBackendFixture::GENERATION
        })
    );

    let invalid_selector = call_parquet_backend(
        &fixture,
        "external_code_read",
        &json!({ "selector": "graph://symbol/bbbbbbbbbbbbbbbb" }),
    )
    .await
    .expect_err("worktree selectors must remain invalid on the external surface");
    assert_eq!(invalid_selector.json_rpc_code(), -32602);
    assert_eq!(
        invalid_selector.to_string(),
        "invalid params: external selector must start with 'pkg:': graph://symbol/bbbbbbbbbbbbbbbb"
    );

    let missing_symbol = call_parquet_backend(
        &fixture,
        "external_code_read",
        &json!({ "selector": "pkg:demo@1.0.0::demo::missing" }),
    )
    .await
    .expect_err("missing read selector must remain not found");
    assert_eq!(missing_symbol.json_rpc_code(), -32004);
    assert_eq!(
        missing_symbol.to_string(),
        "not found: symbol not found: pkg:demo@1.0.0::demo::missing"
    );

    let ambiguous = call_parquet_backend(
        &fixture,
        "external_code_read",
        &json!({ "selector": "pkg:demo@1.0.0::overloaded" }),
    )
    .await
    .expect_err("ambiguous selector must not pick a symbol");
    assert_eq!(ambiguous.json_rpc_code(), -32602);
    assert_eq!(
        ambiguous.to_string(),
        concat!(
            "invalid params: external_code_read failed: ambiguous external selector ",
            "`pkg:demo@1.0.0::overloaded`; candidates: ",
            "pkg-symbol://registry:crates-io/demo/1.0.0/1111111111111111, ",
            "pkg-symbol://registry:crates-io/demo/1.0.0/2222222222222222"
        )
    );

    let mut missing_text = ParquetBackendFixture::new("parquet-missing-source-text")?;
    missing_text.replace_source_sidecar(None)?;
    assert_retryable_backend_error(
        call_parquet_backend(
            &missing_text,
            "external_code_read",
            &json!({ "selector": "pkg:demo@1.0.0::demo::beta" }),
        )
        .await,
        "source_text_unavailable",
    );

    let mut oid_mismatch = ParquetBackendFixture::new("parquet-source-oid-mismatch")?;
    oid_mismatch.replace_source_sidecar(Some((
        "src/lib.rs",
        "0000000000000000000000000000000000000000",
        PARQUET_SOURCE,
    )))?;
    assert_retryable_backend_error(
        call_parquet_backend(
            &oid_mismatch,
            "external_code_read",
            &json!({ "selector": "pkg:demo@1.0.0::demo::beta" }),
        )
        .await,
        "source_content_oid_mismatch",
    );

    let corrupt_sidecar = ParquetBackendFixture::new("parquet-corrupt-sidecar")?;
    corrupt_sidecar.replace_object(
        &corrupt_sidecar.member_uri("source_files.parquet"),
        b"corrupt source sidecar".to_vec(),
    );
    assert_retryable_backend_error(
        call_parquet_backend(
            &corrupt_sidecar,
            "external_code_read",
            &json!({ "selector": "pkg:demo@1.0.0::demo::beta" }),
        )
        .await,
        "artifact_integrity_mismatch",
    );

    let mut stale = ParquetBackendFixture::new("parquet-read-stale-generation")?;
    stale.registry.packages[0].generation -= 1;
    assert_retryable_backend_error(
        call_parquet_backend(
            &stale,
            "external_code_read",
            &json!({ "selector": "pkg:demo@1.0.0::demo::beta" }),
        )
        .await,
        "invalid_serving_registry",
    );

    let mut mixed = ParquetBackendFixture::new("parquet-read-mixed-generation")?;
    mixed.registry.packages[0].source_sidecar.uri = mixed.registry.packages[0]
        .source_sidecar
        .uri
        .replace("generation-42", "generation-41");
    assert_retryable_backend_error(
        call_parquet_backend(
            &mixed,
            "external_code_read",
            &json!({ "selector": "pkg:demo@1.0.0::demo::beta" }),
        )
        .await,
        "invalid_artifact_identity",
    );

    Ok(())
}

#[derive(Clone, Default)]
struct ParquetTestFetcher {
    objects: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ranges: Arc<Mutex<Vec<(String, Range<u64>, Option<String>)>>>,
    range_delay: Arc<Mutex<Option<Duration>>>,
}

#[async_trait]
impl ArtifactFetcher for ParquetTestFetcher {
    async fn fetch(&self, uri: &str) -> Result<ArtifactStream, ArtifactFetchError> {
        let body = self
            .objects
            .lock()
            .expect("Parquet test object mutex should not be poisoned")
            .get(uri)
            .cloned()
            .ok_or_else(|| ArtifactFetchError::new("missing Parquet test object"))?;
        Ok(Box::pin(Cursor::new(body)))
    }

    async fn fetch_range(
        &self,
        uri: &str,
        range: Range<u64>,
        expected_etag: Option<&str>,
    ) -> Result<Vec<u8>, ArtifactFetchError> {
        let delay = *self
            .range_delay
            .lock()
            .expect("Parquet range delay mutex should not be poisoned");
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.ranges
            .lock()
            .expect("Parquet range log mutex should not be poisoned")
            .push((
                uri.to_owned(),
                range.clone(),
                expected_etag.map(str::to_owned),
            ));
        if let Some(expected_etag) = expected_etag {
            let object_etag = format!("etag-{}", uri.rsplit('/').next().unwrap_or_default());
            if expected_etag != object_etag {
                return Err(ArtifactFetchError::integrity_mismatch(
                    "Parquet test object ETag mismatch",
                ));
            }
        }
        let objects = self
            .objects
            .lock()
            .expect("Parquet test object mutex should not be poisoned");
        let body = objects
            .get(uri)
            .ok_or_else(|| ArtifactFetchError::new("missing Parquet test object"))?;
        let start = usize::try_from(range.start)
            .map_err(|error| ArtifactFetchError::integrity_mismatch(error.to_string()))?;
        let end = usize::try_from(range.end)
            .map_err(|error| ArtifactFetchError::integrity_mismatch(error.to_string()))?;
        body.get(start..end).map(<[u8]>::to_vec).ok_or_else(|| {
            ArtifactFetchError::integrity_mismatch("Parquet test range is outside the object")
        })
    }
}

struct ParquetBackendFixture {
    registry: ServingRegistry,
    cache: ArtifactCache,
    fetcher: ParquetTestFetcher,
    graph_prefix: String,
    declared_members: HashMap<String, Vec<u8>>,
    root: PathBuf,
}

impl ParquetBackendFixture {
    const GENERATION: i64 = 42;

    fn new(name: &str) -> Result<Self> {
        let root = unique_temp_dir(name)?;
        let graph_dir = root.join("published-graph");
        fs::create_dir_all(&graph_dir).context("create Parquet fixture directory")?;
        write_graph_compatible_parquet_fixture(&graph_dir)?;

        let mut declared_members = HashMap::new();
        for path in [
            "manifest.json",
            "nodes.parquet",
            "edges.parquet",
            "edges_unresolved.parquet",
            "files.parquet",
            "file_manifests.parquet",
            "source_files.parquet",
        ] {
            declared_members.insert(
                path.to_owned(),
                fs::read(graph_dir.join(path))
                    .with_context(|| format!("read Parquet fixture member {path}"))?,
            );
        }

        let graph_prefix = format!(
            "s3://artifacts/silver/crates.io/{PACKAGE}/{REVISION}/generation-{}/",
            Self::GENERATION
        );
        let root_uri = format!("{graph_prefix}{SILVER_MANIFEST_FILENAME}");
        let root_bytes = silver_root_for_members(&declared_members)?;
        let fetcher = ParquetTestFetcher::default();
        {
            let mut objects = fetcher
                .objects
                .lock()
                .expect("Parquet test object mutex should not be poisoned");
            objects.insert(root_uri.clone(), root_bytes.clone());
            for (path, body) in &declared_members {
                objects.insert(format!("{graph_prefix}{path}"), body.clone());
            }
        }

        let graph_manifest = artifact_ref(root_uri, &root_bytes);
        let source_sidecar_bytes = declared_members
            .get("source_files.parquet")
            .context("source sidecar fixture member")?;
        let source_sidecar = artifact_ref(
            format!("{graph_prefix}source_files.parquet"),
            source_sidecar_bytes,
        );
        let registry = ServingRegistry {
            schema_version: SERVING_REGISTRY_SCHEMA_VERSION,
            generation: Self::GENERATION,
            packages: vec![ServingPackage {
                source: "registry:crates-io".to_owned(),
                package: PACKAGE.to_owned(),
                revision: REVISION.to_owned(),
                revision_kind: "semver".to_owned(),
                refs: vec!["latest".to_owned(), "stable".to_owned()],
                generation: Self::GENERATION,
                graph_prefix_uri: graph_prefix.clone(),
                graph_manifest,
                source_sidecar,
            }],
        };
        registry
            .validate()
            .context("validate Parquet fixture registry")?;

        let capacity = root_bytes.len() as u64
            + declared_members
                .values()
                .map(|body| body.len() as u64)
                .sum::<u64>();
        let cache = ArtifactCache::new(root.join("cache"), capacity, Arc::new(fetcher.clone()))
            .map_err(|error| anyhow::anyhow!(error))?;

        Ok(Self {
            registry,
            cache,
            fetcher,
            graph_prefix,
            declared_members,
            root,
        })
    }

    fn root_uri(&self) -> String {
        format!("{}{SILVER_MANIFEST_FILENAME}", self.graph_prefix)
    }

    fn expand_cache_capacity(&mut self, bundle_count: u64) -> Result<()> {
        let root_bytes = silver_root_for_members(&self.declared_members)?;
        let one_bundle = root_bytes.len() as u64
            + self
                .declared_members
                .values()
                .map(|body| body.len() as u64)
                .sum::<u64>();
        self.cache = ArtifactCache::new(
            self.root.join(format!("cache-{bundle_count}")),
            one_bundle * bundle_count,
            Arc::new(self.fetcher.clone()),
        )
        .map_err(|error| anyhow::anyhow!(error))?;
        Ok(())
    }

    fn member_uri(&self, path: &str) -> String {
        format!("{}{path}", self.graph_prefix)
    }

    fn remove_object(&self, uri: &str) {
        self.fetcher
            .objects
            .lock()
            .expect("Parquet test object mutex should not be poisoned")
            .remove(uri);
    }

    fn replace_object(&self, uri: &str, body: Vec<u8>) {
        self.fetcher
            .objects
            .lock()
            .expect("Parquet test object mutex should not be poisoned")
            .insert(uri.to_owned(), body);
    }

    fn replace_silver_root(&mut self, body: Vec<u8>) {
        self.replace_object(&self.root_uri(), body.clone());
        self.registry.packages[0].graph_manifest = artifact_ref(self.root_uri(), &body);
    }

    fn remove_declared_member(&mut self, path: &str) -> Result<()> {
        self.declared_members.remove(path);
        let root = silver_root_for_members(&self.declared_members)?;
        self.replace_silver_root(root);
        Ok(())
    }

    fn replace_source_sidecar(&mut self, row: Option<(&str, &str, &str)>) -> Result<()> {
        let replacement = self.root.join("source_files.parquet");
        write_source_sidecar_fixture(&replacement, row)?;
        let body = fs::read(&replacement).context("read replacement source sidecar")?;
        self.declared_members
            .insert("source_files.parquet".to_owned(), body.clone());
        self.replace_object(&self.member_uri("source_files.parquet"), body.clone());
        self.registry.packages[0].source_sidecar =
            artifact_ref(self.member_uri("source_files.parquet"), &body);
        let root = silver_root_for_members(&self.declared_members)?;
        self.replace_silver_root(root);
        Ok(())
    }
}

impl Drop for ParquetBackendFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn covered_range_bytes(ranges: &[Range<u64>]) -> u64 {
    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut covered = 0_u64;
    let mut current: Option<Range<u64>> = None;
    for range in ranges {
        match &mut current {
            Some(active) if range.start <= active.end => active.end = active.end.max(range.end),
            Some(active) => {
                covered += active.end - active.start;
                current = Some(range);
            }
            None => current = Some(range),
        }
    }
    covered + current.map_or(0, |range| range.end - range.start)
}

fn find_cached_file(root: &std::path::Path, name: &str) -> Result<PathBuf> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("read cache directory {}", directory.display()))?
        {
            let entry = entry.context("read cache directory entry")?;
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.file_name().is_some_and(|candidate| candidate == name) {
                return Ok(path);
            }
        }
    }
    anyhow::bail!(
        "cached file {name:?} was not materialized below {}",
        root.display()
    )
}

async fn call_parquet_backend(
    fixture: &ParquetBackendFixture,
    name: &str,
    args: &Value,
) -> Result<Value, McpHandlerError> {
    let backend = CodeBackend::new(fixture.registry.clone(), fixture.cache.clone())
        .map_err(|error| retryable_backend_error(error.code()))?;
    handle_tool_with_code_backend(name, args, &backend).await
}

fn retryable_backend_error(code: &'static str) -> McpHandlerError {
    McpHandlerError::Internal(format!(
        "code backend temporarily unavailable: {code} (retryable=true)"
    ))
}

fn assert_retryable_backend_error(result: Result<Value, McpHandlerError>, expected_code: &str) {
    let error = result.expect_err("invalid immutable artifact must fail closed");
    assert_eq!(error.json_rpc_code(), -32603);
    let message = error.to_string();
    assert!(
        message.contains(expected_code),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("retryable=true"),
        "unexpected error: {message}"
    );
    assert!(!message.contains("s3://"), "artifact URI leaked: {message}");
}

fn silver_root_for_members(members: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut files = members
        .iter()
        .map(|(path, body)| SilverManifestFile {
            path: path.clone(),
            size_bytes: body.len() as u64,
            etag: format!("etag-{path}"),
            sha256: sha256_bytes(body),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(serde_json::to_vec(&SilverManifest {
        schema_hash: "sha256:test-schema".to_owned(),
        files,
    })?)
}

fn artifact_ref(uri: String, body: &[u8]) -> ArtifactRef {
    ArtifactRef {
        uri,
        sha256: sha256_bytes(body),
        bytes: body.len() as u64,
    }
}

fn sha256_bytes(body: &[u8]) -> String {
    format!("{:x}", Sha256::digest(body))
}

fn write_graph_compatible_parquet_fixture(graph_dir: &std::path::Path) -> Result<()> {
    let nodes_path = escape_sql_literal(&graph_dir.join("nodes.parquet").display().to_string());
    let edges_path = escape_sql_literal(&graph_dir.join("edges.parquet").display().to_string());
    let unresolved_edges_path = escape_sql_literal(
        &graph_dir
            .join("edges_unresolved.parquet")
            .display()
            .to_string(),
    );
    let files_path = escape_sql_literal(&graph_dir.join("files.parquet").display().to_string());
    let file_manifests_path = escape_sql_literal(
        &graph_dir
            .join("file_manifests.parquet")
            .display()
            .to_string(),
    );
    let alpha_byte_range = byte_range_for_lines(PARQUET_SOURCE, 1, 4)?;
    let beta_byte_range = byte_range_for_lines(PARQUET_SOURCE, 6, 7)?;
    let content_oid = PARQUET_SOURCE_OID.to_owned();
    let conn = Connection::open_in_memory().context("open Parquet fixture DuckDB")?;
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE nodes (
            stable_symbol_id VARCHAR NOT NULL,
            node_id BIGINT NOT NULL,
            file_path VARCHAR NOT NULL,
            byte_range_start BIGINT NOT NULL,
            byte_range_end BIGINT NOT NULL,
            line_start INTEGER NOT NULL,
            line_end INTEGER NOT NULL,
            entity_name VARCHAR NOT NULL,
            qualified_name VARCHAR NOT NULL,
            symbol_kind VARCHAR NOT NULL,
            anchor_hash VARCHAR NOT NULL,
            enclosing_scope VARCHAR
        );
        INSERT INTO nodes VALUES
            ('aaaaaaaaaaaaaaaa', 1, 'src/lib.rs', {alpha_start}, {alpha_end}, 1, 4,
             'alpha', 'demo::alpha', 'function', 'anchor-alpha', NULL),
            ('bbbbbbbbbbbbbbbb', 2, 'src/lib.rs', {beta_start}, {beta_end}, 6, 7,
             'beta', 'demo::beta', 'function', 'anchor-beta', NULL),
            ('1111111111111111', 3, 'src/lib.rs', 0, 1, 1, 1,
             'overloaded', 'demo::one::overloaded', 'function', 'anchor-overload-one', NULL),
            ('2222222222222222', 4, 'src/lib.rs', 0, 1, 1, 1,
             'overloaded', 'demo::two::overloaded', 'function', 'anchor-overload-two', NULL);
        COPY nodes TO '{nodes_path}' (FORMAT PARQUET, COMPRESSION ZSTD);

        CREATE TABLE edges (
            source_stable_id VARCHAR NOT NULL,
            target_stable_id VARCHAR NOT NULL,
            src_id BIGINT NOT NULL,
            dst_id BIGINT NOT NULL,
            target_label VARCHAR,
            relation VARCHAR NOT NULL,
            confidence VARCHAR NOT NULL,
            confidence_score FLOAT NOT NULL,
            edge_kind VARCHAR,
            bind_method VARCHAR,
            import_path VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );
        INSERT INTO edges VALUES
            ('aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 1, 2, NULL,
             'calls', 'syntax_exact', 1.0, 'calls', NULL, NULL, NULL, NULL),
            ('aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 1, 2, NULL,
             'calls', 'heuristic', 1.0, 'calls_dyn', NULL, NULL, NULL, NULL),
            ('aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 1, 2, NULL,
             'references', 'syntax_exact', 1.0, 'references_hof', NULL, NULL, NULL, NULL),
            ('aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 1, 2, NULL,
             'references', 'syntax_exact', 1.0, 'references_other', NULL, NULL, NULL, NULL);
        COPY edges TO '{edges_path}' (FORMAT PARQUET, COMPRESSION ZSTD);

        CREATE TABLE edges_unresolved (
            source_stable_id VARCHAR NOT NULL,
            src_id BIGINT NOT NULL,
            target_label VARCHAR,
            relation VARCHAR NOT NULL,
            confidence VARCHAR NOT NULL,
            confidence_score FLOAT NOT NULL,
            edge_kind VARCHAR,
            bind_method VARCHAR,
            import_path VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );
        INSERT INTO edges_unresolved VALUES
            ('aaaaaaaaaaaaaaaa', 1, 'external::Thing', 'calls', 'syntax_exact', 1.0,
             'calls', NULL, NULL, NULL, NULL);
        COPY edges_unresolved TO '{unresolved_edges_path}' (FORMAT PARQUET, COMPRESSION ZSTD);

        CREATE TABLE files (
            stable_file_id VARCHAR NOT NULL,
            node_id BIGINT NOT NULL,
            file_path VARCHAR NOT NULL
        );
        INSERT INTO files VALUES ('file-demo-lib', 5, 'src/lib.rs');
        COPY files TO '{files_path}' (FORMAT PARQUET, COMPRESSION ZSTD);

        CREATE TABLE file_manifests (
            stable_file_id VARCHAR NOT NULL,
            path VARCHAR NOT NULL,
            content_oid VARCHAR NOT NULL,
            node_ids BIGINT[] NOT NULL
        );
        INSERT INTO file_manifests VALUES
            ('file-demo-lib', 'src/lib.rs', '{content_oid}', [1, 2, 3, 4]::BIGINT[]);
        COPY file_manifests TO '{file_manifests_path}' (FORMAT PARQUET, COMPRESSION ZSTD);
        "#,
        alpha_start = alpha_byte_range[0],
        alpha_end = alpha_byte_range[1],
        beta_start = beta_byte_range[0],
        beta_end = beta_byte_range[1],
    ))
    .context("write graph-compatible Parquet fixture")?;

    write_source_sidecar_fixture(
        &graph_dir.join("source_files.parquet"),
        Some(("src/lib.rs", &content_oid, PARQUET_SOURCE)),
    )?;

    fs::write(
        graph_dir.join("manifest.json"),
        serde_json::to_vec(&json!({
            "graph_index_version": "test-temporal-v1",
            "schema_version": "test-schema-v1",
            "manifest_version": "test-manifest-v1",
            "graph_content_hash": "test-graph-content-hash",
            "indexed_commit_oid": null,
            "extractor_version": "test-extractor",
            "complete": true,
            "row_counts": {
                "nodes": 4,
                "edges": 4,
                "edges_by_dst": null,
                "edges_unresolved": 1,
                "files": 1,
                "file_manifests": 1,
                "tombstones": 0,
                "commits": 0,
                "symbol_snapshots": 0,
                "temporal_edges": 0,
                "diagnostics": 0
            },
            "sidecar_complete": false,
            "sidecar_row_counts": {
                "section_bodies": 0,
                "code_symbols": 0
            },
            "parquet_writer": {
                "compression": "zstd-3",
                "row_group_size": 16384
            },
            "edges_by_dst_present": false,
            "temporal_shards": []
        }))?,
    )
    .context("write graph fixture manifest")?;
    Ok(())
}

fn write_source_sidecar_fixture(
    path: &std::path::Path,
    row: Option<(&str, &str, &str)>,
) -> Result<()> {
    let conn = Connection::open_in_memory().context("open source sidecar fixture DuckDB")?;
    conn.execute_batch(
        "CREATE TABLE source_sidecar (\
         file_path VARCHAR NOT NULL, \
         content_oid VARCHAR NOT NULL, \
         source_text VARCHAR NOT NULL)",
    )?;
    if let Some((file_path, content_oid, source_text)) = row {
        conn.execute(
            "INSERT INTO source_sidecar VALUES ($1, $2, $3)",
            params![file_path, content_oid, source_text],
        )?;
    }
    let path = escape_sql_literal(&path.display().to_string());
    conn.execute_batch(&format!(
        "COPY source_sidecar TO '{path}' (FORMAT PARQUET, COMPRESSION ZSTD)"
    ))?;
    Ok(())
}

fn byte_range_for_lines(source: &str, line_start: usize, line_end: usize) -> Result<[usize; 2]> {
    let mut starts = vec![0];
    starts.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    let start = *starts
        .get(line_start.saturating_sub(1))
        .context("fixture line start")?;
    let end = starts.get(line_end).copied().unwrap_or(source.len());
    Ok([start, end])
}

fn resolved_edge_row(direction: &str, relation: &str, edge_kind: &str, confidence: &str) -> Value {
    let candidate = if direction == "caller" {
        json!({
            "selector": "pkg:demo@1.0.0::demo::alpha",
            "uri": "pkg-symbol://registry:crates-io/demo/1.0.0/aaaaaaaaaaaaaaaa",
            "id": "aaaaaaaaaaaaaaaa",
            "stable_symbol_id": "aaaaaaaaaaaaaaaa",
            "source": "registry:crates-io",
            "package": PACKAGE,
            "revision": REVISION,
            "entity_name": "alpha",
            "qualified_name": "demo::alpha",
            "file_path": "src/lib.rs",
            "line_range": [1, 4],
            "symbol_kind": "function",
            "enclosing_scope": null
        })
    } else {
        json!({
            "selector": "pkg:demo@1.0.0::demo::beta",
            "uri": "pkg-symbol://registry:crates-io/demo/1.0.0/bbbbbbbbbbbbbbbb",
            "id": "bbbbbbbbbbbbbbbb",
            "stable_symbol_id": "bbbbbbbbbbbbbbbb",
            "source": "registry:crates-io",
            "package": PACKAGE,
            "revision": REVISION,
            "entity_name": "beta",
            "qualified_name": "demo::beta",
            "file_path": "src/lib.rs",
            "line_range": [6, 7],
            "symbol_kind": "function",
            "enclosing_scope": null
        })
    };
    let edge = json!({
            "source_stable_id": "aaaaaaaaaaaaaaaa",
            "target_stable_id": "bbbbbbbbbbbbbbbb",
            "target_label": null,
            "target_package": null,
            "relation": relation,
            "edge_kind": edge_kind,
            "confidence": confidence,
            "confidence_score": 1.0,
            "bind_method": null,
            "receiver_text": null,
            "scope_text": null
    });
    if direction == "caller" {
        json!({ "caller": candidate, "edge": edge, "resolved": true })
    } else {
        json!({ "callee": candidate, "edge": edge, "resolved": true })
    }
}

fn unresolved_callee_row() -> Value {
    json!({
        "callee": null,
        "edge": {
            "source_stable_id": "aaaaaaaaaaaaaaaa",
            "target_stable_id": null,
            "target_label": "external::Thing",
            "target_package": null,
            "relation": "calls",
            "edge_kind": "calls",
            "confidence": "syntax_exact",
            "confidence_score": 1.0,
            "bind_method": null,
            "receiver_text": null,
            "scope_text": null
        },
        "resolved": false
    })
}

#[tokio::test]
async fn external_catalog_selects_level_by_coordinates_and_pages() -> Result<()> {
    let fixture = McpFixture::new("catalog-levels")?;

    let packages = handle_tool(
        "external_catalog",
        &json!({ "limit": 1 }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(packages["level"], "packages");
    assert_eq!(packages["total_matches"], 2);
    assert_eq!(packages["truncated"], true);
    assert!(packages["next_cursor"].is_string());
    assert_eq!(packages["catalog_generation"], 9);
    assert_eq!(packages["rows"][0]["package"], "demo");
    assert_eq!(packages["rows"][0]["latest_revision"], REVISION);

    let package_page_2 = handle_tool(
        "external_catalog",
        &json!({
            "limit": 1,
            "cursor": packages["next_cursor"].as_str().context("package cursor")?
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(package_page_2["truncated"], false);
    assert_eq!(package_page_2["rows"][0]["package"], "zebra");

    let revisions = handle_tool(
        "external_catalog",
        &json!({ "package": PACKAGE }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(revisions["level"], "revisions");
    assert_eq!(revisions["total_matches"], 2);
    assert!(revisions["rows"]
        .as_array()
        .context("revision rows")?
        .iter()
        .any(|row| {
            row["revision"] == REVISION
                && row["refs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|reference| reference["ref_name"] == "latest")
        }));

    let root_tree = handle_tool(
        "external_catalog",
        &json!({
            "package": PACKAGE,
            "ref": "latest"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(root_tree["level"], "tree");
    assert!(root_tree["rows"]
        .as_array()
        .context("tree rows")?
        .iter()
        .any(|row| row["name"] == "src" && row["kind"] == "dir"));

    let nested_tree = handle_tool(
        "external_catalog",
        &json!({
            "package": PACKAGE,
            "ref": "latest",
            "path": "src"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(nested_tree["level"], "tree");
    assert!(nested_tree["rows"]
        .as_array()
        .context("nested tree rows")?
        .iter()
        .any(|row| row["path"] == "src/lib.rs" && row["kind"] == "file"));

    let symbols = handle_tool(
        "external_catalog",
        &json!({
            "package": PACKAGE,
            "ref": "latest",
            "path": "src/lib.rs",
            "name_filter": "bet"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(symbols["level"], "symbols");
    assert_eq!(symbols["total_matches"], 1);
    assert_eq!(symbols["rows"][0]["entity_name"], "beta");
    assert_eq!(symbols["rows"][0]["selector"], "pkg:demo@1.0.0::demo::beta");
    assert!(symbols["rows"][0]["next"]
        .as_array()
        .context("symbol next hints")?
        .iter()
        .any(|entry| entry["tool"] == "external_code_read"));
    Ok(())
}

#[tokio::test]
async fn external_catalog_rejects_revision_and_ref_together() -> Result<()> {
    let fixture = McpFixture::new("catalog-revision-ref")?;

    let error = handle_tool(
        "external_catalog",
        &json!({
            "package": PACKAGE,
            "revision": REVISION,
            "ref": "latest"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await
    .expect_err("revision and ref must be mutually exclusive");

    assert!(matches!(error, McpHandlerError::InvalidParams(_)));
    assert!(
        error.to_string().contains("revision") && error.to_string().contains("ref"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn serving_uses_frozen_snapshot_when_postgres_catalog_is_unreachable() -> Result<()> {
    let fixture = FrozenServingFixture::new("serving-frozen-snapshot")?;
    let snapshot_conn = connect_frozen_snapshot(&fixture.snapshot_path, &fixture.data_path)
        .context("serving should attach the frozen snapshot without a live catalog backend")?;
    let catalog = CatalogResolver::from_connection(snapshot_conn);

    let response = handle_tool(
        "external_code_search",
        &json!({
            "query": "bet",
            "package": PACKAGE,
            "limit": 20
        }),
        catalog.connection(),
        &catalog,
    )
    .await?;

    assert_eq!(response["total_matches"], 1);
    assert_eq!(
        response["candidates"][0]["stable_symbol_id"],
        "bbbbbbbbbbbbbbbb"
    );
    assert_eq!(response["candidates"][0]["revision"], REVISION);

    let source = handle_tool(
        "external_code_read",
        &json!({
            "selector": "pkg:demo@latest::demo::beta",
            "context_lines": 0
        }),
        catalog.connection(),
        &catalog,
    )
    .await?;

    assert_eq!(source["file_path"], "src/lib.rs");
    assert_eq!(source["source"], "pub fn beta() {\n}\n");

    let write_error = catalog
        .connection()
        .execute(
            "INSERT INTO gold.refs VALUES ('registry:crates-io', 'demo', 'mutable', '1.0.0', CURRENT_TIMESTAMP)",
            [],
        )
        .expect_err("serving must attach the frozen snapshot read-only");
    assert!(
        write_error.to_string().contains("read-only")
            || write_error.to_string().contains("read only"),
        "unexpected write error: {write_error}"
    );

    let _postgres_catalog_that_must_not_be_used =
        "ducklake:postgresql://127.0.0.1:1/spur_context?connect_timeout=1";
    Ok(())
}

#[tokio::test]
async fn external_code_read_resolves_selector_ref_and_returns_source() -> Result<()> {
    let fixture = McpFixture::new("read")?;

    let response = handle_tool(
        "external_code_read",
        &json!({
            "selector": "pkg:demo@latest::demo::beta",
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(response["stable_symbol_id"], "bbbbbbbbbbbbbbbb");
    assert_eq!(response["file_path"], "src/lib.rs");
    assert_eq!(response["line_range"], json!([6, 7]));
    assert_eq!(response["source"], "pub fn beta() {\n}\n");
    Ok(())
}

#[tokio::test]
async fn external_code_read_resolves_non_default_source() -> Result<()> {
    let fixture = McpFixture::new("read-git-source")?;
    move_fixture_to_source(&fixture, GIT_SOURCE)?;

    let response = handle_tool(
        "external_code_read",
        &json!({
            "source": GIT_SOURCE,
            "selector": "pkg:demo@latest::demo::beta",
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(response["stable_symbol_id"], "bbbbbbbbbbbbbbbb");
    assert_eq!(response["package_source"], GIT_SOURCE);
    assert_eq!(response["source"], "pub fn beta() {\n}\n");
    Ok(())
}

#[tokio::test]
async fn external_code_read_reports_missing_indexed_source_text() -> Result<()> {
    let fixture = McpFixture::new("read-missing-source-text")?;
    fixture
        .conn
        .execute("DELETE FROM files WHERE file_path = 'src/lib.rs'", [])?;

    let search = handle_tool(
        "external_code_search",
        &json!({
            "query": "bet",
            "package": PACKAGE,
            "symbol_kind": "function",
            "limit": 20
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(search["total_matches"], 1);
    assert_eq!(
        search["candidates"][0]["stable_symbol_id"],
        "bbbbbbbbbbbbbbbb"
    );

    let error = handle_tool(
        "external_code_read",
        &json!({
            "selector": "pkg:demo@latest::demo::beta",
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await
    .expect_err("missing source text should not look like a missing symbol");

    assert!(matches!(error, McpHandlerError::Internal(_)));
    assert!(
        error.to_string().contains("source text is not indexed"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn external_code_read_accepts_search_uri_for_ambiguous_symbol_name() -> Result<()> {
    let fixture = McpFixture::new("read-search-uri")?;
    move_fixture_to_source(&fixture, GIT_SOURCE)?;

    let search = handle_tool(
        "external_code_search",
        &json!({
            "source": GIT_SOURCE,
            "query": "Buffer",
            "package": PACKAGE,
            "symbol_kind": "struct",
            "limit": 10
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(search["total_matches"], 1);
    assert_eq!(
        search["candidates"][0]["selector"],
        "pkg:demo@1.0.0::Buffer"
    );
    let uri = search["candidates"][0]["uri"]
        .as_str()
        .context("search candidate URI")?;

    let response = handle_tool(
        "external_code_read",
        &json!({
            "selector": uri,
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(response["stable_symbol_id"], "bufferstruct0001");
    assert_eq!(response["package_source"], GIT_SOURCE);
    assert_eq!(
        response["source"],
        "pub struct Buffer {\n    bytes: [u8; 20],\n}\n"
    );
    Ok(())
}

#[tokio::test]
async fn external_call_graph_accepts_search_uri_selectors() -> Result<()> {
    let fixture = McpFixture::new("call-graph-search-uri")?;

    let beta_uri = search_uri(&fixture, "bet", Some("function")).await?;
    let callers = handle_tool(
        "external_code_callers",
        &json!({
            "selector": beta_uri,
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(
        callers["callers"]
            .as_array()
            .context("callers array")?
            .len(),
        3
    );
    assert_eq!(callers["counts_by_kind"]["unresolved"], 1);

    let alpha_uri = search_uri(&fixture, "alpha", Some("function")).await?;
    let callees = handle_tool(
        "external_code_callees",
        &json!({
            "selector": alpha_uri,
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(
        callees["callees"]
            .as_array()
            .context("callees array")?
            .len(),
        2
    );
    assert_eq!(
        callees["callees"][0]["callee"]["stable_symbol_id"],
        "bbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        callees["callees"][1]["edge"]["target_label"],
        "external::Thing"
    );
    Ok(())
}

#[tokio::test]
async fn external_code_callers_returns_resolved_and_unresolved_edges() -> Result<()> {
    let fixture = McpFixture::new("callers")?;

    let response = handle_tool(
        "external_code_callers",
        &json!({
            "selector": "pkg:demo@latest::demo::beta",
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(
        response["callers"]
            .as_array()
            .context("callers array")?
            .len(),
        3
    );
    assert_eq!(response["counts_by_kind"]["calls"], 1);
    assert_eq!(response["counts_by_kind"]["calls_dyn"], 1);
    assert_eq!(response["counts_by_kind"]["references_hof"], 1);
    assert_eq!(response["counts_by_kind"]["unresolved"], 1);
    assert_eq!(response["unresolved_sample"], json!(["demo::beta"]));
    Ok(())
}

#[tokio::test]
async fn external_code_callees_returns_resolved_and_unresolved_edges() -> Result<()> {
    let fixture = McpFixture::new("callees")?;

    let response = handle_tool(
        "external_code_callees",
        &json!({
            "selector": "pkg:demo@latest::demo::alpha",
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(
        response["callees"]
            .as_array()
            .context("callees array")?
            .len(),
        2
    );
    assert_eq!(
        response["callees"][0]["callee"]["stable_symbol_id"],
        "bbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        response["callees"][1]["edge"]["target_label"],
        "external::Thing"
    );
    assert_eq!(response["counts_by_kind"]["calls"], 2);
    assert_eq!(response["counts_by_kind"]["unresolved"], 1);
    Ok(())
}

#[tokio::test]
async fn external_knowledge_context_resolves_ref_and_returns_evidence_pack() -> Result<()> {
    let fixture = McpFixture::new("knowledge")?;

    let response = handle_tool(
        "external_knowledge_context",
        &json!({
            "query": "parse config loader",
            "package": PACKAGE,
            "ref": "latest",
            "scope": "all",
            "limit": 8
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(response["answerable"], true);
    assert_eq!(
        response["catalog_generation"], 9,
        "Knowledge must report the frozen catalog's global generation, not the selected package row's older generation"
    );
    assert_eq!(response["graph_content_hash"], "fixture-hash");
    assert!(response["primary_evidence"]
        .as_array()
        .context("primary_evidence array")?
        .iter()
        .any(|evidence| evidence["stable_symbol_id"] == "cccccccccccccccc"));
    assert!(response["supporting_docs"]
        .as_array()
        .context("supporting_docs array")?
        .iter()
        .any(|evidence| evidence["stable_symbol_id"] == "doc-parse"));
    assert!(response["confidence"].is_string());
    Ok(())
}

#[tokio::test]
async fn external_knowledge_context_preserves_identity_for_duplicate_qualified_names() -> Result<()>
{
    let fixture = McpFixture::new("knowledge-duplicate-qualified-name")?;
    duplicate_parse_config_loader(&fixture.conn)?;

    let response = handle_tool(
        "external_knowledge_context",
        &json!({
            "query": "parse config loader",
            "package": PACKAGE,
            "revision": REVISION,
            "scope": "code",
            "limit": 8
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    let evidence = response["primary_evidence"]
        .as_array()
        .context("primary_evidence array")?
        .iter()
        .filter(|evidence| evidence["title"] == "parse_config_loader")
        .collect::<Vec<_>>();
    assert_eq!(evidence.len(), 2);

    let mut ids = evidence
        .iter()
        .map(|evidence| {
            evidence["stable_symbol_id"]
                .as_str()
                .context("raw stable symbol ID")
        })
        .collect::<Result<Vec<_>>>()?;
    ids.sort_unstable();
    assert_eq!(ids, ["cccccccccccccccc", "cccccccccccccccd"]);

    for evidence in evidence {
        assert_eq!(
            evidence["selector"],
            "pkg:demo@1.0.0::demo::parse_config_loader"
        );
        let uri = evidence["uri"].as_str().context("canonical evidence URI")?;
        assert!(uri.starts_with("pkg-symbol://registry:crates-io/demo/1.0.0/"));

        let next = evidence["next"].as_array().context("next actions")?;
        assert_eq!(next.len(), 3);
        assert!(next.iter().all(|action| action["selector"] == uri));

        let read = handle_tool(
            "external_code_read",
            &json!({ "selector": uri }),
            &fixture.conn,
            &fixture.catalog,
        )
        .await?;
        assert_eq!(read["stable_symbol_id"], evidence["stable_symbol_id"]);

        for tool in ["external_code_callers", "external_code_callees"] {
            handle_tool(
                tool,
                &json!({ "selector": uri }),
                &fixture.conn,
                &fixture.catalog,
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn external_code_tools_classify_ambiguous_qualified_selectors_as_invalid_params() -> Result<()>
{
    let fixture = McpFixture::new("ambiguous-qualified-selector")?;
    duplicate_parse_config_loader(&fixture.conn)?;

    for tool in [
        "external_code_read",
        "external_code_callers",
        "external_code_callees",
    ] {
        let error = handle_tool(
            tool,
            &json!({
                "selector": "pkg:demo@1.0.0::demo::parse_config_loader"
            }),
            &fixture.conn,
            &fixture.catalog,
        )
        .await
        .expect_err("ambiguous qualified selector should be rejected");

        assert!(matches!(error, McpHandlerError::InvalidParams(_)));
        assert!(error.to_string().contains("matched 2 symbols"));
    }
    Ok(())
}

#[tokio::test]
async fn external_knowledge_context_uses_precomputed_query_vector_for_hybrid_hits() -> Result<()> {
    let fixture = McpFixture::new("knowledge-vector")?;

    let response = handle_tool(
        "external_knowledge_context",
        &json!({
            "query": "unmatched lexical query",
            "package": PACKAGE,
            "ref": "latest",
            "scope": "code",
            "limit": 3,
            "query_vec": unit_vector(0)
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    let top = response["primary_evidence"][0]
        .as_object()
        .context("expected vector primary evidence")?;
    assert_eq!(top["stable_symbol_id"], "9999999999999999");
    assert_eq!(top["grounding"], "hybrid-code");
    assert!(response["supporting_docs"]
        .as_array()
        .context("supporting_docs array")?
        .is_empty());
    Ok(())
}

#[tokio::test]
async fn external_tools_support_multi_round_agent_eval_flow() -> Result<()> {
    let fixture = McpFixture::new("multi-round-eval")?;

    let pack = handle_tool(
        "external_knowledge_context",
        &json!({
            "query": "alpha beta external thing",
            "package": PACKAGE,
            "ref": "latest",
            "scope": "all",
            "limit": 8
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    let alpha = pack["primary_evidence"]
        .as_array()
        .context("primary_evidence array")?
        .iter()
        .find(|evidence| evidence["stable_symbol_id"] == "aaaaaaaaaaaaaaaa")
        .context("alpha evidence")?;
    assert_eq!(alpha["selector"], "pkg:demo@1.0.0::demo::alpha");
    let alpha_selector = alpha["uri"].as_str().context("alpha stable URI")?;
    assert_next_tools(
        alpha,
        alpha_selector,
        [
            "external_code_read",
            "external_code_callers",
            "external_code_callees",
        ],
    )?;

    let alpha_source = handle_tool(
        "external_code_read",
        &json!({
            "selector": alpha_selector,
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert!(alpha_source["source"]
        .as_str()
        .context("alpha source")?
        .contains("external::Thing::new();"));

    let alpha_callees = handle_tool(
        "external_code_callees",
        &json!({
            "selector": alpha_selector,
            "include_unresolved": false
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(alpha_callees["counts_by_kind"]["calls"], 1);
    assert_eq!(alpha_callees["counts_by_kind"]["unresolved"], 0);
    assert_eq!(alpha_callees["unresolved_sample"], json!([]));
    assert_eq!(
        alpha_callees["callees"]
            .as_array()
            .context("alpha callees")?
            .len(),
        1
    );

    let alpha_callees_with_unresolved = handle_tool(
        "external_code_callees",
        &json!({
            "selector": alpha_selector,
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(alpha_callees_with_unresolved["counts_by_kind"]["calls"], 2);
    assert_eq!(
        alpha_callees_with_unresolved["unresolved_sample"],
        json!(["external::Thing"])
    );
    assert_eq!(
        alpha_callees_with_unresolved["callees"]
            .as_array()
            .context("alpha callees with unresolved")?
            .len(),
        2
    );

    let beta_uri = search_uri(&fixture, "bet", Some("function")).await?;
    let beta_source = handle_tool(
        "external_code_read",
        &json!({
            "selector": beta_uri,
            "context_lines": 0
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(beta_source["stable_symbol_id"], "bbbbbbbbbbbbbbbb");

    let beta_callers = handle_tool(
        "external_code_callers",
        &json!({
            "selector": beta_source["selector"],
            "include_unresolved": true
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    assert_eq!(beta_callers["counts_by_kind"]["calls"], 1);
    assert_eq!(beta_callers["counts_by_kind"]["calls_dyn"], 1);
    assert_eq!(beta_callers["counts_by_kind"]["references_hof"], 1);
    assert_eq!(beta_callers["counts_by_kind"]["unresolved"], 1);
    assert_eq!(
        beta_callers["callers"]
            .as_array()
            .context("beta callers")?
            .len(),
        3
    );
    Ok(())
}

#[tokio::test]
async fn handler_reports_unknown_tool_missing_args_and_missing_package() -> Result<()> {
    let fixture = McpFixture::new("errors")?;

    let unknown = handle_tool("missing_tool", &json!({}), &fixture.conn, &fixture.catalog)
        .await
        .expect_err("unknown tool should fail");
    assert!(matches!(unknown, McpHandlerError::InvalidParams(_)));

    let missing_args = handle_tool(
        "external_code_search",
        &json!({ "package": PACKAGE }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await
    .expect_err("missing query should fail");
    assert!(matches!(missing_args, McpHandlerError::InvalidParams(_)));

    let missing_package = handle_tool(
        "external_code_search",
        &json!({ "query": "beta", "package": "missing" }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await
    .expect_err("missing package should fail");
    assert!(matches!(missing_package, McpHandlerError::NotFound(_)));
    Ok(())
}

#[tokio::test]
async fn missing_serving_catalog_returns_empty_read_tool_results() -> Result<()> {
    let catalog = handle_tool_without_catalog("external_catalog", &json!({}))?;
    assert_eq!(catalog["level"], "packages");
    assert_eq!(catalog["total_matches"], 0);
    assert_eq!(catalog["truncated"], false);
    assert_eq!(catalog["rows"], json!([]));
    assert!(catalog["next_cursor"].is_null());
    assert!(catalog["catalog_generation"].is_null());

    let search = handle_tool_without_catalog(
        "external_code_search",
        &json!({
            "query": "alpha",
            "package": PACKAGE,
            "limit": 20
        }),
    )?;
    assert_eq!(search["total_matches"], 0);
    assert_eq!(search["truncated"], false);
    assert_eq!(search["candidates"], json!([]));

    let source = handle_tool_without_catalog(
        "external_code_read",
        &json!({
            "selector": "pkg:demo@latest::demo::alpha",
            "context_lines": 0
        }),
    )?;
    assert!(source.is_null());

    let callers = handle_tool_without_catalog(
        "external_code_callers",
        &json!({
            "selector": "pkg:demo@latest::demo::alpha",
            "include_unresolved": true
        }),
    )?;
    assert_eq!(callers["callers"], json!([]));
    assert_eq!(callers["counts_by_kind"]["calls"], 0);
    assert_eq!(callers["unresolved_sample"], json!([]));

    let callees = handle_tool_without_catalog(
        "external_code_callees",
        &json!({
            "selector": "pkg:demo@latest::demo::alpha",
            "include_unresolved": true
        }),
    )?;
    assert_eq!(callees["callees"], json!([]));
    assert_eq!(callees["counts_by_kind"]["calls"], 0);
    assert_eq!(callees["unresolved_sample"], json!([]));

    let knowledge = handle_tool_without_catalog(
        "external_knowledge_context",
        &json!({
            "query": "how does alpha work",
            "package": PACKAGE,
            "limit": 8
        }),
    )?;
    assert_eq!(knowledge["answerable"], false);
    assert_eq!(knowledge["confidence"], "low");
    assert_eq!(knowledge["primary_evidence"], json!([]));
    assert_eq!(knowledge["supporting_docs"], json!([]));
    assert_eq!(knowledge["candidates"]["total"], 0);

    Ok(())
}

#[tokio::test]
async fn external_index_rejects_missing_source_url() -> Result<()> {
    let fixture = McpFixture::new("index-missing-source-url")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let error = route_index(
        &json!({
            "package": PACKAGE,
            "revision": REVISION,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-missing-source-url",
    )
    .await
    .expect_err("missing source_url should fail validation");

    assert!(matches!(error, McpHandlerError::InvalidParams(_)));
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_without_serving_catalog_enqueues_without_starting() -> Result<()> {
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index_without_catalog(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &jobs,
        &sfn,
        "caller-bootstrap",
    )
    .await?;

    // Enqueue path: job is queued, no execution started (drainer is a later
    // task).
    assert_eq!(response["status"], "queued");
    assert_eq!(response["job_id"], "job-1");
    assert_eq!(response["execution_arn"], Value::Null);
    assert_eq!(response["revision"], "main");
    assert_eq!(sfn.started_count(), 0);
    let stored = jobs.lookup_job_sync("job-1").context("created job")?;
    assert_eq!(stored.caller_id, "caller-bootstrap");
    assert_eq!(stored.execution_arn, None);
    assert_eq!(stored.owner_kind, Some(BacklogOwnerKind::Caller));
    assert_eq!(stored.owner_id.as_deref(), Some("caller-bootstrap"));
    Ok(())
}

#[test]
fn external_index_status_without_job_store_returns_failed_job_result() -> Result<()> {
    let response = handle_tool_without_catalog(
        "external_index_status",
        &json!({ "job_id": "job-without-lambda-routing" }),
    )?;

    assert_eq!(response["status"], "failed");
    assert_eq!(response["job_id"], "job-without-lambda-routing");
    assert_eq!(response["error"]["code"], "lambda_routing_required");
    assert_eq!(
        response["error"]["detail"],
        "external_index_status requires Lambda routing with a job store"
    );
    Ok(())
}

#[tokio::test]
async fn external_index_status_returns_not_found_for_unknown_job() -> Result<()> {
    let jobs = FakeJobStore::default();

    let response = route_index_status(&json!({ "job_id": "missing-job" }), &jobs, None).await?;

    assert_eq!(response, json!({ "status": "not_found" }));
    Ok(())
}

#[tokio::test]
async fn external_index_status_for_caller_hides_jobs_owned_by_other_callers() -> Result<()> {
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:other-caller", |record| {
        record.caller_id = "caller-a".to_owned();
    });
    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Succeeded,
        output: Some(json!({
            "snapshot_id": 777,
        })),
        error: None,
    }));

    let response = route_index_status_for_caller(
        &json!({ "job_id": job.job_id }),
        &jobs,
        Some(&checker),
        "caller-b",
    )
    .await?;

    assert_eq!(response, json!({ "status": "not_found" }));
    assert_eq!(checker.described_arns(), Vec::<String>::new());
    Ok(())
}

#[cfg(feature = "lambda")]
#[tokio::test]
async fn lambda_index_status_route_does_not_require_catalog_initialization() -> Result<()> {
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:lambda-status", |record| {
        record.caller_id = "caller-lambda-status".to_owned();
    });
    let job_id = job.job_id.clone();
    let checker = StubExecutionStatusChecker::new(None);

    let response = spur_context_service::lambda::route_index_status_control_plane(
        &json!({ "job_id": job_id.clone() }),
        &jobs,
        &checker,
        "caller-lambda-status",
    )
    .await?;

    assert_eq!(response["job_id"], job_id);
    assert_eq!(response["status"], "queued");
    assert_eq!(checker.described_arns(), Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn external_index_status_repairs_stale_succeeded_execution() -> Result<()> {
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:success", |record| {
        record.updated_at = "0".to_owned();
    });
    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Succeeded,
        output: Some(json!({
            "snapshot_id": 777,
            "rows_inserted": {
                "nodes": 5,
                "edges": 4
            }
        })),
        error: None,
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "complete");
    assert_eq!(response["snapshot_id"], 777);
    assert_eq!(response["row_counts"], json!({ "nodes": 5, "edges": 4 }));
    assert_eq!(response["execution_arn"], "arn:success");
    assert_eq!(checker.described_arns(), ["arn:success"]);
    Ok(())
}

#[tokio::test]
async fn external_index_status_reconciles_failed_execution() -> Result<()> {
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:failed", |record| {
        record.updated_at = "0".to_owned();
    });
    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Failed,
        output: None,
        error: Some("fetch: clone failed".to_owned()),
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "failed");
    assert_eq!(response["error"]["code"], "fetch");
    assert_eq!(response["error"]["detail"], "clone failed");
    assert_eq!(checker.described_arns(), ["arn:failed"]);
    Ok(())
}

#[tokio::test]
async fn external_index_status_without_checker_returns_stale_job() -> Result<()> {
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:stale", |record| {
        record.updated_at = "0".to_owned();
    });

    let response = route_index_status(&json!({ "job_id": job.job_id }), &jobs, None).await?;

    assert_eq!(response["status"], "queued");
    assert!(response.get("snapshot_id").is_none());
    assert!(response.get("error").is_none());
    Ok(())
}

#[tokio::test]
async fn external_index_status_returns_dynamodb_state_when_describe_execution_fails() -> Result<()>
{
    let jobs = FakeJobStore::default();
    let job = jobs.seed_queued_job("arn:transient", |record| {
        record.status = JobStatus::Running;
        record.stage = Some("building_graph".to_owned());
        record.updated_at = "0".to_owned();
    });
    let checker = StubExecutionStatusChecker::fail("temporary sfn outage");

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "running");
    assert_eq!(response["stage"], "building_graph");
    assert_eq!(response["execution_arn"], "arn:transient");
    assert_eq!(checker.described_arns(), ["arn:transient"]);
    Ok(())
}

// ─── Terminal release wiring (status-repair paths) ──────────────────────────
//
// `external_index_status` repairs stale jobs via `update_stale_job`. After the
// brain feedback, those repairs must:
//   1. Use `mark_*_and_release_running_quota` so owner/global running capacity
//      is freed after a stale succeeded/failed reconciliation.
//   2. Reconcile stale `Dispatching` jobs (pre-first-stage) in addition to
//      `Queued`/`Running`.
//   3. Repair terminal jobs that still hold a running token (the worker's
//      release conflicted) by releasing exactly once on the next poll.

fn seed_owner() -> BacklogOwner {
    BacklogOwner::caller("seed-owner")
}

#[tokio::test]
async fn status_repair_succeeded_releases_running_quota() -> Result<()> {
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:stale-success", JobStatus::Running, |_| {});
    assert_eq!(jobs.owner_running(&owner), 1, "running slot held");
    assert!(jobs.has_running_token(&job.job_id));

    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Succeeded,
        output: Some(json!({ "snapshot_id": 4242, "rows_inserted": { "nodes": 9 } })),
        error: None,
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "complete");
    assert_eq!(response["snapshot_id"], 4242);
    assert_eq!(checker.described_arns(), ["arn:stale-success"]);
    assert_eq!(jobs.owner_running(&owner), 0, "owner running released");
    assert!(
        !jobs.has_running_token(&job.job_id),
        "running token removed"
    );
    assert_eq!(jobs.global_running(), 0, "global running released");
    Ok(())
}

#[tokio::test]
async fn status_repair_failed_releases_running_quota() -> Result<()> {
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:stale-fail", JobStatus::Running, |_| {});
    assert_eq!(jobs.owner_running(&owner), 1);

    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Failed,
        output: None,
        error: Some("translate: duckdb error".to_owned()),
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "failed");
    assert_eq!(response["error"]["code"], "translate");
    assert_eq!(response["error"]["detail"], "duckdb error");
    assert_eq!(jobs.owner_running(&owner), 0, "owner running released");
    assert!(
        !jobs.has_running_token(&job.job_id),
        "running token removed"
    );
    assert_eq!(jobs.global_running(), 0, "global running released");
    Ok(())
}

#[tokio::test]
async fn status_repair_reconciles_stale_dispatching_job() -> Result<()> {
    // A dispatched job sits in `Dispatching` until the worker reports its first
    // stage. If Step Functions fails before that update, the job is stuck
    // holding running capacity. `update_stale_job` must reconcile it.
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:dispatching", JobStatus::Dispatching, |_| {});
    assert_eq!(jobs.owner_running(&owner), 1);
    assert!(jobs.has_running_token(&job.job_id));

    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Failed,
        output: None,
        error: Some("execution: timed out".to_owned()),
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "failed");
    assert_eq!(checker.described_arns(), ["arn:dispatching"]);
    assert_eq!(
        jobs.owner_running(&owner),
        0,
        "dispatching job releases running quota"
    );
    assert!(
        !jobs.has_running_token(&job.job_id),
        "running token removed"
    );
    Ok(())
}

#[tokio::test]
async fn status_repair_dispatching_to_succeeded_releases_running_quota() -> Result<()> {
    // Same dispatching path but reconciling to a succeeded outcome.
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:dispatching-ok", JobStatus::Dispatching, |_| {});

    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Succeeded,
        output: Some(json!({ "snapshot_id": 10, "rows_inserted": { "nodes": 1 } })),
        error: None,
    }));

    let response =
        route_index_status(&json!({ "job_id": job.job_id }), &jobs, Some(&checker)).await?;

    assert_eq!(response["status"], "complete");
    assert_eq!(response["snapshot_id"], 10);
    assert_eq!(jobs.owner_running(&owner), 0);
    assert!(!jobs.has_running_token(&job.job_id));
    Ok(())
}

#[tokio::test]
async fn status_poll_repairs_terminal_job_with_leftover_running_token() -> Result<()> {
    // Simulate the TransactionConflict scenario: the worker marked the job
    // complete via `mark_complete` (raw — terminal status recorded, dedupe
    // released) but the running-quota release conflicted, leaving the
    // `RUNNING#` token and owner/global counters in place. The next status
    // poll must release the leftover token exactly once.
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:terminal-leak", JobStatus::Running, |_| {});
    assert_eq!(jobs.owner_running(&owner), 1);

    // Record terminal status WITHOUT releasing running quota — mirrors the
    // worker's `mark_complete` succeeding but `release_running_quota` failing.
    jobs.mark_complete(&job.job_id, 555, json!({ "nodes": 3 }))
        .await
        .context("mark complete (raw)")?;
    assert_eq!(
        jobs.owner_running(&owner),
        1,
        "token still held after raw mark_complete"
    );
    assert!(jobs.has_running_token(&job.job_id));

    // No checker needed — the terminal-repair branch fires before the checker.
    let response = route_index_status(&json!({ "job_id": job.job_id }), &jobs, None).await?;

    assert_eq!(response["status"], "complete");
    assert_eq!(response["snapshot_id"], 555);
    assert_eq!(
        jobs.owner_running(&owner),
        0,
        "leftover token released by poll"
    );
    assert!(
        !jobs.has_running_token(&job.job_id),
        "running token removed"
    );
    assert_eq!(jobs.global_running(), 0, "global running released");
    Ok(())
}

#[tokio::test]
async fn status_poll_terminal_release_conflict_surfaces_for_retry() -> Result<()> {
    // Same leftover-token scenario, but the release transaction still
    // conflicts. The error must surface (not be swallowed as success) so the
    // next poll retries and the slot is eventually freed.
    let jobs = FakeJobStore::default();
    let owner = seed_owner();
    let job = jobs.seed_dispatched_job("arn:terminal-conflict", JobStatus::Running, |_| {});
    jobs.mark_complete(&job.job_id, 1, json!({})).await?;
    assert_eq!(jobs.owner_running(&owner), 1);

    jobs.set_fail_release(true);

    let error = route_index_status(&json!({ "job_id": job.job_id }), &jobs, None)
        .await
        .unwrap_err();

    match error {
        McpHandlerError::Internal(message) => {
            assert!(
                message.contains("terminal quota release repair"),
                "expected release-repair context in error, got: {message}"
            );
        }
        other => panic!("expected Internal error for release conflict, got {other:?}"),
    }
    // The token and counter must remain so the next poll can retry.
    assert_eq!(
        jobs.owner_running(&owner),
        1,
        "running quota must stay held on conflict"
    );
    assert!(
        jobs.has_running_token(&job.job_id),
        "token must remain for retry"
    );
    Ok(())
}

#[tokio::test]
async fn external_index_returns_complete_for_warm_catalog_hit() -> Result<()> {
    let fixture = McpFixture::new("index-warm")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": REVISION,
            "source_url": "https://warm-hit.invalid/example/demo",
            "source": "registry:crates-io"
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-warm",
    )
    .await?;

    assert_eq!(response["status"], "complete");
    assert_eq!(response["snapshot_id"], 100);
    assert_eq!(response["revision"], REVISION);
    assert_eq!(sfn.started_count(), 0);
    assert_eq!(jobs.job_count(), 0);
    assert_eq!(jobs.rate_count("caller-warm"), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_warm_lookup_releases_catalog_lock_before_await() -> Result<()> {
    let fixture = McpFixture::new("index-warm-lock-boundary")?;
    let catalog_lock = Mutex::new(&fixture.catalog);
    let args = json!({
        "package": "not-yet-indexed",
        "revision": "main",
        "source_url": SOURCE_URL,
        "source_kind": "git"
    });

    let warm_response = {
        let catalog = catalog_lock
            .lock()
            .expect("catalog lock should not be poisoned");
        route_index_warm_lookup(&args, *catalog)?
    };
    assert_eq!(warm_response, None);

    let lock_available_during_external_await = async {
        tokio::task::yield_now().await;
        catalog_lock.try_lock().is_ok()
    }
    .await;

    assert!(
        lock_available_during_external_await,
        "catalog mutex must be released before external_index performs I/O"
    );
    Ok(())
}

#[tokio::test]
async fn external_index_crates_io_aliases_share_the_canonical_warm_identity() -> Result<()> {
    let fixture = McpFixture::new("index-warm-crates-aliases")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    for source in [
        None,
        Some("crates.io"),
        Some("crates-io"),
        Some("registry:crates-io"),
    ] {
        let mut args = json!({
            "package": " DEMO ",
            "revision": " 1.0.0 ",
            "source_url": " HTTPS://CRATES.IO/api/v1/crates/Demo/1.0.0/download "
        });
        if let Some(source) = source {
            args["source"] = Value::String(source.to_owned());
        }

        let response = route_index(
            &args,
            &fixture.conn,
            &fixture.catalog,
            &jobs,
            &sfn,
            "caller-warm-aliases",
        )
        .await?;

        assert_eq!(response["status"], "complete", "source={source:?}");
        assert_eq!(response["snapshot_id"], 100);
    }

    assert_eq!(jobs.rate_count("caller-warm-aliases"), 0);
    assert_eq!(jobs.job_count(), 0);
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_explicit_crates_io_aliases_find_legacy_warm_rows() -> Result<()> {
    for alias in ["crates.io", "crates-io"] {
        let fixture = McpFixture::new(&format!("index-warm-legacy-{alias}"))?;
        fixture.catalog.connection().execute(
            "UPDATE package_catalog SET source = ? WHERE source = 'registry:crates-io'",
            params![alias],
        )?;
        fixture.catalog.connection().execute(
            "UPDATE refs SET source = ? WHERE source = 'registry:crates-io'",
            params![alias],
        )?;
        let jobs = FakeJobStore::default();
        let sfn = StubIndexExecutionStarter::default();

        let response = route_index(
            &json!({
                "package": "demo",
                "revision": REVISION,
                "source_url": CRATES_IO_SOURCE_URL,
                "source": format!(" {alias} ")
            }),
            &fixture.conn,
            &fixture.catalog,
            &jobs,
            &sfn,
            "caller-warm-legacy",
        )
        .await?;

        assert_eq!(response["status"], "complete", "alias={alias}");
        assert_eq!(response["snapshot_id"], 100, "alias={alias}");
        assert_eq!(jobs.rate_count("caller-warm-legacy"), 0, "alias={alias}");
        assert_eq!(jobs.job_count(), 0, "alias={alias}");
        assert_eq!(sfn.started_count(), 0, "alias={alias}");
    }
    Ok(())
}

#[tokio::test]
async fn external_index_cold_crates_io_request_stores_canonical_identity() -> Result<()> {
    let fixture = McpFixture::new("index-cold-crates-canonical")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": " Demo_Crate ",
            "revision": " 2.0.0 ",
            "source_url": " HTTPS://CRATES.IO:443/api/v1/crates/demo-crate/2.0.0/download?mirror=1#fetch ",
            "source": " crates.io "
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-cold-crates",
    )
    .await?;

    assert_eq!(response["status"], "queued");
    let stored = jobs
        .lookup_job_sync(response["job_id"].as_str().context("job_id")?)
        .context("stored job")?;
    let canonical_url = "https://crates.io/api/v1/crates/demo-crate/2.0.0/download";
    assert_eq!(stored.source, "registry:crates-io");
    assert_eq!(stored.package, "demo-crate");
    assert_eq!(stored.revision, "2.0.0");
    assert_eq!(stored.source_url, canonical_url);
    assert_eq!(stored.source_url_hash, source_url_hash(canonical_url));
    assert_eq!(stored.source_kind, "tarball");
    Ok(())
}

#[tokio::test]
async fn external_index_preserves_crates_io_separator_spelling_for_fetch() -> Result<()> {
    let fixture = McpFixture::new("index-crates-fetch-spelling")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": "serde_json",
            "revision": "1.0.150",
            "source_url": "https://crates.io/api/v1/crates/serde_json/1.0.150/download"
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-crates-fetch-spelling",
    )
    .await?;

    assert_eq!(response["status"], "queued");
    let stored = jobs
        .lookup_job_sync(response["job_id"].as_str().context("job_id")?)
        .context("stored job")?;
    assert_eq!(stored.package, "serde-json");
    assert_eq!(
        stored.source_url,
        "https://crates.io/api/v1/crates/serde_json/1.0.150/download"
    );
    Ok(())
}

#[tokio::test]
async fn external_index_equivalent_crates_io_requests_share_one_active_job() -> Result<()> {
    let fixture = McpFixture::new("index-active-crates-canonical")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let first = route_index(
        &json!({
            "package": "demo",
            "revision": "2.0.0",
            "source_url": "https://crates.io/api/v1/crates/demo/2.0.0/download"
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-active-crates",
    )
    .await?;
    let second = route_index(
        &json!({
            "package": " DEMO ",
            "revision": " 2.0.0 ",
            "source_url": "HTTPS://CRATES.IO:443/api/v1/crates/Demo/2.0.0/download?download=1",
            "source": " crates.io "
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-active-crates",
    )
    .await?;

    assert_eq!(first["status"], "queued");
    assert_eq!(second["job_id"], first["job_id"]);
    assert_eq!(jobs.job_count(), 1);
    Ok(())
}

#[tokio::test]
async fn external_index_rejects_crates_io_coordinate_mismatches_before_admission() -> Result<()> {
    let fixture = McpFixture::new("index-crates-coordinate-mismatch")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let package_mismatch = route_index(
        &json!({
            "package": "other",
            "revision": "2.0.0",
            "source_url": "https://crates.io/api/v1/crates/demo/2.0.0/download"
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-crates-mismatch",
    )
    .await?;
    let revision_mismatch = route_index(
        &json!({
            "package": "demo",
            "revision": "3.0.0",
            "source_url": "https://crates.io/api/v1/crates/demo/2.0.0/download"
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-crates-mismatch",
    )
    .await?;

    assert_eq!(package_mismatch["status"], "rejected");
    assert_eq!(package_mismatch["reason"], "source_url_package_mismatch");
    assert_eq!(revision_mismatch["status"], "rejected");
    assert_eq!(revision_mismatch["reason"], "source_url_revision_mismatch");
    assert_eq!(jobs.rate_count("caller-crates-mismatch"), 0);
    assert_eq!(jobs.job_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_custom_identity_only_trims_surrounding_whitespace() -> Result<()> {
    let fixture = McpFixture::new("index-custom-trimming")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": " demo ",
            "revision": " main ",
            "source_url": " https://1.1.1.1/example/demo ",
            "source": " vendor:custom "
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-custom-trimming",
    )
    .await?;

    assert_eq!(response["status"], "queued");
    let stored = jobs
        .lookup_job_sync(response["job_id"].as_str().context("job_id")?)
        .context("stored job")?;
    assert_eq!(stored.source, "vendor:custom");
    assert_eq!(stored.package, "demo");
    assert_eq!(stored.revision, "main");
    assert_eq!(stored.source_url, SOURCE_URL);
    assert_eq!(stored.source_url_hash, source_url_hash(SOURCE_URL));
    Ok(())
}

#[tokio::test]
async fn external_index_enqueues_job_without_starting_execution() -> Result<()> {
    let fixture = McpFixture::new("index-create")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-create",
    )
    .await?;

    // Enqueue path: job is queued, no execution started (drainer is a later
    // task).
    assert_eq!(response["status"], "queued");
    assert_eq!(response["job_id"], "job-1");
    assert_eq!(response["execution_arn"], Value::Null);
    assert_eq!(response["revision"], "main");
    assert_eq!(sfn.started_count(), 0);
    let stored = jobs.lookup_job_sync("job-1").context("created job")?;
    assert_eq!(stored.execution_arn, None);
    assert_eq!(stored.caller_id, "caller-create");
    assert_eq!(stored.source_kind, "git");
    Ok(())
}

#[tokio::test]
async fn external_index_retries_transient_enqueue_conflicts_without_recharging_rate_limit(
) -> Result<()> {
    let fixture = McpFixture::new("index-enqueue-conflict-retry")?;
    let jobs = FakeJobStore::default();
    jobs.fail_next_enqueue_attempts(3);
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "retry",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-enqueue-retry",
    )
    .await?;

    assert_eq!(response["status"], "queued");
    assert_eq!(jobs.enqueue_attempts(), 4);
    assert_eq!(jobs.rate_count("caller-enqueue-retry"), 1);
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[test]
fn external_index_enqueues_second_unique_job_by_default() -> Result<()> {
    let _env = EnvVarGuard::remove("SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER");
    let fixture = McpFixture::new("index-default-queue-cap")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let runtime = tokio::runtime::Runtime::new().context("create tokio runtime")?;

    // First unique request enqueued.
    let first = runtime.block_on(route_index(
        &json!({
            "package": PACKAGE,
            "revision": "first",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-concurrent",
    ))?;
    assert_eq!(first["status"], "queued");

    // Second unique request also enqueued — default queue cap (20) > 1.
    let second = runtime.block_on(route_index(
        &json!({
            "package": PACKAGE,
            "revision": "second",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-concurrent",
    ))?;

    assert_eq!(second["status"], "queued");
    assert_eq!(sfn.started_count(), 0);
    assert_eq!(jobs.job_count(), 2);
    Ok(())
}

#[test]
fn external_index_rejects_when_owner_queue_cap_is_one() -> Result<()> {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER", "1");
    let fixture = McpFixture::new("index-queue-cap-one")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let runtime = tokio::runtime::Runtime::new().context("create tokio runtime")?;

    // First unique request is enqueued (fills the owner's single queued slot).
    let first = runtime.block_on(route_index(
        &json!({
            "package": PACKAGE,
            "revision": "first",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-queue-cap",
    ))?;
    assert_eq!(first["status"], "queued");

    // Second unique request is rejected — owner queue is full.
    let second = runtime.block_on(route_index(
        &json!({
            "package": PACKAGE,
            "revision": "second",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-queue-cap",
    ))?;

    assert_eq!(second["status"], "rejected");
    assert_eq!(second["reason"], "queue_full");
    assert_eq!(second["max_queued_jobs_per_owner"], 1);
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_rejects_when_owner_queue_is_full_at_two() -> Result<()> {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER", "2");
    let fixture = McpFixture::new("index-queue-cap-two")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    // Enqueue two unique requests to fill the owner queue.
    for revision in ["first", "second"] {
        let response = route_index(
            &json!({
                "package": PACKAGE,
                "revision": revision,
                "source_url": SOURCE_URL,
                "source_kind": "git",
            }),
            &fixture.conn,
            &fixture.catalog,
            &jobs,
            &sfn,
            "caller-full",
        )
        .await?;
        assert_eq!(response["status"], "queued");
    }

    // Third unique request overflows the queue cap.
    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "third",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-full",
    )
    .await?;

    assert_eq!(response["status"], "rejected");
    assert_eq!(response["reason"], "queue_full");
    assert_eq!(response["max_queued_jobs_per_owner"], 2);
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_rejects_when_global_queue_is_full() -> Result<()> {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_QUEUED_JOBS_GLOBAL", "1");
    let fixture = McpFixture::new("index-global-queue-full")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    // First caller fills the single global queued slot.
    let first = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "caller-a-rev",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-a",
    )
    .await?;
    assert_eq!(first["status"], "queued");

    // Different caller, different package coordinate — still rejected because
    // the global queue is full.
    let second = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "caller-b-rev",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-b",
    )
    .await?;

    assert_eq!(second["status"], "rejected");
    assert_eq!(second["reason"], "global_queue_full");
    assert_eq!(second["max_queued_jobs_global"], 1);
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_force_true_does_not_bypass_queue_caps_or_dedupe() -> Result<()> {
    let _env = EnvVarGuard::set("SPUR_INDEX_MAX_QUEUED_JOBS_PER_OWNER", "1");
    let fixture = McpFixture::new("index-force-queue-cap")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    // First request enqueued (fills the owner's single slot).
    let first = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
            "force": true,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force",
    )
    .await?;
    assert_eq!(first["status"], "queued");

    // Same dedupe key with force=true — dedupe is NOT bypassed, returns the
    // existing active job.
    let dedup = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
            "force": true,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force",
    )
    .await?;
    assert_eq!(dedup["job_id"], first["job_id"]);
    assert_eq!(dedup["status"], "queued");

    // Different revision with force=true — queue cap is NOT bypassed.
    let rejected = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "other",
            "source_url": SOURCE_URL,
            "source_kind": "git",
            "force": true,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force",
    )
    .await?;

    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["reason"], "queue_full");
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_force_bypasses_warm_lookup_but_not_canonical_dedupe_or_rate_limit(
) -> Result<()> {
    let fixture = McpFixture::new("index-force-canonical")?;
    let jobs = FakeJobStore::default();
    jobs.set_rate_limit_per_minute(2);
    let sfn = StubIndexExecutionStarter::default();

    let first = route_index(
        &json!({
            "package": "demo",
            "revision": REVISION,
            "source_url": CRATES_IO_SOURCE_URL,
            "force": true
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force-canonical",
    )
    .await?;
    let dedup = route_index(
        &json!({
            "package": "DEMO",
            "revision": REVISION,
            "source_url": "HTTPS://CRATES.IO/api/v1/crates/Demo/1.0.0/download",
            "source": "crates-io",
            "force": true
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force-canonical",
    )
    .await?;
    let rate_limited = route_index(
        &json!({
            "package": "demo",
            "revision": REVISION,
            "source_url": CRATES_IO_SOURCE_URL,
            "force": true
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-force-canonical",
    )
    .await?;

    assert_eq!(first["status"], "queued");
    assert_eq!(dedup["job_id"], first["job_id"]);
    assert_eq!(rate_limited["status"], "rejected");
    assert_eq!(rate_limited["reason"], "rate_limit");
    assert_eq!(jobs.job_count(), 1);
    Ok(())
}

#[tokio::test]
async fn external_index_admission_assigns_stable_backlog_owner() -> Result<()> {
    let fixture = McpFixture::new("index-owner-stability")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-owner-stability",
    )
    .await?;

    assert_eq!(response["status"], "queued");
    let stored = jobs
        .lookup_job_sync(response["job_id"].as_str().context("job_id")?)
        .context("stored job")?;
    // The owner is derived from the caller identity, not from the request args.
    // This makes it stable for later per-user backlog extension: only the
    // backlog_owner_from_caller function would need to change.
    assert_eq!(stored.owner_kind, Some(BacklogOwnerKind::Caller));
    assert_eq!(stored.owner_id.as_deref(), Some("caller-owner-stability"));
    // Verify the owner has exactly one queued slot consumed.
    let owner = BacklogOwner::caller("caller-owner-stability");
    assert_eq!(jobs.owner_queued(&owner), 1);
    Ok(())
}

#[tokio::test]
async fn external_index_enqueue_dedupe_returns_existing_running_job() -> Result<()> {
    let fixture = McpFixture::new("index-enqueue-dedup-running")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    // First request enqueues a queued job.
    let first = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-dedup-running",
    )
    .await?;
    assert_eq!(first["status"], "queued");

    // Simulate the drainer dispatching the job (sets status to Dispatching).
    let job_id = first["job_id"].as_str().context("job_id")?.to_owned();
    jobs.update_job(&job_id, |record| {
        record.status = JobStatus::Dispatching;
    })?;

    // Duplicate request returns the existing dispatching job.
    let second = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-dedup-running",
    )
    .await?;

    assert_eq!(second["job_id"], first["job_id"]);
    assert_eq!(second["status"], "dispatching");
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn namespaced_cross_owner_dedupe_preserves_owner_and_hides_status() -> Result<()> {
    let fixture = McpFixture::new("index-auth-owner-dedupe")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();
    let args = json!({
        "package": PACKAGE,
        "revision": "main",
        "source_url": SOURCE_URL,
        "source_kind": "git",
    });
    let human = "cognito:user:opaque-human";
    let m2m = "cognito:client:organization-a";

    let first = route_index(&args, &fixture.conn, &fixture.catalog, &jobs, &sfn, human).await?;
    let duplicate = route_index(&args, &fixture.conn, &fixture.catalog, &jobs, &sfn, m2m).await?;

    assert_eq!(duplicate["job_id"], first["job_id"]);
    assert_eq!(duplicate["status"], "queued");
    assert!(duplicate.get("caller_id").is_none());
    assert!(duplicate.get("owner_id").is_none());

    let job_id = first["job_id"].as_str().context("job_id")?;
    let stored = jobs.lookup_job_sync(job_id).context("stored job")?;
    assert_eq!(stored.caller_id, human);
    assert_eq!(stored.owner_id.as_deref(), Some(human));
    assert_eq!(jobs.owner_queued(&BacklogOwner::caller(human)), 1);
    assert_eq!(jobs.owner_queued(&BacklogOwner::caller(m2m)), 0);

    let hidden =
        route_index_status_for_caller(&json!({ "job_id": job_id }), &jobs, None, m2m).await?;
    let visible =
        route_index_status_for_caller(&json!({ "job_id": job_id }), &jobs, None, human).await?;

    assert_eq!(hidden, json!({ "status": "not_found" }));
    assert_eq!(visible["status"], "queued");
    Ok(())
}

#[tokio::test]
async fn personal_api_keys_reuse_one_human_owner_for_rate_dedupe_queue_and_status() -> Result<()> {
    let fixture = McpFixture::new("index-api-key-owner-reuse")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();
    let owner = "cognito:user:api-key-human";
    let args = json!({
        "package": PACKAGE,
        "revision": "main",
        "source_url": SOURCE_URL,
        "source_kind": "git",
    });

    // The serving Lambda maps every valid personal-key context for this user
    // to the same owner before entering this existing admission path.
    let first = route_index(&args, &fixture.conn, &fixture.catalog, &jobs, &sfn, owner).await?;
    let second = route_index(&args, &fixture.conn, &fixture.catalog, &jobs, &sfn, owner).await?;

    assert_eq!(second["job_id"], first["job_id"]);
    assert_eq!(jobs.rate_count(owner), 2);
    assert_eq!(jobs.owner_queued(&BacklogOwner::caller(owner)), 1);

    let job_id = first["job_id"].as_str().context("job_id")?;
    let stored = jobs.lookup_job_sync(job_id).context("stored job")?;
    assert_eq!(stored.caller_id, owner);
    assert_eq!(stored.owner_id.as_deref(), Some(owner));
    assert_eq!(
        route_index_status_for_caller(&json!({ "job_id": job_id }), &jobs, None, owner).await?
            ["status"],
        "queued"
    );
    assert_eq!(
        route_index_status_for_caller(
            &json!({ "job_id": job_id }),
            &jobs,
            None,
            "cognito:user:different-human",
        )
        .await?,
        json!({ "status": "not_found" })
    );
    Ok(())
}

#[tokio::test]
async fn external_index_rejects_when_caller_rate_limit_is_full() -> Result<()> {
    let fixture = McpFixture::new("index-rate-limit")?;
    let jobs = FakeJobStore::default();
    jobs.set_rate_limit_per_minute(1);
    let sfn = StubIndexExecutionStarter::default();

    let first = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "rate-a",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-rate-limited",
    )
    .await?;

    assert_eq!(first["status"], "queued");

    let second = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "rate-b",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-rate-limited",
    )
    .await?;

    assert_eq!(second["status"], "rejected");
    assert_eq!(second["reason"], "rate_limit");
    // Rate limit fires before enqueue, so no execution is started and the first
    // job was only enqueued (not dispatched).
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_returns_existing_deduped_job_without_starting_execution() -> Result<()> {
    let fixture = McpFixture::new("index-dedup")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();
    let existing = jobs.seed_queued_job("arn:existing", |_| {});

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": SOURCE_URL,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-dedup",
    )
    .await?;

    assert_eq!(response["job_id"], existing.job_id);
    assert_eq!(response["status"], "queued");
    assert_eq!(response["execution_arn"], "arn:existing");
    assert_eq!(response["revision"], "main");
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

#[tokio::test]
async fn external_index_status_supports_cold_index_then_retry_eval_flow() -> Result<()> {
    let fixture = McpFixture::new("index-multi-round")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let queued = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source": "registry:crates-io",
            "source_url": SOURCE_URL,
            "source_kind": "git",
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-multi-round",
    )
    .await?;

    assert_eq!(queued["status"], "queued");
    assert_eq!(queued["job_id"], "job-1");
    // Enqueue path: no execution started yet (drainer is a later task).
    assert_eq!(sfn.started_count(), 0);

    let retry_before_catalog = handle_tool(
        "external_code_search",
        &json!({
            "query": "alpha",
            "package": PACKAGE,
            "revision": "main"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await
    .expect_err("cold revision should not be queryable before catalog is populated");
    assert!(matches!(retry_before_catalog, McpHandlerError::NotFound(_)));

    // Simulate the drainer having dispatched this job (the drainer is a later
    // task): give the job a stale execution ARN so status repair can kick in.
    jobs.update_job("job-1", |record| {
        record.execution_arn = Some("arn:stub:job-1".to_owned());
        record.updated_at = "0".to_owned();
    })?;
    let checker = StubExecutionStatusChecker::new(Some(ExecutionOutcome {
        status: ExecutionOutcomeStatus::Succeeded,
        output: Some(json!({
            "snapshot_id": 101,
            "rows_inserted": {
                "nodes": 9,
                "edges": 4
            }
        })),
        error: None,
    }));

    let complete = route_index_status(&json!({ "job_id": "job-1" }), &jobs, Some(&checker)).await?;

    assert_eq!(complete["status"], "complete");
    assert_eq!(complete["snapshot_id"], 101);
    assert_eq!(checker.described_arns(), ["arn:stub:job-1"]);

    move_fixture_to_revision(&fixture, "main", "git")?;
    let retry_after_catalog = handle_tool(
        "external_code_search",
        &json!({
            "query": "alpha",
            "package": PACKAGE,
            "revision": "main"
        }),
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;

    assert_eq!(retry_after_catalog["total_matches"], 1);
    assert_eq!(
        retry_after_catalog["candidates"][0]["selector"],
        "pkg:demo@main::demo::alpha"
    );
    Ok(())
}

#[tokio::test]
async fn external_index_rejects_localhost_source_url_before_starting_job() -> Result<()> {
    let fixture = McpFixture::new("index-abuse")?;
    let jobs = FakeJobStore::default();
    let sfn = StubIndexExecutionStarter::default();

    let response = route_index(
        &json!({
            "package": PACKAGE,
            "revision": "main",
            "source_url": "https://localhost/example/demo",
            "force": true,
        }),
        &fixture.conn,
        &fixture.catalog,
        &jobs,
        &sfn,
        "caller-abuse",
    )
    .await?;

    assert_eq!(response["status"], "rejected");
    assert!(response["reason"]
        .as_str()
        .context("reason string")?
        .contains("source_url targets localhost"));
    assert_eq!(sfn.started_count(), 0);
    Ok(())
}

fn assert_next_tools<const N: usize>(
    evidence: &Value,
    selector: &str,
    expected_tools: [&str; N],
) -> Result<()> {
    let next = evidence["next"].as_array().context("next array")?;
    for tool in expected_tools {
        assert!(
            next.iter().any(|entry| {
                entry["tool"] == tool && entry["selector"].as_str() == Some(selector)
            }),
            "missing {tool} next entry for {selector}: {next:?}"
        );
    }
    Ok(())
}

fn schema_for<'a>(
    definitions: &'a [spur_context_service::mcp::ToolDefinition],
    name: &str,
) -> &'a Value {
    &definitions
        .iter()
        .find(|definition| definition.name == name)
        .with_context(|| format!("missing tool definition {name}"))
        .unwrap()
        .input_schema
}

async fn search_uri(
    fixture: &McpFixture,
    query: &str,
    symbol_kind: Option<&str>,
) -> Result<String> {
    let mut args = json!({
        "query": query,
        "package": PACKAGE,
        "limit": 20
    });
    if let Some(symbol_kind) = symbol_kind {
        args["symbol_kind"] = json!(symbol_kind);
    }
    let response = handle_tool(
        "external_code_search",
        &args,
        &fixture.conn,
        &fixture.catalog,
    )
    .await?;
    response["candidates"][0]["uri"]
        .as_str()
        .map(str::to_owned)
        .context("search candidate URI")
}

fn required(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect()
}

fn move_fixture_to_source(fixture: &McpFixture, source: &str) -> Result<()> {
    for table in [
        "nodes",
        "edges",
        "edges_unresolved",
        "files",
        "file_manifests",
        "package_catalog",
        "section_bodies",
        "symbol_embeddings",
        "refs",
    ] {
        fixture
            .conn
            .execute(
                &format!("UPDATE {table} SET source = ? WHERE source = 'registry:crates-io'"),
                params![source],
            )
            .with_context(|| format!("move query fixture table {table} to source {source}"))?;
    }

    for table in ["package_catalog", "refs"] {
        fixture
            .catalog
            .connection()
            .execute(
                &format!("UPDATE {table} SET source = ? WHERE source = 'registry:crates-io'"),
                params![source],
            )
            .with_context(|| format!("move catalog fixture table {table} to source {source}"))?;
    }
    Ok(())
}

fn move_fixture_to_revision(
    fixture: &McpFixture,
    revision: &str,
    revision_kind: &str,
) -> Result<()> {
    for table in [
        "nodes",
        "edges",
        "edges_unresolved",
        "files",
        "file_manifests",
        "package_catalog",
        "section_bodies",
        "symbol_embeddings",
    ] {
        fixture
            .conn
            .execute(
                &format!(
                    "UPDATE {table} SET revision = ?, revision_kind = ? WHERE revision = '1.0.0'"
                ),
                params![revision, revision_kind],
            )
            .with_context(|| format!("move query fixture table {table} to revision {revision}"))?;
    }

    fixture
        .conn
        .execute(
            "UPDATE refs SET revision = ? WHERE revision = '1.0.0'",
            params![revision],
        )
        .with_context(|| format!("move query refs to revision {revision}"))?;

    fixture
        .catalog
        .connection()
        .execute(
            "UPDATE package_catalog SET revision = ?, revision_kind = ?, snapshot_id = 101 WHERE revision = '1.0.0'",
            params![revision, revision_kind],
        )
        .with_context(|| format!("move catalog fixture to revision {revision}"))?;
    fixture
        .catalog
        .connection()
        .execute(
            "UPDATE refs SET revision = ? WHERE revision = '1.0.0'",
            params![revision],
        )
        .with_context(|| format!("move catalog refs to revision {revision}"))?;
    Ok(())
}

struct McpFixture {
    conn: Connection,
    catalog: CatalogResolver,
    _root: PathBuf,
}

impl McpFixture {
    fn new(name: &str) -> Result<Self> {
        let conn = Connection::open_in_memory().context("open query duckdb")?;
        create_query_schema(&conn)?;
        seed_query_fixture(&conn)?;

        let root = unique_temp_dir(name)?;
        fs::create_dir_all(root.join("data")).context("create catalog data dir")?;
        let catalog_path = root.join("catalog.sqlite");
        let data_path = root.join("data");
        let catalog_dsn = format!("sqlite:{}", catalog_path.display());
        let data_path = data_path.display().to_string();
        initialize_catalog(&catalog_dsn, &data_path)?;
        let catalog = CatalogResolver::new_with_data_path(&catalog_dsn, &data_path)?;

        Ok(Self {
            conn,
            catalog,
            _root: root,
        })
    }
}

struct FrozenServingFixture {
    snapshot_path: PathBuf,
    data_path: String,
    _root: PathBuf,
}

impl FrozenServingFixture {
    fn new(name: &str) -> Result<Self> {
        let root = unique_temp_dir(name)?;
        let data_path = root.join("data");
        fs::create_dir_all(&data_path).context("create frozen serving data dir")?;

        let catalog_dsn = format!("sqlite:{}", root.join("catalog.sqlite").display());
        let data_path = data_path.display().to_string();
        initialize_gold_serving_catalog(&catalog_dsn, &data_path)?;
        compact_gold_and_export_snapshot(
            &catalog_dsn,
            &data_path,
            SnapshotCleanupOptions {
                older_than: std::time::Duration::from_secs(3600),
                republish_lag: std::time::Duration::from_secs(300),
            },
        )?;

        let snapshot_path = catalog_snapshot_path(&data_path)?;
        assert!(
            snapshot_path.is_file(),
            "expected frozen serving snapshot at {}",
            snapshot_path.display()
        );

        Ok(Self {
            snapshot_path,
            data_path,
            _root: root,
        })
    }
}

fn initialize_gold_serving_catalog(catalog_dsn: &str, data_path: &str) -> Result<()> {
    let conn = Connection::open_in_memory().context("open frozen serving setup DuckDB")?;
    attach_ducklake(&conn, catalog_dsn, data_path)?;
    conn.execute_batch(include_str!("../sql/catalog_tables.sql"))
        .context("create medallion catalog tables")?;

    let source_text = "pub fn beta() {\n}\n";
    conn.execute(
        r"
        INSERT INTO gold.files (
            stable_file_id, file_path, source_text,
            package, source, revision, revision_kind,
            semver_major, semver_minor, semver_patch, generation
        )
        VALUES (
            'file-demo-lib', 'src/lib.rs', ?,
            'demo', 'registry:crates-io', '1.0.0', 'semver',
            1, 0, 0, 7
        )
        ",
        params![source_text],
    )
    .context("insert frozen serving file")?;
    conn.execute(
        r"
        INSERT INTO gold.nodes (
            stable_symbol_id, package, source, revision, revision_kind,
            semver_major, semver_minor, semver_patch, file_path,
            byte_range_start, byte_range_end, line_start, line_end,
            entity_name, qualified_name, symbol_kind, anchor_hash,
            enclosing_scope, generation
        )
        VALUES (
            'bbbbbbbbbbbbbbbb', 'demo', 'registry:crates-io', '1.0.0', 'semver',
            1, 0, 0, 'src/lib.rs',
            0, ?, 1, 2,
            'beta', 'demo::beta', 'function', 'anchor-beta',
            NULL, 7
        )
        ",
        params![source_text.len() as i64],
    )
    .context("insert frozen serving node")?;
    conn.execute_batch(
        r#"
        INSERT INTO gold.package_catalog (
            source, package, revision, revision_kind,
            semver_major, semver_minor, semver_patch,
            snapshot_id, indexed_at, index_status, embeddings_status, row_counts,
            generation, bronze_content_sha256, silver_graph_content_hash,
            builder_version, translate_schema_version
        )
        VALUES (
            'registry:crates-io', 'demo', '1.0.0', 'semver',
            1, 0, 0,
            777, TIMESTAMP '2026-06-28 00:00:00',
            'complete', 'skipped', '{"nodes":1}',
            7, 'bronze-fixture', 'graph-fixture', 'builder-fixture', 'translate-fixture'
        );

        INSERT INTO gold.refs (source, package, ref_name, revision, updated_at)
        VALUES (
            'registry:crates-io', 'demo', 'latest', '1.0.0',
            TIMESTAMP '2026-06-28 00:01:00'
        );
        "#,
    )
    .context("insert frozen serving catalog metadata")?;
    flush_inlined_ducklake_data(&conn)?;
    conn.execute("FORCE CHECKPOINT", [])
        .context("checkpoint frozen serving fixture")?;
    Ok(())
}

fn flush_inlined_ducklake_data(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("CALL ducklake_flush_inlined_data('spur_context')")
        .context("prepare ducklake_flush_inlined_data")?;
    let mut rows = stmt.query([]).context("execute ducklake flush")?;
    while rows.next()?.is_some() {}
    Ok(())
}

fn catalog_snapshot_path(data_path: &str) -> Result<PathBuf> {
    let pointer_path = PathBuf::from(data_path)
        .join("gold")
        .join("catalog-snapshot")
        .join("current.json");
    let pointer: FrozenSnapshotManifest = serde_json::from_slice(
        &fs::read(&pointer_path)
            .with_context(|| format!("read snapshot pointer {}", pointer_path.display()))?,
    )
    .context("parse snapshot pointer")?;
    Ok(PathBuf::from(pointer.snapshot_uri))
}

fn create_query_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('fixture-hash');

        CREATE TABLE nodes (
            stable_symbol_id VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            file_path VARCHAR,
            byte_range_start INTEGER,
            byte_range_end INTEGER,
            line_start INTEGER,
            line_end INTEGER,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            anchor_hash VARCHAR,
            enclosing_scope VARCHAR
        );

        CREATE TABLE edges (
            source_stable_id VARCHAR,
            target_stable_id VARCHAR,
            target_package VARCHAR,
            target_label VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            relation VARCHAR,
            edge_kind VARCHAR,
            confidence VARCHAR,
            confidence_score DOUBLE,
            bind_method VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );

        CREATE TABLE edges_unresolved (
            source_stable_id VARCHAR,
            target_label VARCHAR,
            target_package VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            relation VARCHAR,
            edge_kind VARCHAR,
            confidence VARCHAR,
            confidence_score DOUBLE,
            bind_method VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );

        CREATE TABLE files (
            stable_file_id VARCHAR,
            file_path VARCHAR,
            source_text VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER
        );

        CREATE TABLE file_manifests (
            stable_file_id VARCHAR,
            path VARCHAR,
            content_oid VARCHAR,
            node_ids VARCHAR[],
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER
        );

        CREATE TABLE section_bodies (
            section_id VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            file_path VARCHAR,
            title VARCHAR,
            body_text VARCHAR,
            body_hash VARCHAR,
            token_count INTEGER
        );

        CREATE TABLE symbol_embeddings (
            stable_symbol_id VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            file_path VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            embedding FLOAT[],
            embedding_model VARCHAR,
            embedding_input_hash VARCHAR,
            embed_text_version VARCHAR
        );

        CREATE TABLE package_catalog (
            source VARCHAR,
            package VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            snapshot_id BIGINT,
            indexed_at TIMESTAMP,
            index_status VARCHAR,
            embeddings_status VARCHAR,
            row_counts JSON,
            generation BIGINT,
            bronze_content_sha256 VARCHAR,
            silver_graph_content_hash VARCHAR,
            builder_version VARCHAR,
            translate_schema_version VARCHAR,
            embed_text_version VARCHAR
        );

        CREATE TABLE refs (
            source VARCHAR,
            package VARCHAR,
            ref_name VARCHAR,
            revision VARCHAR,
            updated_at TIMESTAMP
        );

        ",
    )
    .context("create query schema")?;
    Ok(())
}

fn seed_query_fixture(conn: &Connection) -> Result<()> {
    let source_text = concat!(
        "pub fn alpha() {\n",
        "    beta();\n",
        "    external::Thing::new();\n",
        "}\n",
        "\n",
        "pub fn beta() {\n",
        "}\n",
        "\n",
        "pub fn parse_config_loader() {}\n",
        "\n",
        "pub fn caller() {\n",
        "    beta();\n",
        "}\n",
        "\n",
        "pub fn dynamic_caller() {}\n",
        "\n",
        "pub fn hof_caller() {}\n",
        "\n",
        "pub fn task_spawner() {}\n",
        "\n",
        "pub struct Buffer {\n",
        "    bytes: [u8; 20],\n",
        "}\n",
        "\n",
        "impl Buffer {\n",
        "    pub fn new() -> Self { Self { bytes: [0; 20] } }\n",
        "}\n",
    );

    conn.execute(
        r"
        INSERT INTO files VALUES
            ('file-lib', 'src/lib.rs', $1, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0)
        ",
        params![source_text],
    )
    .context("insert file")?;

    insert_node(
        conn,
        "aaaaaaaaaaaaaaaa",
        source_text,
        "alpha",
        "demo::alpha",
        1,
        4,
    )?;
    insert_node(
        conn,
        "bbbbbbbbbbbbbbbb",
        source_text,
        "beta",
        "demo::beta",
        6,
        7,
    )?;
    insert_node(
        conn,
        "cccccccccccccccc",
        source_text,
        "parse_config_loader",
        "demo::parse_config_loader",
        9,
        9,
    )?;
    insert_node(
        conn,
        "dddddddddddddddd",
        source_text,
        "caller",
        "demo::caller",
        11,
        13,
    )?;
    insert_node(
        conn,
        "eeeeeeeeeeeeeeee",
        source_text,
        "dynamic_caller",
        "demo::dynamic_caller",
        15,
        15,
    )?;
    insert_node(
        conn,
        "ffffffffffffffff",
        source_text,
        "hof_caller",
        "demo::hof_caller",
        17,
        17,
    )?;
    insert_node(
        conn,
        "9999999999999999",
        source_text,
        "task_spawner",
        "demo::runtime::task_spawner",
        19,
        19,
    )?;
    insert_node_with_kind(
        conn,
        "bufferstruct0001",
        source_text,
        "Buffer",
        "Buffer",
        "struct",
        "pub struct Buffer",
        21,
        23,
    )?;
    insert_node_with_kind(
        conn,
        "bufferimpl000001",
        source_text,
        "Buffer",
        "impl Buffer",
        "impl",
        "impl Buffer",
        25,
        27,
    )?;

    insert_edge(
        conn,
        "aaaaaaaaaaaaaaaa",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "calls",
        "calls",
    )?;
    insert_edge(
        conn,
        "dddddddddddddddd",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "calls",
        "references_hof",
    )?;
    insert_unresolved_edge(
        conn,
        "eeeeeeeeeeeeeeee",
        "demo::beta",
        None,
        "calls",
        "calls_dyn",
    )?;
    insert_unresolved_edge(
        conn,
        "aaaaaaaaaaaaaaaa",
        "external::Thing",
        Some("external"),
        "calls",
        "calls",
    )?;

    conn.execute(
        r"
        INSERT INTO section_bodies VALUES
            ('doc-parse', 'demo', 'registry:crates-io', '1.0.0', 'semver',
             1, 0, 0, 'docs/parser.md', 'Parser Guide',
             'The parse config loader reads config documents and validates loader inputs.',
             'hash-doc-parse', 9)
        ",
        [],
    )
    .context("insert section body")?;

    conn.execute(
        r"
        INSERT INTO refs VALUES
            ('registry:crates-io', 'demo', 'latest', '1.0.0',
             TIMESTAMP '2026-06-22 00:00:00'),
            ('registry:crates-io', 'demo', 'stable', '1.0.0',
             TIMESTAMP '2026-06-22 00:01:00'),
            ('registry:crates-io', 'demo', 'old', '0.9.0',
             TIMESTAMP '2026-06-21 00:00:00')
        ",
        [],
    )
    .context("insert latest ref")?;

    conn.execute_batch(
        r#"
        INSERT INTO file_manifests VALUES
            ('file-readme', 'README.md', 'oid-readme', []::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-lib', 'src/lib.rs', 'oid-lib',
             ['aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 'cccccccccccccccc',
              'dddddddddddddddd', 'eeeeeeeeeeeeeeee', 'ffffffffffffffff',
              '9999999999999999', 'bufferstruct0001', 'bufferimpl000001']::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-a', 'src/a.rs', 'oid-a', []::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-b', 'src/b.rs', 'oid-b', []::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0);

        INSERT INTO package_catalog (
            source, package, revision, revision_kind,
            semver_major, semver_minor, semver_patch,
            snapshot_id, indexed_at, index_status, embeddings_status,
            row_counts, generation, bronze_content_sha256,
            silver_graph_content_hash, builder_version,
            translate_schema_version, embed_text_version
        )
        VALUES
            ('registry:crates-io', 'demo', '0.9.0', 'semver',
             0, 9, 0, 90, TIMESTAMP '2026-06-21 00:00:00',
             'complete', 'skipped', '{"nodes": 1}', 5,
             'bronze-old', 'graph-old', 'builder', 'translate', 'embed'),
            ('registry:crates-io', 'demo', '1.0.0', 'semver',
             1, 0, 0, 100, TIMESTAMP '2026-06-22 00:00:00',
             'complete', 'complete', '{"nodes": 9, "files": 4}', 7,
             'bronze-current', 'graph-current', 'builder', 'translate', 'embed'),
            ('registry:crates-io', 'zebra', '0.1.0', 'semver',
             0, 1, 0, 10, TIMESTAMP '2026-06-23 00:00:00',
             'complete', 'complete', '{"nodes": 0}', 9,
             'bronze-zebra', 'graph-zebra', 'builder', 'translate', 'embed');
        "#,
    )
    .context("insert catalog rows")?;

    insert_embedding(
        conn,
        "9999999999999999",
        "task_spawner",
        "demo::runtime::task_spawner",
        unit_vector(0),
    )?;
    insert_embedding(
        conn,
        "cccccccccccccccc",
        "parse_config_loader",
        "demo::parse_config_loader",
        unit_vector(1),
    )?;
    Ok(())
}

fn duplicate_parse_config_loader(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r"
        INSERT INTO nodes
        SELECT
            'cccccccccccccccd', package, source, revision, revision_kind,
            semver_major, semver_minor, semver_patch, file_path,
            byte_range_start, byte_range_end, line_start, line_end,
            entity_name, qualified_name, symbol_kind,
            'anchor-cccccccccccccccd', enclosing_scope
        FROM nodes
        WHERE stable_symbol_id = 'cccccccccccccccc';
        ",
    )
    .context("duplicate parse_config_loader node")?;
    Ok(())
}

fn insert_node(
    conn: &Connection,
    id: &str,
    source_text: &str,
    entity_name: &str,
    qualified_name: &str,
    line_start: i64,
    line_end: i64,
) -> Result<()> {
    insert_node_with_kind(
        conn,
        id,
        source_text,
        entity_name,
        qualified_name,
        "function",
        &format!("pub fn {entity_name}"),
        line_start,
        line_end,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_node_with_kind(
    conn: &Connection,
    id: &str,
    source_text: &str,
    entity_name: &str,
    qualified_name: &str,
    symbol_kind: &str,
    marker: &str,
    line_start: i64,
    line_end: i64,
) -> Result<()> {
    let byte_start = source_text
        .find(marker)
        .with_context(|| format!("find marker {marker}"))? as i64;
    let byte_end = source_text[byte_start as usize..]
        .find("\n\n")
        .map(|offset| byte_start + offset as i64 + 1)
        .unwrap_or(source_text.len() as i64);
    conn.execute(
        r"
        INSERT INTO nodes VALUES
            ($1, 'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0,
             'src/lib.rs', $2, $3, $4, $5, $6, $7, $8, $9, NULL)
        ",
        params![
            id,
            byte_start,
            byte_end,
            line_start,
            line_end,
            entity_name,
            qualified_name,
            symbol_kind,
            format!("anchor-{id}")
        ],
    )
    .with_context(|| format!("insert node {qualified_name}"))?;
    Ok(())
}

fn insert_edge(
    conn: &Connection,
    source_id: &str,
    target_id: Option<&str>,
    target_package: Option<&str>,
    target_label: Option<&str>,
    relation: &str,
    edge_kind: &str,
) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO edges VALUES
            ($1, $2, $3, $4, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0, $5, $6, 'syntax_exact', 0.99,
             'singleton', NULL, NULL)
        ",
        params![
            source_id,
            target_id,
            target_package,
            target_label,
            relation,
            edge_kind
        ],
    )
    .context("insert resolved edge")?;
    Ok(())
}

fn insert_unresolved_edge(
    conn: &Connection,
    source_id: &str,
    target_label: &str,
    target_package: Option<&str>,
    relation: &str,
    edge_kind: &str,
) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO edges_unresolved VALUES
            ($1, $2, $3, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0, $4, $5, 'heuristic', 0.50,
             'label', NULL, NULL)
        ",
        params![source_id, target_label, target_package, relation, edge_kind],
    )
    .context("insert unresolved edge")?;
    Ok(())
}

fn insert_embedding(
    conn: &Connection,
    id: &str,
    entity_name: &str,
    qualified_name: &str,
    vector: Vec<f32>,
) -> Result<()> {
    let sql = format!(
        r"
        INSERT INTO symbol_embeddings VALUES
            ('{id}', 'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0,
             'src/lib.rs', '{entity_name}', '{qualified_name}', 'function',
             {}, '{EMBEDDING_MODEL}', 'hash-{id}', '{EMBED_TEXT_VERSION}')
        ",
        vector_sql(&vector)
    );
    conn.execute_batch(&sql)
        .with_context(|| format!("insert embedding {id}"))?;
    Ok(())
}

fn unit_vector(index: usize) -> Vec<f32> {
    let mut vector = vec![0.0; DIMENSIONS];
    vector[index] = 1.0;
    vector
}

fn vector_sql(vector: &[f32]) -> String {
    let values = vector
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]::FLOAT[]")
}

fn initialize_catalog(catalog_dsn: &str, data_path: &str) -> Result<()> {
    let conn = Connection::open_in_memory().context("open catalog duckdb")?;
    attach_ducklake(&conn, catalog_dsn, data_path)?;
    conn.execute_batch(
        r#"
        CREATE TABLE package_catalog (
            source VARCHAR,
            package VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            snapshot_id BIGINT,
            indexed_at TIMESTAMP,
            index_status VARCHAR,
            embeddings_status VARCHAR,
            row_counts JSON
        );
        ALTER TABLE package_catalog SET PARTITIONED BY (source, package);

        CREATE TABLE refs (
            source VARCHAR,
            package VARCHAR,
            ref_name VARCHAR,
            revision VARCHAR,
            updated_at TIMESTAMP
        );
        ALTER TABLE refs SET PARTITIONED BY (source, package);

        INSERT INTO package_catalog VALUES
            ('registry:crates-io', 'demo', '1.0.0', 'semver',
             1, 0, 0, 100, TIMESTAMP '2026-06-22 00:00:00',
             'complete', 'complete', '{"nodes": 6}');

        INSERT INTO refs VALUES
            ('registry:crates-io', 'demo', 'latest', '1.0.0',
             TIMESTAMP '2026-06-22 00:05:00');

        "#,
    )
    .context("seed catalog fixture")?;
    Ok(())
}

fn attach_ducklake(conn: &Connection, catalog_dsn: &str, data_path: &str) -> Result<()> {
    let catalog_dsn = escape_sql_literal(catalog_dsn);
    let data_path = escape_sql_literal(data_path);
    conn.execute_batch("INSTALL ducklake; INSTALL sqlite; LOAD ducklake; LOAD sqlite;")
        .context("load ducklake/sqlite extensions")?;
    conn.execute_batch(&format!(
        "ATTACH 'ducklake:{catalog_dsn}' AS spur_context (DATA_PATH '{data_path}'); USE spur_context;"
    ))
    .context("attach ducklake")
}

fn unique_temp_dir(name: &str) -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_nanos();
    path.push(format!(
        "spur-context-service-mcp-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).with_context(|| format!("create temp dir {}", path.display()))?;
    Ok(path)
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn source_url_hash(source_url: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in source_url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[derive(Default)]
struct StubIndexExecutionStarter {
    started: Mutex<Vec<IndexExecutionRequest>>,
}

impl StubIndexExecutionStarter {
    fn started_count(&self) -> usize {
        self.started.lock().unwrap().len()
    }

    #[allow(dead_code)]
    fn started_requests(&self) -> Vec<IndexExecutionRequest> {
        self.started.lock().unwrap().clone()
    }
}

impl IndexExecutionStarter for StubIndexExecutionStarter {
    fn start_execution<'a>(
        &'a self,
        request: IndexExecutionRequest,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, McpHandlerError>> + Send + 'a>>
    {
        Box::pin(async move {
            let job_id = request.name.clone();
            self.started.lock().unwrap().push(request);
            Ok(format!("arn:stub:{job_id}"))
        })
    }
}

struct StubExecutionStatusChecker {
    result: StubExecutionResult,
    described_arns: Mutex<Vec<String>>,
}

enum StubExecutionResult {
    Ok(Option<ExecutionOutcome>),
    Err(String),
}

impl StubExecutionStatusChecker {
    fn new(outcome: Option<ExecutionOutcome>) -> Self {
        Self {
            result: StubExecutionResult::Ok(outcome),
            described_arns: Mutex::new(Vec::new()),
        }
    }

    fn fail(message: &str) -> Self {
        Self {
            result: StubExecutionResult::Err(message.to_owned()),
            described_arns: Mutex::new(Vec::new()),
        }
    }

    fn described_arns(&self) -> Vec<String> {
        self.described_arns.lock().unwrap().clone()
    }
}

impl ExecutionStatusChecker for StubExecutionStatusChecker {
    fn describe_execution(
        &self,
        arn: &str,
    ) -> std::result::Result<Option<ExecutionOutcome>, McpHandlerError> {
        self.described_arns.lock().unwrap().push(arn.to_owned());
        match &self.result {
            StubExecutionResult::Ok(outcome) => Ok(outcome.clone()),
            StubExecutionResult::Err(message) => Err(McpHandlerError::Internal(message.clone())),
        }
    }
}

#[derive(Default)]
struct FakeJobStore {
    next_id: AtomicU64,
    state: Mutex<FakeJobState>,
    enqueue_attempts: AtomicU64,
    enqueue_conflicts_remaining: AtomicU64,
    /// When true, `release_running_quota` returns a `Conflict` error (leaving
    /// the token and counter untouched) so the terminal-release conflict repair
    /// path can be exercised.
    fail_release: AtomicBool,
}

#[derive(Default)]
struct FakeJobState {
    jobs: HashMap<String, JobRecord>,
    dedupe: HashMap<JobKey, String>,
    rate_limit_max: Option<u32>,
    rate_counts: HashMap<String, u32>,
    // ─── Bounded queueing accounting ──────────────────────────────────────
    owner_counters: HashMap<String, OwnerCounters>,
    global_queued: u32,
    global_running: u32,
    running_tokens: HashSet<String>,
}

#[derive(Default, Clone, Copy)]
struct OwnerCounters {
    queued: u32,
    running: u32,
}

#[async_trait]
impl JobStore for FakeJobStore {
    async fn check_index_rate_limit(
        &self,
        caller_id: &str,
        max_requests_per_minute: u32,
    ) -> spur_context_service::jobs::Result<()> {
        let mut state = self.state.lock().expect("fake store lock");
        let max = state.rate_limit_max.unwrap_or(max_requests_per_minute);
        if max == 0 {
            return Err(JobsError::RateLimited);
        }
        let count = state.rate_counts.entry(caller_id.to_owned()).or_default();
        if *count >= max {
            return Err(JobsError::RateLimited);
        }
        *count += 1;
        Ok(())
    }

    async fn create_or_get_active_job(
        &self,
        request: CreateJobRequest,
    ) -> spur_context_service::jobs::Result<CreateJobOutcome> {
        self.create_or_get_active_job_with_limit(request, u32::MAX)
            .await
    }

    async fn create_or_get_active_job_with_limit(
        &self,
        request: CreateJobRequest,
        max_active_jobs_per_caller: u32,
    ) -> spur_context_service::jobs::Result<CreateJobOutcome> {
        let key = request.key();
        let mut state = self.state.lock().expect("fake store lock");
        if let Some(job_id) = state.dedupe.get(&key) {
            if let Some(record) = state.jobs.get(job_id) {
                return Ok(CreateJobOutcome::Existing(record.clone()));
            }
        }

        let active_jobs = state
            .jobs
            .values()
            .filter(|record| {
                record.caller_id == request.caller_id
                    && matches!(record.status, JobStatus::Queued | JobStatus::Running)
            })
            .count();
        if active_jobs >= max_active_jobs_per_caller as usize {
            return Err(JobsError::ConcurrentLimit);
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
            record.updated_at = "started".to_owned();
        })
    }

    async fn update_stage(
        &self,
        job_id: &str,
        status: JobStatus,
        stage: &str,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        self.update_job(job_id, |record| {
            record.status = status;
            record.stage = Some(stage.to_owned());
            record.updated_at = "stage".to_owned();
        })
    }

    async fn mark_complete(
        &self,
        job_id: &str,
        snapshot_id: i64,
        row_counts: serde_json::Value,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let record = self.update_job(job_id, |record| {
            record.status = JobStatus::Complete;
            record.snapshot_id = Some(snapshot_id);
            record.row_counts = Some(row_counts);
            record.error_code = None;
            record.error_detail = None;
            record.updated_at = "complete".to_owned();
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
            record.updated_at = "failed".to_owned();
        })?;
        self.release_dedupe_if_owner(&record).await?;
        Ok(record)
    }

    async fn lookup_job(
        &self,
        job_id: &str,
    ) -> spur_context_service::jobs::Result<Option<JobRecord>> {
        Ok(self.lookup_job_sync(job_id))
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

    async fn find_active_dedupe_job(
        &self,
        key: &JobKey,
    ) -> spur_context_service::jobs::Result<Option<JobRecord>> {
        let state = self.state.lock().expect("fake store lock");
        Ok(active_dedupe_in_state(&state, key))
    }

    async fn enqueue_job(
        &self,
        request: CreateJobRequest,
        owner: BacklogOwner,
        config: &QueueConfig,
    ) -> spur_context_service::jobs::Result<EnqueueOutcome> {
        self.enqueue_attempts.fetch_add(1, Ordering::Relaxed);
        if self
            .enqueue_conflicts_remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(JobsError::Conflict);
        }

        let key = request.key();
        let mut state = self.state.lock().expect("fake store lock");

        // Idempotent admission: return an existing active job.
        if let Some(existing) = active_dedupe_in_state(&state, &key) {
            return Ok(EnqueueOutcome::Existing(existing));
        }

        if config.max_queued_per_owner == 0 {
            return Err(JobsError::QueueFull);
        }
        let owner_pk = owner.pk();
        let queued = state
            .owner_counters
            .get(&owner_pk)
            .map(|c| c.queued)
            .unwrap_or_default();
        if queued >= config.max_queued_per_owner {
            return Err(JobsError::QueueFull);
        }
        if config.max_queued_global > 0 && state.global_queued >= config.max_queued_global {
            return Err(JobsError::GlobalQueueFull);
        }

        let n = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let job_id = format!("job-{n}");
        let shard = format!("{:02}", n % u64::from(config.shard_count.max(1)));
        let sort_key = format!("{:011}#queued#{job_id}", n);
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
            owner_kind: Some(owner.kind),
            owner_id: Some(owner.id),
            queue_shard: Some(shard),
            queue_sort_key: Some(sort_key),
            next_eligible_at: Some(n),
            dispatched_at: None,
        };

        state.owner_counters.entry(owner_pk).or_default().queued += 1;
        state.global_queued += 1;
        state.dedupe.insert(key, job_id.clone());
        state.jobs.insert(job_id, record.clone());
        Ok(EnqueueOutcome::Enqueued(record))
    }

    async fn dispatch_queued_job(
        &self,
        job_id: &str,
        config: &QueueConfig,
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let mut state = self.state.lock().expect("fake store lock");

        let owner = {
            let record = state.jobs.get(job_id).ok_or(JobsError::NotFound)?;
            if record.status != JobStatus::Queued {
                return Err(JobsError::Conflict);
            }
            record.owner().ok_or(JobsError::Conflict)?
        };
        let owner_pk = owner.pk();

        let running = state
            .owner_counters
            .get(&owner_pk)
            .map(|c| c.running)
            .unwrap_or_default();
        if running >= config.max_running_per_owner {
            return Err(JobsError::Conflict);
        }
        if config.max_running_global > 0 && state.global_running >= config.max_running_global {
            return Err(JobsError::Conflict);
        }

        {
            let counters = state.owner_counters.entry(owner_pk).or_default();
            counters.queued = counters.queued.saturating_sub(1);
            counters.running += 1;
        }
        state.global_queued = state.global_queued.saturating_sub(1);
        state.global_running += 1;
        state.running_tokens.insert(job_id.to_owned());

        let record = state.jobs.get_mut(job_id).ok_or(JobsError::NotFound)?;
        record.status = JobStatus::Dispatching;
        record.queue_shard = None;
        record.queue_sort_key = None;
        record.next_eligible_at = None;
        record.dispatched_at = Some("dispatched".to_owned());
        Ok(record.clone())
    }

    async fn release_running_quota(
        &self,
        record: &JobRecord,
    ) -> spur_context_service::jobs::Result<()> {
        if self.fail_release.load(Ordering::SeqCst) {
            return Err(JobsError::Conflict);
        }
        let mut state = self.state.lock().expect("fake store lock");
        if !state.running_tokens.remove(&record.job_id) {
            return Ok(());
        }
        if let Some(owner) = record.owner() {
            if let Some(counters) = state.owner_counters.get_mut(&owner.pk()) {
                counters.running = counters.running.saturating_sub(1);
            }
        }
        if state.global_running > 0 {
            state.global_running -= 1;
        }
        Ok(())
    }
}

fn active_dedupe_in_state(state: &FakeJobState, key: &JobKey) -> Option<JobRecord> {
    let job_id = state.dedupe.get(key)?;
    let record = state.jobs.get(job_id)?;
    if record.status.holds_running_quota() || record.status == JobStatus::Queued {
        Some(record.clone())
    } else {
        None
    }
}

impl FakeJobStore {
    fn fail_next_enqueue_attempts(&self, attempts: u64) {
        self.enqueue_conflicts_remaining
            .store(attempts, Ordering::Relaxed);
    }

    fn enqueue_attempts(&self) -> u64 {
        self.enqueue_attempts.load(Ordering::Relaxed)
    }

    fn set_rate_limit_per_minute(&self, max: u32) {
        self.state.lock().expect("fake store lock").rate_limit_max = Some(max);
    }

    fn rate_count(&self, caller_id: &str) -> u32 {
        self.state
            .lock()
            .expect("fake store lock")
            .rate_counts
            .get(caller_id)
            .copied()
            .unwrap_or_default()
    }

    fn seed_queued_job(
        &self,
        execution_arn: &str,
        update: impl FnOnce(&mut JobRecord),
    ) -> JobRecord {
        let job_id = format!("job-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let mut record = JobRecord {
            job_id: job_id.clone(),
            status: JobStatus::Queued,
            source: "git:custom".to_owned(),
            package: PACKAGE.to_owned(),
            revision: "main".to_owned(),
            source_url: SOURCE_URL.to_owned(),
            source_url_hash: source_url_hash(SOURCE_URL),
            source_kind: "git".to_owned(),
            caller_id: "seed".to_owned(),
            execution_arn: Some(execution_arn.to_owned()),
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
        update(&mut record);
        let mut state = self.state.lock().expect("fake store lock");
        state.dedupe.insert(record.key(), job_id.clone());
        state.jobs.insert(job_id, record.clone());
        record
    }

    /// Seed a job that already holds a running quota slot (dispatching/running)
    /// and is stale, simulating a dispatched job whose worker has not reported
    /// its first stage — or a terminal job whose release conflicted. The owner
    /// running counter, global running counter, and `RUNNING#<job_id>` token are
    /// all populated so release assertions are meaningful.
    fn seed_dispatched_job(
        &self,
        execution_arn: &str,
        status: JobStatus,
        update: impl FnOnce(&mut JobRecord),
    ) -> JobRecord {
        let job_id = format!("job-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let owner = BacklogOwner::caller("seed-owner");
        let owner_pk = owner.pk();
        let mut record = JobRecord {
            job_id: job_id.clone(),
            status,
            source: "git:custom".to_owned(),
            package: PACKAGE.to_owned(),
            revision: "main".to_owned(),
            source_url: SOURCE_URL.to_owned(),
            source_url_hash: source_url_hash(SOURCE_URL),
            source_kind: "git".to_owned(),
            caller_id: "seed".to_owned(),
            execution_arn: Some(execution_arn.to_owned()),
            attempt: 1,
            stage: None,
            snapshot_id: None,
            row_counts: None,
            error_code: None,
            error_detail: None,
            created_at: "now".to_owned(),
            updated_at: "0".to_owned(),
            owner_kind: Some(owner.kind),
            owner_id: Some(owner.id),
            queue_shard: None,
            queue_sort_key: None,
            next_eligible_at: None,
            dispatched_at: Some("dispatched".to_owned()),
        };
        update(&mut record);
        let mut state = self.state.lock().expect("fake store lock");
        let holds_token = record.status.holds_running_quota();
        if holds_token {
            state.owner_counters.entry(owner_pk).or_default().running += 1;
            state.global_running += 1;
            state.running_tokens.insert(job_id.clone());
        }
        state.dedupe.insert(record.key(), job_id.clone());
        state.jobs.insert(job_id, record.clone());
        record
    }

    fn job_count(&self) -> usize {
        self.state.lock().expect("fake store lock").jobs.len()
    }

    fn owner_queued(&self, owner: &BacklogOwner) -> u32 {
        self.state
            .lock()
            .expect("fake store lock")
            .owner_counters
            .get(&owner.pk())
            .map(|c| c.queued)
            .unwrap_or_default()
    }

    fn owner_running(&self, owner: &BacklogOwner) -> u32 {
        self.state
            .lock()
            .expect("fake store lock")
            .owner_counters
            .get(&owner.pk())
            .map(|c| c.running)
            .unwrap_or_default()
    }

    fn global_running(&self) -> u32 {
        self.state.lock().expect("fake store lock").global_running
    }

    fn has_running_token(&self, job_id: &str) -> bool {
        self.state
            .lock()
            .expect("fake store lock")
            .running_tokens
            .contains(job_id)
    }

    fn set_fail_release(&self, fail: bool) {
        self.fail_release.store(fail, Ordering::SeqCst);
    }

    fn lookup_job_sync(&self, job_id: &str) -> Option<JobRecord> {
        self.state
            .lock()
            .expect("fake store lock")
            .jobs
            .get(job_id)
            .cloned()
    }

    fn update_job(
        &self,
        job_id: &str,
        update: impl FnOnce(&mut JobRecord),
    ) -> spur_context_service::jobs::Result<JobRecord> {
        let mut state = self.state.lock().expect("fake store lock");
        let record = state
            .jobs
            .get_mut(job_id)
            .ok_or(spur_context_service::jobs::JobsError::NotFound)?;
        update(record);
        Ok(record.clone())
    }
}
