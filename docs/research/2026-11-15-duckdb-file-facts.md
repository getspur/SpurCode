# Typed SPUR file facts through DuckDB V2

The extension now exposes `spur_graph_file_facts(logical_path, source)` with all
five native fact collections as typed structs and lists. It reuses SPUR's native
extraction and single-file resolution. SQL handles source deduplication, cache
miss selection, materialization, lineage joins, five independent list expansions
and typed Parquet export. Cache identity is `(blob_oid, logical_path, extractor_sha)`;
numeric IDs are local to that input key.

The four implementation tasks were reviewed and approved, then combined with
current-main extraction optimizations in an isolated worktree. Independent review
found no critical or important issues. Its minor finding about notebook parser
reuse was corrected: notebook inputs cache queries within a notebook and create
parsers per cell; inner state is not retained across notebook rows.

Implementation: [adapter](../../crates/spur-graph-duckdb/src/file_facts.rs),
[SQL workflow](../../crates/spur-graph-duckdb/examples/file_facts.sql),
[API/run instructions](../../crates/spur-graph-duckdb/README.md).

## Rust notebook experiment

Cells 037–044 in `duckdb-temporal-performance.ipynb` use the Rust kernel to launch
the pinned host-native Python DuckDB **2.0.0** runtime. The notebook's managed
DuckDB 1.5.5 runtime remains separate. Input preparation uses one persistent
`git cat-file --batch` process and typed Parquet, with no per-row SQL insertion.

The frozen DuckDB source plan has **3,698 blob/path inputs**, **29,168,895 bytes**,
and seven languages: 3,565 C++, 107 C, 11 Markdown, 7 JSON, 5 Python, 2 SQL and
1 YAML. These are source-plan entries from a bounded 372-commit history, each
with one selected logical path; they do not represent every historical path or
commit/parent occurrence. Frozen head:
`93a59ca9809cf9470239432ee789f4e14392cecb`.

Source preparation took 0.42 seconds. The Parquet input has 29 row groups of at
most 128 rows, in deterministic blob/path order. A 58-input language/size pilot,
including the largest input in each language, matched the independent filesystem
native oracle exactly across all five collections and native list order.

All full runs produced **87,632 nodes, 226,241 edges and 87,632 spans**. This
DuckDB source sample produced zero macro-invocation/scope facts; Rust macro and
nested-evidence behavior is covered separately by the native/SQL contract tests.
Both directions of typed `EXCEPT ALL` found zero differences against the
one-thread baseline in every compared run. All 28 cache/child/occurrence Parquet
round trips retained exact values and DuckDB types in connections without the
extension loaded.

## Measurements and the materialization finding

Each table entry uses two observations; the midpoint is the median of those two.
Direct extraction was ordered 1/4/4/1 threads. The workflow comparison was ordered
generic/candidate/candidate/generic. Runs are sequential with fresh fact caches;
OS source pages are warm. Query timings exclude source preparation, build time,
Python startup, oracle execution and exact output comparisons.

| Same full-facts kernel and inputs | Wall times, seconds | Midpoint |
|---|---:|---:|
| Parquet scan, one thread | 4.728, 4.984 | 4.856 |
| Parquet scan, four threads | 2.169, 2.296 | 2.233 |
| Materialized temporary input, four threads | 4.577, 4.624 | 4.601 |
| Generic SQL workflow, four threads, cold | 5.841, 5.336 | 5.588 |
| Experimental Parquet-source workflow, four threads, cold | 2.743, 2.680 | 2.711 |
| Generic workflow, cache replay | 0.728, 0.634 | 0.681 |
| Experimental workflow, cache replay | 0.661, 0.658 | 0.659 |

Four-thread direct extraction is **2.17×** faster than one-thread extraction on
this input/layout. Measured CPU/wall ratios are about 3.52 versus 1.00, showing
actual concurrent CPU use. Aggregate query CPU rises from 4.73–4.98 to
7.62–8.09 seconds, and process lifetime peak RSS at extraction rises from
343.9–344.5 MB to 455.1–457.9 MB. RSS is a process lifetime peak, not an isolated
operator allocation measurement.

