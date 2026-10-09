# Incremental issue reads implementation plan

> **For SPUR orchestrator:** This plan is designed for the beads-backed plan engine (`execute_epic` / `submit_plan`).
> Each task has an issue and explicit dependencies; the brain reviews before downstream dispatch.

**Source spec:** `docs/performance/2026-10-09-spur-64019/list-issues-proposal.md` and `list-issues-jev-handoff.md`
**Formal notebook cells:** `64019000-2026-4010-8009-000000000306`, `64019000-2026-4010-8009-000000000401` (Python interpretations of saved solver receipts)
**Design issues:** `bd-m6e3r`, `bd-1d2l9` (closed); user approved implementation with TDD
**Implementation epic:** `bd-3jz28`

**Goal:** Make repeated issue-list and hygiene work reuse unchanged results and load only relevant changed data.

**Architecture:** Keep the adapter and SQL work in spur-pm. First add the measured status/order index and preserve reader tracing context. A narrow summary reader then becomes the foundation for a separate complete issue/comment change feed and selective recurring-query reuse. A default-compatible advanced-backend API lets spur-core hygiene consume affected IDs while its timer-driven reconciliation continues.

**Tech stack:** Rust 2021, rusqlite/SQLite, Tokio actors/blocking work, tracing, existing beads adapter, catalog solver/Jev, remote `scripts/spur-cargo`.

**DAG:** `S1 || S2 -> S3 -> S4 -> S5 -> S6 -> S7`. Only S1/S2 write disjoint files. All subsequent shared-file work is sequential.

**Routing:** all tasks now use codex. The initial S2 claude-code attempt could not run because of credits; the successful retry and remaining tasks were routed to codex.

**Verification discipline:** Capture a real failing assertion before production changes, commit `test(spur-pm|spur-core): <issue> ...`, implement minimally, run focused checks and commit the fix. No test-only fake counters. Work counters should measure actual production paths. Resolve review findings before approving a task. Build/test only with `scripts/spur-cargo`, formatting local through the wrapper. Do not install/restart the current SPUR process.

**Solver handoff:** `sol_68e50056a49749b3` passes the declared 15-step lifecycle. The six invalid transitions and exact Jev provenance live in `list-issues-jev-compile.json`. `sol_5d17396943d94ca6` proves the encoded old/new membership predicate only within its bounded one-candidate model. Reload receipts with `get_solve_result`; runtime tests remain mandatory. If actual fallback/coverage policies change, re-encode them honestly before claiming a post-solve match.

## S1: Remove status-filtered issue-list temp sort with TDD

**Issue:** `bd-l2xem`
**Depends on:** none
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/sort_index.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Extend the existing index helper with a status/priority/created_at composite index. Keep existing indexes.
SCOPE
Only sort_index.rs and its inline tests; do not change list semantics or ordering.
ACCEPTANCE
A real failing EXPLAIN QUERY PLAN regression for status IN ('open') ORDER BY priority ASC,created_at DESC with and without LIMIT must fail before production DDL changes and pass after. Differential result tests cover multiple statuses, priorities, creation-time ties and limit windows; do not silently impose a new tie-breaker contract. Existing helper remains idempotent and adapter/reader initialization installs the new index. Check write/index footprint cost on a synthetic fixture and report, without claiming whole-process speedup. Run the targeted spur-pm tests using the wrapper.
ROUTING
codex chosen for one-file mechanical index and regression-test work. Output must include unified diff and one-sentence rationale in addition to shared evidence.

## S2: Preserve issue-read caller context and timing across reader jobs

