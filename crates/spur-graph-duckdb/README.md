# SPUR graph DuckDB extension

This native prototype exposes SPUR's existing symbol extractor as
`spur_graph_symbols(logical_path, source) -> LIST<STRUCT>` through DuckDB's V2
C API. DuckDB supplies source vectors; the extension reuses parser/query state
per worker and language, and writes typed symbol and token vectors.

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

Temporal symbol matching, call edges, production memory/backpressure policy,
community distribution and WASM support are separate integration work.
