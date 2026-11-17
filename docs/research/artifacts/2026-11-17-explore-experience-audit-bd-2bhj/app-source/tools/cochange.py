from __future__ import annotations

import analyst
import ports
import schema

COCHANGE_PORT = "cochange"
DEFAULT_LIMIT = 50


def wb_cochange(file: str):
    """Push file co-change evidence for file into the cochange port."""
    table = analyst.cochange_rows(file, limit=DEFAULT_LIMIT)
    pushed_ids = ports.push_table(COCHANGE_PORT, table)
    pushed_file_paths = _file_paths(table)
    status, message = _guidance_from_metadata(table)
    result = {
        "ok": True,
        "status": status,
        "port": COCHANGE_PORT,
        "file": file,
        "row_count": table.num_rows,
        "stable_symbol_ids": pushed_ids,
        "pushed_stable_symbol_ids": pushed_ids,
        "file_paths": pushed_file_paths,
        "pushed_file_paths": pushed_file_paths,
    }
    if message:
        result["message"] = message
    return result


def _file_paths(table):
    seen: set[str] = set()
    paths: list[str] = []
    for row in table.select(["file_a", "file_b"]).to_pylist():
        for value in (row.get("file_a"), row.get("file_b")):
            if value is None:
                continue
            path = str(value)
            if path in seen:
                continue
            seen.add(path)
            paths.append(path)
    return paths


def _guidance_from_metadata(table):
    metadata = table.schema.metadata or {}
    status = metadata.get(schema.GUIDANCE_STATUS_KEY)
    message = metadata.get(schema.GUIDANCE_MESSAGE_KEY)
    return (
        status.decode() if status else "ok",
        message.decode() if message else None,
    )