**Issue:** `bd-2zw9b`
**Depends on:** none
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/beads_db.rs`
- `crates/spur-pm/src/beads_crate/adapter.rs`
- `crates/spur-pm/src/beads_crate/issue_tracker.rs`
- `crates/spur-pm/src/beads_crate/metrics.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Make issue-list submissions observable across the boxed async-to-reader boundary, separating queue time, SQL/row conversion and label loading with a stable query-shape tag.
SCOPE
beads_db.rs, adapter.rs, issue_tracker.rs, metrics.rs and narrowly colocated tests.
ACCEPTANCE
A RED test proves parent tracing span/caller attribution is lost across submit_read and passes after propagation. Keep context inside the reader closure and handle reply/error paths; test concurrent readers do not leak spans. Add useful list query-shape fields and rows/timings without titles/body/free-text labels. Existing actor queue/read behavior and error/backpressure semantics remain intact. Reuse existing timing/metrics facilities instead of a second competing instrumentation stack. Do not redesign caches in this task. The higher-level caller's tracing ancestry should be recoverable, with a stable list operation label even when no parent span exists.

## S3: Read issue summaries directly with differential TDD coverage

**Issue:** `bd-22m02`
**Depends on:** `S1` (`bd-l2xem`), `S2` (`bd-2zw9b`)
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/summary_query.rs`
- `crates/spur-pm/src/beads_crate/summary_query_tests.rs`
- `crates/spur-pm/src/beads_crate/mod.rs`
- `crates/spur-pm/src/beads_crate/adapter.rs`
- `crates/spur-pm/src/beads_crate/issue_tracker.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Replace full beads_rust Issue decoding in adapter list_issues with a narrow crate-local summary SQL path, retaining batched labels and description. Provide a connection-based summary_query::list_summaries(&rusqlite::Connection, &IssueFilter) -> anyhow::Result<Vec<IssueSummary>> helper for later feed/cache work.
SCOPE
New summary_query.rs + tests, module wiring, adapter-owned persistent read path and list_issues integration. If the persistent read path needs a small beads_db change, emit scope_drift before proceeding so the brain can authorize it.
ACCEPTANCE
Before implementation add a failing behavioral/work regression showing list_issues unnecessarily decodes a non-summary column (for example malformed unneeded due/created metadata while preserving sort comparability) or reads unneeded data; explain any intentional error-semantic difference before making it. Differential tests against old list implementation cover labels AND, statuses (including invalid fallback), include_closed/deferred, template/tombstone behavior, types, assignee, priority range, escaped title substring, since, offset, None/zero/positive limit and ties. Preserve seven summary columns plus labels, source and URL, including description used by loop summaries. Do not copy an entire database into memory or blanket rewrite correlated label predicates. Avoid opening a new connection for every request; perform blocking SQL away from Tokio async workers. No subscription caching yet. Clearly document any unavoidable legacy edge behavior and do not silently fix unrelated semantics.

## S4: Track durable issue and comment changes for incremental reads

**Issue:** `bd-183xz`
**Depends on:** `S3` (`bd-22m02`)
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/issue_changes.rs`
- `crates/spur-pm/src/beads_crate/issue_changes_tests.rs`
- `crates/spur-pm/src/beads_crate/mod.rs`
- `crates/spur-pm/src/beads_crate/adapter.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Add a versioned durable issue-read change feed with monotonic watermark and changed-ID/domain information, independent from the graph clock's incomplete summary coverage. Expose crate-local transactional feed primitives for S5 and a cursor-friendly issue/comment distinction for S6.
SCOPE
New issue_changes.rs + tests and adapter/module setup. Do not modify existing graph tracking semantics.
ACCEPTANCE
Real RED tests for description-only external SQL edits with unchanged updated_at/content_hash, label add/remove/reparent, issue insert/delete/tombstone, ID changes and REPLACE uniqueness victims, comment insert/update/delete, transaction rollback, schema/trigger drift, connection and database replacement. SQL triggers must work for external connections. Distinguish summary changes from comment-only changes; graph-only dependency edits must not refresh plain summaries. Design coverage/reseed detection for missing/corrupt journal/clock/triggers and give S5 a consistent transaction API that reads watermark+changes together. No timestamp/maximum-updated_at-only invalidation and no cross-connection data_version comparisons. Prefer a compact last-change-per-ID representation with explicit deletion/history completeness handling; do not let a silent prune drop information needed by lagging readers. Check relevant solver contract and post-solve; state precise recovery rules.

