# Temporal source producer concurrency

Four independent source producers reduced extraction time on the frozen
1,987-commit DuckDB sample from a two-run midpoint of **94.82 s to 37.13 s**
(2.55×). Exact source facts matched. Two fresh end-to-end observations took
**57.18–59.17 s**, with exact native snapshot and edge parity. This supports
parallelizing the existing byte-bounded source batches before changing the
parser kernel.

These are bounded-prefix results from 2026-11-15, issue `bd-tz4k`. The completed
[full-history evaluation](full-temporal-evaluation.md) retains its own scope,
validation and fourteen encoding failures. This experiment adds no new
full-history runtime measurement.

## Controlled source experiment

The native extension, producer implementation, frozen Git heads, complete
batches, byte budget (18,530,894), and row limit (2,048) stayed fixed. The only
experimental variable was one versus four producer processes. Whole batches
were assigned greedily by input bytes; no source task was split or omitted.
Each process used the unchanged DuckDB default of ten threads. Four processes
therefore do not mean a four-thread engine configuration.

Trials ran serially in order **1, 4, 4, 1**, with new application artifacts and
warm OS caches on macOS ARM64. Each timed source interval includes process
launches, source reads, extraction, persistence, receipt verification and small
result collection. Plan partition construction and later equality checks are
outside that interval. There were two observations per configuration, so the
reported midpoint is the median of two values, not a confidence interval.

| Reachable commits | One producer, individual runs | Four producers, individual runs | Midpoint ratio |
| ---: | ---: | ---: | ---: |
| 372 | 16.15 / 15.26 s | 9.58 / 7.98 s | 1.79× |
| 1,987 | 96.56 / 93.07 s | 38.02 / 36.24 s | 2.55× |

| Reachable commits | Median aggregate CPU, one / four | Largest observed RSS envelope, one / four |
| ---: | ---: | ---: |
| 372 | 15.80 / 19.58 s | 350.03 / 721.44 MiB |
| 1,987 | 95.79 / 115.51 s | 549.47 / 1,357.81 MiB |

CPU is summed user plus system time from `wait4`. RSS uses Darwin byte units.
For four producers, the envelope is the sum of individual process maxima; it
is **not simultaneous sampled RSS**. It excludes the small controller process.
Source byte budgets are not RSS caps. Elapsed time improved at a CPU and memory
cost, and no worker-admission or memory-limit default changed.

Every trial retained exactly one row per `(blob_oid, language)`. Exact schema
and bidirectional `EXCEPT ALL` comparisons retained nested symbol and token
order, statuses, binary flags, input bytes and extractor identity. Only the
execution timestamp `parsed_at` was excluded.

| Reachable commits | Distinct source keys | Symbol facts | Successful empty parses | Binary skips |
| ---: | ---: | ---: | ---: | ---: |
| 372 | 3,698 | 83,934 | 303 | 0 |
| 1,987 | 19,604 | 337,699 | 1,953 | 1 |

All comparisons had zero missing or extra facts. Neither sample contains an
encoding or extension error, so these observations do not add malformed-input
coverage. The heads are `93a59ca9809cf9470239432ee789f4e14392cecb` and
`bbfd653e2ccdf03a65aae08d6a19221a6b4a8588`, respectively.

## Fresh end-to-end comparison

Two further 1,987-commit observations regenerated inventory, enriched commit
metadata, source plans, parsed facts, matching pairs and temporal views. The
reference used four CLI parser jobs with a fresh 64 MiB parse cache. The order
was extension then CLI, followed by CLI then extension. Ingestion and validation
were separate timed stages; no timed trials overlapped.

| Observation order | Four-producer extension | Four-job CLI |
| --- | ---: | ---: |
| Extension → CLI | 57.18 s | 103.32 s |
| CLI → extension | 59.17 s | 101.97 s |

| Extension stage | First observation | Reverse-order observation |
| --- | ---: | ---: |
| Git inventory | 1.01 s | 0.84 s |
| Metadata, sizing and planning | 8.53 s | 9.21 s |
| Source extraction and persistence | 37.57 s | 39.04 s |
| Matching, folding and persisted views | 10.00 s | 10.01 s |

