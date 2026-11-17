# SPUR graph DuckDB extension

`spur_graph` exposes SPUR's native source extraction and symbol-change matcher
as seven typed SQL functions, including cell-aware notebook symbols and changes.
The release candidate targets **native DuckDB
exactly 2.0.0**, `C_STRUCT_UNSTABLE`, with single-file resolution. It preserves
unresolved references and evidence; it does not provide compiler semantics or
repository-wide resolution.

The standalone Cargo workspace pins `duckdb-neo = 0.20000.2`, independently of
the application's DuckDB client. See the [v1 release contract](docs/release-contract.md)
for the complete schema, identity, diagnostics, cache/publication rules and
dated acceptance evidence.

## Build and check a fresh candidate

Run from the repository root with Python >=3.11, the pinned Rust toolchain and
the project's remote-build access. Choose a fresh output on a local POSIX
filesystem; the driver refuses an existing directory.

```sh
python3 -m venv .spur/venvs/graph-duckdb-2
export GRAPH_PYTHON="$PWD/.spur/venvs/graph-duckdb-2/bin/python"
"$GRAPH_PYTHON" -m pip install -r crates/spur-graph-duckdb/requirements-test.txt
export RELEASE_RUN="$PWD/.spur/releases/$("$GRAPH_PYTHON" -c 'import uuid; print(uuid.uuid4().hex)')"
mkdir -p "$(dirname "$RELEASE_RUN")"
PYTHONDONTWRITEBYTECODE=1 SPUR_SCCACHE_S3=0 SPUR_NO_LOCAL_FALLBACK=1 \
  "$GRAPH_PYTHON" crates/spur-graph-duckdb/scripts/check_release.py \
  --out "$RELEASE_RUN" --host-native --expect-platform osx_arm64
export SPUR_GRAPH_EXTENSION="$RELEASE_RUN/spur_graph.duckdb_extension"
"$GRAPH_PYTHON" crates/spur-graph-duckdb/scripts/verify_package.py "$SPUR_GRAPH_EXTENSION"
```

This invocation is for macOS ARM64. `--host-native` explicitly permits a
separate local target through `scripts/spur-cargo`: macOS cannot load the
remote Linux library. Rust unit tests remain remote, with local fallback
disabled. A failed remote phase stops acceptance. The driver runs focused
notebook core tests and adapter unit tests, then builds the library
and all native oracles into its own target, packages and probes the exact
artifact, runs every Python suite, and checks source/artifact identities again
after testing. `receipt.json` and raw phase logs are authoritative. A successful
run permits no skipped tests or expected failures.
The core phase sets the wrapper's `SPUR_SYNC_FULL=1` so tracked frozen notebook
fixtures under `docs/` reach the remote compiler. `--rust-tests-mode host`
declares intentional host Rust testing when explicitly authorized; it never
retries a failed remote compiler or assertion as a local success.

The supported distribution route is a locally built, verified **unsigned**
binary plus its adjacent `.json` sidecar. Copy both dereferenced files from the
same immutable package generation. Reverify after copying; SHA-256 checks
integrity against local metadata and does not authenticate a publisher. No
public release, community install, WASM build or signed package is supplied.

## All seven functions

After verifying the package, open a fresh native connection. This example
uses the environment's verified package, without a machine-specific path:

```sh
"$GRAPH_PYTHON" - <<'PYTHON'
import os
from pathlib import Path
import duckdb

package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    print(con.execute('SELECT spur_graph_info()').fetchone()[0])
    print(con.execute("SELECT spur_graph_symbols('src/a.rs', 'fn a() {}')").fetchone()[0])
    print(con.execute("SELECT spur_graph_file_facts('src/a.rs', 'fn a() {}')").fetchone()[0])
    print(con.execute("SELECT spur_graph_file_facts_result('bad.unknown', 'text')").fetchone()[0])
    print(con.execute("SELECT spur_graph_symbol_changes('src/a.rs','src/a.rs','M', "
        "spur_graph_symbols('src/a.rs','fn a() { let x = 1; }'), "
        "spur_graph_symbols('src/a.rs','fn a() { let x = 2; }'))").fetchone()[0])
    import json
    before = json.dumps({'metadata': {}, 'cells': [{'id': 'r', 'cell_type': 'code',
        'metadata': {'spur': {'code_type': 'rust'}},
        'source': ':dep serde = "1"\nfn helper() -> i32 { 1 }\n'}]})
    after = before.replace('{ 1 }', '{ 2 }')
    print(con.execute("SELECT spur_graph_notebook_symbols('n.ipynb', ?)", [before]).fetchone()[0])
    print(con.execute("SELECT spur_graph_notebook_changes('n.ipynb', NULL, 'M', ?, ?)",
                      [before, after]).fetchone()[0])
PYTHON
```

Keep parent rows before unnesting: empty lists are completed extractions, NULL
is distinct, and empty source can contain a file node/span. The diagnostic API
returns a non-NULL parent for NULL or recoverable bad input, with native facts
unchanged on success. Internal/setup/no-tree failures remain SQL errors.

