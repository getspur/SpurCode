from __future__ import annotations

from collections import deque
from collections.abc import Iterable
from typing import Any

import analyst
import graph
import ports
import schema

SUBGRAPH_PORT = "subgraph"
VALID_EDGE_KINDS = frozenset(graph.DEFAULT_EDGE_KINDS)


def wb_subgraph(
    symbol: str,
    depth: int = 1,
    edge_kinds: list[str] | None = None,
):
    """Push a bounded code subgraph for symbol into the subgraph port."""
    bounded_depth = _bounded_depth(depth)
    selected_edge_kinds = _normalize_edge_kinds(edge_kinds)
    table = _subgraph_rows(symbol, depth=bounded_depth, edge_kinds=selected_edge_kinds)
    pushed_ids = ports.push_table(SUBGRAPH_PORT, table)
    status, message = _guidance_from_metadata(table)
    result = {
        "ok": True,
        "status": status,
        "port": SUBGRAPH_PORT,
        "symbol": symbol,
        "depth": bounded_depth,
        "edge_kinds": selected_edge_kinds,
        "row_count": table.num_rows,
        "stable_symbol_ids": pushed_ids,
        "pushed_stable_symbol_ids": pushed_ids,
    }
    if message:
        result["message"] = message
    return result


def _subgraph_rows(symbol: str, *, depth: int, edge_kinds: list[str]):
    table = analyst.run_query(
        _subgraph_sql(edge_kinds),
        _subgraph_parameters(symbol, depth=depth, edge_kinds=edge_kinds),
        schema.SUBGRAPH_SCHEMA,
    )
    if table.num_rows == 0 or _guidance_from_metadata(table)[0] != "ok":
        return table
    return _mark_shortest_path(table)


def _subgraph_sql(edge_kinds: list[str]) -> str:
    edge_placeholders = ", ".join("?" for _ in edge_kinds)
    return f"""
        WITH RECURSIVE
        seed AS (
          SELECT stable_symbol_id
          FROM duckpgq_nodes
          WHERE stable_symbol_id = ?
             OR qualified_name = ?
             OR entity_name = ?
             OR file_path = ?
          ORDER BY
            CASE
              WHEN stable_symbol_id = ? THEN 0
              WHEN qualified_name = ? THEN 1
              WHEN entity_name = ? THEN 2
              WHEN file_path = ? THEN 3
              ELSE 4
            END,
            stable_symbol_id
          LIMIT 1
        ),
        walk(stable_symbol_id, depth) AS (
          SELECT stable_symbol_id, 0 AS depth
          FROM seed

          UNION

          SELECT
            CASE
              WHEN e.source_stable_id = walk.stable_symbol_id THEN e.target_stable_id
              ELSE e.source_stable_id
            END AS stable_symbol_id,
            walk.depth + 1 AS depth
          FROM walk
          JOIN duckpgq_edges AS e
            ON e.source_stable_id = walk.stable_symbol_id
            OR e.target_stable_id = walk.stable_symbol_id
          WHERE walk.depth < ?
            AND e.edge_kind IN ({edge_placeholders})
        ),
        selected_nodes AS (
          SELECT stable_symbol_id, MIN(depth) AS depth
          FROM walk
          GROUP BY stable_symbol_id
        ),
        selected_edges AS (
          SELECT
            e.source_stable_id,
            e.target_stable_id,
            e.edge_kind,
            e.relation,
            e.confidence,
            e.bind_method,
            GREATEST(src.depth, dst.depth) AS depth
          FROM duckpgq_edges AS e
          JOIN selected_nodes AS src
            ON src.stable_symbol_id = e.source_stable_id
          JOIN selected_nodes AS dst
            ON dst.stable_symbol_id = e.target_stable_id
          WHERE e.edge_kind IN ({edge_placeholders})
        )
        SELECT *
        FROM (
          SELECT
            'node' AS row_kind,
            n.stable_symbol_id,
            CAST(n.node_id AS BIGINT) AS node_id,
            n.qualified_name,
            n.entity_name,
            n.symbol_kind,
            n.file_path,
            CAST(NULL AS VARCHAR) AS source_stable_id,
            CAST(NULL AS VARCHAR) AS target_stable_id,
            CAST(NULL AS VARCHAR) AS edge_kind,
            CAST(NULL AS VARCHAR) AS relation,
            CAST(NULL AS VARCHAR) AS confidence,
            CAST(NULL AS VARCHAR) AS bind_method,
            CAST(sn.depth AS BIGINT) AS depth,
            FALSE AS is_shortest_path
          FROM selected_nodes AS sn
          JOIN duckpgq_nodes AS n
            ON n.stable_symbol_id = sn.stable_symbol_id

          UNION ALL

          SELECT
            'edge' AS row_kind,
            CAST(NULL AS VARCHAR) AS stable_symbol_id,
            CAST(NULL AS BIGINT) AS node_id,
            CAST(NULL AS VARCHAR) AS qualified_name,
            CAST(NULL AS VARCHAR) AS entity_name,
            CAST(NULL AS VARCHAR) AS symbol_kind,
            CAST(NULL AS VARCHAR) AS file_path,
            source_stable_id,
            target_stable_id,
            edge_kind,
            relation,
            confidence,
            bind_method,
            CAST(depth AS BIGINT) AS depth,
            FALSE AS is_shortest_path
          FROM selected_edges
        ) AS rows
        ORDER BY
          CASE row_kind WHEN 'node' THEN 0 ELSE 1 END,
          depth,
          COALESCE(stable_symbol_id, source_stable_id),
          COALESCE(target_stable_id, ''),
          COALESCE(edge_kind, '')
        """