Both observations matched native schemas and exact multisets of **550,720
snapshots and 614,805 edges**, preserving duplicates and ordered tokens.
Enriched commit rows also matched exactly. Diagnostics were compared by count
only: 306 in each approach. Separate parity queries took 7.50–7.95 s.

The earlier CLI observation was **63.82 s**, versus 101.97–103.32 s here.
Its user-plus-system total also differs: 170.34 s previously versus
258.77–259.40 s here. The cause was not isolated. Retain all observations;
do not infer a stable 1.8× approach advantage from the fresh pairs. The source
ABBA experiment provides the more direct process-count comparison.

Physical output layouts also differ: the extension persists immutable pair
payloads and views; the CLI writes flattened native Parquet. This is an
approach-level comparison, not an engine-only benchmark. Four source processes
had a summed peak envelope of 1.39–1.41 GB in these fresh pipeline runs; the
later matching stage alone peaked at 1.29–1.35 GB. Native CLI peak RSS was
0.79–0.85 GB. Separate validation peaked at 1.69–1.79 GB. These stages retain
their original settings and do not establish a universal memory bound.

## Profile and next opportunities

A separate 4 kHz samply recording of the unchanged 372-commit producer captured
22.22 CPU seconds. Stacks entering the extension account for 65.28% of recorded
CPU. This is inclusive attribution across threads, covering the native bridge
and extraction; stripped private symbols prevent treating it as isolated AST
parser time. Inclusive module shares overlap. Profiled and unprofiled elapsed
times are separate observations. The 492 thread records are thread lifetimes,
not a simultaneous thread count.

The measured opportunity is to run existing independent source batches in
parallel and feed their separate immutable caches to
`derive_temporal_pairs.py` through its existing multiple-source configuration.
The parser still extracts whole blobs once per distinct blob/language key;
columnar organization and caching avoid repeated temporal work. No parser,
matcher or extension implementation changed in this investigation.

The next scheduling experiment should account for parse cost. On 372 commits,
one indivisible batch holds 18.53 MB of the 29.17 MB input; worker loads were
18.53 / 5.26 / 4.68 / 0.70 MB. On 1,987 commits, loads were nearly equal at
38.19–38.65 MB, yet worker wall times ranged from 18.1 to 37.8 s. Bytes alone
are an imperfect cost proxy. Smaller batches or measured per-language costs
are candidates to test separately, preserving exact ownership and facts.

Planning and matching together still take about 19–20 s. Bulk source staging,
connection reuse and different DuckDB thread counts are further hypotheses;
these measurements do not isolate their benefits. The next change should
measure one of those variables at a time.

The subsequent [cost-aware scheduling experiment](temporal-scheduling-evaluation.md)
compares matched four-producer assignments and records a 21.16% source-stage
reduction with exact fact and graph checks, while retaining its calibration cost.

## Reproduction and evidence

The [compact results](temporal-concurrency-results.json) retain individual
observations, CPU/RSS measurements, exact table checks and SHA-256 bindings to
the local receipts. Detailed controllers and immutable outputs are under
`docs/research/artifacts/2026-11-15-duckdb-temporal-concurrency/`. They use the
frozen binaries and repositories retained by the full-history evaluation;
they are experiment controllers for this workspace, not a general ingest CLI.
Existing experiment output paths deliberately refuse overwrites.

The notebook is
`/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`.
Rust cells 025–031 profile, launch and reap serial experiments, check results,
and compute source ratios directly from retained JSON. They host the frozen
V2 native extension through Python DuckDB 2.0.0; the notebook's managed SQL
engine remains unchanged. Cell 027 records an initial checker failure from an
unfinished singleton result. The corrected checker drains results with
`fetchall`; exact checks then pass without replacing any timing run.

The frozen extension SHA is
`a2a80be9226254e9348c925a439ea8d012a852e07212722a21396e8d7fd58d23`;
the native reference SHA is
`93525626d0569b7974fb6f3885f2ba3558f23addc49d472f72f546df27e2dd26`.
Raw profile, sidecar, resource results, source parity, pipeline parity and
controller hashes are retained separately. Independent read-only review is
recorded on `bd-p8ui`.
