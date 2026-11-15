# Cost-aware temporal source scheduling

Assigning whole source batches by prior processing time reduced the measured
source-stage midpoint from **36.30 s to 28.62 s** on the frozen 1,987-commit
DuckDB sample: **21.16% less elapsed time**, or a 1.27× wall-time ratio. Both
arms used four producers. Source facts matched the prior single-producer
result; the candidate's replayed graph matched the native CLI exactly.

This is a same-workload scheduling experiment, issue `bd-1pnb`, following the
[producer concurrency evaluation](temporal-concurrency-evaluation.md). Keep it
as an optional strategy when prior timing data already exists. Obtaining the
calibration afresh cost 96.56 s, so it does not pay for itself on one ingest.
No production default or parser implementation changed.

## Controlled experiment

The only changed variable was assignment of the existing 21 whole batches to
four producer processes. Both arms preserved every batch dictionary and all
19,604 source keys exactly once. Within each worker, batches remained in
ascending original batch-ID order. The baseline assignment exactly matches the
previous byte-based four-worker plans.

Both policies use longest-processing-time-first greedy assignment: sort whole
batches by decreasing weight, then choose the least-loaded worker, with stable
ID ties. Baseline weights are source bytes. Candidate weights are initialized
to checkpoint and successive checkpoint wall deltas from the previous
single-producer `source-1987-w1-r0` run. Those deltas include reading, staging,
extraction, persistence and checkpoint work; they are not isolated parser CPU.

The head stayed `bbfd653e2ccdf03a65aae08d6a19221a6b4a8588`. Native extension,
producer, source byte budget (18,530,894), row limit (2,048), and each process's
DuckDB default of ten threads stayed fixed. Plans and training inputs were
frozen before measurements. Trials ran serially in order **bytes, cost, cost,
bytes**, using fresh application artifacts and warm OS caches on macOS ARM64.

The source timer spans process launches through completion and result
collection. It excludes prior calibration, schedule construction and equality
checks. Two observations per policy provide descriptive midpoints, not a
confidence interval or a general runtime guarantee.

| Assignment weights | Individual source runs | Midpoint wall | Midpoint aggregate CPU | Largest summed worker RSS peaks |
| --- | ---: | ---: | ---: | ---: |
| Bytes | 34.56 / 38.04 s | 36.30 s | 110.12 s | 1,331.05 MiB |
| Prior processing time | 28.86 / 28.37 s | 28.62 s | 112.57 s | 1,486.31 MiB |

Aggregate CPU increased 2.23%. The largest sum of individual RSS maxima
increased 11.66%; this is an upper envelope, **not simultaneous measured RSS**.
It excludes the controller. Individual maximum worker RSS was about 415–422
MiB in the baseline and 411–417 MiB in the candidate. Values use Darwin
`wait4` byte units. Source byte budgets do not impose an RSS cap.

The assignment made completion times more even:

| First observation | Worker 0 | Worker 1 | Worker 2 | Worker 3 |
| --- | ---: | ---: | ---: | ---: |
| Byte-based producer wall | 34.38 s | 28.51 s | 16.56 s | 26.40 s |
| Cost-based producer wall | 27.13 s | 27.88 s | 27.98 s | 28.70 s |

The historical model predicted longest worker processing totals of 32.92 s and
24.80 s. Actual times differ under concurrent execution and include launch and
other overhead. The candidate is a greedy heuristic; no optimality is claimed.

## Exact validation and solver scope

All four source runs matched the prior single-producer result in exact schema
and bidirectional `EXCEPT ALL` comparisons of every field except `parsed_at`.
This preserves ordered symbols and token vectors, statuses, binary flags,
input sizes and extractor identity. Each result contains **19,604 keys and
337,699 symbols**, including **1,953 successful empty parses and one binary
skip**. Neither this sample nor the prior control contains an encoding error.

Matching and folding were replayed from the candidate cache with frozen
existing metadata. The replay matched exact native schemas and multisets of
**550,720 snapshots and 614,805 edges**, with zero missing or extra rows and
duplicates retained. Enriched commits also matched. Diagnostics were compared
by count only: 306 each. Matching/folding took 11.52 s; the separate graph
comparison took 8.87 s. This reuses source facts and metadata and is **not a
fresh end-to-end ingestion measurement**; do not add it to the source timer and
present the sum as one.

Catalog scheduling verification checked each supplied job assignment,
placement eligibility, one-batch-per-worker capacity and a makespan bound:
47 rule bindings per policy returned `sat` / `pass`. Durations were historical
batch costs rounded up to whole seconds. Fixed schedules had makespans of 36
and 28 ticks. These are checks of the encoded historical schedules, not solver
optimization or future runtime bounds. Runtime plan hashes match those frozen
assignments.

The solver's existing 512-artifact persistence quota rejected the first
persisted request. Re-execution with `persist:false` passed; the exact executed
requests, responses and quota error are retained and hash-bound in the results.
No prior evidence was deleted and no persisted solve ID is claimed.

## Decision and reproduction

The measured saving is 7.68 s per source stage. Paying a new 96.56 s calibration
would make calibration plus candidate extraction **125.18 s**, versus 36.30 s
for the control source stage. Hypothetical amortization needs 13 identical cold
ingests at this saving; that arithmetic excludes other phases and is not a
deployment recommendation. Persistent source-fact reuse remains useful on
reruns. Measure a cheap metadata-based cost estimate on unseen inputs before
changing the scheduler default.

This experiment establishes bounded scheduling potential when relevant timing
data already exists. It adds no full-history speed estimate, unseen-history
prediction result, general engine comparison or new malformed-input coverage.
The parser and typed fact format remain unchanged.

The [hash-bound results](temporal-scheduling-results.json) retain individual
trials, assignments, training provenance, executed proofs and exact validation
receipts. Detailed controllers and immutable outputs are under
`docs/research/artifacts/2026-11-15-duckdb-temporal-scheduling/`. These are
workspace-specific experiments using retained frozen binaries and repositories;
existing output paths refuse overwrites.

Rust notebook cells 033–035 launch and reap trials, run validation, and compute
midpoints, saving and amortization directly from retained JSON. Cell 036
explains the results. The notebook is
`/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`.
Rust orchestrates Python DuckDB 2.0.0 hosting the native V2 extension; the
managed notebook SQL engine is unchanged. Independent read-only review is
recorded on `bd-4px7`.
