# Analyst Recovery Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan`.
> Each task becomes a beads issue with explicit dependencies and root review gates.

**Source spec:** `docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md`
**Formal @spec cells:** none
**Design epic:** `bd-16wv` (previously approved)
**Recovery source epic:** `bd-36tf`
**Goal:** Repair the remaining graph-query FTS failure and complete analyst acceptance without repeating the recovered fixes.
**Architecture:** Start from protected authored checkpoint `38051998648e0aa6e7ff622b46c4ce654c208588`. Keep graph repair, ordinary regression validation and real-helper validation in separate sequential tasks. The exhausted C task is superseded without approval; packaging waits on the final new execution issue.
**Tech Stack:** Rust, DuckDB, isolated embedding helper, Jev/catalog Z3 verification.

## Task 1 — graph-query-fts

Source issue `bd-3vam`; no implementation dependency within this replacement plan. Worker agent/profile codex, model gpt-6.1-terra, effort xhigh.

Modify `crates/spur-analyst/src/search/graph_candidates.rs`; if a meaningful regression is needed, modify `crates/spur-analyst/tests/search/context_candidates.rs`. Evidence belongs under `docs/research/artifacts/2026-11-04-analyst-recovery/graph-fix/`. Do not touch embedding ownership, query-context source, defaults, schemas or packaging.

1. Inspect the executed PRE packet and run the frozen checkpoint verifier before edits. Positive `sol_be6bf622604848b1`; four adverse/observed snapshots reject.
2. Reproduce the committed public graph-query RED:

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb scripts/spur-cargo test -p spur-analyst --test search context_candidates::graph_candidates_return_primary_and_neighbor_rows -- --exact --nocapture
```

Require one executed failure on missing FTS, not a filtered zero-test pass. Record the failing test/evidence commit before implementation.

3. Follow the existing sibling connection pattern, without global autoload or a preloaded test-only bypass:

```rust
conn.execute_batch(&analyst_extension_load_sql("fts"))
    .context("failed to load FTS extension for graph candidate query")?;
```

Load FTS on the fresh public connection before preparing its query; retain ICU initialization, existing API and limits.

4. Rerun the exact case and the complete search integration target, then relevant library check, formatting and scoped diff checks. Root verifies actual outputs and executes POST before approving.

## Task 2 — analyst-ordinary-acceptance

Source issue `bd-2rzo`; depends on task 1. Same explicit worker routing. Evidence-only writes under `docs/research/artifacts/2026-11-04-analyst-recovery/ordinary-acceptance/`; no production fixes in this node.

Root provides its fresh source-bound PRE before task 1 approval/release. Run:

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb scripts/spur-cargo test -p spur-analyst -- --test-threads=1 --skip knowledge_context_pack_reports_confidence_from_real_hybrid_fusion
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo test -p spur-embedding --features test-support -- --test-threads=1
```

The graph regression must run; no graph skip is allowed. The excluded real confidence fixture and explicitly ignored real tests are assigned to task 3 and listed individually. Verify the four product native-free dependency closures and source/format checks. Root POST must reject hidden skipped graph coverage, invalid PID evidence, suite failures and forbidden dependency leakage.

## Task 3 — analyst-real-final

Source issue `bd-10e2`; depends on task 2. Same explicit worker routing. Evidence-only writes under `docs/research/artifacts/2026-11-04-analyst-recovery/real-acceptance/`.

Root supplies fresh final PRE before task 2 approval/release. Build normal helper/CLI separately through repository wrappers, then use explicit configuration and fresh outputs for:

- The restored public real hybrid-confidence test.
- The moved real Nomic warm/query/v1 multi-text test with two owned lifecycles.
- An actual CLI indexed-pack RPC and checked helper/CLI shutdown.

Record source/build/helper/runtime/model hashes before and after new runs, native vectors/reference checks and actual wait status conventions. Preserve all-three original C2 model captures under their original identities and verify exact final-source equivalence for unchanged neural paths; rerun affected cases if those paths change. Never relabel old captures as new executable runs. Keep v1's duplicate-query/direct-handler and separate graph batch owner limitations explicit.

Root independently checks final evidence and executes POST before approval. Packaging stays blocked until root prepares its own fresh PRE and exact source import, then approves this final execution node. Windows, the eight direct-native graph abort baselines and notebook work remain outside passing claims.

## Recovery and verification rules

All source histories and failed logs remain preserved. Ordinary and real validation use one local Cargo writer. Stop for a genuine new source-scope defect and let root re-plan; do not silently broaden or mark partial checks GREEN. No new timeout, cap or shipping default is chosen. New nodes use the actual canonical dispatch ID, not the source IDs above.

The preserved old C verifier checks frozen bytes; it must not be weakened after task 1 adds the explicitly authorized graph file. Root's new final equivalence checks must distinguish frozen history from final source.
