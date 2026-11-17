from __future__ import annotations

import analyst
import ports
import schema

ANALYST_ROWS_PORT = "analyst_rows"
DEFAULT_LIMIT = 50


def wb_blast_radius(symbol: str):
    """Push ranked blast-radius evidence for symbol into the analyst_rows port."""
    table = _blast_radius_scorecard_rows(symbol)
    pushed_ids = ports.push_table(ANALYST_ROWS_PORT, table)
    status, message = _guidance_from_metadata(table)
    result = {
        "ok": True,
        "status": status,
        "port": ANALYST_ROWS_PORT,
        "symbol": symbol,
        "row_count": table.num_rows,
        "stable_symbol_ids": pushed_ids,
        "pushed_stable_symbol_ids": pushed_ids,
    }
    if message:
        result["message"] = message
    return result


def _blast_radius_scorecard_rows(symbol: str):
    pattern = analyst._contains_pattern(symbol)
    return analyst.run_query(
        """
        SELECT
          br.stable_symbol_id,
          sc.entity_name,
          sc.symbol_kind,
          sc.file_path,
          CAST(br.caller_count AS BIGINT) AS caller_count,
          CAST(br.hot_caller_count AS BIGINT) AS hot_caller_count,
          CAST(br.caller_churn_90d AS BIGINT) AS caller_churn_90d,
          CAST(br.self_churn_90d AS BIGINT) AS self_churn_90d,
          CAST(sc.last_touched AS VARCHAR) AS self_last_touched,
          br.blast_radius_score
        FROM v_blast_radius AS br
        LEFT JOIN v_symbol_scorecard AS sc
          ON sc.stable_symbol_id = br.stable_symbol_id
        WHERE br.stable_symbol_id = ?
           OR sc.stable_symbol_id = ?
           OR sc.qualified_name = ?
           OR sc.entity_name = ?
           OR sc.file_path = ?
           OR COALESCE(sc.qualified_name, '') ILIKE ? ESCAPE '\\'
           OR COALESCE(sc.entity_name, '') ILIKE ? ESCAPE '\\'
           OR COALESCE(sc.file_path, '') ILIKE ? ESCAPE '\\'
        ORDER BY
          br.blast_radius_score DESC NULLS LAST,
          br.caller_count DESC NULLS LAST,
          br.stable_symbol_id
        LIMIT ?
        """,
        [
            symbol,
            symbol,
            symbol,
            symbol,
            symbol,
            pattern,
            pattern,
            pattern,
            DEFAULT_LIMIT,
        ],
        schema.ANALYST_ROWS_SCHEMA,
    )


def _guidance_from_metadata(table):
    metadata = table.schema.metadata or {}
    status = metadata.get(schema.GUIDANCE_STATUS_KEY)
    message = metadata.get(schema.GUIDANCE_MESSAGE_KEY)
    return (
        status.decode() if status else "ok",
        message.decode() if message else None,
    )