def _subgraph_parameters(symbol: str, *, depth: int, edge_kinds: list[str]) -> list[Any]:
    seed_parameters = [symbol, symbol, symbol, symbol]
    return [
        *seed_parameters,
        *seed_parameters,
        depth,
        *edge_kinds,
        *edge_kinds,
    ]


def _mark_shortest_path(table):
    rows = table.to_pylist()
    seed = _seed_stable_symbol_id(rows)
    if seed is None:
        return table

    path_nodes, path_edges = _shortest_path_marks(rows, seed)
    if not path_edges:
        return table

    for row in rows:
        if row["row_kind"] == "node":
            row["is_shortest_path"] = row["stable_symbol_id"] in path_nodes
        elif row["row_kind"] == "edge":
            row["is_shortest_path"] = _edge_key(row) in path_edges
    return graph.table_from_rows(rows)


def _shortest_path_marks(
    rows: Iterable[dict[str, Any]],
    seed: str,
) -> tuple[set[str], set[tuple[str | None, ...]]]:
    adjacency: dict[str, list[tuple[str, tuple[str | None, ...]]]] = {}
    for row in rows:
        if row.get("row_kind") != "edge":
            continue
        source = row.get("source_stable_id")
        target = row.get("target_stable_id")
        if source is None or target is None:
            continue
        edge_key = _edge_key(row)
        adjacency.setdefault(source, []).append((target, edge_key))
        adjacency.setdefault(target, []).append((source, edge_key))

    parents: dict[str, tuple[str | None, tuple[str | None, ...] | None]] = {
        seed: (None, None)
    }
    distances = {seed: 0}
    queue: deque[str] = deque([seed])
    while queue:
        current = queue.popleft()
        neighbors = sorted(adjacency.get(current, []), key=lambda item: item[0])
        for neighbor, edge_key in neighbors:
            if neighbor in parents:
                continue
            parents[neighbor] = (current, edge_key)
            distances[neighbor] = distances[current] + 1
            queue.append(neighbor)

    targets = [node for node in parents if node != seed]
    if not targets:
        return {seed}, set()
    target = max(targets, key=lambda node: (distances[node], node))

    path_nodes: set[str] = set()
    path_edges: set[tuple[str | None, ...]] = set()
    current: str | None = target
    while current is not None:
        path_nodes.add(current)
        parent, edge_key = parents[current]
        if edge_key is not None:
            path_edges.add(edge_key)
        current = parent
    return path_nodes, path_edges


def _seed_stable_symbol_id(rows: Iterable[dict[str, Any]]) -> str | None:
    for row in rows:
        if row.get("row_kind") == "node" and row.get("depth") == 0:
            stable_symbol_id = row.get("stable_symbol_id")
            if stable_symbol_id is not None:
                return str(stable_symbol_id)
    return None


def _edge_key(row: dict[str, Any]) -> tuple[str | None, ...]:
    return (
        row.get("source_stable_id"),
        row.get("target_stable_id"),
        row.get("edge_kind"),
        row.get("relation"),
        row.get("confidence"),
        row.get("bind_method"),
    )


def _bounded_depth(depth: int) -> int:
    try:
        parsed = int(depth)
    except (TypeError, ValueError):
        parsed = 1
    return max(0, parsed)


def _normalize_edge_kinds(edge_kinds: list[str] | None) -> list[str]:
    if not edge_kinds:
        return list(graph.DEFAULT_EDGE_KINDS)

    selected: list[str] = []
    for edge_kind in edge_kinds:
        normalized = str(edge_kind)
        if normalized not in VALID_EDGE_KINDS:
            raise ValueError(f"unsupported edge_kind: {normalized}")
        if normalized not in selected:
            selected.append(normalized)
    return selected


def _guidance_from_metadata(table):
    metadata = table.schema.metadata or {}
    status = metadata.get(schema.GUIDANCE_STATUS_KEY)
    message = metadata.get(schema.GUIDANCE_MESSAGE_KEY)
    return (
        status.decode() if status else "ok",
        message.decode() if message else None,
    )
