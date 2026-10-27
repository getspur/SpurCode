# Beads Automatic Maintenance Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-10-27-beads-automatic-maintenance-design.md`
**Formal @spec cells:** none
**Design epic:** `bd-2md` (closed)
**Implementation issue:** `bd-28k`
**Goal:** Bound redundant beads storage and keep exports fresh automatically.
**Architecture:** Safe history planning/execution runs on the existing writer actor under a nonblocking flock. Monotonic deadlines schedule flush/sweep work; an atomic shared timestamp throttles directory sweeps. Canonical issue data and quarantine remain preserved.
**Tech stack:** Rust, rusqlite, beads_rust, tempfile, Notebook MCP, Z3.

Execution is already authorized. The current agent executes the following beads-backed DAG sequentially; no worker dispatch or additional approval is required.

## Task 1: Retention policy and runtime regression tests

**Task ID / beads:** `retention` / `bd-2b6`
**Depends on:** none
**Suggested Worker:** current Codex agent
**Files:** create `crates/spur-pm/src/beads_crate/maintenance.rs`, `crates/spur-pm/src/beads_crate/maintenance/history.rs`, `crates/spur-pm/tests/beads_maintenance.rs`; wire `beads_crate/mod.rs`.
**Scope:** typed policy, report, strict history scan, oldest-first budget plan, execution identity checks; no canonical-data removal.

Pre-solve: `sol_f3b6ef78501d4fdb` (defaults sat), `sol_1dc164b074404398` (catalog capacity pass), `sol_5043eab2920b428f` (four-entry retention violation search unsat). Full requests and limitations are in `docs/research/2026-10-27-beads-maintenance-solves.json`.

Write real-adapter regressions before production edits:

```rust
let adapter = BeadsCrateAdapter::open(dir.path(), AdapterConfig::default()).await?;
adapter.create_issue(IssueCreate { title: "scheduled export".into(), ..Default::default() }).await?;
// Poll issues.jsonl until the scheduled export is visible; never reopen or call auto_flush.
// Independently seed four sparse 100 MiB histories before open; require two newest retained.
```

Run `scripts/spur-cargo test -p spur-pm --test beads_maintenance -- --nocapture`, observe behavioral failures, then commit tests. Implement a policy with approved defaults (30s flush, 900s sweep, 256 MiB history, two protected available copies, 64 MiB WAL threshold). Expose `plan_history_cleanup` and locked execution via maintenance runtime; planning must not delete files. Validate nonzero durations/budget/protection. Test malformed names, other stems, symlinks, stale plan replacement, protected over-budget copies, idempotence, and unchanged fixtures.

**Acceptance:** runtime RED evidence saved; planning is deterministic and safe; tests cover the specified path protections.

## Task 2: Writer actor scheduling, reporting, and WAL

**Task ID / beads:** `actor` / `bd-eqc`
**Depends on:** `retention`
**Suggested Worker:** current Codex agent
**Files:** `adapter.rs`, `beads_db.rs`, `metrics.rs`, `init.rs`, and `maintenance/runtime.rs` within `beads_crate`.
**Scope:** existing actor/lock integration only; no additional DB writer or subprocess.

Introduce `AdapterConfig.maintenance: MaintenancePolicy`. Before wiring the actor, add tests for dirty export without reopen, queued work holding the flock, export failure retry, busy-stream deadlines, shared sweep throttling, temporary cleanup, and WAL with a pinned reader. Use the solved 20ms test interval to avoid long waits in subsequent runs. Actor code must call `try_blocking_write_lock_once`; contention skips work rather than waiting. Schedule due checks between jobs and while idle. Retain error/report state without turning a successful foreground mutation into an error. Store successful sweep time by atomic rename under the flock. Reuse strict temp cleanup, and perform size-triggered TRUNCATE on the existing zero-timeout checkpoint connection.

**Acceptance:** all relevant runtime tests pass, reports/metrics are readable, live records and recovery artifacts survive, and `AdapterConfig` callers compile.

## Task 3: Post-solve, actual implementation evaluation, and review

**Task ID / beads:** `evaluation` / `bd-22u`
**Depends on:** `actor`
**Suggested Worker:** current Codex agent
**Files:** notebook and solver/evaluation evidence under `docs/research`; a small `spur-pm` example that calls the implemented APIs; update approved design status.
**Scope:** evaluate actual Rust behavior rather than a Python reimplementation.

Run the Rust example through `scripts/spur-cargo run -p spur-pm --example beads_maintenance_eval`; persist its JSON into the worktree for remote sync-back. Use Notebook MCP to load the output, discover schema, aggregate reclaimed/retained bytes, and assert safety/soft-budget outcomes. Re-run pre-solve constraints using shipped policy values and check the implementation against the encoded transition. Preserve exact requests/results and distinguish bounded symbolic evidence from runtime coverage.

Run `scripts/spur-cargo test -p spur-pm`, relevant durability tests with `--features test-helpers`, `SPUR_REMOTE=1 scripts/spur-cargo clippy -p spur-pm --all-targets -- -D warnings`, and local formatting. Review the diff for locking, recovery, and scheduling regressions; fix findings with regression tests. Commit finished implementation and evidence, then record outcomes in beads.

**Acceptance:** actual implementation notebook cells succeed; post-solve maintains sat/unsat expectations; required checks pass or concrete external blockers are reported truthfully.
