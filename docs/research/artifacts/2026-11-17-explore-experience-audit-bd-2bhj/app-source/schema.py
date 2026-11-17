from __future__ import annotations

from collections.abc import Mapping

import pyarrow as pa

GUIDANCE_STATUS_KEY = b"workbench_status"
GUIDANCE_MESSAGE_KEY = b"workbench_message"

SUBGRAPH_SCHEMA = pa.schema(
    [
        pa.field("row_kind", pa.string()),
        pa.field("stable_symbol_id", pa.string()),
        pa.field("node_id", pa.int64()),
        pa.field("qualified_name", pa.string()),
        pa.field("entity_name", pa.string()),
        pa.field("symbol_kind", pa.string()),
        pa.field("file_path", pa.string()),
        pa.field("source_stable_id", pa.string()),
        pa.field("target_stable_id", pa.string()),
        pa.field("edge_kind", pa.string()),
        pa.field("relation", pa.string()),
        pa.field("confidence", pa.string()),
        pa.field("bind_method", pa.string()),
        pa.field("depth", pa.int64()),
        pa.field("is_shortest_path", pa.bool_()),
    ]
)

ANALYST_ROWS_SCHEMA = pa.schema(
    [
        pa.field("stable_symbol_id", pa.string()),
        pa.field("entity_name", pa.string()),
        pa.field("symbol_kind", pa.string()),
        pa.field("file_path", pa.string()),
        pa.field("caller_count", pa.int64()),
        pa.field("hot_caller_count", pa.int64()),
        pa.field("caller_churn_90d", pa.int64()),
        pa.field("self_churn_90d", pa.int64()),
        pa.field("self_last_touched", pa.string()),
        pa.field("blast_radius_score", pa.float64()),
    ]
)

SCORECARD_SCHEMA = pa.schema(
    [
        pa.field("stable_symbol_id", pa.string()),
        pa.field("entity_name", pa.string()),
        pa.field("qualified_name", pa.string()),
        pa.field("symbol_kind", pa.string()),
        pa.field("file_path", pa.string()),
        pa.field("pagerank", pa.float64()),
        pa.field("in_degree", pa.int64()),
        pa.field("out_degree", pa.int64()),
        pa.field("component_id", pa.int64()),
        pa.field("community_id", pa.int64()),
        pa.field("callers", pa.int64()),
        pa.field("importers", pa.int64()),
        pa.field("churn_90d", pa.int64()),
        pa.field("last_touched", pa.string()),
        pa.field("posture", pa.string()),
        pa.field("blast_radius_score", pa.float64()),
    ]
)

COCHANGE_SCHEMA = pa.schema(
    [
        pa.field("file_a", pa.string()),
        pa.field("file_b", pa.string()),
        pa.field("cochange_count", pa.int64()),
        pa.field("has_static_edge", pa.bool_()),
    ]
)

HOTSPOTS_SCHEMA = pa.schema(
    [
        pa.field("file_path", pa.string()),
        pa.field("commits", pa.int64()),
        pa.field("fix_commits", pa.int64()),
        pa.field("fix_pct", pa.float64()),
    ]
)

META_SCHEMA = pa.schema(
    [
        pa.field("node_count", pa.int64()),
        pa.field("resolved_edge_count", pa.int64()),
        pa.field("graph_content_hash", pa.string()),
    ]
)

PORT_SCHEMAS: Mapping[str, pa.Schema] = {
    "subgraph": SUBGRAPH_SCHEMA,
    "analyst_rows": ANALYST_ROWS_SCHEMA,
    "scorecard": SCORECARD_SCHEMA,
    "cochange": COCHANGE_SCHEMA,
}


def schema_for(port: str) -> pa.Schema:
    """Return the Arrow schema for a declared workbench source port."""
    try:
        return PORT_SCHEMAS[port]
    except KeyError as exc:
        raise ValueError(f"unknown workbench port: {port}") from exc


def empty_table(schema: pa.Schema) -> pa.Table:
    """Create an empty Arrow table with the supplied schema."""
    arrays = [pa.array([], type=field.type) for field in schema]
    return pa.Table.from_arrays(arrays, schema=schema)


def guided_empty_table(schema: pa.Schema, status: str, message: str) -> pa.Table:
    """Create an empty table whose schema metadata explains why it is empty."""
    existing = dict(schema.metadata or {})
    existing[GUIDANCE_STATUS_KEY] = status.encode()
    existing[GUIDANCE_MESSAGE_KEY] = message.encode()
    return empty_table(schema.with_metadata(existing))
