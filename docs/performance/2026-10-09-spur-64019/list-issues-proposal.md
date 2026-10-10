# Proposal: reduce recurring issue-list work — bd-m6e3r

Recommendation: measure callers, land the status-plus-order index as a small first change, then add a direct summary query and selective incremental reuse for recurring queries. Re-profile each step independently.

## What the notebook establishes

The saved PID 64019 ambient capture contains **208/402 (51.7%)** observations beneath `SqliteStorage::list_issues` among frames excluding known wait leaves. Inclusive total is 209: **193** SQL execution/row fetch, **13** preparation, **2** decoding and **1** known wait. Counts are not calls or CPU percentages. Four beads reader threads appear; their boxed closures do not reveal the submitting async caller.

The read-only probe copied the database into memory at 2026-10-09T06:43:30.046054+00:00: **2331 issues**, including **396 open**. The installed priority/creation index does not prevent the tested status-filtered query from choosing `idx_issues_status` plus a temporary sort.

| Candidate | Existing median | Candidate median | Evidence |
|---|---:|---:|---|
| Status/order index, open LIMIT 200 | 1.796 ms | 0.859 ms | 200 identical rows; temp sort removed |
| Status/order index, all open | 1.906 ms | 1.671 ms | 396 identical rows; temp sort removed |
| Summary projection, all open | 1.906 ms | 0.693 ms | Preserves selected summary fields, including description |

These are 15-round warm Python SQLite 3.53.1 SELECT+fetchall measurements on private copies. They exclude Rust decoding, label loading, read-queue latency and live disk I/O. They do not predict whole-process speedup. Source pin: beads_rust `47b3b39a54b04c7d51d735c032c82e891c2a0db8`.

## Ordered implementation proposal

1. **Expose the real callers.** Carry a caller tag and tracing span through `BeadsDb::submit_read` into its reader job. Record queue wait, SQL/decoding/label duration, row count, query-shape fingerprint and cache/delta counters. Avoid issue bodies and free-text filter values in labels. Distinguish poll, UI refresh, hygiene, plan lookup and user MCP traffic. This closes the largest attribution gap before expensive changes.

2. **Add the status/order index in spur-pm.** Extend the existing connection-open index helper with `issues(status, priority ASC, created_at DESC)`. The tested single-status predicate can then filter and return ordered rows without a temp sort, and LIMIT can stop early. Keep existing indexes needed by other shapes. Verify using the same SQLite build linked by Rust, and measure index size plus insert/update overhead before retaining it. Test ties and pagination: current ordering lacks an ID tie-breaker, so changing the plan may change unspecified tie order. Treat any contract change to stable ordering separately.

3. **Return summaries directly.** Replace full 36-column Issue decoding with a crate-local summary query or a pinned upstream helper, retaining `id,title,description,status,priority,issue_type,assignee` and existing batched labels; derive source/URL as today. Preserve all filter, escaping, closed/deferred/template/tombstone, invalid-input, limit/offset and error behavior. Keep description: loop summaries parse it. Add IDs/exists APIs for callers that need no summaries. Prepared-statement caching can be assessed afterward; only 13 observations were in preparation, versus 193 in execution/fetch.

4. **Reuse recurring results with a complete issue change feed.** Key a bounded subscription/cache by database identity, schema generation and normalized full filter/order/page/projection. On an unchanged revision, reuse the result without reading issue or label rows. Consume durable changed IDs and field masks, load only affected summaries/labels, and evaluate both old and new query membership. Patch the matching ordered set; unaffected query results retain their version. The commit detector should be a conservative guard, not an instruction to reload the entire database after every write.

5. **Make hygiene incremental without disabling timers.** Track issue and comment/audit changes per issue so unchanged issues do not reread all comments on every hygiene sweep. Keep reconciliation timer work (leases, due loops, backoff, etc.) active even when database content is unchanged. The existing reconciler already backs off; simply increasing its interval is not the proposed fix.

## Incremental correctness requirements

- The graph clock alone is insufficient for general issue summaries. A private-copy test changed only description while keeping updated_at/content_hash fixed: the graph revision did not advance. Extend/version a shared change feed or add an issue-read feed that covers all projected, filtered and ordered fields, labels, deletes/tombstones, ID replacement and external writers. Comments need their own coverage for hygiene; comment-only writes need not invalidate a plain issue summary.
- A complete old/new membership test must include candidates outside the previously returned page. An issue can newly enter a filter or cross the page boundary. For paginated results, maintain the relevant ordered membership set or rerun the bounded indexed query when a delta may affect its window. Do not patch only the visible page.
- Read the watermark and delta rows in one coherent transaction; publish a result with its corresponding revision only. Serialize/coalesce identical refreshes, and retry or leave dirty if a concurrent commit invalidates an in-flight refresh.
- A missing change-feed range, unknown mutation, database replacement, schema change or invalid tracking triggers forces a safe reseed of affected subscriptions. Do not use timestamps or MAX(updated_at) as a complete change detector.
- SQLite data-version values are connection-local: do not compare tokens from different reader connections, and account for writes through the same connection. Notifications may wake readers but are not the durable source of truth.
- Bound memory by measured query diversity/result size; arbitrary TTLs are not a correctness mechanism. Scope initial implementation to recurrent supported query shapes; retain direct indexed SQL for ad hoc/unsupported queries.

