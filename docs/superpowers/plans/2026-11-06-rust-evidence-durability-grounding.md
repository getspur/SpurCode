# Rust Evidence Durability Grounding Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan`.
> Each task becomes a Beads issue with plan and task labels.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md`.
**Formal @spec cells:** none.
**Design epic:** `bd-3cs` (closed).
**Source phase:** `bd-3l2`, already authorized G3; G2 `bd-3nv` is closed and integrated.

**Goal:** Ground the durable Rust evidence implementation DAG in the actual landed G2 and existing R3 interfaces before production edits.

**Architecture:** Preserve one optional versioned `GraphIndexArtifact` carrier for physical declaration and invocation/body evidence. Reuse canonical collectors and validators, represent per-file unknown/unavailable and complete-zero distinctly, and map every composition, storage and query path before assigning implementation stages.

**Tech Stack:** Rust workspace, existing canonical evidence APIs, Parquet/DuckDB transport, source-bound SOLVE/Jev review, Rust notebook replay.

## Task: Ground the durable evidence contract and implementation seams

**Task ID:** `ground-durable-rust-evidence`.
**Depends on:** none within this grounding plan. Approved integrated G2 is the fixed base.
**Worker:** `agent=codex`, `profile=codex`, `model=gpt-6.1-sol`, `effort=xhigh` per the latest user selection.

**Write scope:**

- `docs/research/artifacts/2026-11-06-rust-evidence-durability/grounding/REPORT.md`
- `docs/research/artifacts/2026-11-06-rust-evidence-durability/grounding/source-map.json`
- `docs/superpowers/plans/2026-11-06-rust-evidence-durability.md`

All production files and older evidence are read-only in this task. No new dependency, numeric policy, production carrier or transport is implemented. No Cargo build is needed for source grounding. This task has no fabricated RED/GREEN or solver/runtime claim; fresh PRE followed by TDD and POST belongs to each subsequent implementation task.

**Required investigation:**

1. Query the code knowledge/graph tools first. Record staleness, then bind changed/current symbols to exact checkout bytes and Git OIDs. Revalidate the old `audits/call-evidence.md` against the landed interfaces.
2. Read the actual `GraphIndexArtifact`, canonical hash body, `SourceBucket`/facts composition, full build/reuse/delta/deletion, cache comparison, Parquet manifest/checksum/completeness reader/writer, query client memory/overlay/disk paths, and existing macro transport tests. Enumerate every unavoidable artifact constructor update and cross-crate reader impact.
3. Map `RustDeclarationEvidenceSet` v2 and `RustCallSiteEvidenceSet` v1 public types/validation/source-scope constraints exactly. Explain how a per-file wrapper preserves R3's rejection of parse/cfg uncertainty without silently dropping sources or claiming complete zero. Preserve source/snapshot identity across unchanged reuse and overlays rather than relabeling historical evidence with a new graph hash.
4. Specify public data types, serialization/version compatibility, source/body/site/structural row transport and exact validation boundaries. Existing absent optional evidence means unavailable; unsupported disk schemas require an actionable rebuild. Foreign signatures never acquire executable bodies or callee claims.
5. Produce focused serial contract, composition, storage and query task specs, splitting further where a small coherent diff is possible. Each specifies exact file ownership, predecessor interfaces, compiling test seeds using current APIs, observed RED expectations, GREEN commands through `scripts/spur-cargo`, current-source PRE inputs/catalog families and actual-output POST controls. Identify mechanical cross-crate constructor changes separately from semantic changes.
6. Give exact acceptance cases for complete zero-site/zero-declaration files, unknown cfg/parse sources, missing/duplicate/stale/cross-file body/site owners, unchanged reuse, modified/deleted files, canonical hash tampering, missing/truncated/wrong-checksum Parquet tables, unavailable legacy evidence, query filters and full file replacement overlays. Include a final NEW Rust notebook replay and actual DuckDB/source join. Expected rows cannot substitute for actual exports.

**Acceptance:** Source map records current base, paths, source hashes, exact symbols and verified seam behavior. The plan is an acyclic Beads-compatible DAG with concrete types/tests/commands and no unresolved placeholders. A requirement-to-task matrix covers every G3 obligation and explicitly leaves compiler cfg/expansion/dispatch and complete panic detection to later phases. Self-review checks all source paths/types and protects the existing API and dirty main files. Commit only the three scoped documentation files, return reviewable diff and remaining design risks, leave canonical issue in progress for ROOT.

**Preservation and authorization:** Use the fresh canonical issue supplied by the plan engine, never this source phase or the branch UUID as the audit target. Verify assignment and persist/replay INTENT before work. On an authorization rejection save the exact error, report it and stop; ROOT repairs dispatch. Do not change main, live `../spur-notebook`, graph/CURRENT, existing notebooks, models, native runtime or solver storage. No dispatch of successors, closure or self-approval.

## Execution and review

The user has already authorized implementation and continuation. ROOT submits this bounded grounding stage directly, reviews its current-source mapping and concrete implementation DAG, then supplies fresh PRE/Jev and dispatches each implementation stage with test-first commits and actual-output POST. Historical C receipts validate G2; they do not stand in for a G3 PRE.

ROOT review must compare approved source trees before integration. The G2 generated branch omitted inherited worker commits; later integration must verify complete reviewed content and retain original test/fix history instead of trusting a successful cherry-pick report.