The physical input layout explains a concrete opportunity. `PRAGMA storage_info`
reports **one row group** for the 3,698-row temporary table; the Parquet scan has
29 row groups. The same extraction with four configured threads uses about one
CPU core from the temporary table and 3.52 from Parquet. DuckDB 2 operator profiles
identify the respective scan sources. The profile attributes most direct-query
CPU to the native scalar projection rather than the Parquet reader.

The experimental workflow preserves the materialized missing-key boundary, then
feeds extraction from the original Parquet view using a `SEMI JOIN` on all three
cache-key columns. The generic workflow's source is `file_facts_missing`, whose
small materialized table limits scan parallelism here. This source-only variant
reduces cold full-workflow midpoint from **5.59 to 2.71 seconds**, a **2.06×** gain
on these two observations per variant. It includes cache population, five child
tables, caller occurrences and seven Parquet exports. Both variants extract
**zero rows** on replay and retain identical typed facts.

The generic SQL example remains unchanged: callers can stage arbitrary relations.
The experimental SQL is retained with each candidate run and assumes the known
Parquet-backed `inputs` view and loaded extractor hash. A reusable input-provider
interface would be needed to carry this optimization into the generic example.

This supports preserving partitioned columnar scans across the extraction boundary.
It does not establish an advantage over an equally batched native Rust pipeline,
SIMD AST parsing, full-history throughput, or repository-wide reference resolution.
The native parser still parses a whole source blob per missing key. The 4 GB
DuckDB experiment setting does not bound Rust AST/owned-fact allocations. Cache
writes are serial and Parquet publication follows the database commit without an
atomic multi-file publication protocol.

## Verification and artifact identity

The native artifact was built from integrated commit
`f0218c673ed478f854b64df6223d1817ced3586e`, based on main
`cfccd6129b9ce9ec5e5b448fcba7548245a30424`. Its SHA-256 is
`e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`.
Documentation qualification commit: `d357aad55`.

Fresh verification on that integrated source:

- Remote extension check passed; native committed-tree release build passed.
- All 14 new native extraction API tests passed remotely.
- All 37 real DuckDB tests passed: 6 workflow, 12 native/SQL contract and
  19 symbol/temporal regressions.
- Independent source/API review found no critical or important issue.
- The broad remote library suite passed 588 tests, failed one, and ignored three.
  `text_embedding_service_requires_local_nomic_model_initialization` fails at
  `lance_sections.rs:4900` with `assert!(service.needs_model_init())`. The same
  exact remote test fails identically on the unchanged baseline commit above.
  The full library suite remains red; no local success substitutes for it.

The reviewed branch was fast-forwarded into main after verification. Unrelated
working-tree edits were fingerprinted before integration and preserved.

Measured results and reproducible experiment sources:

- [Summary and physical scan profiles](artifacts/2026-11-15-duckdb-file-facts/frozen-source/summary.json)
- [Input identities](artifacts/2026-11-15-duckdb-file-facts/frozen-source/inputs.json)
- [Preparation](artifacts/2026-11-15-duckdb-file-facts/prepare_frozen_sources.py),
  [runner](artifacts/2026-11-15-duckdb-file-facts/run_frozen_sources.py),
  [analysis and typed export checks](artifacts/2026-11-15-duckdb-file-facts/analyze_frozen_sources.py)
- [Build/test results](artifacts/2026-11-15-duckdb-file-facts/integration/build-results.json),
  [native artifact identity](artifacts/2026-11-15-duckdb-file-facts/integration/artifact-manifest.json)
- [Independent review evidence](artifacts/2026-11-15-duckdb-file-facts/integration/independent-review-audits.json)
- [Frozen artifact hashes](artifacts/2026-11-15-duckdb-file-facts/frozen-source/manifest-sha256.json)

The larger databases, source/fact Parquet files and native binaries stay in the
local artifact directory; their hashes are retained with the results.
