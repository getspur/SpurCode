//! `spur-cli analyst build` - rebuild `.spur/analyst.duckdb` from the current
//! spur-graph parquet artifact.
//!
//! See: `docs/superpowers/specs/2026-05-22-analyst-db-graph-sync-design.md`.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use spur_graph::locking::try_lock_exclusive_with_timeout;
use spur_graph::store::{CODE_SYMBOLS_PARQUET, SECTIONS_PARQUET};

const INIT_SQL: &str = include_str!("../../../spur-context/analyst/init.sql");
const INIT_TEMPORAL_SQL: &str = include_str!("../../../spur-context/analyst/init_temporal.sql");
const INIT_DIAGNOSTICS_SQL: &str =
    include_str!("../../../spur-context/analyst/init_diagnostics.sql");
const INIT_ALGORITHMS_SQL: &str = include_str!("../../../spur-context/analyst/init_algorithms.sql");
const INIT_VIEWS_SQL: &str = include_str!("../../../spur-context/analyst/init_views.sql");
const INIT_TEMPORAL_SEARCH_TOKEN_SQL: &str =
    include_str!("../../../spur-context/analyst/init_search_tokens_temporal.sql");
const INIT_STATIC_SEARCH_VIEWS_SQL: &str =
    include_str!("../../../spur-context/analyst/init_search_views_static.sql");
const INIT_SEARCH_SQL: &str = include_str!("../../../spur-context/analyst/init_search.sql");
const ARTIFACT_PLACEHOLDER: &str = "__SPUR_GRAPH_ARTIFACT_DIR__";
const SECTIONS_SOURCE_PLACEHOLDER: &str = "__SPUR_SECTIONS_SOURCE_SQL__";
const SYMBOL_EMBEDDING_JOIN_PLACEHOLDER: &str = "__SPUR_SYMBOL_EMBEDDING_JOIN__";
const SYMBOL_EMBEDDING_EXPR_PLACEHOLDER: &str = "__SPUR_SYMBOL_EMBEDDING_EXPR__";

/// Compiled-in parquet schema version this analyst build understands.
///
/// Must match `manifest.json::schema_version` in the artifact dir. Hard-fail
/// on mismatch to prevent silent miscompiles where `init.sql` view definitions
/// parse but produce wrong results against a newer parquet schema.
/// Use the graph writer's version so the reader cannot drift on schema upgrades.
pub const SUPPORTED_GRAPH_SCHEMA_VERSION: &str = spur_graph::store::SCHEMA_VERSION;

/// Default relative path to the analyst DuckDB inside a worktree.
pub const DEFAULT_ANALYST_DB_REL: &str = ".spur/analyst.duckdb";

/// Default relative path to the per-worktree lock file.
pub const DEFAULT_ANALYST_LOCK_REL: &str = ".spur/analyst.duckdb.lock";

/// Options accepted by `spur-cli analyst build`.
#[derive(Debug, Clone, Default)]
pub struct AnalystBuildOptions {
    /// Override pointer resolution. Falls back to `SPUR_GRAPH_ARTIFACT_DIR`
    /// env, then `.spur/graph/CURRENT`.
    pub artifact_dir: Option<PathBuf>,
    /// Override output path. Falls back to `SPUR_ANALYST_DB` env, then
    /// `<root>/.spur/analyst.duckdb`.
    pub db_path: Option<PathBuf>,
    /// Suppress non-error output.
    pub quiet: bool,
}

