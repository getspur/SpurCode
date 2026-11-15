# Larger DuckDB file-facts notebook benchmark

The integrated V2 function successfully processed the larger frozen source plan
through actual Rust notebook executions. Multi-threading and cache reuse work;
cold extraction remains substantial. The observed one-to-four-thread median
ratio is **2.28×**. Full-workflow timings vary widely
on this busy development host, so they are observations rather than an isolated
capacity guarantee or proof that DuckDB outperforms an equally batched Rust pipeline.

## Version and workload

Main snapshot: `337345bf84aa30939344983b9edd5d0cc93d8c36`. Runtime code matches native build
`f0218c673ed478f854b64df6223d1817ced3586e`; only README differs in those crate trees.
The immutable loaded extension SHA-256 is
`e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`. No production code was changed for these experiments.

Rust cells 046 onward in `duckdb-temporal-performance.ipynb` launch the pinned
native Python DuckDB **2.0.0** runtime. The notebook's managed SQL runtime remains
separate. Source staging uses one persistent Git batch process and typed Parquet,
with no per-row SQL insertion. Source preparation took **0.990s**.

The plan covers **19,604 blob/path entries, 153,493,818 bytes**
from the frozen 1,987-commit DuckDB prefix at `bbfd653e2ccdf03a65aae08d6a19221a6b4a8588`.
The source Parquet file has **154 row groups** of at most 128 rows.
This is a historical-source plan with one selected logical path per old blob/language
entry, not every historical path occurrence or full temporal reconstruction.

One historical `.hpp` fixture contains invalid UTF-8. Every key is retained;
**19,603 valid UTF-8 inputs (151,671,080 bytes)**
can be supplied to the VARCHAR API. The rejected source has a NULL source value,
explicit `source_status=encoding_error`, an error message and its original bytes
in a sparse BLOB column. Its raw bytes were read back from Parquet and verified
against its Git blob ID. Caller lineage retains the encoding diagnostic; the
generic cache labels this NULL input `null_input`. No lossy decoding was used.

The expanded pilot independently matched all five native collections for
**31 valid text inputs**, plus checked the rejected input's
NULL behavior. Full runs produce **357,302 nodes, 1,013,352 edges and
357,302 spans**, with zero native extraction errors for valid inputs.
The one NULL fact result belongs to the encoding-rejected input. This DuckDB
corpus has no Rust macro facts; macro fields retain typed empty lists, and their
nonempty behavior was covered by the prior native/SQL contract.

## Controlled direct extraction

Each thread count has three observations. The balanced order was
`1/2/4, 4/1/2, 2/4/1`. Every run used the same input, native artifact, typed full-facts
kernel and output fields, with a fresh database/fact cache and warm OS pages.
Query timing excludes preparation, Python startup, export, oracle and parity checks.

| Threads | Observations | Median seconds | Observed range |
|---|---:|---:|---:|
| 1 | 3 | 128.668 | 121.336–132.210 |
| 2 | 3 | 67.649 | 62.815–73.318 |
| 4 | 3 | 56.397 | 47.618–57.660 |

These are shared-host measurements. Load snapshots and later per-run load averages
were recorded; substantial contention and heterogeneous core scheduling can affect
wall time and CPU throughput. Ranges are min/max observations, not confidence bounds.
Process RSS is a lifetime peak, including work earlier in that process, and must
not be interpreted as an isolated phase allocation measurement. The 4 GB DuckDB
setting does not bound native Rust AST/fact allocations.

## Full workflow and durable cache

The generic workflow is the committed example. The Parquet variant preserves
the materialized missing-key boundary but semi-joins it into the Parquet-backed
staged source view before calling the scalar. The whole workload includes cache
population, five separate child tables, caller occurrences and seven Parquet exports.
Variants ran in `generic/parquet/parquet/generic` order, with two observations each.

| Workflow | Observations | Cold midpoint seconds (range) | Same-connection replay midpoint | Reopened-cache replay midpoint |
|---|---:|---:|---:|---:|
| generic | 2 | 111.683 (101.354–122.013) | 8.565 | 6.725 |
| parquet | 2 | 40.994 (21.215–60.773) | 7.723 | 5.743 |

