from __future__ import annotations

import time
from pathlib import Path
from typing import Any

import duckdb
import pyarrow as pa

import schema

TRANSIENT_LOCK_MARKERS = (
    "database is locked",
    "database file is locked",
    "could not set lock",
    "conflicting lock",
    "lock on file",
)


def find_analyst_db(cwd: str | Path | None = None) -> Path | None:
    """Find `.spur/analyst.duckdb` by walking upward from cwd."""
    start = Path(cwd or Path.cwd()).resolve()
    roots = (start, *start.parents)
    for root in roots:
        candidate = root / ".spur" / "analyst.duckdb"
        if candidate.is_file():
            return candidate
    return None


def open_analyst(cwd: str | Path | None = None) -> duckdb.DuckDBPyConnection | None:
    """Open the analyst database read-only, or return None when absent."""
    db_path = find_analyst_db(cwd)
    if db_path is None:
        return None
    return duckdb.connect(str(db_path), read_only=True)


def blast_radius_rows(
    symbol: str,
    *,
    cwd: str | Path | None = None,
    limit: int = 50,
) -> pa.Table:
    """Return `v_blast_radius` rows for a symbol as the analyst_rows port schema."""
    pattern = _contains_pattern(symbol)
    return run_query(
        """
        SELECT
          stable_symbol_id,
          entity_name,
          symbol_kind,
          file_path,
          CAST(caller_count AS BIGINT) AS caller_count,
          CAST(hot_caller_count AS BIGINT) AS hot_caller_count,
          CAST(caller_churn_90d AS BIGINT) AS caller_churn_90d,
          CAST(self_churn_90d AS BIGINT) AS self_churn_90d,
          CAST(self_last_touched AS VARCHAR) AS self_last_touched,
          blast_radius_score
        FROM v_blast_radius
        WHERE stable_symbol_id = ?
           OR entity_name = ?
           OR file_path = ?
           OR entity_name ILIKE ? ESCAPE '\\'
           OR file_path ILIKE ? ESCAPE '\\'
        ORDER BY blast_radius_score DESC NULLS LAST
        LIMIT ?
        """,
        [symbol, symbol, symbol, pattern, pattern, limit],
        schema.ANALYST_ROWS_SCHEMA,
        cwd=cwd,
    )


def scorecard_rows(
    symbol: str,
    *,
    cwd: str | Path | None = None,
    limit: int = 25,
) -> pa.Table:
    """Return `v_symbol_scorecard` rows for a symbol as the scorecard port schema."""
    pattern = _contains_pattern(symbol)
    return run_query(
        """
        SELECT
          stable_symbol_id,
          entity_name,
          qualified_name,
          symbol_kind,
          file_path,
          pagerank,
          CAST(in_degree AS BIGINT) AS in_degree,
          CAST(out_degree AS BIGINT) AS out_degree,
          CAST(component_id AS BIGINT) AS component_id,
          CAST(community_id AS BIGINT) AS community_id,
          CAST(callers AS BIGINT) AS callers,
          CAST(importers AS BIGINT) AS importers,
          CAST(churn_90d AS BIGINT) AS churn_90d,
          CAST(last_touched AS VARCHAR) AS last_touched,
          posture,
          blast_radius_score
        FROM v_symbol_scorecard
        WHERE stable_symbol_id = ?
           OR qualified_name = ?
           OR entity_name = ?
           OR file_path = ?
           OR qualified_name ILIKE ? ESCAPE '\\'
           OR entity_name ILIKE ? ESCAPE '\\'
           OR file_path ILIKE ? ESCAPE '\\'
        ORDER BY blast_radius_score DESC NULLS LAST, pagerank DESC NULLS LAST
        LIMIT ?
        """,
        [symbol, symbol, symbol, symbol, pattern, pattern, pattern, limit],
        schema.SCORECARD_SCHEMA,
        cwd=cwd,
    )


def cochange_rows(
    file: str,
    *,
    cwd: str | Path | None = None,
    limit: int = 50,
) -> pa.Table:
    """Return `v_file_cochange` rows for a file as the cochange port schema."""
    pattern = _contains_pattern(file)
    return run_query(
        """
        SELECT
          file_a,
          file_b,
          CAST(cochange_count AS BIGINT) AS cochange_count,
          has_static_edge
        FROM v_file_cochange
        WHERE file_a = ?
           OR file_b = ?
           OR file_a ILIKE ? ESCAPE '\\'
           OR file_b ILIKE ? ESCAPE '\\'
        ORDER BY cochange_count DESC NULLS LAST, file_a, file_b
        LIMIT ?
        """,
        [file, file, pattern, pattern, limit],
        schema.COCHANGE_SCHEMA,
        cwd=cwd,
    )


def fix_hotspot_rows(
    *,
    cwd: str | Path | None = None,
    limit: int = 50,
) -> pa.Table:
    """Return `v_fix_hotspots` rows for summary panels."""
    return run_query(
        """
        SELECT
          file_path,
          CAST(commits AS BIGINT) AS commits,
          CAST(fix_commits AS BIGINT) AS fix_commits,
          fix_pct
        FROM v_fix_hotspots
        ORDER BY fix_pct DESC NULLS LAST, fix_commits DESC NULLS LAST
        LIMIT ?
        """,
        [limit],
        schema.HOTSPOTS_SCHEMA,
        cwd=cwd,
    )


def meta_rows(cwd: str | Path | None = None) -> pa.Table:
    """Return live analyst `_meta` counts and graph hash."""
    return run_query(
        """
        SELECT
          CAST(node_count AS BIGINT) AS node_count,
          CAST(resolved_edge_count AS BIGINT) AS resolved_edge_count,
          graph_content_hash
        FROM _meta
        LIMIT 1
        """,
        [],
        schema.META_SCHEMA,
        cwd=cwd,
    )


def run_query(
    sql: str,
    parameters: list[Any],
    expected_schema: pa.Schema,
    *,
    cwd: str | Path | None = None,
    retries: int = 3,
    retry_delay_seconds: float = 0.1,
) -> pa.Table:
    """Run a read-only analyst query with transient-lock retry."""
    db_path = find_analyst_db(cwd)
    if db_path is None:
        return schema.guided_empty_table(
            expected_schema,
            "missing_analyst_db",
            "No .spur/analyst.duckdb was found while walking up from cwd.",
        )

    for attempt in range(retries + 1):
        try:
            conn = duckdb.connect(str(db_path), read_only=True)
            try:
                table = conn.execute(sql, parameters).fetch_arrow_table()
            finally:
                conn.close()
            return align_table(table, expected_schema)
        except duckdb.Error as exc:
            if not _is_transient_lock(exc) or attempt >= retries:
                raise
            time.sleep(retry_delay_seconds * (attempt + 1))

    raise RuntimeError("unreachable analyst retry state")


def align_table(table: pa.Table, expected_schema: pa.Schema) -> pa.Table:
    """Select and cast query output to the declared schema."""
    arrays = []
    for field in expected_schema:
        if field.name in table.column_names:
            arrays.append(table.column(field.name).cast(field.type))
        else:
            arrays.append(pa.nulls(table.num_rows, type=field.type))
    return pa.Table.from_arrays(arrays, schema=expected_schema)


def _contains_pattern(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
    return f"%{escaped}%"


def _is_transient_lock(exc: duckdb.Error) -> bool:
    message = str(exc).lower()
    return any(marker in message for marker in TRANSIENT_LOCK_MARKERS)
