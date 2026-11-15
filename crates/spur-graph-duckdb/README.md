# SPUR graph DuckDB extension

This native prototype exposes SPUR's symbol extractor and native single-file
graph facts through DuckDB's V2 C API. DuckDB supplies source vectors; the
extension reuses parser/query state per worker and language, and writes typed
primitive, list and struct vectors.

The adapter follows `duckdb/extension-template-rs` at
`a679601f8e133db7546777c6637bc7a6c2d2780e` and pins `duckdb-neo = 0.20000.2`.
It is a standalone Cargo workspace so the application's DuckDB 1.4.4 client
and lockfile remain independent.

## Build, package and test

Run these commands from the SpurCode repository root. This example is for a
host-native macOS build; Linux uses `libspur_graph_duckdb.so`. Package on the
same platform as the native library: the packager reads the runtime's platform.

```sh
python3 -m venv /tmp/spur-graph-duckdb-test
/tmp/spur-graph-duckdb-test/bin/python -m pip install -r crates/spur-graph-duckdb/requirements-test.txt

SPUR_REMOTE=0 CARGO_TARGET_DIR=/tmp/spur-graph-duckdb-target scripts/spur-cargo --workdir crates/spur-graph-duckdb build --release --lib --examples --locked

/tmp/spur-graph-duckdb-test/bin/python crates/spur-graph-duckdb/scripts/package_extension.py \
  --library /tmp/spur-graph-duckdb-target/release/libspur_graph_duckdb.dylib \
  --out /tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension

SPUR_GRAPH_EXTENSION=/tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension \
  SPUR_HISTORY_CATALOG=/tmp/spur-graph-duckdb-target/release/examples/history_catalog \
  SPUR_TEMPORAL_EXAMPLES_DIR=/tmp/spur-graph-duckdb-target/release/examples \
  SPUR_FILE_FACTS_ORACLE=/tmp/spur-graph-duckdb-target/release/examples/reference_file_facts \
  /tmp/spur-graph-duckdb-test/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -v
```

Local compilation here produces the native artifact for the local test host.
The footer is `C_STRUCT_UNSTABLE` and targets **exactly DuckDB 2.0.0**. It does
not promise compatibility with 1.x or later 2.x versions. Loading a local
unsigned artifact requires `allow_unsigned_extensions` in the test connection
or `duckdb -unsigned` in the CLI.

## SQL

```sql
LOAD '/tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension';

SELECT unnest(
  spur_graph_symbols('input.rs', 'pub fn alpha() -> i32 { 1 }'),
  recursive := true
);

CREATE TABLE parsed_blobs AS
SELECT blob_oid, path, spur_graph_symbols(path, source) AS symbols
FROM missing_blobs;

CREATE TABLE symbol_facts AS
SELECT blob_oid, path, unnest(symbols, recursive := true)
FROM parsed_blobs;
```

Retain the `parsed_blobs` parent rows when recording completion: a valid parse
can return an empty symbol list. NULL input returns NULL; unsupported paths
produce a SQL error. Source must be valid UTF-8 VARCHAR. The language registry,
symbol kinds, byte/line ranges, anchors and tokens come directly from
`spur-graph`; repository/path identity remains in the caller's columns.

The SQL tests exercise real loading, extraction, NULL/empty behavior, nested
vectors across chunk boundaries and Parquet persistence. `reference_symbols`
provides an independent direct-Rust oracle for a Git batch frame;
`catalog_git_sources` inventories a frozen Git tree using the SPUR language
registry.

## Typed single-file facts

```sql
spur_graph_file_facts(logical_path VARCHAR, source VARCHAR) -> STRUCT(
    nodes STRUCT(...)[], edges STRUCT(...)[], spans STRUCT(...)[],
    macro_invocations STRUCT(...)[], macro_scopes STRUCT(...)[]
)
```

