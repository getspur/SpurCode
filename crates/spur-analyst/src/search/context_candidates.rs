use std::path::Path;

use anyhow::{anyhow, Context as _, Result};

use crate::{
    db::{
        connection::open_analyst_connection_read_only,
        extensions::{analyst_extension_load_sql, load_analyst_icu_extension},
        sql::sql_escape_literal,
    },
    search::hybrid::format_query_vec_sql,
    KnowledgeCandidate, KnowledgeQueryOptions, KnowledgeQueryResult, KnowledgeSearchScope,
};

const MAX_CONTEXT_CANDIDATES: usize = 40;

pub fn query_context_candidates(
    db_path: &Path,
    query: &str,
    scope: KnowledgeSearchScope,
    options: KnowledgeQueryOptions,
) -> Result<KnowledgeQueryResult> {
    let conn = open_analyst_connection_read_only(db_path)?;
    load_analyst_icu_extension(&conn);
    conn.execute_batch(&analyst_extension_load_sql("fts"))
        .context("failed to load FTS extension for context candidate query")?;
    query_context_candidates_with_conn(&conn, db_path, query, scope, options)
}

pub fn query_context_candidates_with_conn(
    conn: &duckdb::Connection,
    db_path: &Path,
    query: &str,
    scope: KnowledgeSearchScope,
    options: KnowledgeQueryOptions,
) -> Result<KnowledgeQueryResult> {
    let query = query.trim();
    if query.is_empty() {
        return Err(anyhow!("knowledge context query must be non-empty"));
    }
    let limit = options.limit.clamp(1, MAX_CONTEXT_CANDIDATES);

    let graph_content_hash = conn
        .query_row("SELECT graph_content_hash FROM _meta", [], |row| row.get(0))
        .ok();

    let escaped_query = sql_escape_literal(query);
    let sql_scope = scope.as_sql_scope();
    let sql_intent = options.intent.as_sql_intent();
    let query_vec_sql = format_query_vec_sql(options.query_vec.as_deref());
    let mut hybrid_failed = false;
    let candidates = match query_context_candidates_inner(
        conn,
        &escaped_query,
        sql_scope,
        sql_intent,
        query_vec_sql.as_deref(),
        limit,
    ) {
        Ok(candidates) => candidates,
        Err(error) if query_vec_sql.is_some() => {
            hybrid_failed = true;
            tracing::warn!(
                error = %format!("{error:#}"),
                query,
                scope = sql_scope,
                intent = sql_intent,
                limit,
                "hybrid search failed; degrading to BM25-only context candidate search"
            );
            let _ = conn.execute_batch("ROLLBACK;");
            query_context_candidates_inner(
                conn,
                &escaped_query,
                sql_scope,
                sql_intent,
                None,
                limit,
            )?
        }
        Err(error) => return Err(error),
    };
    if query_vec_sql.is_some()
        && !hybrid_failed
        && candidates
            .iter()
            .all(|candidate| !candidate.grounding.starts_with("hybrid-"))
    {
        tracing::warn!(
            query,
            scope = sql_scope,
            intent = sql_intent,
            limit,
            "hybrid search produced no surviving hybrid-grounded context candidates"
        );
    }

    Ok(KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash,
        candidates,
    })
}

fn query_context_candidates_inner(
    conn: &duckdb::Connection,
    escaped_query: &str,
    sql_scope: &str,
    sql_intent: &str,
    query_vec_sql: Option<&str>,
    limit: usize,
) -> Result<Vec<KnowledgeCandidate>> {
    let macro_call = match query_vec_sql {
        Some(query_vec_sql) => format!(
            "search_context_candidates_hybrid('{escaped_query}', '{sql_scope}', '{sql_intent}', {query_vec_sql})"
        ),
        None => {
            format!("search_context_candidates('{escaped_query}', '{sql_scope}', '{sql_intent}')")
        }
    };
    let sql = format!(
        "SELECT kind, title, file_path, stable_symbol_id, symbol_kind, score, \
         signal, neighbor_kind, edge_bind_method, grounding \
         FROM {macro_call} \
         LIMIT {limit}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .context("failed to prepare context candidate query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(KnowledgeCandidate {
                kind: row.get(0)?,
                title: row.get(1)?,
                file_path: row.get(2)?,
                stable_symbol_id: row.get(3)?,
                symbol_kind: row.get(4)?,
                score: row.get(5)?,
                signal: row.get(6)?,
                neighbor_kind: row.get(7)?,
                edge_bind_method: row.get(8)?,
                grounding: row.get(9)?,
            })
        })
        .context("failed to run context candidate query")?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read context candidate rows")
}
