# Bounded Panic Analysis Implementation Plan

> **For SPUR orchestrator:** This plan is designed for the persisted `submit_plan` workflow (the live tool persists automatically).
> Each execution task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `2026-11-01-panic-implementation-grounding.ipynb`, with detailed evidence at `docs/research/artifacts/2026-11-01-panic-implementation-grounding/README.md`.
**Formal @spec cells (if notebook):** none; the approved acceptance decision is cell `6b7b03e2-60e4-4ad1-9189-cdbba4a87d65`.
**Design epic:** `bd-39y` (closed).
**Pre-submit intent epic:** `bd-zcq`. The persisted engine creates execution issues; source intent IDs below are lineage only and will be superseded by the execution mapping.
**Authorization:** user approved implementation, TDD, SOLVE PRE then POST for every task. No further approval is required to execute this plan.

**Goal:** Ship the evaluated narrow source-first panic-analysis path with honest bounded results and compiler validation.

**Architecture:** Correct Rust resolution first. Add a separately versioned, source-bound evidence sidecar for invocation/body identities and typed semantic facts, then translate validated facts to the existing `spur-solver` typed service. Preserve canonical graph v11 and existing method/compiler evidence APIs. Unknown semantic/dispatch/expansion/frontier states remain visible. The JSON example exposes the working pipeline; root validates it through a new Notebook MCP notebook.

**Tech Stack:** Rust 2021, tree-sitter Rust, serde, existing `spur-solver` typed language/service and local Z3, ordinary rustc fixture replay, pinned CGA for compiler observations. All Cargo commands use `SPUR_REMOTE=0 scripts/spur-cargo`.

**Routing:** All tasks use agent `codex`, profile `codex`, model `gpt-6.1-sol`, effort `xhigh`, honoring the user's persistent selection. Root owns architecture and coordination, leaving each worker a focused implementation unit; this avoids assigning cross-task architecture decisions to the narrow-edit worker.

**DAG:** R1 -> R2 -> R3 -> R4 -> R5 -> R6 -> R7 -> R8. Shared resolver/public evidence files and interface dependencies require the chain. The engine alone dispatches ready tasks after review; root does not manually dispatch dependent work.

**Discovery:** `knowledge_context_pack_2` fails preparing the analyst query and exact `code_symbol_search` reports no graph artifact. Bounded raw reads mapped `extract/tree_sitter.rs` (including the `target == edge.source` discard and method singleton fallback), existing `panic_evidence` validators, and `spur-solver::{types,service,encode}`. No graph rebuild is needed to implement this work.

## Per-task execution and review contract