Each list preserves all native fields, including nullable edge targets,
binding/import/receiver/scope evidence, confidence and nested macro attributes
and resolution candidates. IDs/ranges retain integer precision; confidence
scores use `FLOAT`. Enum labels follow native serialization; payload variants
use explicit typed structs with inactive payloads NULL. Inspect the complete
type with `DESCRIBE SELECT spur_graph_file_facts('input.rs', '')` or the
[independent native parity contract](tests/test_file_facts.py).

The path must be a nonempty relative logical path that does not escape its
root; the file need not exist. Source is UTF-8 `VARCHAR`. NULL in either argument
returns NULL. Invalid/unsupported paths raise SQL errors; `TRY(...)` converts
them to NULL. Empty valid source returns non-NULL native facts and may include
a file node/span. The API performs no file discovery, reads or Git operations.

Extraction and pending-reference resolution cover **one file**. Cross-file
references can remain unresolved, with native evidence retained. Each call
creates fresh local numeric IDs. Join `(blob_oid, logical_path, extractor_sha,
local_id)`; use the same input key with textual macro/symbol IDs. Native AST
extraction still parses a whole source blob for each missing key; SQL batching
does not make parser computation SIMD or replace repository-wide resolution.

[examples/file_facts.sql](examples/file_facts.sql) accepts already staged
`staged_sources(blob_oid VARCHAR, logical_path VARCHAR, source VARCHAR,
extractor_sha VARCHAR)` and a separate `source_lineage` relation containing
those three key columns plus arbitrary caller lineage columns (reserve
`extraction_status`). Cache keys must be non-NULL. Duplicate source rows are
deduplicated; conflicting source values for one key are rejected. The caller
must supply the actual blob identity and loaded extractor version; an artifact
SHA-256 is one suitable `extractor_sha`. Stage only keys for that loaded version.

The safe full-facts cache key is **`(blob_oid, logical_path, extractor_sha)`**.
Identical bytes at different paths can produce different file facts; the
temporal-symbol `(blob_oid, language)` identity below does not apply here. The
script materializes an anti-join of missing keys, stores the typed extraction
result once, then expands each of the five lists into a separate table. DuckDB
`TRY` can internally retry a failed vector row-by-row; there is no exactly-once
native parser-call guarantee on error. `fact_ordinal` retains list position.
For example, resolve edge sources without mixing file-local IDs:

```sql
SELECT e.logical_path, e.edge_id, n.label
FROM file_facts_edges e JOIN file_facts_nodes n
  USING (blob_oid, logical_path, extractor_sha)
WHERE e.source_node_id = n.node_id;
```

`file_facts_cache` retains every parent and its status: `ok`, `empty_source`,
`empty_facts` (all five lists empty), `null_input` or `extraction_error` (a
non-NULL input whose `TRY` result is NULL; no error text is retained).
`file_facts_occurrences` preserves every caller row, including duplicate
commit/parent occurrences and NULL root parents. Absent cache keys get
`not_cached`. Join occurrences to child tables on all three cache-key columns.
Empty lists emit no children; never infer completion from child row counts.

Run this self-contained example after building/packaging above. It stages one
synthetic blob and two parent occurrences, runs the exact SQL twice, and writes
the database plus seven typed Parquet snapshots in `/tmp/spur-file-facts-demo`:

```sh
SPUR_GRAPH_EXTENSION=/tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension \
SPUR_FILE_FACTS_SQL="$PWD/crates/spur-graph-duckdb/examples/file_facts.sql" \
/tmp/spur-graph-duckdb-test/bin/python - <<'PY'
import hashlib, os
from pathlib import Path
import duckdb

extension = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve()
workflow = Path(os.environ['SPUR_FILE_FACTS_SQL']).read_text()
extractor_sha = hashlib.sha256(extension.read_bytes()).hexdigest()
output = Path('/tmp/spur-file-facts-demo')
output.mkdir(exist_ok=True)
os.chdir(output)
with duckdb.connect('facts.duckdb', config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(extension).replace("'", "''") + "'")
    con.execute('''CREATE TEMP TABLE staged_sources AS
        SELECT 'demo-blob'::VARCHAR blob_oid, 'src/demo.rs'::VARCHAR logical_path,
               'fn helper() {} fn entry() { helper(); remote_call(); }'::VARCHAR source,
               ?::VARCHAR extractor_sha''', [extractor_sha])
    con.execute('''CREATE TEMP TABLE source_lineage AS
        SELECT 'demo-repo' repository, 'merge' commit_oid, p.parent_oid,
               blob_oid, logical_path, extractor_sha
        FROM staged_sources CROSS JOIN (VALUES ('p1'), ('p2')) p(parent_oid)''')
    con.execute(workflow)
    print('first run missing:', con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0])
    con.execute(workflow)
    print('second run missing:', con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0])
    print(con.execute('SELECT blob_oid, logical_path, status FROM file_facts_cache').fetchall())
print(output)
PY
```

Use one cache writer and no surrounding transaction. The database transaction
commits before Parquet export; snapshots are replaced on rerun and the file set
is not published atomically. Child tables cover all cached versions; occurrences
reflect the current caller relation. All statuses, including failures/NULL, are
cached: delete the exact cache key explicitly to retry it, then rerun to rebuild
children/snapshots. A changed extractor hash creates new keys. An incompatible
output schema requires an explicit cache migration or fresh database.

Run the focused real-engine workflow checks with the pinned test environment:

```sh
SPUR_GRAPH_EXTENSION=/tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension \
  /tmp/spur-graph-duckdb-test/bin/python -m unittest discover \
  -s crates/spur-graph-duckdb/tests -p test_file_facts_pipeline.py -v
```

## Repository ingestion example

`examples/ingest_plan.py` demonstrates persistent source ingestion around the
native kernel. Generate the catalog with the Rust helper against a frozen Git
commit, then pass a batch plan covering every distinct supported
`(blob_oid, language)` in that catalog:

```sh
/tmp/spur-graph-duckdb-target/release/examples/catalog_git_sources /path/to/repo COMMIT_SHA /tmp/catalog.jsonl

/tmp/spur-graph-duckdb-test/bin/python crates/spur-graph-duckdb/examples/ingest_plan.py \
  --repo /path/to/repo --catalog /tmp/catalog.jsonl --plan /tmp/plan.json \
  --extension /tmp/spur-graph-duckdb-target/spur_graph.duckdb_extension \
  --output /tmp/ingested-repository
```

The plan is a JSON object with `head`, `row_limit` (the runtime's
`duckdb.__standard_vector_size__`), an explicitly chosen `byte_budget`, and
`batches`. Each batch has a unique `batch_id` and `tasks`: catalog rows with
`path`, `blob_oid`, `language`, and `byte_length`. Use the SPUR-generated language
classification; the example validates plan/catalog agreement. At least one
supported task is required. Byte budgets constrain input batches, and do not
bound AST or output memory.

Task metadata and source blobs enter SQL in bulk. One Git batch reader supplies
missing blobs; native parsing still runs a whole-source parser per distinct
blob. Cache keys include blob, language and the packaged extractor hash.
Transactions checkpoint each batch, including successful empty results. Failed
parse rows retain source and status, produce a nonzero exit, and require explicit
cache invalidation to retry with the same extractor. Changed frozen-HEAD catalogs
are rejected. Restarting a valid plan skips cached keys.

The output contains `duckdb-spur-graph.duckdb`, `head-symbols.parquet`,
`head-parse-cache.parquet`, progress and result JSON. Query `head_symbols` for
path-associated facts, `ingestion_errors` for failures, and `batches` for timings.
`symbol_facts` and the cache export cover all cached blobs for the selected
extractor; `head_symbols` is restricted to the selected HEAD. See
[measured DuckDB ingestion](examples/duckdb-ingestion-results.md).

## Temporal history experiments

