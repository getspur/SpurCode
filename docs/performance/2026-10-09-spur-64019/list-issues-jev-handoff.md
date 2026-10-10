# Issue-list Jev handoff

## Jev compile and solver handoff — bd-1d2l9

**Jev gate: open.** Model **jev-1.13.0**, weakest confidence **0.89**. The compiler selected **workflow.transition_allowed / verify**, preserving all supplied facts and bindings.

The executed request adds **initial_state_allowed** and **safety_invariant** from the catalog, plus persistence and a 5-second tool budget. Jev confidence is interpretation evidence; the following results come from Z3.

| Scenario | Z3 status | Verification outcome | Persisted result |
|---|---|---|---|
| intended | sat | pass | `sol_68e50056a49749b3` |
| unchanged read full scan | unsat | fail | `sol_e0417cd662a9495f` |
| unrelated change full scan | unsat | fail | `sol_25eba4487eae40cd` |
| relevant change full scan | unsat | fail | `sol_83f4bbb1b2d34fae` |
| publish after race | unsat | fail | `sol_e4c095a449774b22` |
| reuse after feed gap | unsat | fail | `sol_1883678157544ef9` |
| serve stale result | unsat | fail | `sol_cba80d382b054968` |

The intended **15-step** trace passes. Six deliberately changed traces are rejected. Python independently reproduced the catalog transition/initial/safe-state checks against each exact request.

### Implementation contract

| Trigger | Required action | Acceptance evidence |
|---|---|---|
| First read of a supported subscription | Seed its bounded result from indexed SQL | Full-load counter increments once |
| Same revision, same database/filter identity | Reuse the result | Zero issue/label rows read after warmup |
| Proven unrelated change | Retain this result; advance its observed watermark | Zero result refreshes |
| Relevant change with complete history | Load changed IDs and patch affected membership/order | Work counters follow changed rows, not total database size |
| Commit races with an in-flight load | Retry or leave dirty; do not publish as current | Deterministic concurrency test |
| Missing feed range, replacement or incompatible schema | Invalidate and reseed affected subscriptions | Recovery test; no stale reuse |

“Relevant” requires both old and new membership, all projected/filter/order fields, and correct page-boundary handling. The existing graph revision alone is insufficient for descriptions; that private-copy counterexample and the earlier bounded membership proof remain applicable. Comments/audits require additional tracking for hygiene. Timer-driven reconciler work must continue independently.

### Concrete implementation sequence

1. **spur-pm: caller observability.** Carry caller identity and tracing context through submit_read; separate queue, SQL, decode and label times. Use this to establish the live query mix.
2. **spur-pm: status/order index.** Extend the existing index helper; first add a failing query-plan/equivalence regression, then add the index and verify with the Rust-linked SQLite build. Check write overhead and pagination ties.
3. **spur-pm: direct summary projection.** Preserve description, labels and current filter/paging behavior; add differential tests against the existing adapter.
4. **spur-pm: selective subscription updates.** Add a versioned, complete change feed and bounded subscriptions for recurring query shapes. Preserve direct indexed SQL for unsupported/ad hoc queries. Implement the table above with row-load counters and concurrent/external-writer tests.
5. **spur-core: incremental hygiene.** Consume per-issue/comment changes while retaining due timers, leases and retry behavior. Re-profile after each independent change.

Do not apply the sparse-label SQL rewrite globally: the notebook measured a broad-label regression. No performance weights, TTL, cache capacity or global speedup were inferred from these lifecycle checks.

These results verify **declared finite traces**. They do not prove Rust behavior, trigger coverage, multi-row pagination, all concurrent interleavings, unbounded liveness or achieved runtime savings. Production code remains unchanged in this pass.

[Original measured proposal](list-issues-proposal.md) · [Exact Jev input/output and solver requests/receipts](list-issues-jev-compile.json) · [Python cross-check](list-issues-jev-verification.json)


[Executed notebook](../../../spur-64019-performance-20261009.ipynb)
