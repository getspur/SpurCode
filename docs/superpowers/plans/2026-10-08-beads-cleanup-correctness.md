# Beads Cleanup Correctness Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-10-08-beads-cleanup-correctness.md`
**Formal @spec cells:** none
**Design epic:** `bd-1kc17` (closed review; user approved fixing its findings)
**Goal:** Repair the two reviewed cleanup defects without changing retention defaults.
**Architecture:** Keep filename recognition in SPUR's shared startup matcher.
Fix backup rotation in the upstream owner, then update the exact dependency pin.
**Tech Stack:** Rust, SQLite, tempfile, catalog solver, remote Cargo wrapper.

### Task 1: Fix and integrate cleanup correctness

**Task ID:** `bd-33a3b`
**Depends on:** none
**Executor:** current Codex session; no worker dispatch is required.
**Files:** SPUR `crates/spur-pm/src/beads_crate/init.rs`,
`crates/spur-pm/Cargo.toml`, dependency entry in `Cargo.lock`;
beads_rust `src/sync/history.rs`, `docs/SYNC_SAFETY.md`.

**Acceptance criteria:** Actual exporter temp files and legacy PID files are
recognized; fresh files remain; startup cannot skip pending cleanup; deduplication
enforces age and count retention; an expired identical generation is refreshed
before removal; changes pass relevant tests and solver verification.

1. Add regression tests before implementation. Produce the actual temp fixture
   with `sync::export_to_jsonl` against a directory destination, forcing final
   rename to fail. Verify fresh retention and stale removal while holding the
   write lock. Add a fast-path test with `issues.jsonl.tmp`.
2. In upstream history tests, exercise a fresh identical backup with an expired
   predecessor, two recent backups with `max_count: 1`, and an expired sole
   identical backup. Assert retained contents and preserve unrelated stems.
3. Run both focused suites remotely using `scripts/spur-cargo`; commit the tests
   only after observing the expected assertion failures.
4. Add `name == "issues.jsonl.tmp"` to the matcher. In upstream, reuse an
   identical backup only while it remains within the age window, and call
   `rotate_history` before returning from that branch. Otherwise use the existing
   copy-before-rotation path. Update the backup documentation.
5. Run upstream history tests, required check/clippy/format and sync safety tests;
   commit the upstream fix on its isolated branch and publish that branch so
   the exact commit is fetchable. Update only SPUR's dependency pin and matching
   lockfile source, preserving the user's independent lockfile hunk.
6. Run SPUR library and maintenance integration tests, clippy and format; repeat
   catalog verification using the implemented `plain_and_pid` capability.
   Commit the SPUR fix and record outcomes in `bd-33a3b`.

**Scope boundary:** No live-data deletion, no retention-default changes, no gzip
archive policy, no issue compaction, no unrelated source changes. Report a real
test or integration blocker before expanding this scope.