CONTEXT: User explicitly approved implementation with TDD and SOLVE PRE then POST per task. Approved design: 2026-11-01-panic-implementation-grounding.ipynb (acceptance decision cell 6b7b03e2-60e4-4ad1-9189-cdbba4a87d65), docs/research/artifacts/2026-11-01-panic-implementation-grounding/README.md. Baseline 41b058e4c8021f0808d73962545647fe020d9aa3; design issue bd-39y closed. Root owns architecture and plan review. User routing stays agent=codex, profile=codex, model=gpt-6.1-sol, effort=xhigh.
CONSTRAINTS COMMON TO EVERY TASK:
- Use the ACTUAL assigned plan issue from delegation context. A source intent issue is historical input, not your assigned target. Verify actual issue is in_progress before editing and use that issue for audit/signal calls.
- Catalog-first SOLVE PRE BEFORE RED/production edits; implemented families via solve_rules; generic only when no family fits, exact solve_constraint_check preflight. Persist and reload every PRE/POST receipt; do not inspect .spur/solver files. Historical research receipts are context, not a substitute for fresh task PRE.
- Genuine TDD: commit failing tests as test(spur-graph): <R#> ...; run and capture the expected assertion failure (compile RED for a brand-new API must be labeled honestly). Then minimal fix/feature commit; GREEN checks; SOLVE POST over ACTUAL shipped/exported facts, not unchanged manual PRE fixtures. Require positive and adversarial controls; unknown/timeout never means UNSAT or safety.
- Cargo only SPUR_REMOTE=0 scripts/spur-cargo, including check/test/build/run/fmt; no bare Cargo. Preserve existing graph schema v11/API; new evidence is a separately versioned source-bound sidecar. Source retrieval: graph-first; current orientation and exact graph tools fail, so document bounded raw fallback.
- Keep detailed verification, commands, hashes, exact requests, preflights, responses and get_solve_result reloads under docs/research/artifacts/2026-11-01-panic-implementation/<R#>/ (this evidence directory is additional allowed output for each task). Full source/solver evidence stays LOCAL. Tracker audits/comments only compact local path+commit references. If report_audit assignment validation fails, retain compact request/error and report to root; do not retry full payload through another channel.
- No notebook reads/writes, graph CURRENT/publication changes, global installs/config edits, original CGA checkout/driver/launcher changes, upstream messages or unrelated user-file edits. Five pre-existing user files remain protected; root handles notebook verification.
- Commit finished scoped work; leave task issue in_progress for root review. Signal scope_drift before out-of-scope code or a materially larger implementation; don't silently weaken a gate or skip an approved requirement.
EXPECTED OUTPUT COMMON: Unified diff + one-sentence rationale (worker output_shape), commits, exact RED/GREEN commands/outcomes, selected rules and PRE/POST IDs, local verification path, source/build/bound assumptions and unresolved obligations. Full acceptance is per task, then root review.

A task is accepted only when its actual assigned beads issue has a completion audit, its commits show RED before implementation, all required checks are green, and complete PRE/POST requests and reloaded receipts exist locally. Root repairs compact assignment-audit routing failures through its authorized issue API; workers never bypass a sensitive-payload rejection. Schema/number/proof claims are limited to the encoded domain.

## Existing API regression example

R1 must preserve the existing API and catch the concrete self-call omission with a runtime assertion:

```rust
#[test]
fn lexical_self_call_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"),
        "pub fn recur(n: u8) -> u8 { if n == 0 { 0 } else { recur(n-1) } }\n").unwrap();
    let (facts, _) = spur_graph::build_facts(dir.path(), None).unwrap();
    let owner = facts.nodes.iter().find(|n| n.label == "recur").unwrap();
    assert!(facts.edges.iter().any(|e|
        e.relation == spur_graph::RelationKind::Calls &&
        e.source_node_id == owner.node_id && e.target_node_id == Some(owner.node_id)));
}
```

R2 must reject a local homonym as the target of a standard receiver:

```rust
#[test]
fn string_receiver_never_binds_to_custom_len() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"),
        "struct Pretender; impl Pretender { fn len(&self)->usize {77} }\nfn string_len(text:&str)->usize {text.len()}\n").unwrap();
    let (facts, _) = spur_graph::build_facts(dir.path(), None).unwrap();
    let owner = facts.nodes.iter().find(|n| n.label == "string_len").unwrap();
    let custom = facts.nodes.iter().find(|n| n.label == "len").unwrap();
    assert!(!facts.edges.iter().any(|e|
        e.relation == spur_graph::RelationKind::Calls &&
        e.source_node_id == owner.node_id && e.target_node_id == Some(custom.node_id)));
}
```

These two tests are examples, not the entire matrix. Each task below specifies its distinct positive, negative and integration assertions. New public contracts are defined by their introducing task and frozen at root review; dependent workers read those accepted APIs rather than inventing parallel types. No test may be treated as runtime RED when it only failed compilation.

## R1: Restore lexical Rust self-calls without shadowing false positives

