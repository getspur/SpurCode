use std::process::Command;

use spur_cli::commands::analyst::SUPPORTED_GRAPH_SCHEMA_VERSION;

fn spur_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_spur"))
}

fn fixture_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    };
    run(&["init"]);
    run(&["config", "user.email", "spur@example.test"]);
    run(&["config", "user.name", "Spur Test"]);
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn hello() -> u32 { 42 }\n",
    )
    .unwrap();
    run(&["add", "src/lib.rs"]);
    run(&["commit", "-m", "initial"]);
    dir
}

#[test]
fn graph_build_temporal_without_embeddings_materializes_analyst() {
    let dir = fixture_git_repo();
    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args([
            "graph",
            "build",
            "--workspace",
            "--no-section-embeddings",
            "--no-code-symbol-embeddings",
            "--with-temporal",
            "--quiet",
        ])
        .env_remove("SPUR_GRAPH_SKIP_ANALYST")
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .env_remove("SPUR_GRAPH_ARTIFACT_DIR")
        .env_remove("SPUR_ANALYST_DB")
        .env_remove("SPUR_GRAPH_TEMPORAL_JOBS")
        .output()
        .expect("spawn");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "graph build failed: {stderr}");

    let artifact_dir = std::fs::canonicalize(dir.path().join(".spur/graph/CURRENT"))
        .expect("CURRENT resolves to the fresh graph artifact");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(artifact_dir.join("manifest.json")).expect("read fresh manifest"),
    )
    .expect("parse fresh manifest");
    assert_eq!(
        manifest["schema_version"],
        spur_graph::store::SCHEMA_VERSION
    );

    let db_path = dir.path().join(".spur/analyst.duckdb");
    let metadata = std::fs::metadata(&db_path).unwrap_or_else(|error| {
        panic!("analyst DB must be materialized: {error}; stderr: {stderr}")
    });
    assert!(metadata.len() > 0, "analyst DB must be non-empty");

    let conn = duckdb::Connection::open(&db_path).expect("open materialized analyst DB");
    let (schema_version, content_hash, node_count, commit_count, snapshot_count): (
        String,
        String,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT schema_version, graph_content_hash,
                    (SELECT count(*) FROM nodes),
                    (SELECT count(*) FROM commits),
                    (SELECT count(*) FROM symbol_snapshots)
             FROM _meta",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("query fresh analyst metadata and structural/temporal views");
    assert_eq!(schema_version, spur_graph::store::SCHEMA_VERSION);
    assert_eq!(
        content_hash,
        manifest["graph_content_hash"].as_str().unwrap()
    );
    assert!(node_count > 0, "fixture must have graph nodes");
    assert_eq!(commit_count, 1, "fixture has one Git commit");
    assert!(snapshot_count > 0, "fixture must have temporal snapshots");
}

#[test]
fn analyst_build_skipped_by_flag() {
    let dir = fixture_git_repo();
    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--no-analyst", "--quiet"])
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.path().join(".spur/analyst.duckdb").exists(),
        "analyst DB should not have been created with --no-analyst"
    );
}

#[test]
fn analyst_build_skipped_by_env() {
    let dir = fixture_git_repo();
    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--quiet"])
        .env("SPUR_GRAPH_SKIP_ANALYST", "1")
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.path().join(".spur/analyst.duckdb").exists(),
        "analyst DB should not have been created with SPUR_GRAPH_SKIP_ANALYST=1"
    );
}

#[test]
fn analyst_build_materializes_without_duckdb_on_path() {
    let dir = fixture_git_repo();
    // Build graph first (with analyst skipped so this test stays isolated).
    let pre = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--no-analyst", "--quiet"])
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(pre.status.success());

    // Now invoke `analyst build` with a PATH that has no duckdb. The build
    // must use the bundled DuckDB library, not a system CLI.
    let empty_path = dir.path().join("empty-path");
    std::fs::create_dir_all(&empty_path).unwrap();
    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["analyst", "build"])
        .env("PATH", empty_path)
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "analyst build should exit 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("CLI not found"),
        "build should not warn about a missing duckdb CLI, got: {stderr}"
    );
    let db = dir.path().join(".spur/analyst.duckdb");
    let meta = std::fs::metadata(&db).unwrap_or_else(|error| {
        panic!(
            "analyst DB should be materialized without duckdb on PATH: {error}; stderr: {stderr}"
        )
    });
    assert!(meta.len() > 0, "analyst DB must be non-empty");
}

