# Incremental beads graph implementation plan

> For SPUR orchestrator: this plan is designed for submit_plan(persist_as_epic=true).
> Execution here is direct, sequential work in the isolated bd-potu6 worktree; no worker dispatch is required.

Source spec: docs/superpowers/specs/2026-10-09-incremental-beads-graph-design.md
Tracking issue: bd-potu6. S1, S2 and S3 implemented and verified; final results in docs/performance/2026-10-09-incremental-graph/README.md.
Goal: maintain graph scopes from relevant committed deltas without reloading or scanning all graph data on ordinary reads or small edits.
Architecture: transactional SQLite change records plus one retained graph per queried scope, coherent delta reads, serialized publication, incremental fingerprints.
Tech stack: existing Rust, rusqlite, petgraph, chrono, sha2, tokio.

## S1: Regression and change tracking
Depends on: none.
Files: graph_engine/mod.rs tests; graph_engine/change_tracking.rs (new).
Scope: spur-pm only; no live database changes.
Write and run unchanged-request regression first. Install transactional relevant-column triggers with coalesced node/edge revisions and indexed cursor queries. Cover no-op writes, external writes, rollback, deletion, and independent readers. Commit failing regression before implementation.
Acceptance: repeated retrieval demonstrably fails on the old full-loader path; all relevant changes are recorded, irrelevant writes are not.

## S2: Incremental graph store
Depends on: S1.
Files: graph_engine/incremental.rs (new); graph_engine/incremental_tests.rs (new).
Add coherent staged loading, per-scope cursors, targeted node/edge patching, swapped-index repair and versioned record fingerprints. Preserve fresh report time and return errors without publishing partial state. Expose work counters used by diagnostics and validation.
Acceptance: structural parity against full loading; one-node mutation loads one node, not the whole graph; no full hash pass on a cache hit or ordinary delta; scope membership and deletion correct; recovery explicit.

## S3: Facade integration and verification
Depends on: S2.
Files: graph_engine/mod.rs; tests and design evidence.
Route all six GraphEngine entrypoints through the retained store on the blocking pool. Share one mutex for concurrent requests, keeping synchronous SQLite off async runtime threads. Run focused tests, spur-pm library/integration tests and targeted lint. Replay solver fixtures, record actual work counts and limitations, and review the final diff before committing the implementation.
Acceptance: unchanged reads have zero graph data loads; concurrent cold requests build once; external mutations appear at a coherent read point; all existing facade behavior tests remain green.

Commands:
- scripts/spur-cargo test -p spur-pm --lib unchanged_graph_requests_reuse_materialization -- --nocapture
- scripts/spur-cargo test -p spur-pm --lib graph_engine
- scripts/spur-cargo test -p spur-pm
- SPUR_REMOTE=1 scripts/spur-cargo clippy -p spur-pm --all-targets -- -D warnings

Verification outcome: full crate suite 282 passed, two existing ignored; 16 final incremental tests passed; formatting and strict production-library Clippy passed. All-target Clippy exposes existing test lint debt, recorded with a baseline source comparison; its single new-test semicolon finding was corrected. Independent review bd-2z2ht has no remaining findings and accepts the scoped change with the existing lint limitation disclosed. PRE/POST solver receipts and measured graph work are saved alongside the result report.