/// Entry point for `spur-cli analyst build`.
///
/// `root` is the worktree root (already canonicalized by the caller).
pub fn build(root: &Path, options: AnalystBuildOptions) -> Result<()> {
    let quiet = options.quiet;
    let artifact_dir = resolve_artifact_dir(root, &options)?;
    verify_schema_version(&artifact_dir)?;
    verify_required_files(&artifact_dir)?;
    let want_temporal = temporal_files_present(&artifact_dir);
    let want_diag = diagnostics_present(&artifact_dir);

    let db_path = options
        .db_path
        .clone()
        .or_else(|| std::env::var_os("SPUR_ANALYST_DB").map(PathBuf::from))
        .unwrap_or_else(|| root.join(DEFAULT_ANALYST_DB_REL));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let lock_path = root.join(DEFAULT_ANALYST_LOCK_REL);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;
    let acquired = try_lock_exclusive_with_timeout(&lock_file, Duration::ZERO)?;
    if !acquired {
        if !quiet {
            eprintln!("[spur] another analyst build in progress, skipping");
        }
        return Ok(());
    }

    // Assemble the (pre-substitution) SQL template now so the freshness guard can
    // fingerprint it. Which scripts are included depends on optional temporal,
    // diagnostics, and parquet sidecar presence, so the fingerprint captures config
    // as well as SQL content.
    let sections_available = sections_parquet_available(&artifact_dir, quiet);
    let symbols_available = symbols_parquet_available(&artifact_dir, quiet);
    let init_search_sql = render_init_search_sql(sections_available, symbols_available)?;
    let sql_template = [
        INIT_SQL,
        if want_temporal { INIT_TEMPORAL_SQL } else { "" },
        if want_diag { INIT_DIAGNOSTICS_SQL } else { "" },
        // Graph-algorithm materialization (PageRank/components/communities) only
        // needs the static graph + onager (both from init.sql), so it always
        // runs — and must precede init_views.sql, whose Tier-B views join the
        // v_symbol_centrality/_component/_community surfaces it creates.
        INIT_ALGORITHMS_SQL,
        // Temporal views use historical shards when present. Structural fallback
        // views keep scorecard/search surfaces available when temporal is disabled.
        if want_temporal { INIT_VIEWS_SQL } else { "" },
        if want_temporal {
            INIT_TEMPORAL_SEARCH_TOKEN_SQL
        } else {
            INIT_STATIC_SEARCH_VIEWS_SQL
        },
        // Search appliance: materializes prose/code FTS indexes, DuckDB
        // embedding arrays, and BM25+cosine hybrid search() macros.
        init_search_sql.as_str(),
    ]
    .concat();

    // ---- Freshness skip-guard ----
    // The materialized DB is a pure function of (graph content, analyst SQL). If a
    // prior build recorded the same fingerprint, there is nothing to recompute, so
    // a redundant `graph build` / `analyst build` becomes an instant no-op. The
    // fingerprint folds in the analyst SQL too, so a spur-cli upgrade that changes
    // the views still forces a rebuild even when the graph hash is unchanged.
    let content_hash = read_graph_content_hash(&artifact_dir).unwrap_or_default();
    let fingerprint = build_fingerprint(&content_hash, &sql_template);
    let fp_path = fingerprint_path(&db_path);
    if db_path.is_file() && !content_hash.is_empty() {
        if let Ok(prev) = std::fs::read_to_string(&fp_path) {
            if prev.trim() == fingerprint {
                if !quiet {
                    eprintln!(
                        "[spur] Analyst DB already fresh (graph {} + analyst SQL unchanged), skipping rebuild",
                        short_hash(&content_hash)
                    );
                }
                return Ok(());
            }
        }
    }

    let started = Instant::now();
    if !quiet {
        eprintln!("[spur] Refreshing analyst DB at {}", db_path.display());
    }

    let tmp_db = db_path.with_extension(format!("duckdb.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp_db);

    if !sections_available && !quiet {
        eprintln!(
            "[spur] warning: valid parquet section sidecar not found at {} - using empty sections table",
            artifact_dir.join(SECTIONS_PARQUET).display(),
        );
    }
    let sql = render_artifact_path_placeholders(&sql_template, &artifact_dir);

    let build_result = execute_build_script(&tmp_db, &sql);

    if let Err(err) = build_result {
        let _ = std::fs::remove_file(&tmp_db);
        if !quiet {
            eprintln!("[spur] warning: duckdb init failed: {err:#}; previous analyst DB preserved");
        }
        return Ok(());
    }

    std::fs::rename(&tmp_db, &db_path).with_context(|| {
        format!(
            "failed to rename {} over {}",
            tmp_db.display(),
            db_path.display()
        )
    })?;

    // Record the fingerprint so the next invocation can skip an identical rebuild.
    // Best-effort: a missing/unwritable sidecar simply forces a (correct) rebuild.
    if let Err(err) = std::fs::write(&fp_path, &fingerprint) {
        if !quiet {
            eprintln!(
                "[spur] note: could not write freshness fingerprint {}: {err}",
                fp_path.display()
            );
        }
    }

    if !quiet {
        // Surface the schema/content hash from the manifest we already validated.
        let observed_hash = std::fs::read(artifact_dir.join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                v.get("graph_content_hash")
                    .and_then(|x| x.as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| "<unknown>".to_string());
        let elapsed = started.elapsed();
        eprintln!(
            "[spur] Analyst DB ready (graph_content_hash={}, {:.1}s)",
            short_hash(&observed_hash),
            elapsed.as_secs_f64()
        );
    }

    // Lock auto-released on file drop.
    Ok(())
}

/// Convenience wrapper used by `spur-cli graph build` to invoke the default
/// build with only the quiet flag threaded through.
pub fn build_default(root: &Path, quiet: bool) -> Result<()> {
    build(
        root,
        AnalystBuildOptions {
            quiet,
            ..Default::default()
        },
    )
}

fn execute_build_script(db_path: &Path, sql: &str) -> Result<()> {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .context("failed to configure read-write duckdb")?;
    let conn = duckdb::Connection::open_with_flags(db_path, config).with_context(|| {
        format!(
            "failed to open analyst DuckDB read-write at {}",
            db_path.display()
        )
    })?;
    // Bootstrap first so extension parser hooks are registered before parsing
    // init SQL statements such as CREATE PROPERTY GRAPH.
    conn.execute_batch(&spur_analyst::analyst_extension_bootstrap_sql())
        .context("failed to initialize duckdb analyst extensions")?;
    execute_duckdb_script(&conn, sql)
}

fn execute_duckdb_script(conn: &duckdb::Connection, sql: &str) -> Result<()> {
    // duckdb-rs execute_batch uses duckdb_query_arrow for the whole script, which
    // does not expose inter-statement catalog visibility. prepare() extracts the
    // script, but still prepares each DDL statement before executing it; in
    // 1.4.4 that fails for FTS pragmas/macros referencing tables created earlier
    // in the script. Keep the rendered SQL bytes/order intact and execute each
    // statement through the bundled connection after SQL-aware statement
    // boundary detection.
    for (idx, statement) in split_sql_statements(sql).into_iter().enumerate() {
        conn.execute_batch(statement)
            .with_context(|| format!("failed to execute analyst init SQL statement {}", idx + 1))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlScanState {
    Normal,
    SingleQuoted,
    DoubleQuoted,
    LineComment,
    BlockComment,
}

fn split_sql_statements(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut state = SqlScanState::Normal;
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        match state {
            SqlScanState::Normal => match bytes[i] {
                b'\'' => {
                    state = SqlScanState::SingleQuoted;
                    i += 1;
                }
                b'"' => {
                    state = SqlScanState::DoubleQuoted;
                    i += 1;
                }
                b'-' if bytes.get(i + 1) == Some(&b'-') => {
                    state = SqlScanState::LineComment;
                    i += 2;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    state = SqlScanState::BlockComment;
                    i += 2;
                }
                b';' => {
                    let statement = &sql[start..=i];
                    if statement_has_sql(statement) {
                        statements.push(statement);
                    }
                    start = i + 1;
                    i += 1;
                }
                _ => i += 1,
            },
            SqlScanState::SingleQuoted => {
                if bytes[i] == b'\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::DoubleQuoted => {
                if bytes[i] == b'"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::LineComment => {
                if bytes[i] == b'\n' {
                    state = SqlScanState::Normal;
                }
                i += 1;
            }
            SqlScanState::BlockComment => {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = SqlScanState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    let statement = &sql[start..];
    if statement_has_sql(statement) {
        statements.push(statement);
    }
    statements
}

fn statement_has_sql(statement: &str) -> bool {
    let bytes = statement.as_bytes();
    let mut state = SqlScanState::Normal;
    let mut i = 0;
    while i < bytes.len() {
        match state {
            SqlScanState::Normal => match bytes[i] {
                b'-' if bytes.get(i + 1) == Some(&b'-') => {
                    state = SqlScanState::LineComment;
                    i += 2;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    state = SqlScanState::BlockComment;
                    i += 2;
                }
                b if b.is_ascii_whitespace() || b == b';' => i += 1,
                _ => return true,
            },
            SqlScanState::LineComment => {
                if bytes[i] == b'\n' {
                    state = SqlScanState::Normal;
                }
                i += 1;
            }
            SqlScanState::BlockComment => {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = SqlScanState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            SqlScanState::SingleQuoted | SqlScanState::DoubleQuoted => return true,
        }
    }
    false
}

fn short_hash(hash: &str) -> String {
    if hash.len() > 8 {
        format!("{}...", &hash[..8])
    } else {
        hash.to_string()
    }
}

/// Sidecar path holding the last build's freshness fingerprint.
fn fingerprint_path(db_path: &Path) -> PathBuf {
    let mut s = db_path.as_os_str().to_owned();
    s.push(".fingerprint");
    PathBuf::from(s)
}

/// `graph_content_hash` from the artifact manifest, if present.
fn read_graph_content_hash(artifact_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(artifact_dir.join("manifest.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    parsed
        .get("graph_content_hash")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

#[derive(Debug, Clone, Copy)]
struct LanceSidecarManifestStatus {
    section_bodies: usize,
    code_symbols: usize,
}

fn manifest_sidecar_status(artifact_dir: &Path) -> Option<LanceSidecarManifestStatus> {
    let Ok(bytes) = std::fs::read(artifact_dir.join("manifest.json")) else {
        return None;
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return None;
    };
    if !parsed
        .get("sidecar_complete")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let row_counts = parsed.get("sidecar_row_counts")?;
    Some(LanceSidecarManifestStatus {
        section_bodies: manifest_usize_field(row_counts, "section_bodies"),
        code_symbols: manifest_usize_field(row_counts, "code_symbols"),
    })
}

fn manifest_usize_field(value: &serde_json::Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn sections_parquet_available(artifact_dir: &Path, quiet: bool) -> bool {
    let Some(status) = manifest_sidecar_status(artifact_dir) else {
        return false;
    };
    if status.section_bodies == 0 {
        if !quiet {
            eprintln!(
                "[spur] warning: parquet sidecar manifest has section_bodies=0 - using empty sections table"
            );
        }
        return false;
    }
    parquet_file_has_rows(
        &artifact_dir.join(SECTIONS_PARQUET),
        "section sidecar",
        quiet,
    )
}

fn symbols_parquet_available(artifact_dir: &Path, quiet: bool) -> bool {
    let Some(status) = manifest_sidecar_status(artifact_dir) else {
        return false;
    };
    if status.code_symbols == 0 {
        return false;
    }
    parquet_file_has_rows(
        &artifact_dir.join(CODE_SYMBOLS_PARQUET),
        "code symbol sidecar",
        quiet,
    )
}

fn parquet_file_has_rows(path: &Path, table_label: &str, quiet: bool) -> bool {
    match parquet_row_count(path) {
        Ok(rows) if rows > 0 => true,
        Ok(_) => {
            if !quiet {
                eprintln!(
                    "[spur] warning: {table_label} at {} is empty - using BM25-only search",
                    path.display(),
                );
            }
            false
        }
        Err(err) => {
            if !quiet {
                eprintln!(
                    "[spur] warning: could not open {table_label} at {}: {err:#} - using BM25-only search",
                    path.display(),
                );
            }
            false
        }
    }
}

fn parquet_row_count(path: &Path) -> Result<usize> {
    let path_sql = sql_escape_path(path);
    let conn = duckdb::Connection::open_in_memory()
        .context("failed to open in-memory duckdb parquet sidecar probe")?;
    conn.query_row(
        &format!("SELECT count(*) FROM read_parquet('{path_sql}');"),
        [],
        |row| row.get::<_, i64>(0),
    )
    .with_context(|| {
        format!(
            "failed to query parquet sidecar row count for {}",
            path.display()
        )
    })
    .and_then(|count| {
        usize::try_from(count)
            .with_context(|| format!("parquet sidecar row count was negative: {count}"))
    })
}

fn render_init_search_sql(sections_available: bool, symbols_available: bool) -> Result<String> {
    let sections_source = if sections_available {
        parquet_sections_source_sql()
    } else {
        empty_sections_source_sql()
    };
    let (symbol_join, symbol_expr) = if symbols_available {
        (
            format!(
                "LEFT JOIN (\n\
                 SELECT stable_symbol_id, vector::FLOAT[768] AS embedding\n\
                 FROM read_parquet('{ARTIFACT_PLACEHOLDER}/{CODE_SYMBOLS_PARQUET}')\n\
             ) symbol_embeddings USING (stable_symbol_id)"
            ),
            "symbol_embeddings.embedding".to_string(),
        )
    } else {
        (String::new(), "CAST(NULL AS FLOAT[768])".to_string())
    };
    Ok(INIT_SEARCH_SQL
        .replace(SECTIONS_SOURCE_PLACEHOLDER, &sections_source)
        .replace(SYMBOL_EMBEDDING_JOIN_PLACEHOLDER, &symbol_join)
        .replace(SYMBOL_EMBEDDING_EXPR_PLACEHOLDER, &symbol_expr))
}

fn parquet_sections_source_sql() -> String {
    format!(
        "CREATE OR REPLACE TABLE sections AS\n\
         SELECT stable_symbol_id, parent_stable_id, qualified_name, file_path,\n\
                heading_level, child_count, content_hash, body_byte_start, body_text,\n\
                vector::FLOAT[768] AS embedding\n\
         FROM read_parquet('{ARTIFACT_PLACEHOLDER}/{SECTIONS_PARQUET}')\n\
         WHERE body_text IS NOT NULL AND length(body_text) > 0;"
    )
}

fn empty_sections_source_sql() -> String {
    "CREATE OR REPLACE TABLE sections AS\n\
     SELECT CAST(NULL AS VARCHAR) AS stable_symbol_id,\n\
            CAST(NULL AS VARCHAR) AS parent_stable_id,\n\
            CAST(NULL AS VARCHAR) AS qualified_name,\n\
            CAST(NULL AS VARCHAR) AS file_path,\n\
            CAST(NULL AS INTEGER) AS heading_level,\n\
            CAST(NULL AS INTEGER) AS child_count,\n\
            CAST(NULL AS VARCHAR) AS content_hash,\n\
            CAST(NULL AS UBIGINT) AS body_byte_start,\n\
            CAST(NULL AS VARCHAR) AS body_text,\n\
            CAST(NULL AS FLOAT[768]) AS embedding\n\
     WHERE FALSE;"
        .to_string()
}

fn render_artifact_path_placeholders(sql_template: &str, artifact_dir: &Path) -> String {
    sql_template.replace(ARTIFACT_PLACEHOLDER, &sql_escape_path(artifact_dir))
}

fn sql_escape_path(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

/// FNV-1a fingerprint over `(graph content hash, assembled analyst SQL)`. Changes
/// whenever the indexed graph OR the analyst build logic changes, so the
/// skip-guard never serves a stale DB after a spur-cli upgrade. Not cryptographic
/// — change-detection only.
fn build_fingerprint(content_hash: &str, sql_template: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in content_hash
        .as_bytes()
        .iter()
        .chain(b"\0".iter())
        .chain(sql_template.as_bytes().iter())
    {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{content_hash}:{h:016x}")
}

pub(crate) fn resolve_artifact_dir(root: &Path, options: &AnalystBuildOptions) -> Result<PathBuf> {
    if let Some(explicit) = options.artifact_dir.as_ref() {
        return std::fs::canonicalize(explicit)
            .with_context(|| format!("failed to canonicalize --artifact-dir {explicit:?}"));
    }
    if let Some(env_dir) = std::env::var_os("SPUR_GRAPH_ARTIFACT_DIR") {
        let env_path = PathBuf::from(env_dir);
        return std::fs::canonicalize(&env_path).with_context(|| {
            format!("failed to canonicalize SPUR_GRAPH_ARTIFACT_DIR={env_path:?}")
        });
    }
    let current = root.join(".spur").join("graph").join("CURRENT");
    if current.exists() {
        return std::fs::canonicalize(&current)
            .with_context(|| format!("failed to resolve {} symlink", current.display()));
    }
    Err(anyhow!(
        "spur-graph CURRENT pointer not found at {} - run `spur-cli graph build --workspace` \
         or set SPUR_GRAPH_ARTIFACT_DIR to a parquet artifact directory",
        current.display()
    ))
}

pub(crate) fn verify_schema_version(artifact_dir: &Path) -> Result<()> {
    let manifest_path = artifact_dir.join("manifest.json");
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {} as JSON", manifest_path.display()))?;
    let observed = parsed
        .get("schema_version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow!(
                "manifest.json at {} is missing string field `schema_version`",
                manifest_path.display()
            )
        })?;
    if observed != SUPPORTED_GRAPH_SCHEMA_VERSION {
        return Err(anyhow!(
            "analyst build refuses to run: parquet schema_version {observed:?} does not match \
             SUPPORTED_GRAPH_SCHEMA_VERSION {SUPPORTED_GRAPH_SCHEMA_VERSION:?}. Rebuild spur-cli \
             against the new schema or rebuild the graph against the supported schema."
        ));
    }
    Ok(())
}

const REQUIRED_PARQUETS: &[&str] = &[
    "nodes.parquet",
    "edges.parquet",
    "edges_by_dst.parquet",
    "edges_unresolved.parquet",
    "files.parquet",
    "file_manifests.parquet",
    "tombstones.parquet",
    "manifest.json",
];

pub(crate) fn verify_required_files(artifact_dir: &Path) -> Result<()> {
    let missing: Vec<&str> = REQUIRED_PARQUETS
        .iter()
        .copied()
        .filter(|name| !artifact_dir.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "spur-graph artifact at {} is missing required file(s): {}",
        artifact_dir.display(),
        missing.join(", ")
    ))
}

pub(crate) fn temporal_files_present(artifact_dir: &Path) -> bool {
    let manifest_path = artifact_dir.join("manifest.json");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    parsed
        .get("temporal_shards")
        .and_then(|value| value.as_array())
        .is_some_and(|shards| !shards.is_empty())
}

pub(crate) fn diagnostics_present(artifact_dir: &Path) -> bool {
    artifact_dir.join("diagnostics.parquet").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn supported_graph_schema_version_matches_graph_store() {
        assert_eq!(
            SUPPORTED_GRAPH_SCHEMA_VERSION,
            spur_graph::store::SCHEMA_VERSION
        );
    }

    #[test]
    fn split_sql_statements_ignores_semicolons_in_literals_and_comments() {
        let statements = split_sql_statements(
            "SELECT ';' AS literal; -- ignored ;\n\
             /* ignored ; */\n\
             CREATE TABLE \"semi;colon\"(value VARCHAR DEFAULT 'a; b')",
        );

        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "SELECT ';' AS literal;");
        assert!(statements[1].contains("CREATE TABLE \"semi;colon\""));
    }

    #[test]
    fn execute_duckdb_script_handles_fts_forward_reference() {
        let conn = duckdb::Connection::open_in_memory().expect("open duckdb");
        conn.execute_batch("INSTALL fts; LOAD fts;")
            .expect("load fts");

        execute_duckdb_script(
            &conn,
            "CREATE OR REPLACE TABLE sections AS
             SELECT 'section-1' AS stable_symbol_id, 'delegation text' AS body_text;
             CREATE OR REPLACE TABLE sections_search AS
             SELECT stable_symbol_id, body_text FROM sections;
             PRAGMA create_fts_index('sections_search', 'stable_symbol_id', 'body_text', overwrite=1);",
        )
        .expect("execute fts script");

        let hit_count: i64 = conn
            .query_row(
                "SELECT count(*)
                 FROM sections_search s
                 WHERE fts_main_sections_search.match_bm25(s.stable_symbol_id, 'delegation')
                       IS NOT NULL;",
                [],
                |row| row.get(0),
            )
            .expect("query fts hits");
        assert_eq!(hit_count, 1);
    }

    #[test]
    fn duckdb_cosine_hybrid_ranks_nearer_embedding_first() {
        let conn = duckdb::Connection::open_in_memory().expect("open duckdb");
        conn.execute_batch(
            "CREATE TABLE items (
                 id VARCHAR,
                 embedding FLOAT[2]
             );
             INSERT INTO items VALUES
                 ('near', [1.0, 0.0]::FLOAT[2]),
                 ('far', [0.0, 1.0]::FLOAT[2]);",
        )
        .expect("create cosine fixture");
        let winner: String = conn
            .query_row(
                "SELECT id
                 FROM items
                 ORDER BY array_cosine_distance(embedding, [1.0, 0.0]::FLOAT[2])
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("rank by cosine");
        assert_eq!(winner, "near");
    }

    #[test]
    fn resolve_artifact_dir_prefers_explicit_option() {
        let root = temp_root();
        let target = root.path().join("explicit-artifact");
        fs::create_dir_all(&target).unwrap();

        let opts = AnalystBuildOptions {
            artifact_dir: Some(target.clone()),
            ..Default::default()
        };
        let resolved = resolve_artifact_dir(root.path(), &opts).expect("resolve");
        assert_eq!(resolved, fs::canonicalize(&target).unwrap());
    }

    #[test]
    fn resolve_artifact_dir_falls_back_to_env() {
        let _env_guard = env_lock();
        let root = temp_root();
        let target = root.path().join("env-artifact");
        fs::create_dir_all(&target).unwrap();
        let prev = std::env::var_os("SPUR_GRAPH_ARTIFACT_DIR");
        std::env::set_var("SPUR_GRAPH_ARTIFACT_DIR", &target);

        let resolved = resolve_artifact_dir(root.path(), &AnalystBuildOptions::default());

        // Restore env before asserting so test isolation is maintained even on
        // assertion failure.
        match prev {
            Some(v) => std::env::set_var("SPUR_GRAPH_ARTIFACT_DIR", v),
            None => std::env::remove_var("SPUR_GRAPH_ARTIFACT_DIR"),
        }

        let resolved = resolved.expect("resolve");
        assert_eq!(resolved, fs::canonicalize(&target).unwrap());
    }

    #[test]
    fn resolve_artifact_dir_uses_current_pointer() {
        let _env_guard = env_lock();
        let root = temp_root();
        let artifact = root.path().join(".git/spur-graph/artifacts/v/h.parquet");
        fs::create_dir_all(&artifact).unwrap();
        let graph_dir = root.path().join(".spur/graph");
        fs::create_dir_all(&graph_dir).unwrap();
        std::os::unix::fs::symlink(&artifact, graph_dir.join("CURRENT")).unwrap();

        let prev = std::env::var_os("SPUR_GRAPH_ARTIFACT_DIR");
        std::env::remove_var("SPUR_GRAPH_ARTIFACT_DIR");

        let resolved = resolve_artifact_dir(root.path(), &AnalystBuildOptions::default());

        if let Some(v) = prev {
            std::env::set_var("SPUR_GRAPH_ARTIFACT_DIR", v);
        }

        let resolved = resolved.expect("resolve");
        assert_eq!(resolved, fs::canonicalize(&artifact).unwrap());
    }

    #[test]
    fn resolve_artifact_dir_errors_when_nothing_resolves() {
        let _env_guard = env_lock();
        let root = temp_root();
        let prev = std::env::var_os("SPUR_GRAPH_ARTIFACT_DIR");
        std::env::remove_var("SPUR_GRAPH_ARTIFACT_DIR");

        let err = resolve_artifact_dir(root.path(), &AnalystBuildOptions::default())
            .expect_err("should error");

        if let Some(v) = prev {
            std::env::set_var("SPUR_GRAPH_ARTIFACT_DIR", v);
        }

        let msg = format!("{err:#}");
        assert!(
            msg.contains("CURRENT") || msg.contains("graph build"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn verify_schema_version_accepts_graph_writer_version() {
        let dir = temp_root();
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::json!({ "schema_version": spur_graph::store::SCHEMA_VERSION }).to_string(),
        )
        .unwrap();
        verify_schema_version(dir.path()).expect("graph writer schema version must be accepted");
    }

    #[test]
    fn verify_schema_version_rejects_mismatch() {
        let dir = temp_root();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"schema_version":"spur-graph-schema-vNEXT"}"#,
        )
        .unwrap();
        let err = verify_schema_version(dir.path()).expect_err("mismatch should error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(SUPPORTED_GRAPH_SCHEMA_VERSION) && msg.contains("vNEXT"),
            "expected both versions in error, got: {msg}"
        );
    }

    #[test]
    fn verify_schema_version_rejects_old_writer_version() {
        let dir = temp_root();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"schema_version":"spur-graph-schema-v10"}"#,
        )
        .unwrap();
        let err = verify_schema_version(dir.path()).expect_err("old writer schema should error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(spur_graph::store::SCHEMA_VERSION) && msg.contains("v10"),
            "expected both versions in error, got: {msg}"
        );
    }

    #[test]
    fn verify_schema_version_errors_on_missing_manifest() {
        let dir = temp_root();
        let err = verify_schema_version(dir.path()).expect_err("missing manifest should error");
        assert!(format!("{err:#}").contains("manifest.json"));
    }

    #[test]
    fn verify_schema_version_errors_on_malformed_manifest() {
        let dir = temp_root();
        std::fs::write(dir.path().join("manifest.json"), "not json").unwrap();
        let err = verify_schema_version(dir.path()).expect_err("malformed should error");
        assert!(format!("{err:#}").contains("manifest.json"));
    }

    #[test]
    fn build_fingerprint_is_deterministic_and_sensitive() {
        let base = build_fingerprint("hashA", "SQL-1");
        // deterministic
        assert_eq!(base, build_fingerprint("hashA", "SQL-1"));
        // a different graph hash changes the fingerprint
        assert_ne!(base, build_fingerprint("hashB", "SQL-1"));
        // a different analyst SQL changes the fingerprint (spur-cli upgrade case)
        assert_ne!(base, build_fingerprint("hashA", "SQL-2"));
        // the human-readable prefix is the content hash
        assert!(base.starts_with("hashA:"));
    }

    #[test]
    fn read_graph_content_hash_round_trips() {
        let dir = temp_root();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"graph_content_hash":"abc123","schema_version":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            read_graph_content_hash(dir.path()).as_deref(),
            Some("abc123")
        );
        // missing manifest → None (forces a rebuild, never a wrong skip)
        assert_eq!(read_graph_content_hash(temp_root().path()), None);
    }

    const REQUIRED_PARQUETS: &[&str] = &[
        "nodes.parquet",
        "edges.parquet",
        "edges_by_dst.parquet",
        "edges_unresolved.parquet",
        "files.parquet",
        "file_manifests.parquet",
        "tombstones.parquet",
        "manifest.json",
    ];

    fn populate_required(dir: &Path) {
        for name in REQUIRED_PARQUETS {
            std::fs::write(dir.join(name), b"").unwrap();
        }
    }

    #[test]
    fn verify_required_files_ok_when_all_present() {
        let dir = temp_root();
        populate_required(dir.path());
        verify_required_files(dir.path()).expect("all present");
    }

    #[test]
    fn verify_required_files_errors_listing_missing() {
        let dir = temp_root();
        populate_required(dir.path());
        std::fs::remove_file(dir.path().join("edges.parquet")).unwrap();
        std::fs::remove_file(dir.path().join("tombstones.parquet")).unwrap();
        let err = verify_required_files(dir.path()).expect_err("missing should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("edges.parquet"), "missing edges in: {msg}");
        assert!(
            msg.contains("tombstones.parquet"),
            "missing tombstones in: {msg}"
        );
    }

    #[test]
    fn temporal_files_present_true_when_any_temporal_parquet_exists() {
        let dir = temp_root();
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"temporal_shards":[{"shard_idx":0}]}"#,
        )
        .unwrap();

        assert!(temporal_files_present(dir.path()));
    }

    #[test]
    fn temporal_files_present_false_on_empty_dir() {
        let dir = temp_root();

        assert!(!temporal_files_present(dir.path()));
    }

    #[test]
    fn diagnostics_present_true() {
        let dir = temp_root();
        std::fs::write(dir.path().join("diagnostics.parquet"), b"").unwrap();

        assert!(diagnostics_present(dir.path()));
    }

    #[test]
    fn diagnostics_present_false_otherwise() {
        let dir = temp_root();

        assert!(!diagnostics_present(dir.path()));
    }

    #[test]
    fn init_views_sql_guards_direct_symbol_snapshot_coverage_without_bridge() {
        let bridge_view_name = ["v", "symbol", "id", "bridge"].join("_");

        assert!(
            !INIT_VIEWS_SQL.contains(&bridge_view_name),
            "init_views.sql must not define the transitional bridge view"
        );
        // The guard must measure DISTINCT node coverage, not raw join rows
        // (which are inflated by snapshot churn multiplicity).
        assert!(
            INIT_VIEWS_SQL.contains("COUNT(*) FROM trackable_nodes"),
            "init_views.sql must assert distinct trackable-node snapshot coverage"
        );
        assert!(
            INIT_VIEWS_SQL.contains("covered_nodes * 100 >= expected_nodes * 90"),
            "trackable-node snapshot coverage must be at least 90%"
        );
        // section/mcp_tool kinds are never emitted as snapshots, so they must be
        // excluded from the denominator or the guard can never pass.
        assert!(
            INIT_VIEWS_SQL.contains("NOT IN ('section', 'mcp_tool')"),
            "coverage guard must exclude non-temporally-trackable kinds"
        );
    }

    #[test]
    fn analyst_bootstrap_sql_skips_cdn_when_extension_dir_is_set() {
        let _guard = env_lock();
        let previous = std::env::var_os(spur_analyst::ANALYST_EXTENSION_DIR_ENV);
        std::env::set_var(
            spur_analyst::ANALYST_EXTENSION_DIR_ENV,
            "/opt/duckdb/extensions",
        );
        let sql = spur_analyst::analyst_extension_bootstrap_sql();
        match previous {
            Some(value) => std::env::set_var(spur_analyst::ANALYST_EXTENSION_DIR_ENV, value),
            None => std::env::remove_var(spur_analyst::ANALYST_EXTENSION_DIR_ENV),
        }
        assert!(
            !sql.contains("INSTALL "),
            "vendored extension dir must not INSTALL from the CDN: {sql}"
        );
        assert!(sql.contains("LOAD onager;"));
        assert!(sql.contains("LOAD duckpgq;"));
    }

    #[test]
    fn init_sql_does_not_network_install_community_extensions() {
        assert!(
            !INIT_SQL.contains("INSTALL duckpgq FROM community"),
            "init.sql must not INSTALL duckpgq from the community CDN; analyst bootstrap LOADs it"
        );
        assert!(
            !INIT_SQL.contains("INSTALL onager"),
            "init.sql must not INSTALL onager from the network; analyst bootstrap LOADs it"
        );
        assert!(
            !INIT_SEARCH_SQL.contains("INSTALL fts"),
            "init_search.sql must not INSTALL fts from the network after bootstrap LOAD"
        );
    }

    #[test]
    fn init_sql_defines_external_dependency_surface() {
        assert!(
            INIT_SQL.contains("CREATE OR REPLACE VIEW external_nodes"),
            "init.sql must define external_nodes in the structural analyst layer"
        );
        assert!(
            INIT_SQL.contains("CREATE OR REPLACE VIEW v_dependency_surface"),
            "init.sql must define v_dependency_surface in the structural analyst layer"
        );
        assert!(
            INIT_SQL.contains("LABEL External"),
            "DuckPGQ graph must expose External-labeled vertices"
        );
        assert!(
            INIT_SQL.contains("LABEL imports"),
            "DuckPGQ graph must expose imports-labeled edges"
        );
    }

    #[test]
    fn init_sql_defines_cross_crate_call_surface() {
        assert!(
            INIT_SQL.contains("CREATE OR REPLACE VIEW v_cross_crate_calls"),
            "init.sql must define v_cross_crate_calls in the structural analyst layer"
        );
        assert!(
            INIT_SQL.contains("bind_method"),
            "cross-crate call surface must expose bind_method provenance"
        );
        assert!(
            INIT_SQL.contains("CREATE OR REPLACE VIEW v_import_licensed_precision_gate"),
            "init.sql must define the import_licensed precision gate query"
        );
    }

    #[test]
    fn init_sql_unions_file_and_stub_vertices_into_duckpgq_nodes() {
        let duckpgq_nodes_sql = INIT_SQL
            .split("CREATE OR REPLACE TABLE duckpgq_nodes AS")
            .nth(1)
            .and_then(|rest| {
                rest.split("CREATE OR REPLACE TABLE duckpgq_external_nodes")
                    .next()
            })
            .expect("duckpgq_nodes table should be present");
        assert!(
            duckpgq_nodes_sql.contains("FROM files")
                && duckpgq_nodes_sql.contains("stable_file_id")
                && duckpgq_nodes_sql.contains("'file'"),
            "duckpgq_nodes must union file vertices so contains/imports FKs MATCH SIMPLE"
        );
        assert!(
            duckpgq_nodes_sql.contains("node_dense_id_map")
                && duckpgq_nodes_sql.contains("'stub'"),
            "duckpgq_nodes must materialize stub vertices for edge endpoints missing from nodes.parquet"
        );
    }

    #[test]
    fn init_sql_excludes_contains_from_onager_dep_edges() {
        let dep_sql = INIT_SQL
            .split("CREATE OR REPLACE VIEW onager_dep_edges AS")
            .nth(1)
            .and_then(|rest| rest.split("CREATE OR REPLACE PROPERTY GRAPH").next())
            .expect("onager_dep_edges view should be present");
        assert!(
            dep_sql.contains("relation <> 'contains'")
                || dep_sql.contains("relation != 'contains'"),
            "onager_dep_edges must exclude structural contains edges from connectivity algorithms"
        );
    }

    #[test]
    fn analyst_sql_leaves_missing_pagerank_null() {
        assert!(
            !INIT_ALGORITHMS_SQL.contains("COALESCE(pr.pagerank, 0.0)"),
            "v_symbol_centrality must not zero-fill missing PageRank"
        );
        assert!(
            !INIT_VIEWS_SQL.contains("COALESCE(ct.pagerank, 0.0)"),
            "temporal scorecard/risk must not zero-fill missing PageRank"
        );
        assert!(
            !INIT_STATIC_SEARCH_VIEWS_SQL.contains("COALESCE(ct.pagerank, 0.0)"),
            "structural scorecard/risk must not zero-fill missing PageRank"
        );
    }

    #[test]
    fn init_search_sql_ranks_code_with_graph_signals() {
        // search_code/search must fold centrality + kind/test weighting into the ORDER BY,
        // not order by bare bm25 (which let a leaf constant outrank a load-bearing impl).
        let search_code_sql = INIT_SEARCH_SQL
            .split("CREATE OR REPLACE MACRO search_code(q) AS TABLE")
            .nth(1)
            .and_then(|rest| rest.split("-- Unified:").next())
            .expect("search_code macro should be present");
        assert!(
            !search_code_sql.contains("ORDER BY bm25 DESC NULLS LAST\n  LIMIT 25"),
            "search_code must not order by bare bm25"
        );
        assert!(
            search_code_sql.contains("ln(1 + pagerank * 1e4)")
                && search_code_sql.contains("'%/tests/%'"),
            "code ranking must boost by pagerank and penalize test paths"
        );
    }

    #[test]
    fn init_search_sql_dedups_and_diversifies() {
        let sections_search_sql = INIT_SEARCH_SQL
            .split("CREATE OR REPLACE TABLE sections_search AS")
            .nth(1)
            .and_then(|rest| {
                rest.split("PRAGMA create_fts_index('sections_search'")
                    .next()
            })
            .expect("sections_search table should be present");
        assert!(
            sections_search_sql.contains(
                "PARTITION BY heading_level,\n               regexp_replace(COALESCE(body_text, ''), '<!-- SPUR-MANAGED[^>]*-->\\n?', '')",
            ) && !sections_search_sql.contains("COALESCE(qualified_name"),
            "sections_search must dedup on heading level plus full normalized body"
        );
        assert!(
            INIT_SEARCH_SQL.contains("PARTITION BY file ORDER BY rank DESC) <= 2")
                && INIT_SEARCH_SQL.contains("PARTITION BY s.file_path ORDER BY bm25 DESC) <= 2"),
            "search/search_docs must cap results at 2 per document"
        );
    }

    #[test]
    fn init_search_sql_graph_macro_has_gate_and_neighbor_kind() {
        assert!(
            INIT_SEARCH_SQL.contains("posture != 'load-bearing wall' OR"),
            "search_graph macro must contain the sink-bailout gate"
        );
        assert!(
            INIT_SEARCH_SQL.contains("neighbor_kind")
                && INIT_SEARCH_SQL.contains("edge_bind_method"),
            "search_graph macro must project neighbor_kind and edge_bind_method"
        );
        assert!(
            INIT_SEARCH_SQL.contains("CREATE OR REPLACE MACRO search_graph(q, intent)"),
            "search_graph macro must be defined"
        );
    }

    #[test]
    fn init_search_sql_materializes_symbol_scorecard() {
        // search_code / search_context_candidates join v_symbol_scorecard. The
        // init_views definition recomputes v_blast_radius + churn per query
        // (~800 ms). Snapshot it before the macros so those joins hit a table.
        let snapshot_at = INIT_SEARCH_SQL
            .find("CREATE OR REPLACE TABLE symbol_scorecard AS\nSELECT * FROM v_symbol_scorecard;")
            .expect("init_search.sql must snapshot v_symbol_scorecard into symbol_scorecard");
        let wrap_at = INIT_SEARCH_SQL
            .find("CREATE OR REPLACE VIEW v_symbol_scorecard AS\nSELECT * FROM symbol_scorecard;")
            .expect("init_search.sql must re-point v_symbol_scorecard at the snapshot table");
        let search_code_at = INIT_SEARCH_SQL
            .find("CREATE OR REPLACE MACRO search_code(q) AS TABLE")
            .expect("search_code macro should be present");
        assert!(
            snapshot_at < wrap_at && wrap_at < search_code_at,
            "scorecard snapshot must replace the view before search macros"
        );
    }

    #[test]
    fn init_search_sql_context_candidates_macro_present() {
        assert!(
            INIT_SEARCH_SQL.contains(
                "CREATE OR REPLACE MACRO search_context_candidates(q, requested_scope, intent) AS TABLE",
            ),
            "init_search.sql must define the context candidate macro"
        );
        for required in [
            "stable_symbol_id",
            "neighbor_kind",
            "edge_bind_method",
            "grounding",
            "requested_scope",
        ] {
            assert!(
                INIT_SEARCH_SQL.contains(required),
                "context candidate macro must project {required}"
            );
        }
    }

    #[test]
    fn init_sql_does_not_load_or_attach_lance() {
        // SOLVE PRE sol_9ec16f460ca241b0: lance_sidecar excluded from duckdb_only.
        assert!(
            !INIT_SQL.contains("LOAD lance")
                && !INIT_SQL.contains("__SPUR_LANCE_ATTACH_SQL__")
                && !INIT_SQL.contains("TYPE LANCE")
                && !INIT_SQL.contains("TYPE lance"),
            "analyst init must be DuckDB-only after Lance teardown"
        );
    }

    #[test]
    fn render_init_search_sql_reads_sections_from_parquet() {
        let sql = render_init_search_sql(true, true).expect("render parquet sections source");
        assert!(
            sql.contains("read_parquet(")
                && sql.contains("sections.parquet")
                && sql.contains("vector::FLOAT[768] AS embedding")
                && !sql.contains("lance_ns")
                && !sql.contains("TYPE LANCE")
                && !sql.contains("TYPE lance"),
            "sections source must read parquet embeddings, not ATTACH Lance"
        );
    }

    #[test]
    fn init_search_sql_duckdb_only_embeddings_and_cosine_hybrid() {
        // SOLVE PRE: sol_4b92ab5eeff746b6 brute_force + embeddings_parquet;
        // sol_63f13d778ff141b5 vector_kind=0 impl_cost=1; persist HNSW unsat.
        assert_eq!(spur_graph::EMBEDDING_VECTOR_DIMENSIONS, 768);
        assert!(
            INIT_SEARCH_SQL.contains("CAST(NULL AS FLOAT[768]) AS embedding")
                || INIT_SEARCH_SQL.contains("embedding FLOAT[768]")
                || INIT_SEARCH_SQL.contains("AS embedding"),
            "sections_search/symbol_text must carry DuckDB embedding arrays"
        );
        assert!(
            INIT_SEARCH_SQL.contains(
                "CREATE OR REPLACE MACRO search_context_candidates_hybrid(q, requested_scope, intent, query_vec) AS TABLE",
            ) && INIT_SEARCH_SQL.contains("array_cosine_distance(")
                && INIT_SEARCH_SQL.contains("query_vec IS NULL"),
            "hybrid fusion must rank DuckDB embeddings with array_cosine_distance and keep BM25 fallback"
        );
        assert!(
            !INIT_SEARCH_SQL.contains("lance_hybrid_search(")
                && !INIT_SEARCH_SQL.contains("USING HNSW")
                && !INIT_SEARCH_SQL.contains("hnsw_enable_experimental_persistence"),
            "duckdb-only hybrid must not call Lance ANN or persistent HNSW"
        );
    }

    #[test]
    fn render_init_search_sql_keeps_cosine_hybrid_without_lance() {
        let sql = render_init_search_sql(false, false).expect("render bm25-only sections source");
        assert!(
            sql.contains("search_context_candidates_hybrid")
                && sql.contains("array_cosine_distance(")
                && !sql.contains("lance_hybrid_search("),
            "cosine hybrid must stay available when Lance sidecars are absent"
        );
    }

    #[test]
    fn init_search_sql_hybrid_macro_has_per_doc_dedup() {
        assert!(
            INIT_SEARCH_SQL
                .contains("COALESCE(stable_symbol_id, file_path || ':' || title) AS candidate_id")
                && INIT_SEARCH_SQL.contains("PARTITION BY candidate_id")
                && INIT_SEARCH_SQL.contains("WHERE representative_rank = 1"),
            "hybrid context candidates must deduplicate by stable symbol before final ranking"
        );
    }

    #[test]
    fn init_search_sql_hybrid_does_not_reference_lance_dataset_paths() {
        let artifact_dir = Path::new("/tmp/spur-graph-artifact");
        let rendered = render_artifact_path_placeholders(INIT_SEARCH_SQL, artifact_dir);
        assert!(
            !rendered.contains("lance_hybrid_search(")
                && !rendered.contains("/code_symbols.lance")
                && !rendered.contains("/section_bodies.lance"),
            "duckdb-only hybrid must not reference Lance dataset paths"
        );
    }

    #[test]
    #[cfg(unix)]
    fn build_happy_path_against_bundled_duckdb_if_artifact_present() {
        let _env_guard = env_lock();
        // Real artifacts are required; this test piggybacks on the
        // repo's own .spur/graph/CURRENT if available, otherwise skips.
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let artifact_manifest_present =
            |path: &std::path::Path| path.join("manifest.json").is_file();
        let copied_artifact = repo_root.join("real-artifact/current.parquet");
        let artifact_dir = std::env::var_os("SPUR_GRAPH_ARTIFACT_DIR")
            .map(PathBuf::from)
            .filter(|path| artifact_manifest_present(path))
            .or_else(|| artifact_manifest_present(&copied_artifact).then_some(copied_artifact));
        let current = repo_root.join(".spur/graph/CURRENT");
        if artifact_dir.is_none() && !artifact_manifest_present(&current) {
            eprintln!(
                "skipping: no SPUR_GRAPH_ARTIFACT_DIR, real-artifact/current.parquet, or .spur/graph/CURRENT in repo"
            );
            return;
        }
        let tmp_db = repo_root
            .join(".spur")
            .join(format!("analyst.test-{}.duckdb", std::process::id()));
        let _ = std::fs::remove_file(&tmp_db);
        let _ = std::fs::remove_file(fingerprint_path(&tmp_db));

        let opts = AnalystBuildOptions {
            artifact_dir,
            db_path: Some(tmp_db.clone()),
            quiet: true,
        };
        build(repo_root, opts).expect("build");

        assert!(
            tmp_db.is_file(),
            "db file not created at {}",
            tmp_db.display()
        );
        let db_size = std::fs::metadata(&tmp_db).expect("db metadata").len();
        assert!(db_size > 0, "db file is empty at {}", tmp_db.display());

        let conn = duckdb::Connection::open(&tmp_db).expect("open real-artifact analyst db");
        conn.execute_batch("INSTALL fts; LOAD fts;")
            .expect("load fts");
        let fts_hit_count: i64 = conn
            .query_row(
                "SELECT count(*)
                 FROM sections_search s
                 WHERE fts_main_sections_search.match_bm25(s.stable_symbol_id, 'delegation')
                       IS NOT NULL;",
                [],
                |row| row.get(0),
            )
            .expect("query sections_search FTS hits");
        assert!(
            fts_hit_count > 0,
            "expected real artifact FTS query to return hits"
        );
        eprintln!("real-artifact analyst db_size_bytes={db_size} fts_hit_count={fts_hit_count}");
        let _ = std::fs::remove_file(&tmp_db);
        let _ = std::fs::remove_file(fingerprint_path(&tmp_db));
    }
}
