# Typed file extraction through DuckDB V2

Issue: `bd-ns7b`. User requests running `crates/spur-graph/src/extract/` as
DuckDB functions in a columnar ingestion pipeline. Existing symbols/matching
functions remain useful; this feature exposes the broader static file facts.

## API and semantics

Add `BytesExtractor::extract_file_facts(logical_path, bytes)` in `spur-graph`.
It uses the existing configured parser and native graph-fact extraction and
pending-edge resolution for one file. It returns all five `GraphFacts`
collections. No filesystem discovery, file reads, canonicalization, Git, or
embeddings occur in this API. It accepts a nonempty UTF-8 relative logical file
path, rejecting absolute/escaping paths. A logical file need not exist.
Implementation may normalize harmless current-directory components lexically.
Do not change existing build_facts or temporal-symbol behavior.

Add scalar SQL function:

```sql
spur_graph_file_facts(logical_path VARCHAR, source VARCHAR) -> STRUCT(
  nodes LIST<STRUCT<...>>,
  edges LIST<STRUCT<...>>,
  spans LIST<STRUCT<...>>,
  macro_invocations LIST<STRUCT<...>>,
  macro_scopes LIST<STRUCT<...>>
)
```

All existing fact fields must be represented with typed primitive/list/struct
columns, including nullable targets, import/receiver/scope evidence, confidence,
binding provenance, macro attributes and resolution candidates. No JSON or
opaque serialized fact payloads. Enum labels use native serialized semantics;
payload-bearing variants use explicit typed structs. Field mappings that differ
from native serialization must have one documented deterministic normalization
in the oracle test. Numeric IDs/ranges must retain their values and precision.

Each query worker reuses a parser per language across input rows/chunks. Each
file gets a fresh native fact builder: IDs are local to that input row, not
query-global. Callers join on `(input_key, local_id)` and retain repository/blob/
path/version lineage as ordinary columns. Unsupported languages and invalid
paths are SQL errors; NULL argument yields NULL. Empty valid source is a
successful non-NULL native fact result, which may contain a file node/span.
Notebook/Markdown/structured-data behavior follows the native extractor.

This is **file-local extraction**. A reference needing another file can remain
unresolved, with native evidence retained. Do not claim equivalence to a whole
repository's cross-file resolver or implement a weaker resolver under that name.
Native one-file behavior is the parity contract. No invented language/parser
semantics or generated-ICU shortcuts in the full graph path.

## Columnar workflow

The SQL example reads an already staged source relation and:

1. Selects missing distinct source keys before invoking extraction.
2. Materializes the typed function result once per key.
3. Keeps parent rows/status even when child lists are empty or extraction fails.
4. Uses UNNEST into separate node/edge/span/evidence tables and writes Parquet.
5. Uses an anti-join for incremental reuse; rerunning avoids cached keys.

Unlike temporal symbols, full graph facts depend on logical path. The safe cache
identity includes `(blob_oid, logical_path, extractor_sha)`; `(blob_oid,
language)` alone is insufficient. Repository/commit/parent lineage remains
separate and must not disappear through DISTINCT on emitted occurrences.
No filesystem ingestion or Python per-commit/line loop belongs in the kernel.

## Verification

Test first, then implement. Add an independent direct-native oracle example
that writes supplied fixtures to an isolated temporary root and invokes existing
`build_facts_for_paths` for one file, instead of calling the new pure method.
Serialize oracle output outside SQL for test comparison only. Exact comparison
covers all five fact collections and nullable/nested fields, with deterministic
native enum normalization when needed. Include Rust with nested macro attributes
and a local macro definition, C++, Python, Markdown, structured/OpenAPI data,
notebook input, empty, NULL and invalid paths. Exercise constant and selected
vectors, repeated inputs and more than the runtime vector size, Parquet round
trip and TRY over mixed good/bad inputs. Test one-file unresolved-reference
scope and repeated-call ID isolation. Preserve prior symbols/matcher tests.

Use `scripts/spur-cargo` for every Rust build/test/fmt. Normal checks use remote
default. Host-native extension packaging/runtime tests require an explicitly
local macOS build (`SPUR_REMOTE=0`), independently of any remote check failure;
never rerun a remote failure locally to call it passing. Pin DuckDB 2.0.0 and
duckdb-neo 0.20000.2 as existing adapter does. Keep nested vector capacities valid
before writing lists; all FFI/schema changes require real engine execution.

The Rust notebook must execute the new function on a bounded frozen DuckDB
project source sample and demonstrate typed materialization, flattening,
Parquet and cache reuse. If comparing set-based SQL with per-row SQL, hold input,
function, threads and outputs fixed; use fresh application outputs and serial
measurements. Report native oracle equality separately from performance. More
output categories are more work than symbols-only extraction: do not compare
their elapsed time as equivalent workloads or claim that wrapping a parser
makes AST computation SIMD. No full-history or general engine speed claim.

## Scope and integration

Expected changes: minimal public extraction seam/tests in `spur-graph`; adapter
module/registration, SQL tests, oracle, SQL example and docs in
`spur-graph-duckdb`. The user's dirty `macro_invocations.rs` is explicitly out of
scope. Build/review from isolated committed worktrees and preserve those bytes.
Independent review must check memory ownership, nested capacities, per-worker
state, precise scope, native parity and cache identity before final approval.
