# Rust Graph G1 Acceptance Repair Plan

> **For SPUR orchestrator:** Submit this single reviewed task through `submit_plan`.
> The actual assigned Beads issue is authoritative; source issue records lineage.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md`.
**Formal @spec cells:** none.
**Design epic:** `bd-3cs` (closed).
**Phase:** `bd-1go`, under approved implementation `bd-hdi`.
**Goal:** Refresh the sole stale canonical snapshot version and pass complete integrated graph acceptance.
**Architecture:** Keep canonical serialization, hashing, and version validation unchanged; update its expected snapshot after the required G1 extractor-version bump.
**Tech Stack:** Rust, insta snapshots, existing SOLVE catalog.

## Task G1d — Canonical snapshot and complete regression

**Source issue:** `bd-1pa`; **task slug:** `G1d`; **depends on:** none in this plan (G1a–G1c already approved and integrated).
**Worker:** `agent=codex`, `profile=codex`, `model=gpt-6.1-terra`, `effort=xhigh`.
**Base:** integrated G1 `8118c0dc1da520d8b108994658f3437e2d208f32`, plus this planning/evidence commit.

**Files:**

- Modify only `crates/spur-graph/src/store/snapshots/spur_graph__store__canonical_hash__tests__canonical_bytes_layout.snap`.
- Add evidence under `docs/research/artifacts/2026-11-02-rust-gap-implementation/acceptance/`.
- No production Rust/queries or other fixture changes; signal any additional regression before extending scope.

**Binding:** The Plan Engine creates a new actual issue. If its ID is not in the envelope, call `list_issues` with labels `spur:plan-task-id:G1d`, status `in_progress`, then verify the one current assignment and call `report_audit` using that ID. Source `bd-1pa` is lineage only. Branch components are brain/session IDs, never delegation IDs. Do not invent a mismatch from branch parsing or silently bypass a failed audit.

**Evidence:** Root's `acceptance/root-red.json` and compressed full log preserve a real compiled assertion RED: 570 library passes, one failed canonical snapshot guard, three ignores; Cargo stopped subsequent targets. Root parsed the old snapshot and runtime diff and proved only `$.macro_invocations.extractor_version` differs. There is no `.snap.new` file in the base.

**Execution:**

1. Discover the SOLVE catalog; run fresh persisted/reloaded PRE for exact snapshot version/layout compatibility, including the stale-version rejection. Catalog verify SAT is pass and UNSAT is rejection. This models the finite snapshot, not all graph semantics.
2. Reproduce the existing meaningful RED before changing the snapshot:

   ```bash
   SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph --lib canonical_bytes_snapshot_guard
   ```

   Expect exit 101 on the changed extractor-version string. Preserve exact output and base; no artificial failing test or production stub is needed for this existing guard.
3. Replace only the snapshot's extractor version `2026-10-31-physical-rust-macro-evidence-v1` with `2026-11-02-rust-tuple-fields-v1`. Parse both JSON bodies and prove all other paths equal. Do not remove the version from canonical bytes, weaken the guard, regenerate arbitrary snapshots, or compute expected bytes from the implementation at test runtime.
4. Run the complete regression, retaining every exit and result count:

   ```bash
   env -u INSTA_UPDATE -u SPUR_GRAPH_BLESS SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target RUST_TEST_THREADS=1 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph --no-fail-fast
   ```

   Expect zero failed targets, including doctests; distinguish existing ignores from the two new opt-in export harnesses. If another real failure appears, record it and signal root; do not hide it or change unrelated files. Run final diff checks. Builds are already authorized locally for SpurCode; do not build or modify `../spur-notebook` or the active main checkout.
5. POST uses actual current snapshot/runtime output, verifies the exact one-field migration and preserved layout, and rejects a wrong-version control. Persist/reload receipts through tools, never private solver storage. Retain all root and worker RED evidence. Commit the snapshot repair and compact evidence with intent-focused messages; don't recursively embed full diffs or uncompressed noisy logs.
6. Return unified diff plus one-sentence rationale, commands/exits, complete suite totals, PRE/POST IDs/interpretations, exact source/snapshot hashes and a completion audit on the actual assigned task. Leave it `in_progress` for root review.

**Acceptance:** Fresh PRE precedes mutation; genuine existing RED is recorded; sole expected JSON path changes; complete unblessed graph suite has zero failures; POST and adversarial control reloaded; canonical serializer/hash unchanged. This task is necessary acceptance work inside the existing implementation approval, not a new feature or permission gate. G2 remains blocked until root approves this task and closes G1.

### Root-authorized G1d rework

The first complete run exposed a separate real `semantic_benchmark_relation_contracts` failure. The Rust fixture declares `Worker(pub u32)` and `Config { pub id: u32 }`; tuple extraction adds the `Worker::0` field and its direct containment and definition edges. Root authorized updating `crates/spur-graph/tests/semantic_benchmark.rs` and this amendment in addition to the snapshot and acceptance evidence. Capture the existing compiled failure and fresh catalog PRE first; change only the Rust counts (Field 1→2, Contains 24→25, Defines 7→8), require both direct field owners and exact source spans, and preserve the other language and generic contracts. Run the complete unblessed suite with loopback access, then persist and reload POST checks of the actual fixture facts, counts, owner negative control, and snapshot version. Retain all prior RED and solver evidence; leave `bd-2zq` in progress for root review.