**Task ID:** `R1`
**Source intent:** `bd-202`
**Depends on:** none
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/extract/tree_sitter.rs`
- `crates/spur-graph/src/extract/rust_bindings.rs`
- `crates/spur-graph/src/extract/mod.rs`
- `crates/spur-graph/tests/rust_lexical_calls.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R1/`.

**Scope / outcome:** Fix actual Rust call resolution using lexical source binding intervals. All four held-out plain/before-shadow/after-block/qualified self-calls must resolve to their own exact function. The shadowing closure must not become a recursive edge; local nested and module homonyms keep correct owners. Do not just remove target==source suppression globally. Preserve non-Rust behavior, macro captures and unresolved evidence.

**Public contract:** Existing build_facts/artifact APIs remain compatible. A small Rust-only private binding helper may be introduced at extract/rust_bindings.rs; no graph schema change.

**Acceptance and failing tests:** Build the exact lexical held-out source through build_facts; assert a Calls edge source_node_id==target_node_id for each genuine recursive owner and no such edge for the shadowed closure invocation. Add renamed/local-block variants. Runtime assertion RED required, then fix.

**SOLVE PRE / POST:** Catalog-first PRE: finite exact owner/callee compatibility and lexical binding intervals, with genuine recursion positives and closure/nested/module negatives. POST must use actual exported resolver outcomes and reject wrong same-name bindings.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_lexical_calls`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R2: Require compatible Rust receiver evidence before resolving methods

**Task ID:** `R2`
**Source intent:** `bd-1pv`
**Depends on:** `R1`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/extract/tree_sitter.rs`
- `crates/spur-graph/src/extract/rust_bindings.rs`
- `crates/spur-graph/tests/rust_receiver_resolution.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R2/`.

**Scope / outcome:** Eliminate both wrong standard string length targets and all eleven Option unwrap site observations binding to the unrelated local Lookalike::unwrap. Preserve correct explicit Alpha, borrowed Beta and custom method targets. Track simple typed/local constructor origins and binding/shadow intervals where exact; unresolved is required when inference, trait visibility, autoderef or ambiguous types cannot be established. Resolve obvious Alpha/Beta constructors/rebinding when supported by exact syntax; do not guess a crate singleton by method spelling.

**Public contract:** Reuse R1 Rust binding helper; public GraphFacts remains compatible. Add binding provenance or private source-site metadata only as needed inside this focused resolution seam.

**Acceptance and failing tests:** Run build_facts on separate callers for fn string_len(text:&str){text.len()} versus Pretender::len, Option vs Lookalike::unwrap, explicit/ref receiver types, constructor-bound locals and shadowing. Assert exact targets where grounded and absence of incompatible targets. Runtime RED for wrong bindings must precede fix.

**SOLVE PRE / POST:** PRE/POST catalog exact typed receiver/target allowed tuples, presence/owner checks and ambiguity controls. POST supplied relation must be exported by the new resolver, not copied from the PRE oracle.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R3: Add precise source-bound free-call and executable-body evidence

**Task ID:** `R3`
**Source intent:** `bd-rp2`
**Depends on:** `R2`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/panic_evidence/call_sites.rs`
- `crates/spur-graph/src/panic_evidence/mod.rs`
- `crates/spur-graph/tests/panic_call_sites.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R3/`.

**Scope / outcome:** Extend the evidence lane to every intact direct method/free call with per-invocation identity and explicit function/nested-function/closure/async body identity. Body creation is distinct from execution; never make parent-to-body execution claims merely by containment. Preserve current MethodCallSiteEvidenceSet behavior/API. Snapshot-bound identifiers include exact source and ranges; do not promise cross-edit stability. Canonical validation rejects forged same-file owners, stale bytes, omissions, duplicates and parse/cfg uncertainty.

**Public contract:** Define public RustBodyEvidence, RustBodyKind, RustCallSiteEvidence, RustCallSiteEvidenceSet and collect_rust_call_sites(graph:&GraphIndexArtifact,sources:&[PanicEvidenceSource])->Result<RustCallSiteEvidenceSet>. The set exposes bodies/sites/source bindings; sites have site_id, source_oid, file_path, byte_range, call_text and body_id; bodies have body_id, byte_range, kind and optional definition identity. A canonical validate method recollects exact input evidence.

