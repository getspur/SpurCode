from __future__ import annotations

import analyst
import ports
import schema

SCORECARD_PORT = "scorecard"
DEFAULT_LIMIT = 25


def wb_scorecard(symbol: str):
    """Push inspector scorecard evidence for symbol into the scorecard port."""
    table = analyst.scorecard_rows(symbol, limit=DEFAULT_LIMIT)
    pushed_ids = ports.push_table(SCORECARD_PORT, table)
    status, message = _guidance_from_metadata(table)
    result = {
        "ok": True,
        "status": status,
        "port": SCORECARD_PORT,
        "symbol": symbol,
        "row_count": table.num_rows,
        "stable_symbol_id": pushed_ids[0] if pushed_ids else None,
        "stable_symbol_ids": pushed_ids,
        "pushed_stable_symbol_ids": pushed_ids,
    }
    if message:
        result["message"] = message
    return result


def _guidance_from_metadata(table):
    metadata = table.schema.metadata or {}
    status = metadata.get(schema.GUIDANCE_STATUS_KEY)
    message = metadata.get(schema.GUIDANCE_MESSAGE_KEY)
    return (
        status.decode() if status else "ok",
        message.decode() if message else None,
    )
