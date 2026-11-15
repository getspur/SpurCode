"""Read-only structural audit of a prebuilt DuckDB 2 file-facts database.

    python project_quality.py --database snapshot.duckdb --out audit.json

The output file must be new. No extension is loaded and no extraction is run.
Required main-schema tables are inventory, sources, facts and the five raw
collections listed in COLLECTIONS. Discovery precedes all data queries; missing
fields fail explicitly. Each query receipt includes exact executable SQL and
parameters, a full result-row count, and a bounded, ordered display. --limit
(1..100, default 25) caps distributions and examples. Schema discovery and
the fixed invariant catalog have separate explicit ceilings of 256 and 100.

Audit design: retain inventory denominators (including unsupported files),
compare raw/typed collection counts per path, count structural invariants,
then display diagnostic evidence and one-hop code exploration. All numeric-ID
joins include logical_path. No label is promoted to a binding, and no traversal
or source-text output is unbounded. Repeated stable keys, file-only results and
unresolved calls are diagnostics, not proof of invalid extraction. A successful
CLI exit means the audit ran; it does not mean the extraction passed a quality
threshold. Source-byte fidelity and curated semantic validation belong to the
parent experiment. The approved extension hash is a contract identifier, not
verified ingestion provenance; this reader cannot authenticate how a DB arose.
"""

import argparse
import hashlib
import json
from pathlib import Path
import sys


APPROVED_EXTENSION_SHA256 = "e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028"
COLLECTIONS = ("nodes", "edges", "spans", "macro_invocations", "macro_scopes")
REQUIRED = {
    "inventory": "path git_blob_oid bytes suffix area status error".split(),
    "sources": "logical_path source source_bytes file_oid sha256".split(),
    "facts": ["logical_path", "value"],
    "nodes": "logical_path node_id stable_key label kind file_id source_span_id first_seen_run_id".split(),
    "edges": ("logical_path edge_id source_node_id target_node_id relation target_label import_path "
              "receiver_text scope_text confidence confidence_score edge_kind bind_method evidence_id "
              "directed change_kind").split(),
    "spans": "logical_path span_id file_id start_byte end_byte start_line end_line".split(),
    "macro_invocations": ("logical_path site_id file_path source_oid caller_stable_symbol_id enclosing "
                          "raw_macro_path normalized_macro_path byte_range line_range attributes "
                          "test_context debug_assertions_candidate execution_phase resolution "
                          "sink_family_candidate").split(),
    "macro_scopes": "logical_path scope file_path source_oid byte_range line_range".split(),
}
DISCOVERY_SQL = """SELECT table_name, column_name, data_type, ordinal_position
FROM information_schema.columns
WHERE table_catalog = current_database() AND table_schema = 'main'
  AND table_name IN (?, ?, ?, ?, ?, ?, ?, ?)
ORDER BY table_name, ordinal_position"""

# Aggregate first so duplicate inventory/source rows cannot multiply graph rows.
AREA_CTE = """areas AS (
  SELECT path, CASE WHEN count(DISTINCT struct_pack(v := area)) = 1
                   THEN min(area) END AS area,
         count(*) AS inventory_rows,
         count(DISTINCT struct_pack(v := area)) AS area_variants
  FROM main.inventory GROUP BY path
)"""
SOURCE_CTE = """source_sizes AS (
  SELECT logical_path, count(*) AS source_rows,
         min(octet_length(source_bytes)) AS byte_length
  FROM main.sources GROUP BY logical_path
)"""
NODE_CTE = """node_ids AS (
  SELECT logical_path, node_id, count(*) AS copies
  FROM main.nodes GROUP BY logical_path, node_id
)"""

# These are search candidates only. Owners are witnessed by contains edges or
# an explicit C++ qualified label. We do not reconstruct scope by guessing names.
SEEDS_CTE = """seeds AS (
  SELECT n.logical_path, n.node_id, n.label, n.kind, n.source_span_id,
         wanted.owner_label, wanted.method_label
  FROM main.nodes n
  CROSS JOIN (VALUES ('Connection', 'Query'), ('Parser', 'ParseQuery'))
    AS wanted(owner_label, method_label)
  WHERE (n.label = wanted.method_label OR
         n.label = wanted.owner_label || '::' || wanted.method_label OR
         n.label = 'duckdb::' || wanted.owner_label || '::' || wanted.method_label)
    AND (n.label <> wanted.method_label OR EXISTS (
      SELECT 1 FROM main.edges c JOIN main.nodes p
        ON p.logical_path = c.logical_path AND p.node_id = c.source_node_id
      WHERE c.logical_path = n.logical_path AND c.target_node_id = n.node_id
        AND c.relation = 'contains'
        AND p.label IN (wanted.owner_label, 'duckdb::' || wanted.owner_label)))
)"""


