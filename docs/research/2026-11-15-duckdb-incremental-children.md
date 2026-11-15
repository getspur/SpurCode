# Incremental DuckDB file-facts children: implementation and notebook benchmark

Incremental child maintenance reduced median cached workflow time from **5.683 s
to 1.630 s (3.49×, 71.3% less elapsed time)** on the frozen 19,604-key historical
source workload. The workflow still refreshes caller lineage and exports all
seven complete Parquet snapshots. The same native Rust extractor is used; the
gain comes from retaining derived rows for unchanged cache keys.

## Implemented behavior

`examples/file_facts.sql` now owns a durable key-only `file_facts_child_state`.
For current cache keys C, prior checkpoint S and newly extracted keys M:

```
refresh = (C - S) UNION M
removed = S - C
```

It removes prior children for both sets, materializes refresh facts once, expands
the five lists independently and checkpoints all cache parents. NULL/error/empty
parents remain represented even when they have no children. A cache deletion and
retry replaces the old children without duplicates. Deletion without restaging
removes children/state and leaves current lineage visible as `not_cached`.

Missing checkpoint or any missing child triggers a cache-driven repair without
reparsing cached sources. Intact no-op layouts preserve physical child tables and
expand zero facts. Caller occurrences refresh independently on every run. Cache,
checkpoint, children and occurrences remain in one transaction; the unchanged
seven full snapshot exports follow commit.

Independent review found and blocked a hardcoded `main` metadata predicate.
Test-first correction now scopes detection to `current_database()` and
`current_schema()`. Regression tests cover every missing collection with another
schema's intact layout and verify that the other schema's rows/OIDs are unchanged.
Follow-up review independently reproduced the correction in an attached catalog.

No Rust, Cargo, native API, extractor, input-provider or partition-publication
change was made.

## Matched experiment

The Rust notebook cell launches native Python/DuckDB **2.0.0**, with four threads
and a 4 GB limit per connection. The notebook's managed SQL engine was unchanged.
The native extension hash is
`e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`.

Both variants start from separate copies of the complete prior cache. The frozen
input contains **19,604 keys / 153,493,818 source bytes**, including 19,603 valid
text inputs and one preserved encoding-error entry. Its source is NULL through
VARCHAR; raw bytes and the diagnostic remain in the existing source artifact and
caller lineage. The input Parquet SHA-256 is
`e1aefc5fcb74bd1975c8a2e88a2acfa85b130321009988be592f23a5263fbef3`.

Old SQL is frozen from `71430cd5b`; corrected SQL is from the reviewed tree shared
by `4cd27e756` and normalized worker `11991f874`. One-time cache migration/bootstrap
is separate from steady-state timings. Warm order is old/new/new/old/old/new.
Verification, preparation and process startup are outside each workflow timer.
Every timed workflow includes full snapshot export.

| Condition | Full rebuild | Incremental | Observed ratio |
|---|---:|---:|---:|
| Warm median, three observations | 5.683 s | 1.630 s | 3.49× |
| Warm range | 5.504–5.800 s | 1.559–1.636 s | — |
| Lineage-only update | 6.907 s | 1.686 s | 4.10× |
| 1% cache eviction/refill, 188 keys | 8.537 s | 3.547 s | 2.41× |
| 10% cache eviction/refill, 1,963 keys | 15.347 s | 11.348 s | 1.35× |
| Reopened cache replay | 9.106 s | 2.178 s | 4.18× |
| Initial cached bootstrap/migration | 5.803 s | 6.754 s | — |

Only warm replay has repeated observations. The other rows are single diagnostic
comparisons. Eviction/refill simulates missing immutable cache keys; it is not an
observed real commit sequence. The lineage probe adds three occurrences, including
a duplicate merge-parent entry; all 19,607 occurrences match the caller relation
with zero extraction and zero child refresh in the incremental version.

This is a shared 10-CPU host. Warm observations started at one-minute load
11.52–12.45; conditions changed later. Ratios describe this run, not isolated
capacity guarantees. Recorded RSS is a process-lifetime high-water mark including
both connections and verification, not per-query memory consumption.

## Remaining cost

A separate cached statement trace, after the refill probes, measured:

| Phase | Full rebuild | Incremental |
|---|---:|---:|
| Child maintenance | 6.098 s | 0.093 s |
| Full Parquet export | 1.610 s | 1.721 s |
| Staging/validation/cache setup | 0.691 s | 0.178 s |
| Empty extraction statement | 0.009 s | 0.002 s |
| Caller lineage | 0.021 s | 0.024 s |

The trace is excluded from warm medians. Its incremental total is 2.019 s, with
**85.3% in full exports**. This supports investigating incremental snapshot
publication next, with an explicit consistency/manifest contract and independent
lineage publication. It does not justify skipping lineage on a fact-cache hit.
Cold native AST parsing was not rebenchmarked in this SQL-only step.

## Verification and provenance

- Integrated real-engine suite: **50 tests passed, zero skips**, including all
  19 workflow tests, independent native-field oracle tests, and existing
  symbol/temporal regressions.
- Both variants' child tables match complete-cache UNNEST references in both
  `EXCEPT ALL` directions, with exact types and list ordinals. Refill, lineage
  and reopened-cache probes pass. Incremental refresh counts are exactly 0,
  188 and 1,963 for the corresponding conditions; removed sets are empty.
- Final old/new cache, five children and caller occurrences match exactly.
  Fact counts remain **357,302 nodes, 1,013,352 edges and 357,302 spans**. This
  workload has no macro rows; nonempty macro collections are covered by tests.
- **14 typed Parquet round trips** pass through fresh connections without
  loading the extension: seven snapshots per variant, exact values and types.
- Catalog PRE/POST receipts accept the valid finite snapshot and reject duplicate,
  orphan and missing-checkpoint snapshots. These are finite witnesses, not a proof
  of arbitrary SQL transitions or transactions.

Original test-first commits are `ebde1ed25` / `8d3f7a285`; R1 regression/fix commits
are `ab6784912` / `4cd27e756`. The active checkout retains that test-before-fix
history and exactly matches the independently reviewed normalized tree. The plan
merge tool reported an empty-file conflict on its read-only review overlay; native
Git integration used the verified identical tree. The premature original staging
attempt was rejected before any merge; only corrected, reviewed code was integrated.

Notebook: `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`.
Rust preflight cell `fea11100-1115-47c0-9abc-111520260063` succeeded as execution 125;
benchmark cell `...0064` succeeded as execution **126**. The synchronous MCP call
hit its 300-second response timeout; subsequent cell readback confirms successful
durable output and a passed child result. The benchmark was not restarted.
Analysis cell `...0065` completed as execution **129** after replacing a borrowed
JSON map with an owned value required by the persistent Rust notebook kernel.

Evidence directory: [artifacts/2026-11-15-duckdb-incremental-children](artifacts/2026-11-15-duckdb-incremental-children).
It contains the harness, frozen SQL, review/solver receipts, integrated test log,
notebook execution receipt, matched result JSON and a SHA-256 manifest. Large
database/Parquet artifacts remain local and are named in the manifest.

Reproduce with a fresh run name:

```sh
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python \
  docs/research/artifacts/2026-11-15-duckdb-incremental-children/benchmark.py \
  --candidate-sql docs/research/artifacts/2026-11-15-duckdb-incremental-children/approved-candidate.sql \
  --name matched-r1
```

This benchmark covers a bounded historical-source workload. It does not complete
the full approximately 98,000-commit DuckDB temporal ingest or establish an overall
DuckDB-versus-native-engine performance ranking.