The observed midpoint ratio is **2.72×**, with wide variation;
the full ranges are more informative than a single favorable pair. Both replay
forms materialize **zero missing/extracted rows**, including after database reopen.
The experimental variant is specific to the known Parquet input view and does not
change the generic example's arbitrary-relation interface.

## Incremental probes

These are single diagnostic observations per variant and miss fraction, not
repeated performance estimates. Deterministic hash buckets evict cache keys from
a copied complete cache; input content and caller lineage stay unchanged. The
observed key counts can differ from exactly 1%/10%. Every refresh reconstructs the
same full typed cache, child tables and exports, then survives a zero-miss restart.

| Requested bucket fraction | Variant | Missing-key results | Refresh seconds | Reopened replay seconds |
|---|---|---:|---:|---:|
| 1% | generic | 188 | 11.084 | 8.952 |
| 10% | generic | 1963 | 16.415 | 3.302 |
| 1% | parquet | 188 | 10.791 | 8.846 |
| 10% | parquet | 1963 | 15.644 | 8.502 |

This confirms that cache misses control native input work. At small miss counts,
rebuilding/exporting the complete cached dataset leaves a substantial fixed cost.
`extracted` counts materialized missing-key results; it is not an exactly-once
parser-call guarantee under DuckDB TRY retries.

## Where cached replay spends time

One additional complete-cache diagnostic executes the real SQL statement by
statement, with zero extraction, then validates the same typed outputs. It is
excluded from the cold timing medians; Python dispatch adds some diagnostic overhead.

| Phase | Accumulated seconds |
|---|---:|
| staging_validation_cache_setup | 0.116 |
| extraction | 0.001 |
| child_tables | 2.655 |
| lineage | 0.012 |
| parquet_export | 0.875 |

Child-table rebuilds account for **72.5%**, and Parquet export for
**23.9%**, of that diagnostic's accumulated statement time. These are
concrete next targets: incrementally maintain unchanged
fact/child tables and publish only changed fact partitions, while refreshing caller
lineage independently. A zero fact-cache miss count does not imply unchanged
commit/parent occurrence lineage, so skipping the whole workflow would lose data.
No such production optimization is implemented by this benchmark.

## Profiles, correctness and limits

The first serial operator profile attributes 0.246s to Parquet scan and 130.395s
elapsed worker time to the scalar projection, with zero temporary-file spill.
Projection includes native extraction and typed result construction; it does not
isolate parsing from conversion. Operator elapsed times are distinct from OS
`getrusage` CPU time. A separate cold four-thread sample was captured successfully
and excluded from timing medians. The release binary is stripped and `atos` did
not recover individual Rust names, so no per-function hotspot is claimed.

Every full/partial result matched the reference in both directions of typed
`EXCEPT ALL`, including list order and NULLs. **63 cache/child/lineage
Parquet round trips preserved values and DuckDB types in connections without
loading the extension. Every same-connection and reopened-cache replay processed
zero misses. All source bytes, input keys and artifacts retain version/hash lineage.

There were two operational observations: the first preparation attempt correctly
failed strict UTF-8 decoding before diagnostics were added; its logs remain under
`attempts/`. One notebook MCP call timed out at 300s while its cell subsequently
finished successfully; saved process results and cell readback recovered completion.
Claims here rely on child exits, saved result status and parity checks, rather than
transport status alone. No user processes were stopped to quiet the shared host.

Evidence: [summary](artifacts/2026-11-15-duckdb-file-facts-scale/summary.json),
[protocol](artifacts/2026-11-15-duckdb-file-facts-scale/protocol.json),
[runner](artifacts/2026-11-15-duckdb-file-facts-scale/benchmark.py),
[inputs](artifacts/2026-11-15-duckdb-file-facts-scale/inputs.json),
[cached trace](artifacts/2026-11-15-duckdb-file-facts-scale/cached-trace/result.json),
[profile assessment](artifacts/2026-11-15-duckdb-file-facts-scale/profile-control/assessment.json),
[artifact hashes](artifacts/2026-11-15-duckdb-file-facts-scale/manifest-sha256.json).
Large databases, source/fact Parquet and raw profile output remain local with hashes.
Reproduction requires the persisted frozen source plan/repository and pinned native
runtime/artifact named in the protocol, plus fresh output run names.