**Acceptance and failing tests:** Use repeated p.mark()/ping() and unentered closure/async/nested bodies from the held-out body fixture. Assert distinct repeated callsite IDs, separate executable body IDs and no asserted execution from creation; forged metadata must fail. New API compile RED may precede runtime contract RED, recorded accurately.

**SOLVE PRE / POST:** PRE/POST catalog unique plus required identity/presence and exact source-body-site tuples. Retain NULLS DISTINCT negative controls so missing identity fields cannot certify completeness.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_call_sites`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R4: Define validated typed panic facts and explicit uncertainty

**Task ID:** `R4`
**Source intent:** `bd-2wn`
**Depends on:** `R3`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/panic_evidence/semantic_facts.rs`
- `crates/spur-graph/src/panic_evidence/mod.rs`
- `crates/spur-graph/tests/panic_semantic_contract.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R4/`.

**Scope / outcome:** Implement the versioned, serializable semantic fact contract independently of canonical graph schema v11. Represent source/body/site bindings, bool/u8 domains, value versions/origins, guarded assignments/phi, ordered control flow, primitive contract authority, exact build/cfg, unknown obligations and exploration limits. Absence is not false; opaque/mutation/cfg/parse/callee limitations are explicit. Enforce source/body/site foreign keys, types, range containment, graph/build hash binding and deterministic order. No assertion of generic Rust semantic coverage.

**Public contract:** Define public PanicBuildConfig and PanicSemanticFacts with a validate_against(graph,sources,call_sites) contract. Build config requires explicit target/edition/cfg, overflow_checks, debug_assertions and panic strategy. Facts must identify each typed expression/guard/origin derivation and its source spans; unknown obligations must survive JSON. R5 supplies canonical automatic collection; R6 consumes this typed contract.

**Acceptance and failing tests:** Contract round-trip and adversarial tests must reject missing domains, reused site IDs, wrong existing body, stale source/build, mismatched sorts, detached guards and missing origin presence. Test explicit unknown obligations survive serialization and cannot become complete evidence.

**SOLVE PRE / POST:** PRE/POST catalog foreign-key, unique, required exact tuple and typed range rules where applicable. Persist and reload the exact requests; POST binds actual serialized facts and invalid variants.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R5: Extract narrow typed value origins and evaluation guards from Rust AST

**Task ID:** `R5`
**Source intent:** `bd-2z9`
**Depends on:** `R4`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/panic_evidence/semantics.rs`
- `crates/spur-graph/src/panic_evidence/semantic_facts.rs`
- `crates/spur-graph/src/panic_evidence/mod.rs`
- `crates/spur-graph/tests/panic_semantics.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R5/`.

**Scope / outcome:** Implement an automatic source collector for the evaluated bool/u8 subset, not fixture-name lookup: Option Some/None constructors, simple locals, reassignment and shadowing/phi, if/else and early returns, short-circuit evaluation, checked_add+unwrap, primitive addition under explicit overflow setting, bounded constant array indexing, and direct helper calls with argument bindings. Retain a literal for-range representation so R6 can explore within caller limits. Unknown effects, aliasing, opaque calls, custom/shadowed unwrap/constructors, unsupported casts/patterns/cfg and unsupported macro-expanded code must produce obligations. Use exact source/body/site identities from R3 and the R4 typed IR.

**Public contract:** Expose collect_panic_semantics(graph:&GraphIndexArtifact,sources:&[PanicEvidenceSource],build:&PanicBuildConfig)->Result<PanicSemanticFacts>. Implement canonical validation by recollection in addition to R4 structural checks. No manually supplied conclusions may be accepted as automatically extracted facts.

**Acceptance and failing tests:** Use the semantic fixture shapes, with additional renamed locals/functions and whitespace variations. Correct mutation produces a new None origin under x==3; shadowed values stay separate; && guard dominates only its RHS; a preceding unrelated if does not guard the final unwrap. Test custom Lookalike and malicious external mutation become unsupported/unknown. No snapshots blindly blessed.

**SOLVE PRE / POST:** PRE/POST typed uncatalogued guard/value equations after catalog discovery and exact preflight; catalog ownership checks where matched. POST constraints must be derived by the shipped collector from source and tied to origin/guard spans; compare to independent finite and ordinary Rust oracles.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_semantics`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R6: Solve bounded panic predicates through the existing typed solver service

