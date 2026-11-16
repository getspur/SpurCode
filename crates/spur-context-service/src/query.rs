//! Query builders for the external code context MCP tools.

use crate::catalog::readable_table;
use anyhow::{anyhow, bail, Context as _, Result};
use duckdb::{params, Connection, Row, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const DEFAULT_SOURCE: &str = "registry:crates-io";
const PKG_SYMBOL_URI_PREFIX: &str = "pkg-symbol://";
const CALL_EDGE_KIND_CALLS: &str = "calls";
const CALL_EDGE_KIND_DYN: &str = "calls_dyn";
const CALL_EDGE_KIND_HOF: &str = "references_hof";

#[derive(Debug)]
struct QueryTables {
    nodes: String,
    files: String,
    file_manifests: String,
    edges: String,
    edges_unresolved: String,
    refs: String,
    package_catalog: String,
    uses_gold: bool,
}

impl QueryTables {
    fn load(db: &Connection) -> Result<Self> {
        let nodes = readable_table(db, "nodes")?;
        let uses_gold = nodes.starts_with("gold.");
        Ok(Self {
            nodes,
            files: readable_table(db, "files")?,
            file_manifests: readable_table(db, "file_manifests")?,
            edges: readable_table(db, "edges")?,
            edges_unresolved: readable_table(db, "edges_unresolved")?,
            refs: readable_table(db, "refs")?,
            package_catalog: readable_table(db, "package_catalog")?,
            uses_gold,
        })
    }

    fn published_filter(&self, alias: &str) -> String {
        if self.uses_gold {
            format!(
                r"
                AND {alias}.generation = (
                    SELECT pc.generation
                    FROM {} pc
                    WHERE pc.source = {alias}.source
                      AND pc.package = {alias}.package
                      AND pc.revision = {alias}.revision
                    LIMIT 1
                )
                ",
                self.package_catalog
            )
        } else {
            String::new()
        }
    }

    fn same_generation_join(&self, left: &str, right: &str) -> &'static str {
        if self.uses_gold {
            match (left, right) {
                ("n", "e") => "AND n.generation = e.generation",
                ("f", "n") => "AND f.generation = n.generation",
                _ => "",
            }
        } else {
            ""
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Exact,
    Prefix,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub source: String,
    pub package: String,
    pub revision: String,
    pub query: String,
    pub mode: SearchMode,
    pub symbol_kind: Option<String>,
    pub file_glob: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeSearchResult {
    pub candidates: Vec<CodeCandidate>,
    pub total_matches: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogLevel {
    Packages,
    Revisions,
    Tree,
    Symbols,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogPage<T> {
    pub level: CatalogLevel,
    pub rows: Vec<T>,
    pub total_matches: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub catalog_generation: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageCatalogRow {
    pub source: String,
    pub package: String,
    pub latest_revision: Option<String>,
    pub revision_count: usize,
    pub indexed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RevisionCatalogRow {
    pub revision: String,
    pub revision_kind: String,
    pub semver: Option<String>,
    pub indexed_at: String,
    pub embeddings_status: Option<String>,
    pub row_counts: Value,
    pub generation: Option<i64>,
    pub snapshot_id: i64,
    pub refs: Vec<RevisionRefRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevisionRefRow {
    pub ref_name: String,
    pub revision: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeCatalogEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub file_count: usize,
    pub symbol_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolCatalogRow {
    pub entity_name: String,
    pub qualified_name: String,
    pub symbol_kind: String,
    pub line_range: [usize; 2],
    pub selector: String,
    pub next: Vec<CatalogNextTool>,
    #[serde(skip_serializing)]
    pub stable_symbol_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogNextTool {
    pub tool: String,
    pub selector: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeCandidate {
    pub selector: String,
    pub uri: String,
    pub id: String,
    pub stable_symbol_id: String,
    pub source: String,
    pub package: String,
    pub revision: String,
    pub entity_name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_range: [usize; 2],
    pub symbol_kind: String,
    pub enclosing_scope: Option<String>,
}

pub type CandidateRow = CodeCandidate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolSource {
    pub selector: String,
    pub stable_symbol_id: String,
    pub package_source: String,
    pub package: String,
    pub revision: String,
    pub file_path: String,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeMetadata {
    pub source_stable_id: String,
    pub target_stable_id: Option<String>,
    pub target_label: Option<String>,
    pub target_package: Option<String>,
    pub relation: String,
    pub edge_kind: String,
    pub confidence: Option<String>,
    pub confidence_score: Option<f64>,
    pub bind_method: Option<String>,
    pub receiver_text: Option<String>,
    pub scope_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CountsByKind {
    pub calls: usize,
    pub calls_dyn: usize,
    pub references_hof: usize,
    pub references_other: usize,
    pub unresolved: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallerRecord {
    pub caller: CodeCandidate,
    pub edge: EdgeMetadata,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalleeRecord {
    pub callee: Option<CodeCandidate>,
    pub edge: EdgeMetadata,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallerResult {
    pub callers: Vec<CallerRecord>,
    pub counts_by_kind: CountsByKind,
    pub unresolved_sample: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalleeResult {
    pub callees: Vec<CalleeRecord>,
    pub counts_by_kind: CountsByKind,
    pub unresolved_sample: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SelectorResolution {
    Resolved(ResolvedSymbol),
    Ambiguous { candidates: Vec<CandidateRow> },
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSymbol {
    pub stable_symbol_id: String,
}

#[derive(Debug, Error)]
#[error("ambiguous selector `{selector}` matched {candidate_count} symbols")]
pub struct AmbiguousSelectorError {
    pub selector: String,
    pub candidate_count: usize,
    pub candidates: Vec<CodeCandidate>,
}

pub fn search_symbols(db: &Connection, opts: &SearchOptions) -> Result<CodeSearchResult> {
    let tables = QueryTables::load(db)?;
    let (count_where, count_params, _) = search_filter(opts);
    let count_sql = format!(
        "SELECT COUNT(*) FROM {} n {count_where} {}",
        tables.nodes,
        tables.published_filter("n")
    );
    let total_matches = query_count(db, &count_sql, count_params)
        .context("failed to count matching code symbols")?;

    let (select_where, mut select_params, query_param) = search_filter(opts);
    let limit = opts.limit.clamp(1, 200) as i64;
    let limit_param = select_params.push(limit);
    let order_by = search_order_by(opts.mode, &query_param);
    let select_sql = format!(
        r"
        SELECT
            source,
            package,
            revision,
            stable_symbol_id,
            entity_name,
            qualified_name,
            file_path,
            line_start,
            line_end,
            symbol_kind,
            enclosing_scope
        FROM {nodes} n
        {select_where}
        {published_filter}
        {order_by}
        LIMIT {limit_param}
        ",
        nodes = tables.nodes,
        published_filter = tables.published_filter("n")
    );
    let candidates = collect_rows(db, &select_sql, select_params, |row| {
        code_candidate_from_row(row, 0)
    })
    .context("failed to search code symbols")?;

    Ok(CodeSearchResult {
        candidates,
        total_matches,
        truncated: total_matches > limit as usize,
    })
}

pub fn list_packages(
    db: &Connection,
    source: &str,
    name_filter: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<CatalogPage<PackageCatalogRow>> {
    let tables = QueryTables::load(db)?;
    let catalog_generation = catalog_generation(db, &tables)?;
    let refs_available = refs_table_available(db, &tables)?;
    let cursor_parts = decode_cursor(cursor, 2)?;

    let filter = crate::package_search::PackageNameFilter::new(name_filter);
    let mut params = SqlParams::default();
    let source_param = params.push(source.to_owned());
    let latest_refs = if refs_available {
        format!(
            r",
            latest_refs AS (
                SELECT source, package, revision
                FROM (
                    SELECT
                        source,
                        package,
                        revision,
                        row_number() OVER (
                            PARTITION BY source, package, ref_name
                            ORDER BY updated_at DESC NULLS LAST, revision DESC
                        ) AS rn
                    FROM {refs}
                    WHERE ref_name = 'latest'
                )
                WHERE rn = 1
            )
            ",
            refs = tables.refs
        )
    } else {
        String::new()
    };
    let latest_join = if refs_available {
        r"
        LEFT JOIN latest_refs lr
          ON lr.source = p.source
         AND lr.package = p.package
        "
    } else {
        ""
    };
    let latest_select = if refs_available {
        "lr.revision"
    } else {
        "NULL::VARCHAR"
    };
    let sql = format!(
        r"
        WITH packages AS (
            SELECT
                pc.source,
                pc.package,
                COUNT(*)::BIGINT AS revision_count,
                CAST(MAX(pc.indexed_at) AS VARCHAR) AS indexed_at
            FROM {package_catalog} pc
            WHERE {where_clause}
            GROUP BY pc.source, pc.package
        )
        {latest_refs}
        SELECT
            p.source,
            p.package,
            {latest_select} AS latest_revision,
            p.revision_count,
            p.indexed_at
        FROM packages p
        {latest_join}
        ORDER BY p.source, p.package
        ",
        package_catalog = tables.package_catalog,
        where_clause = format!("pc.source = {source_param}"),
    );
    let mut rows = collect_rows(db, &sql, params, |row| {
        Ok(PackageCatalogRow {
            source: row.get(0)?,
            package: row.get(1)?,
            latest_revision: row.get(2)?,
            revision_count: i64_to_usize(row.get(3)?, "revision_count")?,
            indexed_at: row.get(4)?,
        })
    })
    .context("failed to list catalog packages")?;
    // Share the exact matching policy with the Parquet backend. Count before
    // the cursor so total_matches describes the complete filtered catalog.
    rows.retain(|row| filter.matches(&row.package));
    let total_matches = rows.len();
    if let Some(parts) = cursor_parts {
        rows.retain(|row| {
            (row.source.as_str(), row.package.as_str()) > (parts[0].as_str(), parts[1].as_str())
        });
    }
    let (truncated, next_cursor) = trim_catalog_page(&mut rows, limit, |row| {
        encode_cursor([row.source.as_str(), row.package.as_str()])
    });

    Ok(CatalogPage {
        level: CatalogLevel::Packages,
        rows,
        total_matches,
        truncated,
        next_cursor,
        catalog_generation,
    })
}

pub fn list_revisions_and_refs(
    db: &Connection,
    source: &str,
    package: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<CatalogPage<RevisionCatalogRow>> {
    let tables = QueryTables::load(db)?;
    let catalog_generation = catalog_generation(db, &tables)?;
    let refs_available = refs_table_available(db, &tables)?;
    let cursor_parts = decode_cursor(cursor, 1)?;

    let total_matches = query_count(
        db,
        &format!(
            r"
            SELECT COUNT(*)
            FROM {package_catalog}
            WHERE source = $1 AND package = $2
            ",
            package_catalog = tables.package_catalog
        ),
        SqlParams::from_values([source.to_owned(), package.to_owned()]),
    )
    .context("failed to count catalog revisions")?;

    let mut params = SqlParams::default();
    let source_param = params.push(source.to_owned());
    let package_param = params.push(package.to_owned());
    let mut filters = vec![
        format!("source = {source_param}"),
        format!("package = {package_param}"),
    ];
    if let Some(parts) = cursor_parts {
        let revision_param = params.push(parts[0].clone());
        filters.push(format!("revision > {revision_param}"));
    }
    let limit_param = params.push((clamp_catalog_limit(limit) + 1) as i64);
    let sql = format!(
        r"
        SELECT
            revision,
            revision_kind,
            semver_major,
            semver_minor,
            semver_patch,
            CAST(indexed_at AS VARCHAR) AS indexed_at,
            embeddings_status,
            CAST(row_counts AS VARCHAR) AS row_counts,
            generation,
            snapshot_id
        FROM {package_catalog}
        WHERE {where_clause}
        ORDER BY revision
        LIMIT {limit_param}
        ",
        package_catalog = tables.package_catalog,
        where_clause = filters.join(" AND "),
    );
    let mut rows = collect_rows(db, &sql, params, |row| {
        let semver_major: Option<i64> = row.get(2)?;
        let semver_minor: Option<i64> = row.get(3)?;
        let semver_patch: Option<i64> = row.get(4)?;
        let row_counts_raw: Option<String> = row.get(7)?;
        Ok(RevisionCatalogRow {
            revision: row.get(0)?,
            revision_kind: row.get(1)?,
            semver: semver_string(semver_major, semver_minor, semver_patch),
            indexed_at: row.get(5)?,
            embeddings_status: row.get(6)?,
            row_counts: parse_json_column(row_counts_raw.as_deref(), "row_counts")?,
            generation: row.get(8)?,
            snapshot_id: row.get(9)?,
            refs: Vec::new(),
        })
    })
    .context("failed to list catalog revisions")?;

    if refs_available && !rows.is_empty() {
        let refs_by_revision = revision_refs(db, &tables, source, package)?;
        for row in &mut rows {
            row.refs = refs_by_revision
                .get(&row.revision)
                .cloned()
                .unwrap_or_default();
        }
    }

    let (truncated, next_cursor) = trim_catalog_page(&mut rows, limit, |row| {
        encode_cursor([row.revision.as_str()])
    });

    Ok(CatalogPage {
        level: CatalogLevel::Revisions,
        rows,
        total_matches,
        truncated,
        next_cursor,
        catalog_generation,
    })
}

pub fn list_tree_entries(
    db: &Connection,
    source: &str,
    package: &str,
    revision: &str,
    path: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<CatalogPage<TreeCatalogEntry>> {
    let tables = QueryTables::load(db)?;
    let catalog_generation = catalog_generation(db, &tables)?;
    let prefix = normalize_catalog_path(path);
    let cursor_parts = decode_cursor(cursor, 1)?;
    let sql = format!(
        r"
        WITH scoped AS (
            SELECT
                fm.path,
                fm.node_ids,
                CASE
                    WHEN $4 = '' THEN fm.path
                    WHEN fm.path = $4 THEN ''
                    ELSE substr(fm.path, length($4) + 2)
                END AS remainder
            FROM {file_manifests} fm
            WHERE fm.source = $1
              AND fm.package = $2
              AND fm.revision = $3
              AND ($4 = '' OR fm.path = $4 OR starts_with(fm.path, $4 || '/'))
              {published_filter}
        ),
        mapped AS (
            SELECT
                CASE
                    WHEN contains(remainder, '/') THEN split_part(remainder, '/', 1)
                    ELSE remainder
                END AS name,
                CASE
                    WHEN contains(remainder, '/') THEN 'dir'
                    ELSE 'file'
                END AS kind,
                COALESCE(array_length(node_ids), 0)::BIGINT AS symbol_count
            FROM scoped
            WHERE remainder <> ''
        )
        SELECT
            name,
            kind,
            COUNT(*)::BIGINT AS file_count,
            SUM(symbol_count)::BIGINT AS symbol_count
        FROM mapped
        GROUP BY name, kind
        ORDER BY name, kind
        ",
        file_manifests = tables.file_manifests,
        published_filter = tables.published_filter("fm")
    );
    let mut rows = collect_rows(
        db,
        &sql,
        SqlParams::from_values([
            source.to_owned(),
            package.to_owned(),
            revision.to_owned(),
            prefix.clone(),
        ]),
        |row| {
            let name: String = row.get(0)?;
            let kind: String = row.get(1)?;
            Ok(TreeCatalogEntry {
                path: child_catalog_path(&prefix, &name),
                name,
                kind,
                file_count: i64_to_usize(row.get(2)?, "file_count")?,
                symbol_count: Some(i64_to_usize(row.get(3)?, "symbol_count")?),
            })
        },
    )
    .context("failed to list catalog tree entries")?;
    let total_matches = rows.len();
    if let Some(parts) = cursor_parts {
        rows.retain(|row| row.name.as_str() > parts[0].as_str());
    }
    let (truncated, next_cursor) =
        trim_catalog_page(&mut rows, limit, |row| encode_cursor([row.name.as_str()]));

    Ok(CatalogPage {
        level: CatalogLevel::Tree,
        rows,
        total_matches,
        truncated,
        next_cursor,
        catalog_generation,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "catalog list functions keep source/package/revision coordinates explicit"
)]
pub fn list_file_symbols(
    db: &Connection,
    source: &str,
    package: &str,
    revision: &str,
    path: &str,
    name_filter: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<CatalogPage<SymbolCatalogRow>> {
    let tables = QueryTables::load(db)?;
    let catalog_generation = catalog_generation(db, &tables)?;
    let cursor_parts = decode_cursor(cursor, 2)?;
    let path = normalize_catalog_path(Some(path));

    let mut count_params = SqlParams::default();
    let source_param = count_params.push(source.to_owned());
    let package_param = count_params.push(package.to_owned());
    let revision_param = count_params.push(revision.to_owned());
    let path_param = count_params.push(path.clone());
    let mut count_filters = vec![
        format!("n.source = {source_param}"),
        format!("n.package = {package_param}"),
        format!("n.revision = {revision_param}"),
        format!("n.file_path = {path_param}"),
    ];
    if let Some(name_filter) = name_filter {
        let name_param = count_params.push(name_filter.to_owned());
        count_filters.push(format!(
            "(contains(n.entity_name, {name_param}) OR contains(n.qualified_name, {name_param}))"
        ));
    }
    let count_sql = format!(
        r"
        SELECT COUNT(*)
        FROM {nodes} n
        WHERE {where_clause}
        {published_filter}
        ",
        nodes = tables.nodes,
        where_clause = count_filters.join(" AND "),
        published_filter = tables.published_filter("n")
    );
    let total_matches =
        query_count(db, &count_sql, count_params).context("failed to count catalog symbols")?;

    let mut params = SqlParams::default();
    let source_param = params.push(source.to_owned());
    let package_param = params.push(package.to_owned());
    let revision_param = params.push(revision.to_owned());
    let path_param = params.push(path);
    let mut filters = vec![
        format!("n.source = {source_param}"),
        format!("n.package = {package_param}"),
        format!("n.revision = {revision_param}"),
        format!("n.file_path = {path_param}"),
    ];
    if let Some(name_filter) = name_filter {
        let name_param = params.push(name_filter.to_owned());
        filters.push(format!(
            "(contains(n.entity_name, {name_param}) OR contains(n.qualified_name, {name_param}))"
        ));
    }
    if let Some(parts) = cursor_parts {
        let line_start: i64 = parts[0]
            .parse()
            .with_context(|| format!("invalid catalog cursor line: {}", parts[0]))?;
        let line_param = params.push(line_start);
        let id_param = params.push(parts[1].clone());
        filters.push(format!(
            "(n.line_start > {line_param} OR (n.line_start = {line_param} AND n.stable_symbol_id > {id_param}))"
        ));
    }
    let limit_param = params.push((clamp_catalog_limit(limit) + 1) as i64);
    let sql = format!(
        r"
        SELECT
            n.entity_name,
            n.qualified_name,
            n.symbol_kind,
            n.line_start,
            n.line_end,
            n.package,
            n.revision,
            n.stable_symbol_id
        FROM {nodes} n
        WHERE {where_clause}
        {published_filter}
        ORDER BY n.line_start, n.stable_symbol_id
        LIMIT {limit_param}
        ",
        nodes = tables.nodes,
        where_clause = filters.join(" AND "),
        published_filter = tables.published_filter("n")
    );
    let mut rows = collect_rows(db, &sql, params, |row| {
        let qualified_name: String = row.get(1)?;
        let entity_name: String = row.get(0)?;
        let selector_name = if qualified_name.is_empty() {
            entity_name.as_str()
        } else {
            qualified_name.as_str()
        };
        let package: String = row.get(5)?;
        let revision: String = row.get(6)?;
        let selector = format!("pkg:{package}@{revision}::{selector_name}");
        Ok(SymbolCatalogRow {
            entity_name,
            qualified_name,
            symbol_kind: row.get(2)?,
            line_range: [
                i64_to_usize(row.get(3)?, "line_start")?,
                i64_to_usize(row.get(4)?, "line_end")?,
            ],
            next: catalog_symbol_next_tools(&selector),
            selector,
            stable_symbol_id: row.get(7)?,
        })
    })
    .context("failed to list catalog file symbols")?;
    let (truncated, next_cursor) = trim_catalog_page(&mut rows, limit, |row| {
        let line_start = row.line_range[0].to_string();
        encode_cursor([line_start.as_str(), row.stable_symbol_id.as_str()])
    });

    Ok(CatalogPage {
        level: CatalogLevel::Symbols,
        rows,
        total_matches,
        truncated,
        next_cursor,
        catalog_generation,
    })
}

pub(crate) fn catalog_file_exists(
    db: &Connection,
    source: &str,
    package: &str,
    revision: &str,
    path: &str,
) -> Result<bool> {
    let tables = QueryTables::load(db)?;
    let path = normalize_catalog_path(Some(path));
    let exists: i64 = db
        .query_row(
            &format!(
                r"
                SELECT COUNT(*)
                FROM {file_manifests} fm
                WHERE fm.source = $1
                  AND fm.package = $2
                  AND fm.revision = $3
                  AND fm.path = $4
                  {published_filter}
                ",
                file_manifests = tables.file_manifests,
                published_filter = tables.published_filter("fm")
            ),
            params![source, package, revision, path],
            |row| row.get(0),
        )
        .context("failed to inspect catalog file manifest")?;
    Ok(exists > 0)
}

pub fn read_symbol(
    db: &Connection,
    selector: &str,
    context_lines: usize,
) -> Result<Option<SymbolSource>> {
    let tables = QueryTables::load(db)?;
    let Some(symbol) = resolve_required(db, selector)? else {
        return Ok(None);
    };

    let sql = format!(
        r"
        SELECT
            n.stable_symbol_id,
            n.source,
            n.package,
            n.revision,
            n.file_path,
            n.byte_range_start,
            n.byte_range_end,
            n.line_start,
            n.line_end,
            n.qualified_name,
            f.source_text
        FROM {nodes} n
        LEFT JOIN {files} f
          ON f.source = n.source
         AND f.package = n.package
         AND f.revision = n.revision
         AND f.file_path = n.file_path
         {files_generation_join}
        WHERE n.source = $1
          AND n.package = $2
          AND n.revision = $3
          AND n.stable_symbol_id = $4
          {published_filter}
        LIMIT 1
        ",
        nodes = tables.nodes,
        files = tables.files,
        files_generation_join = tables.same_generation_join("f", "n"),
        published_filter = tables.published_filter("n")
    );
    let row = optional_no_rows(
        db.query_row(
            &sql,
            params![
                symbol.source,
                symbol.package,
                symbol.revision,
                symbol.stable_symbol_id
            ],
            |row| {
                Ok(SymbolSourceRow {
                    stable_symbol_id: row.get(0)?,
                    package_source: row.get(1)?,
                    package: row.get(2)?,
                    revision: row.get(3)?,
                    file_path: row.get(4)?,
                    byte_start: i64_to_usize(row.get(5)?, "byte_range_start")?,
                    byte_end: i64_to_usize(row.get(6)?, "byte_range_end")?,
                    line_start: i64_to_usize(row.get(7)?, "line_start")?,
                    line_end: i64_to_usize(row.get(8)?, "line_end")?,
                    qualified_name: row.get(9)?,
                    source_text: row.get(10)?,
                })
            },
        ),
        "failed to read symbol source row",
    )?;

    let Some(row) = row else {
        return Ok(None);
    };
    let source_text = row.source_text.ok_or_else(|| {
        anyhow!(
            "source text is not indexed for {} {}@{} `{}` ({})",
            row.package_source,
            row.package,
            row.revision,
            row.file_path,
            row.stable_symbol_id
        )
    })?;
    let (source, line_range) = slice_symbol_source(
        &source_text,
        row.byte_start,
        row.byte_end,
        row.line_start,
        row.line_end,
        context_lines,
    )?;
    let selector = format!(
        "pkg:{}@{}::{}",
        row.package, row.revision, row.qualified_name
    );

    Ok(Some(SymbolSource {
        selector,
        stable_symbol_id: row.stable_symbol_id,
        package_source: row.package_source,
        package: row.package,
        revision: row.revision,
        file_path: row.file_path,
        byte_range: [row.byte_start, row.byte_end],
        line_range,
        source,
    }))
}

pub fn find_callers(
    db: &Connection,
    selector: &str,
    include_unresolved: bool,
) -> Result<CallerResult> {
    let Some(target) = resolve_required(db, selector)? else {
        return Ok(CallerResult {
            callers: Vec::new(),
            counts_by_kind: CountsByKind::default(),
            unresolved_sample: Vec::new(),
        });
    };

    let mut callers = resolved_callers(db, &target)?;
    if include_unresolved {
        callers.extend(unresolved_callers(db, &target)?);
    }
    let edges = callers.iter().map(|caller| &caller.edge);
    let counts_by_kind = counts_by_kind(edges);
    let unresolved_sample = unresolved_sample(
        callers
            .iter()
            .filter(|caller| !caller.resolved)
            .filter_map(|caller| caller.edge.target_label.as_deref()),
    );

    Ok(CallerResult {
        callers,
        counts_by_kind,
        unresolved_sample,
    })
}

pub fn find_callees(
    db: &Connection,
    selector: &str,
    include_unresolved: bool,
) -> Result<CalleeResult> {
    let Some(source) = resolve_required(db, selector)? else {
        return Ok(CalleeResult {
            callees: Vec::new(),
            counts_by_kind: CountsByKind::default(),
            unresolved_sample: Vec::new(),
        });
    };

    let mut callees = resolved_callees(db, &source)?;
    if include_unresolved {
        callees.extend(unresolved_callees(db, &source)?);
    }
    let edges = callees.iter().map(|callee| &callee.edge);
    let counts_by_kind = counts_by_kind(edges);
    let unresolved_sample = unresolved_sample(
        callees
            .iter()
            .filter(|callee| !callee.resolved)
            .filter_map(|callee| callee.edge.target_label.as_deref()),
    );

    Ok(CalleeResult {
        callees,
        counts_by_kind,
        unresolved_sample,
    })
}

pub fn resolve_selector(db: &Connection, selector: &str) -> Result<SelectorResolution> {
    if let Some(parsed) = parse_pkg_symbol_uri(selector) {
        return Ok(match stable_selector_node(db, &parsed)? {
            Some(node) => SelectorResolution::Resolved(ResolvedSymbol {
                stable_symbol_id: node.stable_symbol_id,
            }),
            None => SelectorResolution::NotFound,
        });
    }

    let Some(parsed) = parse_pkg_selector(selector) else {
        return Ok(SelectorResolution::NotFound);
    };

    let revision = match parsed.revision {
        Some(revision) => Some(revision),
        None => latest_revision(db, &parsed.source, &parsed.package)?,
    };
    let candidates = selector_candidates(
        db,
        &parsed.source,
        &parsed.package,
        revision.as_deref(),
        &parsed.name,
    )?;

    Ok(resolution_from_candidates(candidates))
}

fn resolved_callers(db: &Connection, target: &ResolvedNode) -> Result<Vec<CallerRecord>> {
    let tables = QueryTables::load(db)?;
    let sql = format!(
        r"
        SELECT
            n.source,
            n.package,
            n.revision,
            n.stable_symbol_id,
            n.entity_name,
            n.qualified_name,
            n.file_path,
            n.line_start,
            n.line_end,
            n.symbol_kind,
            n.enclosing_scope,
            e.source_stable_id,
            e.target_stable_id,
            e.target_label,
            e.target_package,
            e.relation,
            e.edge_kind,
            e.confidence,
            e.confidence_score,
            e.bind_method,
            e.receiver_text,
            e.scope_text
        FROM {edges} e
        JOIN {nodes} n
          ON n.source = e.source
         AND n.package = e.package
         AND n.revision = e.revision
         AND n.stable_symbol_id = e.source_stable_id
         {node_edge_generation_join}
        WHERE e.source = $1
          AND e.package = $2
          AND e.revision = $3
          AND e.target_stable_id = $4
          AND e.relation = 'calls'
          AND e.edge_kind IN ('calls', 'calls_dyn', 'references_hof')
          {published_filter}
        ORDER BY n.file_path, n.line_start, n.line_end, n.qualified_name, e.edge_kind
        ",
        edges = tables.edges,
        nodes = tables.nodes,
        node_edge_generation_join = tables.same_generation_join("n", "e"),
        published_filter = tables.published_filter("e")
    );
    collect_rows(
        db,
        &sql,
        SqlParams::from_values([
            target.source.clone(),
            target.package.clone(),
            target.revision.clone(),
            target.stable_symbol_id.clone(),
        ]),
        |row| {
            Ok(CallerRecord {
                caller: code_candidate_from_row(row, 0)?,
                edge: edge_metadata_from_row(row, 11)?,
                resolved: true,
            })
        },
    )
    .context("failed to query resolved callers")
}

fn unresolved_callers(db: &Connection, target: &ResolvedNode) -> Result<Vec<CallerRecord>> {
    let tables = QueryTables::load(db)?;
    let sql = format!(
        r"
        SELECT
            n.source,
            n.package,
            n.revision,
            n.stable_symbol_id,
            n.entity_name,
            n.qualified_name,
            n.file_path,
            n.line_start,
            n.line_end,
            n.symbol_kind,
            n.enclosing_scope,
            e.source_stable_id,
            NULL AS target_stable_id,
            e.target_label,
            e.target_package,
            e.relation,
            e.edge_kind,
            e.confidence,
            e.confidence_score,
            e.bind_method,
            e.receiver_text,
            e.scope_text
        FROM {edges_unresolved} e
        JOIN {nodes} n
          ON n.source = e.source
         AND n.package = e.package
         AND n.revision = e.revision
         AND n.stable_symbol_id = e.source_stable_id
         {node_edge_generation_join}
        WHERE e.source = $1
          AND e.package = $2
          AND e.revision = $3
          AND e.target_label IN ($4, $5, $6)
          AND e.relation = 'calls'
          AND e.edge_kind IN ('calls', 'calls_dyn', 'references_hof')
          {published_filter}
        ORDER BY n.file_path, n.line_start, n.line_end, n.qualified_name, e.edge_kind
        ",
        edges_unresolved = tables.edges_unresolved,
        nodes = tables.nodes,
        node_edge_generation_join = tables.same_generation_join("n", "e"),
        published_filter = tables.published_filter("e")
    );
    collect_rows(
        db,
        &sql,
        SqlParams::from_values([
            target.source.clone(),
            target.package.clone(),
            target.revision.clone(),
            target.entity_name.clone(),
            target.qualified_name.clone(),
            target.stable_symbol_id.clone(),
        ]),
        |row| {
            Ok(CallerRecord {
                caller: code_candidate_from_row(row, 0)?,
                edge: edge_metadata_from_row(row, 11)?,
                resolved: false,
            })
        },
    )
    .context("failed to query unresolved callers")
}

fn resolved_callees(db: &Connection, source: &ResolvedNode) -> Result<Vec<CalleeRecord>> {
    let tables = QueryTables::load(db)?;
    let sql = format!(
        r"
        SELECT
            n.source,
            n.package,
            n.revision,
            n.stable_symbol_id,
            n.entity_name,
            n.qualified_name,
            n.file_path,
            n.line_start,
            n.line_end,
            n.symbol_kind,
            n.enclosing_scope,
            e.source_stable_id,
            e.target_stable_id,
            e.target_label,
            e.target_package,
            e.relation,
            e.edge_kind,
            e.confidence,
            e.confidence_score,
            e.bind_method,
            e.receiver_text,
            e.scope_text
        FROM {edges} e
        JOIN {nodes} n
          ON n.source = e.source
         AND n.package = e.package
         AND n.revision = e.revision
         AND n.stable_symbol_id = e.target_stable_id
         {node_edge_generation_join}
        WHERE e.source = $1
          AND e.package = $2
          AND e.revision = $3
          AND e.source_stable_id = $4
          AND e.relation = 'calls'
          AND e.edge_kind IN ('calls', 'calls_dyn', 'references_hof')
          {published_filter}
        ORDER BY n.file_path, n.line_start, n.line_end, n.qualified_name, e.edge_kind
        ",
        edges = tables.edges,
        nodes = tables.nodes,
        node_edge_generation_join = tables.same_generation_join("n", "e"),
        published_filter = tables.published_filter("e")
    );
    collect_rows(
        db,
        &sql,
        SqlParams::from_values([
            source.source.clone(),
            source.package.clone(),
            source.revision.clone(),
            source.stable_symbol_id.clone(),
        ]),
        |row| {
            Ok(CalleeRecord {
                callee: Some(code_candidate_from_row(row, 0)?),
                edge: edge_metadata_from_row(row, 11)?,
                resolved: true,
            })
        },
    )
    .context("failed to query resolved callees")
}

fn unresolved_callees(db: &Connection, source: &ResolvedNode) -> Result<Vec<CalleeRecord>> {
    let tables = QueryTables::load(db)?;
    let sql = format!(
        r"
        SELECT
            e.source_stable_id,
            NULL AS target_stable_id,
            e.target_label,
            e.target_package,
            e.relation,
            e.edge_kind,
            e.confidence,
            e.confidence_score,
            e.bind_method,
            e.receiver_text,
            e.scope_text
        FROM {edges_unresolved} e
        WHERE e.source = $1
          AND e.package = $2
          AND e.revision = $3
          AND e.source_stable_id = $4
          AND e.relation = 'calls'
          AND e.edge_kind IN ('calls', 'calls_dyn', 'references_hof')
          {published_filter}
        ORDER BY e.target_package NULLS LAST, e.target_label, e.edge_kind
        ",
        edges_unresolved = tables.edges_unresolved,
        published_filter = tables.published_filter("e")
    );
    collect_rows(
        db,
        &sql,
        SqlParams::from_values([
            source.source.clone(),
            source.package.clone(),
            source.revision.clone(),
            source.stable_symbol_id.clone(),
        ]),
        |row| {
            Ok(CalleeRecord {
                callee: None,
                edge: edge_metadata_from_row(row, 0)?,
                resolved: false,
            })
        },
    )
    .context("failed to query unresolved callees")
}

fn selector_candidates(
    db: &Connection,
    source: &str,
    package: &str,
    revision: Option<&str>,
    name: &str,
) -> Result<Vec<CodeCandidate>> {
    let tables = QueryTables::load(db)?;
    let (revision_filter, params) = if let Some(revision) = revision {
        (
            "AND revision = $3 AND (qualified_name = $4 OR entity_name = $4)",
            SqlParams::from_values([
                source.to_owned(),
                package.to_owned(),
                revision.to_owned(),
                name.to_owned(),
            ]),
        )
    } else {
        (
            "AND (qualified_name = $3 OR entity_name = $3)",
            SqlParams::from_values([source.to_owned(), package.to_owned(), name.to_owned()]),
        )
    };
    let sql = format!(
        r"
        SELECT
            source,
            package,
            revision,
            stable_symbol_id,
            entity_name,
            qualified_name,
            file_path,
            line_start,
            line_end,
            symbol_kind,
            enclosing_scope
        FROM {nodes} n
        WHERE source = $1
          AND package = $2
          {revision_filter}
          {published_filter}
        ORDER BY file_path, line_start, line_end, qualified_name, stable_symbol_id
        ",
        nodes = tables.nodes,
        published_filter = tables.published_filter("n")
    );
    collect_rows(db, &sql, params, |row| code_candidate_from_row(row, 0))
        .context("failed to resolve external code selector")
}

fn resolution_from_candidates(candidates: Vec<CodeCandidate>) -> SelectorResolution {
    match candidates.as_slice() {
        [] => SelectorResolution::NotFound,
        [candidate] => SelectorResolution::Resolved(ResolvedSymbol {
            stable_symbol_id: candidate.stable_symbol_id.clone(),
        }),
        _ => SelectorResolution::Ambiguous { candidates },
    }
}

fn resolve_required(db: &Connection, selector: &str) -> Result<Option<ResolvedNode>> {
    if let Some(parsed) = parse_pkg_symbol_uri(selector) {
        return stable_selector_node(db, &parsed);
    }

    let Some(parsed) = parse_pkg_selector(selector) else {
        return Ok(None);
    };

    let revision = match parsed.revision {
        Some(revision) => Some(revision),
        None => latest_revision(db, &parsed.source, &parsed.package)?,
    };
    let candidates = selector_candidates(
        db,
        &parsed.source,
        &parsed.package,
        revision.as_deref(),
        &parsed.name,
    )?;

    match candidates.as_slice() {
        [] => Ok(None),
        [candidate] => Ok(Some(ResolvedNode::from(candidate))),
        _ => Err(AmbiguousSelectorError {
            selector: selector.to_owned(),
            candidate_count: candidates.len(),
            candidates,
        }
        .into()),
    }
}

fn stable_selector_node(
    db: &Connection,
    parsed: &ParsedStableSelector,
) -> Result<Option<ResolvedNode>> {
    let tables = QueryTables::load(db)?;
    let sql = format!(
        r"
        SELECT stable_symbol_id, source, package, revision, entity_name, qualified_name
        FROM {nodes} n
        WHERE source = $1
          AND package = $2
          AND revision = $3
          AND stable_symbol_id = $4
          {published_filter}
        LIMIT 1
        ",
        nodes = tables.nodes,
        published_filter = tables.published_filter("n")
    );
    optional_no_rows(
        db.query_row(
            &sql,
            params![
                &parsed.source,
                &parsed.package,
                &parsed.revision,
                &parsed.stable_symbol_id
            ],
            |row| {
                Ok(ResolvedNode {
                    stable_symbol_id: row.get(0)?,
                    source: row.get(1)?,
                    package: row.get(2)?,
                    revision: row.get(3)?,
                    entity_name: row.get(4)?,
                    qualified_name: row.get(5)?,
                })
            },
        ),
        "failed to resolve stable external code selector",
    )
}

fn latest_revision(db: &Connection, source: &str, package: &str) -> Result<Option<String>> {
    let tables = QueryTables::load(db)?;
    if tables.refs == "refs" && !table_exists(db, "refs")? {
        return Ok(None);
    }

    let sql = format!(
        r"
        SELECT revision
        FROM {refs}
        WHERE source = $1
          AND package = $2
          AND ref_name = 'latest'
        ORDER BY updated_at DESC NULLS LAST, revision DESC
        LIMIT 1
        ",
        refs = tables.refs
    );
    optional_no_rows(
        db.query_row(&sql, params![source, package], |row| row.get(0)),
        "failed to resolve latest package revision",
    )
}

fn table_exists(db: &Connection, table_name: &str) -> Result<bool> {
    let count: i64 = db
        .query_row(
            r"
            SELECT COUNT(*)
            FROM information_schema.tables
            WHERE table_name = $1
            ",
            params![table_name],
            |row| row.get(0),
        )
        .context("failed to inspect DuckDB tables")?;
    Ok(count > 0)
}

fn parse_pkg_selector(selector: &str) -> Option<ParsedSelector> {
    let selector = selector.trim();
    let selector = selector.strip_prefix("pkg:")?;
    let (package_revision, name) = selector.split_once("::")?;
    if package_revision.is_empty() || name.is_empty() {
        return None;
    }

    let (source, package_revision) = match package_revision.split_once('|') {
        Some((source, package_revision)) if !source.is_empty() && !package_revision.is_empty() => {
            (source.to_owned(), package_revision)
        }
        Some(_) => return None,
        None => (DEFAULT_SOURCE.to_owned(), package_revision),
    };

    let (package, revision) = match package_revision.split_once('@') {
        Some((package, revision)) if !package.is_empty() && !revision.is_empty() => {
            (package.to_owned(), Some(revision.to_owned()))
        }
        Some(_) => return None,
        None => (package_revision.to_owned(), None),
    };

    Some(ParsedSelector {
        source,
        package,
        revision,
        name: name.to_owned(),
    })
}

/// Encode one `pkg-symbol://` path component so it contains no `/`. Each of
/// source/package/revision can legitimately contain slashes (github
/// `owner/repo` packages, `feature/x` git revisions, `git:host/owner/repo`
/// sources), which would otherwise make the 4-component URI ambiguous. Encode
/// `%` first, then `/`, so `decode_uri_component` is an exact inverse.
fn encode_uri_component(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

pub(crate) fn pkg_symbol_uri(
    source: &str,
    package: &str,
    revision: &str,
    stable_symbol_id: &str,
) -> String {
    format!(
        "{PKG_SYMBOL_URI_PREFIX}{}/{}/{}/{}",
        encode_uri_component(source),
        encode_uri_component(package),
        encode_uri_component(revision),
        encode_uri_component(stable_symbol_id),
    )
}

/// Inverse of [`encode_uri_component`]. Decode `/` before `%`. A plain
/// (already slash-free, `%`-free) component decodes to itself, so simple URIs
/// remain unaffected.
fn decode_uri_component(value: &str) -> String {
    value.replace("%2F", "/").replace("%25", "%")
}

fn parse_pkg_symbol_uri(selector: &str) -> Option<ParsedStableSelector> {
    let body = selector.trim().strip_prefix(PKG_SYMBOL_URI_PREFIX)?;
    // Components are slash-encoded, so there are exactly three separator slashes.
    let parts: Vec<&str> = body.split('/').collect();
    if parts.len() != 4 {
        return None;
    }
    let source = decode_uri_component(parts[0]);
    let package = decode_uri_component(parts[1]);
    let revision = decode_uri_component(parts[2]);
    let stable_symbol_id = decode_uri_component(parts[3]);
    if source.is_empty() || package.is_empty() || revision.is_empty() || stable_symbol_id.is_empty()
    {
        return None;
    }
    Some(ParsedStableSelector {
        source,
        package,
        revision,
        stable_symbol_id,
    })
}

fn search_filter(opts: &SearchOptions) -> (String, SqlParams, String) {
    let mut params = SqlParams::default();
    let source_param = params.push(opts.source.clone());
    let package_param = params.push(opts.package.clone());
    let revision_param = params.push(opts.revision.clone());
    let query_param = params.push(opts.query.clone());

    let query_predicate = match opts.mode {
        SearchMode::Exact => {
            format!("(entity_name = {query_param} OR qualified_name = {query_param})")
        }
        SearchMode::Prefix => format!(
            "(starts_with(entity_name, {query_param}) OR starts_with(qualified_name, {query_param}))"
        ),
        SearchMode::Substring => format!(
            "(contains(entity_name, {query_param}) OR contains(qualified_name, {query_param}))"
        ),
    };

    let mut filters = vec![
        format!("source = {source_param}"),
        format!("package = {package_param}"),
        format!("revision = {revision_param}"),
        query_predicate,
    ];

    if let Some(symbol_kind) = &opts.symbol_kind {
        let symbol_kind_param = params.push(symbol_kind.clone());
        filters.push(format!("symbol_kind = {symbol_kind_param}"));
    }

    if let Some(file_glob) = &opts.file_glob {
        let file_glob_param = params.push(file_glob.clone());
        filters.push(format!("file_path GLOB {file_glob_param}"));
    }

    (
        format!("WHERE {}", filters.join(" AND ")),
        params,
        query_param,
    )
}

fn search_order_by(mode: SearchMode, query_param: &str) -> String {
    match mode {
        SearchMode::Exact => format!(
            r"
            ORDER BY
                CASE WHEN entity_name = {query_param} THEN 0 ELSE 1 END,
                file_path,
                line_start,
                line_end,
                stable_symbol_id
            "
        ),
        SearchMode::Prefix => r"
            ORDER BY
                LENGTH(entity_name),
                file_path,
                line_start,
                line_end,
                stable_symbol_id
            "
        .to_owned(),
        SearchMode::Substring => format!(
            r"
            ORDER BY
                CASE
                    WHEN contains(entity_name, {query_param}) THEN strpos(entity_name, {query_param})
                    ELSE strpos(qualified_name, {query_param})
                END,
                LENGTH(entity_name),
                file_path,
                line_start,
                line_end,
                stable_symbol_id
            "
        ),
    }
}

fn code_candidate_from_row(row: &Row<'_>, offset: usize) -> duckdb::Result<CodeCandidate> {
    let source: String = row.get(offset)?;
    let package: String = row.get(offset + 1)?;
    let revision: String = row.get(offset + 2)?;
    let stable_symbol_id: String = row.get(offset + 3)?;
    let entity_name: String = row.get(offset + 4)?;
    let qualified_name: String = row.get(offset + 5)?;
    let file_path: String = row.get(offset + 6)?;
    let line_start = i64_to_usize(row.get(offset + 7)?, "line_start")?;
    let line_end = i64_to_usize(row.get(offset + 8)?, "line_end")?;
    let symbol_kind: String = row.get(offset + 9)?;
    let enclosing_scope: Option<String> = row.get(offset + 10)?;
    let selector_name = if qualified_name.is_empty() {
        entity_name.as_str()
    } else {
        qualified_name.as_str()
    };
    let selector = format!("pkg:{package}@{revision}::{selector_name}");
    let uri = pkg_symbol_uri(&source, &package, &revision, &stable_symbol_id);

    Ok(CodeCandidate {
        selector,
        uri,
        id: stable_symbol_id.clone(),
        stable_symbol_id,
        source,
        package,
        revision,
        entity_name,
        qualified_name,
        file_path,
        line_range: [line_start, line_end],
        symbol_kind,
        enclosing_scope,
    })
}

fn edge_metadata_from_row(row: &Row<'_>, offset: usize) -> duckdb::Result<EdgeMetadata> {
    Ok(EdgeMetadata {
        source_stable_id: row.get(offset)?,
        target_stable_id: row.get(offset + 1)?,
        target_label: row.get(offset + 2)?,
        target_package: row.get(offset + 3)?,
        relation: row.get(offset + 4)?,
        edge_kind: row.get(offset + 5)?,
        confidence: row.get(offset + 6)?,
        confidence_score: row.get(offset + 7)?,
        bind_method: row.get(offset + 8)?,
        receiver_text: row.get(offset + 9)?,
        scope_text: row.get(offset + 10)?,
    })
}

fn counts_by_kind<'a>(edges: impl IntoIterator<Item = &'a EdgeMetadata>) -> CountsByKind {
    let mut counts = CountsByKind::default();
    for edge in edges {
        match edge.edge_kind.as_str() {
            CALL_EDGE_KIND_CALLS => counts.calls += 1,
            CALL_EDGE_KIND_DYN => counts.calls_dyn += 1,
            CALL_EDGE_KIND_HOF => counts.references_hof += 1,
            _ => counts.references_other += 1,
        }
        if edge.target_stable_id.is_none() {
            counts.unresolved += 1;
        }
    }
    counts
}

fn unresolved_sample<'a>(labels: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut sample = Vec::new();
    let mut bytes = 0usize;

    for label in labels {
        if sample.len() >= 5 || !seen.insert(label) {
            continue;
        }
        let next_bytes = bytes + label.len();
        if next_bytes > 120 {
            break;
        }
        bytes = next_bytes;
        sample.push(label.to_owned());
    }

    sample
}

fn slice_symbol_source(
    source_text: &str,
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    line_end: usize,
    context_lines: usize,
) -> Result<(String, [usize; 2])> {
    if byte_start > byte_end || byte_end > source_text.len() {
        bail!(
            "invalid symbol byte range [{byte_start}, {byte_end}) for source text of {} bytes",
            source_text.len()
        );
    }

    if context_lines == 0 {
        let source = source_text
            .get(byte_start..byte_end)
            .ok_or_else(|| anyhow!("symbol byte range is not on UTF-8 character boundaries"))?
            .to_owned();
        return Ok((source, [line_start, line_end]));
    }

    let line_starts = line_starts(source_text);
    if line_starts.is_empty() {
        return Ok((String::new(), [0, 0]));
    }

    let expanded_start = line_start.saturating_sub(context_lines).max(1);
    let expanded_end = (line_end + context_lines).min(line_starts.len());
    let start_byte = line_starts[expanded_start - 1];
    let end_byte = if expanded_end < line_starts.len() {
        line_starts[expanded_end]
    } else {
        source_text.len()
    };
    let source = source_text
        .get(start_byte..end_byte)
        .ok_or_else(|| anyhow!("expanded line range is not on UTF-8 character boundaries"))?
        .to_owned();
    Ok((source, [expanded_start, expanded_end]))
}

fn line_starts(source_text: &str) -> Vec<usize> {
    if source_text.is_empty() {
        return Vec::new();
    }

    let mut starts = vec![0];
    for (index, byte) in source_text.bytes().enumerate() {
        if byte == b'\n' && index + 1 < source_text.len() {
            starts.push(index + 1);
        }
    }
    starts
}

fn clamp_catalog_limit(limit: usize) -> usize {
    limit.clamp(1, 200)
}

fn catalog_generation(db: &Connection, tables: &QueryTables) -> Result<Option<i64>> {
    optional_no_rows(
        db.query_row(
            &format!("SELECT MAX(generation) FROM {}", tables.package_catalog),
            [],
            |row| row.get(0),
        ),
        "failed to read catalog generation",
    )
    .map(Option::flatten)
}

fn refs_table_available(db: &Connection, tables: &QueryTables) -> Result<bool> {
    if tables.refs == "refs" && !table_exists(db, "refs")? {
        return Ok(false);
    }
    Ok(true)
}

fn normalize_catalog_path(path: Option<&str>) -> String {
    path.unwrap_or("")
        .trim()
        .trim_matches('/')
        .trim_end_matches('/')
        .to_owned()
}

fn child_catalog_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

fn semver_string(major: Option<i64>, minor: Option<i64>, patch: Option<i64>) -> Option<String> {
    Some(format!("{}.{}.{}", major?, minor?, patch?))
}

fn parse_json_column(raw: Option<&str>, field: &'static str) -> duckdb::Result<Value> {
    let Some(raw) = raw else {
        return Ok(Value::Null);
    };
    serde_json::from_str(raw).map_err(|error| conversion_error(field, error))
}

fn catalog_symbol_next_tools(selector: &str) -> Vec<CatalogNextTool> {
    [
        "external_code_read",
        "external_code_callers",
        "external_code_callees",
    ]
    .into_iter()
    .map(|tool| CatalogNextTool {
        tool: tool.to_owned(),
        selector: selector.to_owned(),
    })
    .collect()
}

fn revision_refs(
    db: &Connection,
    tables: &QueryTables,
    source: &str,
    package: &str,
) -> Result<BTreeMap<String, Vec<RevisionRefRow>>> {
    let sql = format!(
        r"
        SELECT ref_name, revision, CAST(updated_at AS VARCHAR) AS updated_at
        FROM {refs}
        WHERE source = $1 AND package = $2
        ORDER BY revision, ref_name
        ",
        refs = tables.refs
    );
    let refs = collect_rows(
        db,
        &sql,
        SqlParams::from_values([source.to_owned(), package.to_owned()]),
        |row| {
            Ok(RevisionRefRow {
                ref_name: row.get(0)?,
                revision: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    )
    .context("failed to list catalog refs")?;

    let mut refs_by_revision = BTreeMap::<String, Vec<RevisionRefRow>>::new();
    for reference in refs {
        refs_by_revision
            .entry(reference.revision.clone())
            .or_default()
            .push(reference);
    }
    Ok(refs_by_revision)
}

fn encode_cursor<const N: usize>(parts: [&str; N]) -> String {
    parts
        .into_iter()
        .map(encode_uri_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn decode_cursor(cursor: Option<&str>, expected_parts: usize) -> Result<Option<Vec<String>>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let parts = cursor
        .split('/')
        .map(decode_uri_component)
        .collect::<Vec<_>>();
    if parts.len() != expected_parts || parts.iter().any(|part| part.is_empty()) {
        bail!("invalid catalog cursor");
    }
    Ok(Some(parts))
}

fn trim_catalog_page<T, F>(rows: &mut Vec<T>, limit: usize, cursor_for: F) -> (bool, Option<String>)
where
    F: FnOnce(&T) -> String,
{
    let limit = clamp_catalog_limit(limit);
    let truncated = rows.len() > limit;
    if truncated {
        rows.truncate(limit);
    }
    let next_cursor = if truncated {
        rows.last().map(cursor_for)
    } else {
        None
    };
    (truncated, next_cursor)
}

fn query_count(db: &Connection, sql: &str, params: SqlParams) -> Result<usize> {
    let mut stmt = db.prepare(sql).context("failed to prepare count query")?;
    let param_refs = params.refs();
    let count: i64 = stmt
        .query_row(param_refs.as_slice(), |row| row.get(0))
        .context("failed to execute count query")?;
    usize::try_from(count).context("count did not fit usize")
}

fn collect_rows<T, F>(db: &Connection, sql: &str, params: SqlParams, mut map: F) -> Result<Vec<T>>
where
    F: for<'r> FnMut(&Row<'r>) -> duckdb::Result<T>,
{
    let mut stmt = db.prepare(sql).context("failed to prepare query")?;
    let param_refs = params.refs();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| map(row))
        .context("failed to execute query")?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read query rows")
}

fn optional_no_rows<T>(result: duckdb::Result<T>, context: &'static str) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error).context(context),
    }
}

fn i64_to_usize(value: i64, field: &'static str) -> duckdb::Result<usize> {
    usize::try_from(value).map_err(|error| conversion_error(field, error))
}

fn conversion_error(
    field: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> duckdb::Error {
    duckdb::Error::FromSqlConversionFailure(
        0,
        duckdb::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{field} value could not be converted: {error}"),
        )),
    )
}

impl From<&CodeCandidate> for ResolvedNode {
    fn from(candidate: &CodeCandidate) -> Self {
        Self {
            stable_symbol_id: candidate.stable_symbol_id.clone(),
            source: candidate.source.clone(),
            package: candidate.package.clone(),
            revision: candidate.revision.clone(),
            entity_name: candidate.entity_name.clone(),
            qualified_name: candidate.qualified_name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedNode {
    stable_symbol_id: String,
    source: String,
    package: String,
    revision: String,
    entity_name: String,
    qualified_name: String,
}

#[derive(Debug)]
struct ParsedSelector {
    source: String,
    package: String,
    revision: Option<String>,
    name: String,
}

#[derive(Debug)]
struct ParsedStableSelector {
    source: String,
    package: String,
    revision: String,
    stable_symbol_id: String,
}

#[derive(Debug)]
struct SymbolSourceRow {
    stable_symbol_id: String,
    package_source: String,
    package: String,
    revision: String,
    file_path: String,
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    line_end: usize,
    qualified_name: String,
    source_text: Option<String>,
}

#[derive(Default)]
struct SqlParams {
    values: Vec<Box<dyn ToSql>>,
}

impl SqlParams {
    fn from_values<T, const N: usize>(values: [T; N]) -> Self
    where
        T: ToSql + 'static,
    {
        Self {
            values: values
                .into_iter()
                .map(|value| Box::new(value) as Box<dyn ToSql>)
                .collect(),
        }
    }

    fn push<T>(&mut self, value: T) -> String
    where
        T: ToSql + 'static,
    {
        self.values.push(Box::new(value));
        format!("${}", self.values.len())
    }

    fn refs(&self) -> Vec<&dyn ToSql> {
        self.values
            .iter()
            .map(|value| value.as_ref() as &dyn ToSql)
            .collect()
    }
}

#[cfg(test)]
mod pkg_symbol_uri_tests {
    use super::{decode_uri_component, encode_uri_component, parse_pkg_symbol_uri};

    fn build_uri(source: &str, package: &str, revision: &str, id: &str) -> String {
        format!(
            "pkg-symbol://{}/{}/{}/{}",
            encode_uri_component(source),
            encode_uri_component(package),
            encode_uri_component(revision),
            encode_uri_component(id),
        )
    }

    #[test]
    fn round_trips_simple_components() {
        let uri = build_uri("registry:crates-io", "memchr", "0.1.0", "465192211e73e0bf");
        let parsed = parse_pkg_symbol_uri(&uri).expect("parse");
        assert_eq!(parsed.source, "registry:crates-io");
        assert_eq!(parsed.package, "memchr");
        assert_eq!(parsed.revision, "0.1.0");
        assert_eq!(parsed.stable_symbol_id, "465192211e73e0bf");
    }

    #[test]
    fn round_trips_github_owner_repo_package_with_slash() {
        let uri = build_uri("github", "BurntSushi/memchr", "master", "465192211e73e0bf");
        // Exactly one encoded slash inside the package; URI has 3 literal slashes.
        assert_eq!(
            uri.matches('/').count(),
            3 + "pkg-symbol://".matches('/').count()
        );
        let parsed = parse_pkg_symbol_uri(&uri).expect("parse");
        assert_eq!(parsed.source, "github");
        assert_eq!(parsed.package, "BurntSushi/memchr");
        assert_eq!(parsed.revision, "master");
        assert_eq!(parsed.stable_symbol_id, "465192211e73e0bf");
    }

    #[test]
    fn round_trips_slashy_source_and_revision() {
        let uri = build_uri(
            "git:github.com/DeusData/codebase-memory-mcp",
            "DeusData/codebase-memory-mcp",
            "feature/x",
            "abcd1234",
        );
        let parsed = parse_pkg_symbol_uri(&uri).expect("parse");
        assert_eq!(parsed.source, "git:github.com/DeusData/codebase-memory-mcp");
        assert_eq!(parsed.package, "DeusData/codebase-memory-mcp");
        assert_eq!(parsed.revision, "feature/x");
        assert_eq!(parsed.stable_symbol_id, "abcd1234");
    }

    #[test]
    fn rejects_wrong_segment_count() {
        assert!(parse_pkg_symbol_uri("pkg-symbol://only/three/parts").is_none());
        assert!(parse_pkg_symbol_uri("not-a-pkg-symbol-uri").is_none());
    }

    #[test]
    fn decode_is_inverse_of_encode() {
        for raw in ["a/b%c", "100%/done", "plain", "a%2Fb"] {
            assert_eq!(decode_uri_component(&encode_uri_component(raw)), raw);
        }
    }
}