class QueryBook:
    """Counts scan the full relation; LIMIT affects displayed rows only."""

    def __init__(self, connection, limit):
        self.connection = connection
        self.limit = limit
        self.receipts = {}

    def add(self, name, sql, params=(), limit=None):
        sql = sql.strip()
        params = list(params)
        limit = self.limit if limit is None else limit
        count_sql = f"SELECT count(*) FROM ({sql}) AS audit_count"
        total = self.connection.execute(count_sql, params).fetchone()[0]
        display_sql = sql + "\nLIMIT ?"
        cursor = self.connection.execute(display_sql, params + [limit])
        columns = [field[0] for field in cursor.description]
        rows = [dict(zip(columns, row)) for row in cursor.fetchall()]
        self.receipts[name] = {
            "count_sql": count_sql, "count_params": params, "total_rows": total,
            "sql": display_sql, "params": params + [limit], "limit": limit,
            "columns": columns, "rows": rows, "displayed_rows": len(rows),
            "truncated": total > len(rows),
        }
        return rows


def _audit(connection, limit):
    book = QueryBook(connection, limit)
    schema = book.add("schema", DISCOVERY_SQL, tuple(REQUIRED), limit=256)
    if book.receipts["schema"]["truncated"]:
        raise ValueError("schema discovery exceeds 256 columns")
    discovered = {}
    for row in schema:
        discovered.setdefault(row["table_name"], {})[row["column_name"]] = row["data_type"]
    missing = [f"{table}.{column}" for table, columns in REQUIRED.items()
               for column in columns if column not in discovered.get(table, {})]
    if missing:
        raise ValueError("missing contract fields: " + ", ".join(missing))
    if discovered["sources"]["source_bytes"] != "BLOB":
        raise ValueError("sources.source_bytes must be BLOB")
    if not discovered["facts"]["value"].startswith("STRUCT("):
        raise ValueError("facts.value must be the raw file-facts STRUCT")

    book.add("inventory_totals", """SELECT count(*) AS inventory_rows,
      count(DISTINCT path) AS distinct_paths, coalesce(sum(bytes), 0) AS bytes,
      count(*) FILTER (WHERE path IS NULL) AS null_paths,
      count(*) FILTER (WHERE error IS NOT NULL) AS rows_with_errors
    FROM main.inventory""")
    for name, fields in (("status", "status"), ("suffix", "suffix"), ("area", "area"),
                         ("area_status_suffix", "area, status, suffix")):
        book.add("inventory_by_" + name, f"""SELECT {fields}, count(*) AS inventory_rows,
          coalesce(sum(bytes), 0) AS bytes FROM main.inventory
        GROUP BY {fields} ORDER BY inventory_rows DESC, {fields}""")
    book.add("inventory_errors", """SELECT path, status, suffix, area,
      left(error, 500) AS error_excerpt, length(error) AS error_characters
    FROM main.inventory WHERE error IS NOT NULL ORDER BY path, status, error""")
    book.add("extraction_totals", """SELECT
      (SELECT count(*) FROM main.sources) AS source_rows,
      (SELECT count(*) FROM main.facts) AS fact_rows,
      (SELECT count(*) FROM main.facts WHERE value IS NULL) AS null_fact_rows""")
    book.add("typed_counts", "\nUNION ALL\n".join(
        f"""SELECT '{table}' AS collection, count(*) AS typed_rows,
          count(DISTINCT logical_path) AS paths,
          (SELECT coalesce(sum(array_length(value.{table})), 0) FROM main.facts) AS raw_rows
        FROM main.{table}""" for table in COLLECTIONS) + "\nORDER BY collection")
    for name, table, fields in (
        ("node_kinds", "nodes", "kind"), ("edge_relations", "edges", "relation"),
        ("edge_confidence_binding", "edges", "relation, confidence, bind_method, edge_kind"),
    ):
        book.add(name, f"""SELECT {fields}, count(*) AS rows FROM main.{table}
        GROUP BY {fields} ORDER BY rows DESC, {fields}""")

    # Missing and duplicate IDs are checked within paths, never globally.
    for table, identifier in (
        ("nodes", "node_id"), ("edges", "edge_id"), ("spans", "span_id"),
        ("macro_invocations", "site_id"), ("macro_scopes", '"scope".stable_symbol_id'),
    ):
        # A scope is identified by its raw stable_symbol_id AND kind.
        extra = ', "scope".kind' if table == "macro_scopes" else ""
        missing_id = f"{identifier} IS NULL"
        if table == "macro_scopes":
            missing_id += ' OR "scope".kind IS NULL'
        book.add(table + "_id_health", f"""SELECT count(*) AS rows,
          count(*) FILTER (WHERE {missing_id}) AS missing_ids,
          count(*) FILTER (WHERE logical_path IS NULL) AS missing_paths,
          (SELECT count(*) FROM (
            SELECT logical_path, {identifier}{extra} FROM main.{table}
            WHERE {identifier} IS NOT NULL GROUP BY logical_path, {identifier}{extra}
            HAVING count(*) > 1) d) AS duplicate_id_groups
        FROM main.{table}""")
        book.add(table + "_duplicate_ids", f"""SELECT logical_path,
          {identifier} AS local_id{extra}, count(*) AS copies
        FROM main.{table} WHERE {identifier} IS NOT NULL
        GROUP BY logical_path, {identifier}{extra} HAVING count(*) > 1
        ORDER BY copies DESC, logical_path, local_id{extra}""")

    invariant_parts = []

    def invariant(name, sql):
        invariant_parts.append(f"SELECT '{name}' AS invariant, count(*) AS affected_rows FROM ({sql}) bad")

    for table, path_column in (("inventory", "path"), ("sources", "logical_path"),
                               ("facts", "logical_path")):
        invariant(table + "_duplicate_paths", f"""SELECT {path_column}
        FROM main.{table} GROUP BY {path_column} HAVING count(*) > 1""")
    for table in ("sources", "facts") + COLLECTIONS:
        invariant(table + "_without_inventory", f"""SELECT 1 FROM main.{table} t
        WHERE NOT EXISTS (SELECT 1 FROM main.inventory i WHERE i.path = t.logical_path)""")
    invariant("nodes_without_span", """SELECT 1 FROM main.nodes n WHERE NOT EXISTS (
      SELECT 1 FROM main.spans s WHERE s.logical_path = n.logical_path
      AND s.span_id = n.source_span_id)""")
    invariant("edges_without_source_node", """SELECT 1 FROM main.edges e WHERE NOT EXISTS (
      SELECT 1 FROM main.nodes n WHERE n.logical_path = e.logical_path
      AND n.node_id = e.source_node_id)""")
    invariant("edges_with_nonnull_missing_target", """SELECT 1 FROM main.edges e
    WHERE e.target_node_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM main.nodes n WHERE n.logical_path = e.logical_path
      AND n.node_id = e.target_node_id)""")
    book.add("orphan_examples", """SELECT 'node_span' AS reference, n.logical_path,
      n.node_id AS local_id, n.source_span_id AS missing_id
    FROM main.nodes n WHERE NOT EXISTS (SELECT 1 FROM main.spans s
      WHERE s.logical_path = n.logical_path AND s.span_id = n.source_span_id)
    UNION ALL SELECT 'edge_source', e.logical_path, e.edge_id, e.source_node_id
    FROM main.edges e WHERE NOT EXISTS (SELECT 1 FROM main.nodes n
      WHERE n.logical_path = e.logical_path AND n.node_id = e.source_node_id)
    UNION ALL SELECT 'edge_target', e.logical_path, e.edge_id, e.target_node_id
    FROM main.edges e WHERE e.target_node_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM main.nodes n WHERE n.logical_path = e.logical_path AND n.node_id = e.target_node_id)
    ORDER BY reference, logical_path, local_id, missing_id""")
    invariant("spans_without_unique_source_bytes", f"""WITH {SOURCE_CTE}
    SELECT 1 FROM main.spans s LEFT JOIN source_sizes z USING (logical_path)
    WHERE z.source_rows IS DISTINCT FROM 1 OR z.byte_length IS NULL""")
    invalid_spans = f"""WITH {SOURCE_CTE}
    SELECT s.logical_path, s.span_id, s.start_byte, s.end_byte, z.byte_length
    FROM main.spans s LEFT JOIN source_sizes z USING (logical_path)
    WHERE s.start_byte IS NULL OR s.end_byte IS NULL OR s.start_byte > s.end_byte
       OR (z.source_rows = 1 AND (s.start_byte > z.byte_length OR s.end_byte > z.byte_length))"""
    invariant("invalid_byte_spans", invalid_spans)
    book.add("invalid_span_examples", invalid_spans + "\nORDER BY logical_path, span_id, start_byte, end_byte")
    invariant("invalid_line_spans", """SELECT 1 FROM main.spans
    WHERE start_line IS NULL OR end_line IS NULL OR start_line < 1 OR start_line > end_line""")
    # Per-path comparison catches offsetting losses that global totals conceal.
    for table in COLLECTIONS:
        mismatch = f"""WITH raw_counts AS (
          SELECT logical_path, coalesce(sum(array_length(value.{table})), 0) AS raw_rows
          FROM main.facts GROUP BY logical_path
        ), typed AS (SELECT logical_path, count(*) AS typed_rows
          FROM main.{table} GROUP BY logical_path)
        SELECT coalesce(r.logical_path, t.logical_path) AS logical_path,
          coalesce(raw_rows, 0) AS raw_rows, coalesce(typed_rows, 0) AS typed_rows
        FROM raw_counts r FULL OUTER JOIN typed t ON r.logical_path = t.logical_path
        WHERE coalesce(raw_rows, 0) <> coalesce(typed_rows, 0)"""
        invariant(table + "_raw_typed_count_mismatch_paths", mismatch)
        book.add(table + "_raw_typed_count_mismatches", mismatch + "\nORDER BY logical_path")
    book.add("structural_invariants", "\nUNION ALL\n".join(invariant_parts) + "\nORDER BY invariant",
             limit=100)

    # Counts and bounded evidence only: stable_key is not assumed unique.
    stable_duplicates = """SELECT stable_key, count(*) AS copies,
      count(DISTINCT logical_path) AS paths, count(DISTINCT label) AS labels,
      count(DISTINCT kind) AS kinds FROM main.nodes WHERE stable_key IS NOT NULL
    GROUP BY stable_key HAVING count(*) > 1"""
    book.add("stable_key_duplicate_groups", stable_duplicates + "\nORDER BY copies DESC, stable_key")
    book.add("stable_key_duplicate_evidence", f"""WITH duplicate_keys AS ({stable_duplicates})
    SELECT n.logical_path, n.node_id, n.stable_key, n.label, n.kind, n.source_span_id,
      d.copies, d.paths FROM main.nodes n JOIN duplicate_keys d USING (stable_key)
    ORDER BY d.copies DESC, n.stable_key, n.logical_path, n.node_id""")

    file_health = """WITH nc AS (
      SELECT logical_path, count(*) AS nodes,
        count(*) FILTER (WHERE kind = 'file') AS file_nodes
      FROM main.nodes GROUP BY logical_path
    ), ec AS (SELECT logical_path, count(*) AS edges FROM main.edges GROUP BY logical_path),
    fc AS (SELECT logical_path, count(*) AS fact_rows,
      count(*) FILTER (WHERE value IS NOT NULL) AS nonnull_fact_rows
      FROM main.facts GROUP BY logical_path)
    SELECT i.path, i.status, i.area, i.suffix, coalesce(fc.fact_rows, 0) AS fact_rows,
      coalesce(nc.nodes, 0) AS nodes, coalesce(ec.edges, 0) AS edges,
      CASE WHEN fc.fact_rows IS NULL THEN 'no_fact_row'
           WHEN fc.nonnull_fact_rows = 0 THEN 'null_facts'
           WHEN coalesce(nc.nodes, 0) = 0 THEN 'empty_nodes'
           WHEN nc.nodes = nc.file_nodes THEN 'file_only' ELSE 'symbols_present' END AS extraction
    FROM main.inventory i LEFT JOIN fc ON fc.logical_path = i.path
    LEFT JOIN nc ON nc.logical_path = i.path LEFT JOIN ec ON ec.logical_path = i.path"""
    book.add("extraction_by_status_area", f"""WITH health AS ({file_health})
    SELECT status, area, extraction, count(*) AS inventory_rows FROM health
    GROUP BY status, area, extraction ORDER BY inventory_rows DESC, status, area, extraction""")
    book.add("file_only_empty_examples", f"""WITH health AS ({file_health})
    SELECT * FROM health WHERE extraction <> 'symbols_present' ORDER BY path, status, area""")

    call_health = f"""WITH {AREA_CTE}, {NODE_CTE}, call_rows AS (
      SELECT e.*, a.area,
        CASE WHEN a.inventory_rows IS NULL THEN 'missing_inventory'
             WHEN a.area_variants > 1 THEN 'ambiguous_area' ELSE 'inventory_area' END AS area_basis,
        coalesce(n.copies, 0) AS target_copies
      FROM main.edges e LEFT JOIN areas a ON a.path = e.logical_path
      LEFT JOIN node_ids n ON n.logical_path = e.logical_path AND n.node_id = e.target_node_id
      WHERE e.relation = 'calls'
    ) SELECT area, area_basis, count(*) AS calls,
      count(*) FILTER (WHERE target_node_id IS NOT NULL) AS nonnull_target_calls,
      count(*) FILTER (WHERE target_node_id IS NOT NULL AND target_copies = 1) AS resolved_only_calls,
      count(*) FILTER (WHERE target_node_id IS NULL) AS unresolved_calls,
      count(*) FILTER (WHERE target_node_id IS NOT NULL AND target_copies = 0) AS dangling_target_calls,
      count(*) FILTER (WHERE target_copies > 1) AS ambiguous_target_calls,
      count(*) FILTER (WHERE target_node_id IS NOT NULL AND target_copies = 1)::DOUBLE / count(*) AS resolved_only_rate,
      count(*) FILTER (WHERE target_node_id IS NULL)::DOUBLE / count(*) AS unresolved_rate
    FROM call_rows GROUP BY area, area_basis"""
    book.add("call_rates_by_area", call_health + "\nORDER BY calls DESC, area, area_basis")
    book.add("unresolved_call_labels", f"""WITH {AREA_CTE}
    SELECT a.area, e.target_label, e.bind_method, e.confidence, count(*) AS calls
    FROM main.edges e LEFT JOIN areas a ON a.path = e.logical_path
    WHERE e.relation = 'calls' AND e.target_node_id IS NULL
    GROUP BY a.area, e.target_label, e.bind_method, e.confidence
    ORDER BY calls DESC, a.area, e.target_label, e.bind_method, e.confidence""")

    overloads = """SELECT label, count(*) AS definitions, count(DISTINCT logical_path) AS paths
    FROM main.nodes WHERE kind IN ('function', 'method') AND label IS NOT NULL
    GROUP BY label HAVING count(*) > 1"""
    book.add("overloaded_names", overloads + "\nORDER BY definitions DESC, label")
    book.add("overloaded_name_examples", f"""WITH overloaded AS ({overloads})
    SELECT n.logical_path, n.node_id, n.label, n.kind, s.start_byte, s.end_byte,
      o.definitions, o.paths FROM main.nodes n JOIN overloaded o USING (label)
    LEFT JOIN main.spans s ON s.logical_path = n.logical_path AND s.span_id = n.source_span_id
    WHERE n.kind IN ('function', 'method')
    ORDER BY o.definitions DESC, n.label, n.logical_path, n.node_id, s.start_byte, s.end_byte""")
    book.add("exploration_seeds", f"""WITH {SEEDS_CTE}
    SELECT q.*, s.start_byte, s.end_byte, s.start_line, s.end_line FROM seeds q
    LEFT JOIN main.spans s ON s.logical_path = q.logical_path AND s.span_id = q.source_span_id
    ORDER BY q.owner_label, q.method_label, q.logical_path, q.node_id, s.start_byte, s.end_byte""")
    # Semijoins preserve each raw edge once even when seed/node IDs duplicate.
    for direction, endpoint in (("outgoing", "source_node_id"), ("incoming", "target_node_id")):
        selected_edges = f"""WITH {SEEDS_CTE} SELECT e.* FROM main.edges e
        WHERE e.relation = 'calls' AND EXISTS (SELECT 1 FROM seeds q
          WHERE q.logical_path = e.logical_path AND q.node_id = e.{endpoint})"""
        book.add("exploration_" + direction + "_counts", f"""WITH selected AS ({selected_edges})
        SELECT relation, confidence, bind_method, target_node_id IS NOT NULL AS has_target_id,
          count(*) AS edges FROM selected GROUP BY ALL
        ORDER BY edges DESC, relation, confidence, bind_method, has_target_id""")
        book.add("exploration_" + direction + "_examples", f"""WITH selected AS ({selected_edges}),
        labels AS (SELECT logical_path, node_id, count(*) AS copies,
          CASE WHEN count(*) = 1 THEN min(label) END AS node_label
          FROM main.nodes GROUP BY logical_path, node_id)
        SELECT e.logical_path, e.edge_id, e.source_node_id, e.target_node_id,
          left(src.node_label, 500) AS source_label, src.copies AS source_node_matches,
          left(dst.node_label, 500) AS target_node_label, dst.copies AS target_node_matches,
          left(e.target_label, 500) AS recorded_target_label, length(e.target_label) AS target_label_characters,
          e.confidence, e.confidence_score, e.bind_method, e.edge_kind,
          left(e.receiver_text, 500) AS receiver_excerpt,
          length(e.receiver_text) AS receiver_characters,
          left(e.scope_text, 500) AS scope_excerpt, length(e.scope_text) AS scope_characters
        FROM selected e LEFT JOIN labels src
          ON src.logical_path = e.logical_path AND src.node_id = e.source_node_id
        LEFT JOIN labels dst ON dst.logical_path = e.logical_path AND dst.node_id = e.target_node_id
        ORDER BY e.logical_path, e.edge_id, e.source_node_id, e.target_node_id, e.target_label""")

    return {
        "format_version": 1, "status": "audit_executed", "display_limit": limit,
        "expected_ingestion_extension_sha256": APPROVED_EXTENSION_SHA256,
        "extension_provenance_verified": False,
        "definitions": {
            "identity": "IDs are qualified by logical_path; macro scopes use raw scope (stable_symbol_id, kind).",
            "resolved_only": "calls with a non-null target ID matching exactly one node in the same logical_path; no semantic correctness assertion",
            "unresolved": "calls with NULL target_node_id; target_label is evidence only",
            "call_denominator": "all typed edges with relation = 'calls'; unique + null + dangling + ambiguous targets partition it",
            "denominators": "inventory row/byte totals include every status; grouping displays can truncate but full counts remain",
            "invariants": "affected_rows counts offending records except duplicate-path and mismatch checks, which count groups/paths",
            "span_bounds": "byte ranges are half-open; end equal to source byte length is valid; absent/duplicate source rows counted separately",
            "diagnostics": "stable-key duplication, overloaded labels, unresolved calls and file-only extraction are not automatically invalid",
            "exploration": "one-hop recorded call edges around explicit owner/method candidates; zero seeds is an observable result",
            "scope": "structural audit only; no extraction, global binding, parser fallback, semantic accuracy or speed verdict",
        },
        "queries": book.receipts,
    }