## Solver results and their limits

Catalog workflow verification passes the declared seven-step read/refresh/invalidate/race trace (`sol_cbd5925023324430`). Publishing clean after a detected race is rejected by the transition rule (`sol_52b17749e1d94604`). These validate the supplied protocol, not every possible runtime interleaving.

The generic typed fallback models one changing candidate and a fixed query. Checking only previous members produces a stale-reuse counterexample: old membership false, new membership true (`sol_d0022380d8f34cfc`). Testing both old and new membership yields **unsat** for stale reuse (`sol_5d17396943d94ca6`), with a separate feasible reuse witness (`sol_a7dd5963a52b462f`). Python independently checked **144** bounded cases: **9** failures for the old-only policy, **0** for the proposed predicate.

This proof assumes complete change coverage and matching database/filter identity. Values range 0–2, absence is -1. It does not prove multi-row sorting/pagination, transaction implementation, SQL equivalence, unbounded liveness or performance.

## Avoid a blanket label-query rewrite

ID-subquery lookup helps selective labels but regresses a broad label in this copy:

| Selectivity | Returned rows | Correlated EXISTS | ID subquery |
|---|---:|---:|---:|
| no_match | 0 | 1.039 ms | 0.007 ms |
| broad | 1427 | 7.969 ms | 20.638 ms |
| sparse | 1 | 1.109 ms | 0.015 ms |

Retain this as a targeted, caller/selectivity-dependent candidate. The zero-match test label is not proof of a frequent live workload. Do not switch every label query to the fastest sparse-case plan.

## Acceptance evidence for implementation

- Differential Rust tests preserve query rows/fields and defined ordering across status/type/label combinations, text escaping, ties, offsets and limits.
- After warmup, unchanged repeated reads load **zero issue/label rows**; one relevant issue edit loads only its changed data; unrelated edits preserve the query result. Vary database size to test work counters rather than fixed timing assertions.
- Test entry/exit from filters, closed/tombstoned/deleted rows, label additions/removals, description-only external SQL edits, same-timestamp updates, concurrent publish races, tracking gaps, process restart, database replacement and schema change.
- Verify no missed comment/audit work and no suppressed time-based reconciliation.
- Repeat native profiling plus caller-tagged measurements under matched workload. Report process CPU, end-to-end p50/p95, SQL and queue time, rows/bytes loaded, write cost and memory. Land one change at a time.

## Source grounding

- [Adapter list and poll](../../../crates/spur-pm/src/beads_crate/issue_tracker.rs): list_issues lines 660–716; poll_with_limit lines 526–624; summary conversion lines 509–523.
- [Reader job submission](../../../crates/spur-pm/src/beads_crate/beads_db.rs): submit_read lines 150–169, reader_loop lines 326–338.
- [Existing index helper](../../../crates/spur-pm/src/beads_crate/sort_index.rs): lines 1–30.
- [Reconciler](../../../crates/spur-core/src/plan/reconciler/mod.rs): hygiene lines 1838–1862; run/backoff lines 1197–1251; configured defaults are not measured live cadence.
- [UI refresh](../../../crates/spur-core/src/orchestrator/pm_bridge.rs): lines 98–158.
- [Graph trigger coverage](../../../crates/spur-pm/src/graph_engine/change_tracking.rs): lines 6–90.
- [Description consumer](../../../crates/spur-core/src/plan/loops/status.rs): load_loop_summaries lines 111–200.

Graph retrieval was indexed at 90ec0c4d4; inspected source paths were checked against the working tree and unchanged. The external package index lacked this pinned revision, so its local Cargo checkout was read and its Git revision verified. Source paths identify possible callers, not measured per-caller frequency.

[Executed notebook](../../../spur-64019-performance-20261009.ipynb) · [Raw profile breakdown](list-issues-analysis.json) · [Query plans](list-issues-query-plans.json) · [SQL benchmark](list-issues-sql-benchmark.json) · [Coverage probe](list-issues-coverage-probe.json) · [Solver requests and receipts](list-issues-solver-receipts.json)