See the [full-history evaluation](examples/full-temporal-evaluation.md) for
equivalent-workload measurements, coverage and remaining integration limits.
The [producer concurrency follow-up](examples/temporal-concurrency-evaluation.md)
measures one versus four source producers with exact source and graph parity.
The [scheduling experiment](examples/temporal-scheduling-evaluation.md) then
tests assignment by prior batch processing time with four producers in both arms.

`history_catalog REPO COMMIT OUTPUT` inventories every commit reachable from a
frozen commit, including every merge parent and the root comparison. It writes
commit, comparison and file-change JSONL plus a completion summary. The output
directory must be absent or empty. It uses Git CLI rename detection; large
changes can encounter Git's default rename limit. Its `author_time` is author
time, whereas the reference walk's timestamp is committer time.

`temporal_reference REPO OUTPUT JOBS cli|gix` runs the original reachable-history
walker into native temporal shards. Supply a fresh isolated repository with a
frozen `HEAD`; the example does not install a SPUR pointer in that repository.
CLI and gix can produce different rename classifications and commit summaries.
Compare their outputs before treating their timings as equivalent workloads.

The alternative pipeline separates source extraction from temporal matching:

1. Prepare metadata tables `commits`, `comparisons`, `changes`, `parse_tasks`,
   `symbol_pairs` and `change_pairs`. Classify source paths with SPUR's language
   registry. A parse key is `(blob_oid, language)`; both sides of a rename use
   the current path's language, matching the original walker.
2. Run `examples/parse_history_shards.py` with a frozen source repository,
   a complete batch plan, the native extension and a fresh output directory.
   It reads Git blobs in batches and publishes immutable Parquet batches with
   checksummed receipts. Completed empty parses, binary skips and failures
   retain separate parent rows. Restart verifies receipts and skips completed
   keys. See `--help` for the complete invocation and plan contract.
3. Run `examples/derive_temporal_pairs.py --metadata META.duckdb
   --parse-inputs INPUTS.json --extension EXTENSION --output OUTPUT
   --byte-budget BYTES`. It invokes the shared native matcher once per distinct
   file/version pair and joins those results back to actual commits and
   parents. The resulting database exposes `symbol_snapshots`, `temporal_edges`,
   `pair_occurrences` and `diagnostics`.

The original pair runner's final global fold exhausted spill capacity on the
full DuckDB history. Its input byte budget does not bound that fold. The
full-history experiment completed the same matched-pair cache through a
separate bounded finalizer: narrow numeric emissions, then whole-commit
partitions, without repeating extraction or matching. See the evaluation for
the configuration, completion and independent validation receipts; the command
above alone is not a demonstrated full-history recipe.
The [bounded finalization guide](examples/bounded-temporal-finalization.md)
documents the promoted helper, exact input contract, planning and restart
commands. Its regression fixtures run with:

```sh
python -m unittest discover -s crates/spur-graph-duckdb/tests \
  -p test_bounded_temporal_finalize.py -v
```

`INPUTS.json` contains a `sources` array. Each source specifies `cache_glob` and
`expected_extractor_sha`; older caches also need a separate `flags_glob`.
Producers must have disjoint parse keys. Source encoding errors remain explicit
and omit symbol changes for the affected comparisons; file-level touches remain.
`current-status.json` is authoritative for the latest verification attempt.
An earlier `result.json` alone does not establish that a restart succeeded.

The typed SQL seam is
`spur_graph_symbol_changes(path, old_path, kind, old_symbols, new_symbols)`.
It accepts `A`, `M`, `D`, `R` and returns typed changes plus diagnostics.
The caller normalizes Git type changes to `M` and copies to `R`. The matcher
shares the original Rust implementation, including rename and per-parent merge
semantics. Actual commit and parent keys belong to the caller's projection.

These experiments batch SQL staging and reuse content-addressed facts. Native
AST extraction still parses a whole source blob for each distinct parse key.
Input byte budgets do not bound AST, nested-vector or output memory; Parquet
row-group layout also affects cache-read memory and throughput. Production
admission/backpressure policy, temporal call-edge integration, community
distribution and WASM support remain separate integration work.
