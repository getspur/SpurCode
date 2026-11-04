use std::path::Path;

use anyhow::{anyhow, Context as _, Result};

use crate::{
    db::{
        connection::open_analyst_connection_read_only,
        extensions::{analyst_extension_load_sql, load_analyst_icu_extension},
        sql::sql_escape_literal,
    },
    KnowledgeCandidate, KnowledgeQueryOptions, KnowledgeQueryResult,
};

const MAX_GRAPH_CANDIDATES: usize = 30;

pub fn query_graph_candidates(
    db_path: &Path,
    query: &str,
    options: KnowledgeQueryOptions,
) -> Result<KnowledgeQueryResult> {
    let conn = open_analyst_connection_read_only(db_path)?;
    load_analyst_icu_extension(&conn);
    conn.execute_batch(&analyst_extension_load_sql("fts"))
        .context("failed to load FTS extension for graph candidate query")?;
    query_graph_candidates_with_conn(&conn, db_path, query, options)
}

pub fn query_graph_candidates_with_conn(
    conn: &duckdb::Connection,
    db_path: &Path,
    query: &str,
    options: KnowledgeQueryOptions,
) -> Result<KnowledgeQueryResult> {
    let query = query.trim();
    if query.is_empty() {
        return Err(anyhow!("knowledge graph query must be non-empty"));
    }
    let limit = options.limit.clamp(1, MAX_GRAPH_CANDIDATES);

    let graph_content_hash = conn
        .query_row("SELECT graph_content_hash FROM _meta", [], |row| row.get(0))
        .ok();

    let escaped_query = sql_escape_literal(query);
    let sql_intent = options.intent.as_sql_intent();
    let sql = format!(
        "SELECT kind, title, file_path, stable_symbol_id, symbol_kind, score, \
         signal, neighbor_kind, edge_bind_method, grounding \
         FROM search_graph('{escaped_query}', '{sql_intent}') \
         LIMIT {limit}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .context("failed to prepare graph candidate query")?;
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
        .context("failed to run graph candidate query")?;
    let candidates = rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read graph candidate rows")?;

    Ok(KnowledgeQueryResult {
        db_path: db_path.display().to_string(),
        graph_content_hash,
        candidates,
    })
}

pub fn merge_graph_candidates(
    result: &mut KnowledgeQueryResult,
    graph_result: KnowledgeQueryResult,
) {
    result.candidates.extend(graph_result.candidates);

    let mut deduped = Vec::with_capacity(result.candidates.len());
    for candidate in result.candidates.drain(..) {
        let Some(stable_symbol_id) = candidate.stable_symbol_id.as_deref() else {
            deduped.push(candidate);
            continue;
        };

        if let Some(existing) = deduped
            .iter_mut()
            .find(|existing| existing.stable_symbol_id.as_deref() == Some(stable_symbol_id))
        {
            if candidate.score > existing.score {
                *existing = candidate;
            }
        } else {
            deduped.push(candidate);
        }
    }

    result.candidates = deduped;
}
