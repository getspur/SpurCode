-- DuckDB 2.0.0; LOAD the host-native spur_graph extension before this script.
-- Caller stages two relations (VARCHAR columns; source may be NULL):
--   staged_sources(blob_oid, logical_path, source, extractor_sha)
--   source_lineage(blob_oid, logical_path, extractor_sha, <caller lineage...>)
-- Lineage may repeat; its other columns must not be named extraction_status.
-- Non-NULL cache keys identify immutable source bytes, logical path and the
-- loaded extractor artifact/version. Keep repository/commit/parent occurrences
-- in source_lineage. Only stage versions implemented by the loaded artifact.
-- Run on a connection without an active transaction, with one cache writer.
-- Legacy COPY destinations are relative to the caller's output directory.
-- Supported coherent app consumption: close the writer, then run
--   python publish_snapshot.py --database facts.duckdb --output snapshots \
--       --version UNIQUE_VERSION --schema main --source-label commit=CALLER_LABEL
-- Read current.json once and use only its versioned paths. Labels are supplied,
-- not independently certified. See publish_snapshot.py for failure semantics.

BEGIN TRANSACTION;

-- This physical staging boundary also rejects contradictory source values
-- (including NULL versus non-NULL) for one key before any extraction occurs.
CREATE OR REPLACE TEMP TABLE file_facts_sources AS
SELECT DISTINCT blob_oid, logical_path, source, extractor_sha FROM staged_sources;

SELECT CASE WHEN EXISTS (
    SELECT 1 FROM file_facts_sources
    WHERE blob_oid IS NULL OR logical_path IS NULL OR extractor_sha IS NULL
) THEN error('staged_sources requires non-NULL blob_oid, logical_path and extractor_sha')
ELSE true END;

SELECT CASE WHEN EXISTS (
    SELECT 1 FROM file_facts_sources
    GROUP BY blob_oid, logical_path, extractor_sha HAVING count(*) > 1
) THEN error('staged_sources has conflicting source values for one cache key')
ELSE true END;

-- Bind the full native type without executing extraction on any input row.
CREATE TABLE IF NOT EXISTS file_facts_cache AS
SELECT blob_oid, logical_path, extractor_sha,
       (spur_graph_file_facts_result(logical_path, source)).facts AS facts,
       NULL::VARCHAR AS status
FROM file_facts_sources WHERE false;

-- Explicit additive migration from the prior five-column cache. Existing rows
-- retain facts/status and receive NULL diagnostic columns: old details are
-- unknown, never inferred or reconstructed by reparsing immutable cache hits.
ALTER TABLE file_facts_cache ADD COLUMN IF NOT EXISTS error_code VARCHAR;
ALTER TABLE file_facts_cache ADD COLUMN IF NOT EXISTS error_message VARCHAR;

CREATE UNIQUE INDEX IF NOT EXISTS file_facts_cache_key
ON file_facts_cache (blob_oid, logical_path, extractor_sha);

-- Materialize the anti-join BEFORE evaluating the native scalar. The path is
-- part of the full-facts identity even for identical bytes and language.
CREATE OR REPLACE TEMP TABLE file_facts_missing AS
SELECT s.* FROM file_facts_sources s
ANTI JOIN file_facts_cache c USING (blob_oid, logical_path, extractor_sha);

-- One diagnostic result per missing key; internal/setup failures remain SQL
-- errors and abort this transaction. All later projections read stored results.
CREATE OR REPLACE TEMP TABLE file_facts_extracted AS
SELECT blob_oid, logical_path, extractor_sha,
       spur_graph_file_facts_result(logical_path, source) AS result
FROM file_facts_missing;

INSERT INTO file_facts_cache
    (blob_oid, logical_path, extractor_sha, facts, status, error_code, error_message)
SELECT blob_oid, logical_path, extractor_sha, result.facts, result.status,
       result.error_code, result.error_message
FROM file_facts_extracted;

