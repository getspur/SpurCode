from __future__ import annotations

from collections.abc import Iterable

import pyarrow as pa
import pyarrow.ipc as pa_ipc
from spur_app import NotebookClient

import schema

STABLE_ID_COLUMNS = ("stable_symbol_id", "source_stable_id", "target_stable_id")


def table_to_ipc_bytes(table: pa.Table) -> bytes:
    """Serialize a PyArrow table as Arrow IPC stream bytes."""
    sink = pa.BufferOutputStream()
    with pa_ipc.new_stream(sink, table.schema) as writer:
        writer.write_table(table)
    return sink.getvalue().to_pybytes()


def push_table(
    port: str,
    table: pa.Table,
    *,
    client: NotebookClient | None = None,
) -> list[str]:
    """Push an Arrow table to a source port and return pushed stable ids."""
    _validate_port_schema(port, table.schema)
    ipc_bytes = table_to_ipc_bytes(table)
    resolved_client = client or NotebookClient()
    resolved_client.push_source(port, ipc_bytes)
    return stable_symbol_ids(table)


def stable_symbol_ids(table: pa.Table) -> list[str]:
    """Return unique stable ids from symbol-id columns in table order."""
    seen: set[str] = set()
    ids: list[str] = []
    for value in _iter_stable_values(table):
        if value and value not in seen:
            seen.add(value)
            ids.append(value)
    return ids


def _iter_stable_values(table: pa.Table) -> Iterable[str]:
    for column_name in STABLE_ID_COLUMNS:
        if column_name not in table.column_names:
            continue
        for value in table.column(column_name).to_pylist():
            if value is not None:
                yield str(value)


def _validate_port_schema(port: str, actual_schema: pa.Schema) -> None:
    expected_schema = schema.schema_for(port)
    if actual_schema.remove_metadata() != expected_schema:
        raise ValueError(
            f"table schema does not match port {port!r}: "
            f"expected {expected_schema.names}, got {actual_schema.names}"
        )
