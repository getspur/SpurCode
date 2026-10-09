# Incremental beads graph — bd-potu6

GraphEngine now retains graph state across requests and applies committed issue,
label and dependency changes by affected ID. Unchanged requests read only the
small revision/schema metadata and reuse the existing graph. Ordinary edits do
not reload or hash the whole graph.

This implements the follow-up to the [live snapshot investigation](../2026-10-09-snapshot-rebuilds/).
The earlier CPU samples are in the repository-root
`spur-71291-live-performance-20261009.ipynb`. This change concerns the beads issue
graph in `spur-pm`, not the source-code index in `spur-graph`.

## Measured graph work

An isolated SQLite fixture contains 2,003 nodes and two edges. The Rust test emits
cumulative counters; the table below shows additional work for each phase.

| Operation | Full builds | Nodes hydrated | Edge payloads hydrated |
| --- | ---: | ---: | ---: |
| First request | 1 | 2,003 | 2 |
| Ten unchanged requests combined | 0 | 0 | 0 |
| One node title edit, then request | 0 | 1 | 0 |

[work-sample.json](work-sample.json) contains the observations, extracted from
[test-work-sample.log](test-work-sample.log). Remote and wrapper-fallback local
runs produced identical counters. These are graph-maintenance work counts, not
live CPU measurements or a latency speedup claim. Counters track decoded records;
they do not count metadata queries or every SQLite candidate visit. A separate
EXPLAIN QUERY PLAN regression rejects base-table scans in cold label queries,
including repeated empty-label requests.

## Correctness and review

- Namespaced transactional triggers capture relevant writes from independent
  SQLite connections. No-op writes, unrelated comment/config rows and rollbacks
  do not advance the graph revision.
- A coalescing, indexed change table retains the latest node and edge revisions
  per affected ID. Readers do not consume each other's changes.
- Delta reads run in one SQLite transaction. The engine publishes only after
  every affected record has been read successfully. A mutex shares initialization
  and publication between concurrent requests without cloning the whole graph.
- Field edits hydrate only affected nodes. Edge/membership changes refresh their
  incident connections. Dense-node removal repairs the swapped index.
- Label initialization uses indexed ID lookups. Both endpoints must belong to
  the projected graph before edge payloads are decoded, including staged changes.
- A legacy-loader oracle checks normalized nodes and edges across mutation
  sequences. Separate cold/incremental comparisons check fingerprints.

Independent review (bd-2z2ht) found normalization, empty-label scan and scoped
edge-decoding defects. All three were reproduced as failing tests before the
fixes; see [test-review-red.log](test-review-red.log). Re-review found those
production defects resolved. The database-replacement test was also strengthened
to retain the obsolete cache and assert the file-identity recovery path.

## Jev and solver evidence

The post-implementation `jev_compile` response opened its gate using
`jev-1.13.0`, with weakest confidence 0.91, and selected
`workflow.transition_allowed` in verify mode. After catalog inspection, the
executed request also checks initial state and safety.

| Declared six-step trace | Solver status | Outcome | Receipt |
| --- | --- | --- | --- |
| Initialize, reuse, relevant commit, delta refresh, rollback, reuse | SAT | pass | `sol_39e55e0ab1b84607` |
| Same trace ending with a whole-graph scan on an unchanged read | UNSAT | fail | `sol_d7bb274166e844be` |

Exact Jev requests, provenance, solver requests and results are saved in
[solver-pre.json](solver-pre.json) and [solver-post.json](solver-post.json).
The post responses report `cached=true` because the declared model is unchanged.
This checks finite caller-declared traces. It does not prove the Rust code, SQL
trigger coverage, concurrent publication or unbounded safety; the tests and
review provide implementation evidence separately.

## Verification

- Original unchanged-request regression failed on the old loader (adapter read
  count 2 versus expected 1); committed first in `b2203a430`.
- `scripts/spur-cargo test -p spur-pm`: 282 passed, two existing tests ignored;
  [test.log](test.log). This full run preceded the final test-only strengthening.
- Latest 16 incremental tests passed, including the strengthened file-replacement
  test and emitted work sample; [test-work-sample.log](test-work-sample.log).
- `scripts/spur-cargo fmt -p spur-pm -- --check`: passed.
- `SPUR_REMOTE=1 scripts/spur-cargo clippy -p spur-pm --lib -- -D warnings`:
  passed remotely; [clippy-library.log](clippy-library.log).
- Strict all-target lint failed in existing test code and on one new-test missing
  semicolon, which was corrected. [clippy.log](clippy.log) records the failure;
  [lint-baseline.json](lint-baseline.json) verifies all other diagnostic files
  are byte-identical to the pre-change base. No claim that all-target lint passes.

The remote wrapper lost its VM after the focused remote tests had passed, then
automatically fell back locally when both cloud endpoints were unavailable.
This was an infrastructure fallback, not a rerun to bypass a failing test.

## Limits and compatibility

Cold initialization and detected schema/database replacement still build a fresh
view. Nonempty label scopes retain separate graphs and can duplicate memory;
empty scopes are not retained. Change records grow with distinct affected IDs,
including deleted IDs, so lagging independent readers do not lose deletions.

`data_hash` is now an opaque `g2:` fingerprint updated from affected node/edge
records. It intentionally differs from the old sorted whole-graph hash; consumers
should invalidate old fingerprints once. It is a cache fingerprint, not an
authentication primitive. Unused database fields are not validated by retrieval.

Ranking, global analytics and full-result serialization may still traverse the
graph when requested. The running SPUR process was not rebuilt or restarted, so
the original live-process CPU profile has not been remeasured with this change.

The notebook MCP transport was closed during this follow-up. Existing notebook
analysis was preserved; new implementation/solver evidence is recorded here.