**Implementation refinement (2026-10-09):** Pinned beads_rust `add_comment` also updates `issues.updated_at`. Track timestamp-only relevance separately from summary projection/order/membership so ordinary comment writes do not reload payloads for queries without `since`. Queries with `since` must consume the timestamp signal or use direct SQL. Exercise the actual adapter comment API and retain earlier domain revisions for lagging readers. Validate unchanged feeds through metadata/index lookups, without recurring whole-data or whole-journal scans. Recovery tests include feed-object restart plus database replacement, exact trigger restoration after a missed edit, and ignored/failed conflict paths.

## S5: Reuse and incrementally update recurring issue-list results

**Issue:** `bd-2ef6y`
**Depends on:** `S4` (`bd-183xz`)
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/issue_reads.rs`
- `crates/spur-pm/src/beads_crate/issue_reads_tests.rs`
- `crates/spur-pm/src/beads_crate/summary_query.rs`
- `crates/spur-pm/src/beads_crate/adapter.rs`
- `crates/spur-pm/src/beads_crate/issue_tracker.rs`
- `crates/spur-pm/src/beads_crate/mod.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Implement selective reuse of recurring supported issue-list queries using S4's complete durable feed and S3's summary projection.
SCOPE
New issue_reads.rs + work-counter tests and necessary summary/adapter/list wiring. This is the central behavior approved by the user.
ACCEPTANCE
RED behavior tests and production-usable work counters must show: after warmup unchanged reads load zero issue/label rows; a relevant change loads affected IDs/labels only; unrelated edits preserve the result and load no irrelevant full issue payload; both old and new filter membership handled; deleting/closing/removing a label removes old members; newly matching rows enter; description-only edits observed. Handle order/limit/offset/pagination boundaries correctly (maintain ordered matching membership for supported recurring shapes or a clearly justified bounded SQL fallback). Do not rescan entire issue/label tables on every request or every commit. Cache key includes complete normalized filter/order/projection + database identity/schema generation. Coalesce identical in-flight refreshes; transactionally pair published rows with watermark; race must leave dirty/retry, not publish current falsely. Missing coverage/db replacement/schema changes safely reseed affected subscriptions. Bound subscription memory using an explicit configured/derived policy; unsupported/ad hoc requests retain direct indexed SQL. Record full-build/delta/reuse/row-load counters; vary fixture size to verify work scaling. Test concurrent readers and external writers. Preserve all S3 differential behavior and existing graph path. Re-run Jev/solve against actual supported/fallback policy; disclose and resolve mismatches, never claim model proves Rust.

## S6: Make hygiene issue/comment-driven while preserving reconciliation timers