**Task ID:** `R6`
**Source intent:** `bd-3la`
**Depends on:** `R5`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/panic_evidence/analysis.rs`
- `crates/spur-graph/src/panic_evidence/mod.rs`
- `crates/spur-graph/Cargo.toml`
- `Cargo.lock`
- `crates/spur-graph/tests/panic_analysis.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R6/`.

**Scope / outcome:** Translate validated R4/R5 facts into spur_solver::types::SolveConstraintsRequest and use injected spur_solver::service::SolverService. Add only spur-solver workspace dependency if required; do not change its backend/grammar. Model path guards, value origins, ordered panic conditions, exact input domains, helper argument substitution and explicit call/loop bounds. Preserve unsupported or open frontiers; no UNSAT safety label when any required fact/obligation is unresolved. Distinguish raw solver status, candidate witness, no counterexample within complete scope, incomplete semantics/frontier, timeout/unknown and operational error. Validate returned models against exact emitted constraints and bind receipts to source/build/request identity.

**Public contract:** Expose PanicAnalysisLimits, PanicAnalysisReport and async analyze_panic_facts(service:&SolverService,facts:&PanicSemanticFacts,limits:&PanicAnalysisLimits)->Result<PanicAnalysisReport>. Report includes exact typed request, raw solver response/status/model/core, source/build/fact digests, assumptions/bounds and completeness obligations. A candidate is not marked runtime reproduced by this function.

**Acceptance and failing tests:** Regression matrix: safe/changed Option guard; mutation x3 and shadowing x8; checked-add x255; plain addition on/off; index safe/bad; short circuit and non-dominating guard; helper depth0 incomplete vs expanded witness; loop prefix incomplete vs sufficient-bound witness. Actual Z3 positive paths and controlled unknown/timeout/unavailable statuses must remain distinct. Tests should exercise real validation; injected backend only for otherwise inaccessible operational branches.

**SOLVE PRE / POST:** Task-level PRE feasibility and counterexample queries follow catalog-first/preflight discipline. POST re-run shipped typed requests for evaluated primitive domains, SAT witnesses and UNSAT controls. Exhaust finite u8 controls independently and retain exact bounds; never duplicate family formulas as generic.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_analysis`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R7: Correlate panic facts with compiler observations without promoting uncertainty

**Task ID:** `R7`
**Source intent:** `bd-3su`
**Depends on:** `R6`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/panic_evidence/compiler_validation.rs`
- `crates/spur-graph/src/panic_evidence/cga.rs`
- `crates/spur-graph/src/panic_evidence/mod.rs`
- `crates/spur-graph/tests/panic_compiler_validation.rs`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R7/`.

**Scope / outcome:** Bridge new call/body/primitive evidence to existing validated pinned CGA capture identity and raw instance/location references. Preserve Called, VTableCalled, FnPtrCast, BuiltinAssert and AsyncPanic categories. Direct source location correspondence is not path feasibility or type inference. Require build/source/config agreement, preserve raw macro-definition observations, and report missing expansion-stack/invocation identity and generic virtual producer trust limitations. The successful concrete fixture must retain exact unit instance, expanded call and implicit check observations; wrong/stale compiler captures cannot certify semantic completeness.

**Public contract:** Expose a PanicCompilerValidation report and correlation entrypoint using existing CaptureBuildIdentity/CgaGraph import validation plus new evidence. Keep legacy compiler importer API and pinned producer format compatible; share validators with crate-private visibility only when necessary. No compiler observation may clear an unknown obligation without the exact evidence it requires.

**Acceptance and failing tests:** Use the held-out raw six-node/seven-observation capture and exact source fixture, plus existing compiler-evidence fixtures. Reject forged owner/build/source/edge references; FnPtrCast never becomes execution; virtual enumeration never proves a feasible call; macro-definition span never silently maps to invocation; implicit assertions remain typed categories without invented payload/reasons.