-- Capture layout completeness BEFORE creating missing tables. Legacy caches,
-- a dropped checkpoint or any missing child require one cache-driven repair.
CREATE OR REPLACE TEMP TABLE file_facts_child_layout AS
SELECT count(*) <> 6 AS bootstrap
FROM duckdb_tables()
WHERE database_name = current_database() AND schema_name = current_schema()
  AND NOT temporary AND table_name IN (
      'file_facts_child_state', 'file_facts_nodes', 'file_facts_edges',
      'file_facts_spans', 'file_facts_macro_invocations', 'file_facts_macro_scopes'
  );

CREATE TABLE IF NOT EXISTS file_facts_child_state AS
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_cache WHERE false;

-- C = current cache keys, S = checkpoint, M = newly extracted keys.
-- Refresh (C - S) UNION M, or all C during bootstrap; remove S - C.
-- M forces replacement after a supported cache delete/retry even if S has it.
CREATE OR REPLACE TEMP TABLE file_facts_child_refresh AS
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_cache
WHERE (SELECT bootstrap FROM file_facts_child_layout)
UNION
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_cache
ANTI JOIN file_facts_child_state USING (blob_oid, logical_path, extractor_sha)
UNION
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_extracted;

CREATE OR REPLACE TEMP TABLE file_facts_child_removed AS
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_child_state
ANTI JOIN file_facts_cache USING (blob_oid, logical_path, extractor_sha);

CREATE OR REPLACE TEMP TABLE file_facts_child_invalidated AS
SELECT * FROM file_facts_child_refresh
UNION
SELECT * FROM file_facts_child_removed;

-- Select wide payloads once, before the five independent expansions. A replay
-- materializes no facts; cache-hit detection only compares the three key columns.
CREATE OR REPLACE TEMP TABLE file_facts_child_refresh_facts AS
SELECT c.* FROM file_facts_cache c
JOIN file_facts_child_refresh r USING (blob_oid, logical_path, extractor_sha);

-- Bind existing child schemas without replacing intact physical tables.
CREATE TABLE IF NOT EXISTS file_facts_nodes AS
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, node.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.nodes) WITH ORDINALITY AS children(node, fact_ordinal) WHERE false;

CREATE TABLE IF NOT EXISTS file_facts_edges AS
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, edge.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.edges) WITH ORDINALITY AS children(edge, fact_ordinal) WHERE false;

CREATE TABLE IF NOT EXISTS file_facts_spans AS
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, span.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.spans) WITH ORDINALITY AS children(span, fact_ordinal) WHERE false;

CREATE TABLE IF NOT EXISTS file_facts_macro_invocations AS
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, invocation.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.macro_invocations) WITH ORDINALITY AS children(invocation, fact_ordinal) WHERE false;

CREATE TABLE IF NOT EXISTS file_facts_macro_scopes AS
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, macro_scope.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.macro_scopes) WITH ORDINALITY AS children(macro_scope, fact_ordinal) WHERE false;

-- Bootstrap must also clear rows whose keys are absent from a lost checkpoint.
-- These constant predicates skip full deletion on established layouts.
DELETE FROM file_facts_nodes WHERE (SELECT bootstrap FROM file_facts_child_layout);
DELETE FROM file_facts_edges WHERE (SELECT bootstrap FROM file_facts_child_layout);
DELETE FROM file_facts_spans WHERE (SELECT bootstrap FROM file_facts_child_layout);
DELETE FROM file_facts_macro_invocations WHERE (SELECT bootstrap FROM file_facts_child_layout);
DELETE FROM file_facts_macro_scopes WHERE (SELECT bootstrap FROM file_facts_child_layout);

DELETE FROM file_facts_nodes USING file_facts_child_invalidated k
WHERE file_facts_nodes.blob_oid = k.blob_oid
  AND file_facts_nodes.logical_path = k.logical_path
  AND file_facts_nodes.extractor_sha = k.extractor_sha;
DELETE FROM file_facts_edges USING file_facts_child_invalidated k
WHERE file_facts_edges.blob_oid = k.blob_oid
  AND file_facts_edges.logical_path = k.logical_path
  AND file_facts_edges.extractor_sha = k.extractor_sha;