**Issue:** `bd-1prez`
**Depends on:** `S5` (`bd-2ef6y`)
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/advanced.rs`
- `crates/spur-pm/src/beads_crate/beads_advanced.rs`
- `crates/spur-pm/src/beads_crate/issue_changes.rs`
- `crates/spur-core/src/plan/reconciler/mod.rs`
- `crates/spur-core/src/plan/reconciler/tests.rs`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Use issue/comment changes to avoid rescanning unchanged issue comments/audits during run_index_hygiene_sweep without skipping timer-driven reconciler work.
SCOPE
A minimal optional/default-compatible BeadsAdvanced change-cursor API backed by S4, plus reconciler hygiene state and tests. If needed add a focused reconciler/hygiene.rs module instead of expanding mod.rs substantially.
ACCEPTANCE
RED tests show second unchanged hygiene pass rereads comments under old behavior and zero unchanged rereads after. One issue/comment edit processes only the affected eligible issue; membership entering/leaving open set and deletion handled; labels written by hygiene do not lose subsequent changes. Cursor advances only after successful work; failure retries. Startup/history gap and unsupported backends safely perform the existing full hygiene path. Use default trait methods so mocks/other adapters remain compatible. Clock-based lease expiry, due loops, cancellation/backoff and retry tests must keep passing when DB is unchanged. Do not return early from tick_once just because a DB revision is unchanged. Track failed/affected issues so snapshot publication or hygiene writes cannot cause lost work.

## S7: Verify integrated issue-read changes and record measured evidence

**Issue:** `bd-a96om`
**Depends on:** `S6` (`bd-1prez`)
**Worker:** `codex`

**Planned files:**

- `crates/spur-pm/src/beads_crate/issue_reads_tests.rs`
- `crates/spur-core/src/plan/reconciler/tests.rs`
- `docs/performance/2026-10-09-issue-reads-implementation/README.md`

CONTEXT
Approved user request: implement with TDD the measured issue-list proposal and Jev handoff committed at f54376d9d. Read docs/performance/2026-10-09-spur-64019/list-issues-proposal.md and list-issues-jev-handoff.md. Notebook profiling found 208/402 non-wait-classified observations under list_issues, chiefly SQLite execution. These are not invocation counts.
CONSTRAINTS
Work only in your orchestrator-created isolated worktree. Use scripts/spur-cargo, never bare cargo; remote is default and a remote test failure is real. Use RED -> test commit -> GREEN -> implementation commit; capture expected assertion failure, not merely a compilation error. Inspect solve_rule_spec before RED; reload existing solve_id sol_68e50056a49749b3 for the declared lifecycle and sol_5d17396943d94ca6 for bounded old/new membership, then execute relevant post-solve against landed policy. Preserve exact query/filter/description/label/error and external-writer semantics. Do not edit notebooks, restart or install the live binary, modify .cargo/config.toml, or touch unrelated files. Do not close your own issue. Signal any necessary scope/interface change in beads early. No invented TTL/cache cap without explicit existing configuration or solved bound. Do not run broad workspace tests if focused crate checks suffice.
EXPECTED OUTPUT
Unified diff, intent-focused commits, summary paragraph, exact RED/GREEN commands and results, test-plan bullets, public/internal interface additions, solve IDs and remaining limitations.
GOAL
Validate the combined implementation against every approved contract clause, adding regression tests via TDD for any gaps and saving reproducible evidence.
SCOPE
Focused acceptance tests and docs/performance/2026-10-09-issue-reads-implementation evidence. Fixes outside those files require a scoped signal and brain coordination.
ACCEPTANCE
Run all spur-pm tests and relevant spur-core reconciler tests, strict production-library clippy for touched crates if existing baseline permits, and formatting checks via scripts/spur-cargo. Record exact commands/exit statuses, RED/GREEN commit chain, work-counter results across small/large fixtures, external-writer/race/recovery/paging checks, and matched Rust query benchmarks including index write cost. Do not claim live PID improvement without a newly built binary actually running. Prepare a requirements matrix linking implementation and test evidence; report any unsupported shapes honestly. Re-run applicable persisted solver inputs adapted to implemented policy. All severe review findings must be fixed before final acceptance. Do not change or execute Notebook MCP from this worker; the brain owns notebook follow-up.

## Review and final integration

Review the diff, RED/GREEN evidence and beads completion audit for every task. Use request_changes for substantive findings. After all tasks pass, merge the approved plan to its dedicated integration branch, run the relevant integrated checks, obtain independent code review, and integrate onto the user's branch without staging their unrelated edits. Close the epic only when the complete approved scope is verified.

The notebook's earlier live measurements are baselines, not measurements of this implementation. New Rust fixture work counts and benchmarks must be labeled separately. A live process resample requires the new binary to be running; no fabricated before/after speedup.

## Requirements audit

- Caller identity/queue and query attribution: S2.
- Status/order query plan and write cost: S1, S7.
- Full summary/filter/description semantics: S3, S7.
- Durable external-writer, deletion and comment coverage: S4.
- Unchanged/irrelevant reuse and affected-ID updates, pagination, bounded memory, coherent publication, gap/replacement recovery: S5.
- Per-issue/comment hygiene and retained timers: S6.
- Integrated checks, source-grounded solver POST and measured evidence: S7 plus brain review/integration.