**SOLVE PRE / POST:** PRE/POST catalog source/build/owner tuples and workflow transition-policy controls, preserving separate execution/address/virtual sets. POST consumes actual correlation reports and exercises invalid identities. Retain all receipts locally.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_compiler_validation`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## R8: Expose the bounded pipeline and verify real witness replay end to end

**Task ID:** `R8`
**Source intent:** `bd-13l`
**Depends on:** `R7`
**Worker:** codex / codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/examples/panic_evidence.rs`
- `crates/spur-graph/tests/panic_analysis_end_to_end.rs`
- `crates/spur-graph/tests/fixtures/panic_analysis`
- `crates/spur-graph/README.md`
- Local verification: `docs/research/artifacts/2026-11-01-panic-implementation/R8/`.

**Scope / outcome:** Make the shipped source collector, typed solver service and compiler-validation report runnable through the panic_evidence JSON example while preserving every existing snapshot/collect/import/validate command. Provide analyze/solve JSON operations with explicit build, roots, domains and limits, and a source/build-bound witness replay record. Exercise the bounded fixtures through ordinary Rust execution and actual solver calls; replay bounds, overflow and assertion witnesses with their exact panic payloads. Add held-out renaming/shadowing/cfg/opaque controls. Never claim replay of an arbitrary workspace binary or general safety.

**Public contract:** Document stable JSON command schemas and error/unknown statuses in crate README. Wire existing synchronous example execute paths compatibly; async solving may use a separate entrypoint. Returned witness replay identity includes source/build/request digest and actual outcome; solving alone cannot create a reproduced record. Root will use these commands through Notebook MCP after plan integration; workers must not access any notebook.

**Acceptance and failing tests:** End-to-end tests must fail before wiring the new commands, missing-fact gates or replay binding. Run real source->facts->request->Z3->witness->ordinary-rustc fixture replay with both arithmetic settings; verify unsupported cases stay unknown, safe assumptions remain bounded and altered source/build/replay identities fail. Existing panic/macro/extractor regressions stay green.

**SOLVE PRE / POST:** PRE/POST exact shipped end-to-end requests, source provenance and workflow/identity negatives. Re-load persisted results and independently enumerate expected u8 cases. Final full spur-graph suite with RUST_TEST_THREADS=1, dependent checks if exposed APIs impact them, all-target compilation, scoped formatting and diff checks. Do not add ignores to force acceptance.

**Execution:**

1. Discover the rule catalog, run the applicable PRE models, persist/reload receipts and use their witnesses/predicates in tests.
2. Commit the test changes and run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end`; retain the actual expected failure.
3. Implement only this task's behavior and commit it separately. Retain source/build/identity checks and unknown obligations.
4. Run the same target to GREEN, directly affected existing tests, scoped formatting and compilation; R8 additionally runs the full graph and dependency acceptance suite.
5. Re-encode actual shipped/exported policy for SOLVE POST, persist/reload complete receipts and independently check bounded results. Treat a counterexample or inconclusive result according to its actual semantics.
6. Produce a unified diff, one-sentence rationale, commits, commands/exits and compact completion reference. Leave the assigned issue in_progress for root review.

**Scope drift checkpoint:** Before touching code outside these files, changing the public predecessor contract or materially expanding the task, emit a durable scope_drift/risk signal. Routine implementation choices within these boundaries are already authorized.

## Integration acceptance

Root reviews task-by-task, verifies actual issue/dispatch IDs and receipt artifacts, then lets the reconciler advance. Once all tasks pass, merge the approved plan branch without touching unrelated user edits. Build the shipped example locally, create a separate Notebook MCP verification notebook and run the real source->facts->typed request->Z3->replay/compiler-correlation path. The prior research notebooks/dossiers, published graph tables and original CGA producer remain protected. Close the engine epic only after integration and final verification. General arbitrary Rust semantics, unbounded safety, full CLI capture and modification of the CGA producer are outside this bounded implementation.