#[test]
fn analyst_build_rejects_schema_version_mismatch() {
    let dir = fixture_git_repo();
    // Build graph first to populate parquets.
    let pre = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--no-analyst", "--quiet"])
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(
        pre.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&pre.stderr)
    );

    // Tamper with manifest.json under the resolved CURRENT artifact.
    let current = dir.path().join(".spur/graph/CURRENT");
    let resolved = std::fs::canonicalize(&current).expect("CURRENT resolves");
    let manifest_path = resolved.join("manifest.json");
    let original = std::fs::read_to_string(&manifest_path).unwrap();
    let tampered = original.replace(SUPPORTED_GRAPH_SCHEMA_VERSION, "spur-graph-schema-vNEXT");
    assert_ne!(
        original, tampered,
        "fixture invariant: schema_version must have been present"
    );
    std::fs::write(&manifest_path, &tampered).unwrap();

    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["analyst", "build"])
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "schema mismatch should hard-fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema") && stderr.contains("vNEXT"),
        "expected schema-mismatch error, got: {stderr}"
    );
}

#[test]
fn analyst_build_atomic_under_duckdb_failure() {
    let dir = fixture_git_repo();
    // Build the graph (parquets only - skip analyst so we control the next step).
    let pre = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--no-analyst", "--quiet"])
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(
        pre.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&pre.stderr)
    );

    // Pre-populate the analyst DB with sentinel bytes that must survive the
    // failed second invocation.
    let db_path = dir.path().join(".spur/analyst.duckdb");
    std::fs::write(&db_path, b"SENTINEL-BYTES-MUST-NOT-CHANGE").unwrap();
    let before = std::fs::read(&db_path).unwrap();

    let current = dir.path().join(".spur/graph/CURRENT");
    let artifact_dir = std::fs::canonicalize(&current).expect("CURRENT resolves");
    std::fs::write(artifact_dir.join("nodes.parquet"), b"not parquet").unwrap();

    let empty_path = dir.path().join("empty-path");
    std::fs::create_dir_all(&empty_path).unwrap();

    let out = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["analyst", "build"])
        .env("PATH", &empty_path)
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "soft-degrade should exit 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read(&db_path).unwrap();
    assert_eq!(
        before, after,
        "previous analyst DB must be byte-identical after a failed duckdb run"
    );

    // No leftover tmp files alongside the DB.
    let tmp_count = std::fs::read_dir(dir.path().join(".spur"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("analyst.duckdb.tmp-")
        })
        .count();
    assert_eq!(tmp_count, 0, "leftover tmp file(s) present after failure");
}

#[test]
fn analyst_build_concurrent_skip() {
    let dir = fixture_git_repo();
    // Build the parquets first (analyst skipped - we'll exercise it concurrently below).
    let pre = Command::new(spur_binary())
        .current_dir(dir.path())
        .args(["graph", "build", "--workspace", "--no-analyst", "--quiet"])
        .env_remove("SPUR_CODE_GRAPH_INDEX")
        .output()
        .expect("spawn");
    assert!(
        pre.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&pre.stderr)
    );

    let empty_path = dir.path().join("empty-path");
    std::fs::create_dir_all(&empty_path).unwrap();

    // Launch two `analyst build` invocations close enough in time that one
    // should observe the other's flock.
    let spawn = || {
        Command::new(spur_binary())
            .current_dir(dir.path())
            .args(["analyst", "build"])
            .env("PATH", &empty_path)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn")
    };
    let a = spawn();
    let b = spawn();
    let a_out = a.wait_with_output().expect("wait a");
    let b_out = b.wait_with_output().expect("wait b");

    // Both must exit 0 (one builds, the other skips).
    assert!(
        a_out.status.success(),
        "a stderr: {}",
        String::from_utf8_lossy(&a_out.stderr)
    );
    assert!(
        b_out.status.success(),
        "b stderr: {}",
        String::from_utf8_lossy(&b_out.stderr)
    );

    let combined_stderr = format!(
        "{}{}",
        String::from_utf8_lossy(&a_out.stderr),
        String::from_utf8_lossy(&b_out.stderr)
    );
    assert!(
        combined_stderr.contains("another analyst build in progress"),
        "expected at least one process to log the contention skip; got: {combined_stderr}"
    );

    // Final DB must be valid (exists and non-empty).
    let db = dir.path().join(".spur/analyst.duckdb");
    let meta = std::fs::metadata(&db).unwrap_or_else(|error| {
        panic!("DB exists after concurrent builds: {error}; stderr: {combined_stderr}")
    });
    assert!(meta.len() > 0, "DB must be non-empty");
}