def audit_database(database, limit=25):
    """Read a consistent snapshot. Importable by the parent's notebook."""
    if not isinstance(limit, int) or isinstance(limit, bool) or not 1 <= limit <= 100:
        raise ValueError("limit must be an integer in 1..100")
    import duckdb
    import _duckdb

    if duckdb.__version__ != "2.0.0":
        raise ValueError("requires the DuckDB 2.0.0 native2 Python environment")
    database = Path(database).resolve(strict=True)
    with duckdb.connect(str(database), read_only=True,
                        config={"enable_external_access": "false"}) as connection:
        connection.execute("BEGIN TRANSACTION")
        report = _audit(connection, limit)
        connection.execute("ROLLBACK")
    report["database"] = str(database)
    report["runtime"] = {
        "duckdb": duckdb.__version__, "python": sys.version,
        "native_engine_sha256": hashlib.sha256(Path(_duckdb.__file__).read_bytes()).hexdigest(),
        "reader_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "read_only": True, "extension_loaded_by_reader": False,
    }
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new JSON output file; parent directory must exist")
    parser.add_argument("--limit", type=int, default=25, help="display limit per query (1..100)")
    args = parser.parse_args(argv)
    try:
        if args.out.exists():
            raise ValueError("output already exists; choose a new JSON file")
        report = audit_database(args.database, args.limit)
        payload = json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
        with args.out.open("x", encoding="utf-8") as output:
            output.write(payload)
    except Exception as exc:
        print(json.dumps({"status": "audit_failed", "error": str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({"status": "audit_executed", "out": str(args.out),
                      "queries": len(report["queries"])}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