For `.ipynb`, pass the original notebook JSON to the two notebook functions.
The ordinary symbol/change functions reject notebook paths with migration
instructions. Keep each document/cell's status and diagnostics before unnesting
its typed lists. An empty list can mean a complete empty/import-only cell or
partial coverage; inspect its parent. Ranges refer to original decoded cell
UTF-8 bytes, not the JSON wrapper. Leading single-line Rust `:dep` headers are
masked for parsing with byte positions preserved; commands are never executed.
See the [exact notebook v1 contract](docs/release-contract.md#notebook-symbols-and-changes-v1)
for NULL/absence rules, supported preprocessing and identity limits.

After a successful driver run, execute the frozen real-notebook acceptance:

```sh
"$GRAPH_PYTHON" docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release/check_native_notebook.py \
  --release "$RELEASE_RUN" --out "$RELEASE_RUN/notebook-acceptance"
```

It validates the fresh source probe and package/sidecar, all 28 independent
function anchors (17 base, 11 head), 70 source refs, same-document and duplicate
helper no-ops, Rust original coordinates, typed exports and child failure
publication guards. Its exclusively created output retains native documents,
schemas, counts, original-slice evidence and raw child streams.

## Headless Explore/Review quickstart

Use the fresh successful release receipt's independently probed source ID:

```sh
export SPUR_RELEASE_SOURCE_IDENTITY="$("$GRAPH_PYTHON" -c \
  'import json,os; print(json.load(open(os.path.join(os.environ["RELEASE_RUN"],"receipt.json")))["release_trust"]["source_identity"])')"
"$GRAPH_PYTHON" crates/spur-graph-duckdb/examples/app_consumer.py \
  --fixture crates/spur-graph-duckdb/tests/fixtures/app_consumer.json \
  --output "$RELEASE_RUN/headless-demo" --package "$SPUR_GRAPH_EXTENSION" \
  --extractor-id "$SPUR_RELEASE_SOURCE_IDENTITY"
```

The fixture freezes base/head sources and lineage, including invalid/NULL/empty
inputs, same bytes at different paths, unresolved calls/macros and an ambiguous
rename. The parent runs native work in a private child process, checks its exit
and identities, then publishes `headless-demo/current.json`. That manifest
names immutable typed Parquet files for `sources`, `explore`, `review`, seven
cache tables and `notebook_explore`/`notebook_review`, with schemas, counts,
checksums and coverage. Consumer contract v2 retains native notebook contract v1;
historical consumer v1 results remain readable. Use
`examples/app_consumer.py:read_result(output, 'base'|'head')` to reopen and
validate a requested snapshot; do not relabel another snapshot's results.
`read_notebook_source(output, snapshot_id, blob_oid, source_ref, run_id=...)`
validates and returns original cell text from the exact immutable Explore or
Review source context, including pairs with no Explore occurrence.

This is the app-facing data seam, not an app UI or general SQL service. A
frontend should read one manifest and use its named tables and completeness
fields. The managed notebook SQL runtime is separate; native 2.0 extension
loading is not supported in its 1.5.5 runtime. Root owns notebook readback.

## Cache and coherent publication

Stage `staged_sources(blob_oid, logical_path, source, extractor_sha)` plus
`source_lineage` with the three identity columns and caller lineage. The full
facts key is **`(blob_oid, logical_path, extractor_sha)`**. Use an actual source
byte identity and a verified extractor identity consistently. File-local IDs
must always be joined with that full key. Repository, commit and parent
occurrences remain caller columns.

The [cache recipe](docs/release-contract.md#cache-migration-retry-and-coherent-publication)
uses the transactional prefix of [file_facts.sql](examples/file_facts.sql),
closes the writer, and calls [publish_snapshot.py](examples/publish_snapshot.py).
It persists diagnostics, migrates old error columns additively, checkpoints all
statuses, and replays missing keys only. Retry failures by explicitly deleting
their exact cache key and restaging it. Historical missing diagnostics remain
NULL. Incompatible typed schemas need a separate migration or fresh database.

The supported publisher exports all seven tables from one read-only transaction
to a fresh immutable version, then atomically replaces `current.json`. Readers
load that manifest once and verify its paths/checksums. Trailing legacy SQL
`COPY` exports are non-atomic and are not the app publication route.

## Repository and temporal examples

These helpers remain experiments with their own input contracts. Pass explicit
caller-owned repository, catalog, plan and output paths; do not use historical
artifact paths as writable outputs.

- `catalog_git_sources REPO COMMIT OUTPUT.jsonl` uses the native language
  registry. `examples/ingest_plan.py --help` documents frozen-commit ingestion,
  batch checkpointing and `(blob_oid, language, extractor_sha)` symbol caching.
  That symbol-only key does not replace the full-facts path-sensitive key.
- `history_catalog REPO COMMIT OUTPUT` inventories reachable history with root
  and merge-parent comparisons. Git CLI rename limits and author-versus-committer
  time differences remain relevant.
- `temporal_reference REPO OUTPUT JOBS cli|gix` runs the original walker;
  CLI/gix classification must be compared before timing equivalent workloads.
- [Full-history evaluation](examples/full-temporal-evaluation.md),
  [producer concurrency](examples/temporal-concurrency-evaluation.md),
  [scheduling](examples/temporal-scheduling-evaluation.md), and
  [bounded finalization](examples/bounded-temporal-finalization.md) retain the
  historical measurements and restart recipes. The original global temporal
  fold exhausted spill capacity; the bounded helper is the demonstrated
  full-history finalization route.
- [Native retrieval parity](tests/test_retrieval_parity.py) and
  [ingestion measurements](examples/duckdb-ingestion-results.md) describe their
  respective workloads; neither implies compiler resolution or a new release
  performance guarantee.

SQL batches source staging and cache reuse. Each missing input still requires
whole-source AST parsing; input byte budgets do not bound AST, nested-vector,
output or global-fold memory. Ordinary parser/query state is reused per worker
and language; notebook cells keep their own native parser lifecycle. Admission,
backpressure, production supervision, temporal call-edge integration and wider
platform distribution remain separate work.