DELETE FROM file_facts_spans USING file_facts_child_invalidated k
WHERE file_facts_spans.blob_oid = k.blob_oid
  AND file_facts_spans.logical_path = k.logical_path
  AND file_facts_spans.extractor_sha = k.extractor_sha;
DELETE FROM file_facts_macro_invocations USING file_facts_child_invalidated k
WHERE file_facts_macro_invocations.blob_oid = k.blob_oid
  AND file_facts_macro_invocations.logical_path = k.logical_path
  AND file_facts_macro_invocations.extractor_sha = k.extractor_sha;
DELETE FROM file_facts_macro_scopes USING file_facts_child_invalidated k
WHERE file_facts_macro_scopes.blob_oid = k.blob_oid
  AND file_facts_macro_scopes.logical_path = k.logical_path
  AND file_facts_macro_scopes.extractor_sha = k.extractor_sha;

-- UNNEST(NULL) and UNNEST([]) emit zero rows. Keep one list per expansion:
-- simultaneous list joins multiply rows. Ordinals retain native list positions.
INSERT INTO file_facts_nodes
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, node.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.nodes) WITH ORDINALITY AS children(node, fact_ordinal);

INSERT INTO file_facts_edges
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, edge.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.edges) WITH ORDINALITY AS children(edge, fact_ordinal);

INSERT INTO file_facts_spans
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, span.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.spans) WITH ORDINALITY AS children(span, fact_ordinal);

INSERT INTO file_facts_macro_invocations
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, invocation.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.macro_invocations) WITH ORDINALITY AS children(invocation, fact_ordinal);

INSERT INTO file_facts_macro_scopes
SELECT blob_oid, logical_path, extractor_sha, fact_ordinal, macro_scope.*
FROM file_facts_child_refresh_facts,
UNNEST(facts.macro_scopes) WITH ORDINALITY AS children(macro_scope, fact_ordinal);

-- Checkpoint all parents, including NULL/error/empty facts with no children.
DELETE FROM file_facts_child_state USING file_facts_child_removed k
WHERE file_facts_child_state.blob_oid = k.blob_oid
  AND file_facts_child_state.logical_path = k.logical_path
  AND file_facts_child_state.extractor_sha = k.extractor_sha;
INSERT INTO file_facts_child_state
SELECT blob_oid, logical_path, extractor_sha FROM file_facts_child_refresh
ANTI JOIN file_facts_child_state USING (blob_oid, logical_path, extractor_sha);

-- Preserve caller multiplicity, root parents (NULL), merge parents and repeats.
-- Lineage keys absent from the cache stay visible with status not_cached.
CREATE OR REPLACE TABLE file_facts_occurrences AS
SELECT l.*, coalesce(c.status, 'not_cached') AS extraction_status
FROM source_lineage l
LEFT JOIN file_facts_cache c USING (blob_oid, logical_path, extractor_sha);

COMMIT;

-- LEGACY NON-ATOMIC direct exports, retained for existing SQL callers. Reruns
-- replace these files after the database commit; failure can leave a mixed set.
-- These files are NOT the supported coherent app consumption path. Use
-- publish_snapshot.py and its atomically advanced current.json instead.
COPY file_facts_cache TO 'file-facts-cache.parquet' (FORMAT PARQUET);
COPY file_facts_nodes TO 'file-facts-nodes.parquet' (FORMAT PARQUET);
COPY file_facts_edges TO 'file-facts-edges.parquet' (FORMAT PARQUET);
COPY file_facts_spans TO 'file-facts-spans.parquet' (FORMAT PARQUET);
COPY file_facts_macro_invocations TO 'file-facts-macro_invocations.parquet' (FORMAT PARQUET);
COPY file_facts_macro_scopes TO 'file-facts-macro_scopes.parquet' (FORMAT PARQUET);
COPY file_facts_occurrences TO 'file-facts-occurrences.parquet' (FORMAT PARQUET);
