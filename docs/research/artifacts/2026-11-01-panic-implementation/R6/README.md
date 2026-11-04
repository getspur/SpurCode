# R6 — bounded panic analysis through the typed solver service

Rationale: translate the existing source-bound bool/u8 facts into exact, validated solver requests while keeping candidate witnesses, complete bounded UNSAT, unresolved semantics, and operational outcomes distinct.

Assigned issue: **bd-cn2** (left `in_progress` for root review). Plan: `04f4a253-7e5d-46f3-99ba-afcd3f016f85`. Source issue `bd-ei5` is lineage only. Baseline: `9e2ee0feef4de01070fce015f5c8716fbe019fc8`. The authoritative plan projection in `startup.json` confirms R2C/R3/R4/R5 approval. Historical R1/R2/R2C/R3/R4/R5 evidence is unchanged.

## Implementation and public boundary

`PanicAnalysisLimits`, `PanicAnalysisReport`, and async `analyze_panic_facts(&SolverService, &PanicSemanticFacts, &PanicAnalysisLimits)` are re-exported from `panic_evidence`. The only new workspace dependency is `spur-solver`; its service, grammar, backend, and graph schema v11 are unchanged.

The compiler models guarded flow reachability, normal continuation after earlier effects, lexical value versions and assignments, guarded phi alternatives, exact root input domains, checked and wrapping u8 addition, bounded array selection, Option presence/unwrap, R4 primitive predicates, helper argument substitution and guarded return relations, literal-loop instances, and explicit call/loop/state budgets. Unsigned arithmetic uses bounded Ints and exact piecewise wrapping; no solver language extension is required.

The report retains the complete typed request, raw response/status/model/core/solver version, source OIDs, graph/build/fact digests, bounds, assumptions, explored-state count, obligations, request identity, response digest and receipt binding. SAT models undergo an independent Int/Bool evaluation of every emitted hard constraint and declaration domain. Candidate events are evaluated against that same model. `runtime_reproduced` is always false here.

Opaque effects and existing source obligations, open call/loop/state frontiers, absent helper return relations, missing implicit panic operations, and ambiguous within-node effect order prevent a complete UNSAT classification. The solver's own variable/constraint/depth/request caps remain enforced. Validation rejection and transport/process failures retain operational diagnostics; raw unknown and timeout remain distinct. A valid known candidate can coexist with an unrelated frontier; it is a bounded model candidate only.

This API receives facts, not source bytes or a graph handle. Its explicit precondition is that the caller authenticated the facts with R4 `validate_against` and, for R5 automatic collection, `validate_canonical`, against an independently expected build. The limits additionally pin the expected build hash. The report states that assumption; its hashes do not constitute producer authentication, compiler attestation, or runtime reproduction. Test normalizations use R4 structural validation rather than pretending to be canonical R5 outputs.

## TDD and validation

Initial test commits: `909b06116`, `1ccf1c3bf`. Completeness regression commit: `9509f102c`. Implementation: `9c1403684`. Final test-only persistence refinement: `238c26eb9`; all exported POST requests are identical to their emitted requests, including transport controls.

Every Cargo command used the wrapper and `SPUR_REMOTE=0`. The retained shared target is a build cache only; it is not a source fixture. The first cold-cache invocation was interrupted (exit 130); `red-cached.log` is the actual compile RED (exit **101**) for the new API/dependency. It is not described as an assertion RED. `red-completeness.log` separately records an assertion RED (exit **101**): omitted overflow semantics were incorrectly classified complete before the minimal obligation fix.

```sh
# Compile RED, exit 101.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis

# Assertion RED, exit 101.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis missing_relations_and_ambiguous_event_order_remain_incomplete -- --nocapture

# Final GREEN: 9 tests, including 4,352 real-Z3 finite-domain probes.
SPUR_R6_EXHAUST=1 SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis -- --nocapture

# Directly affected GREEN: 93 tests.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end

SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r6-scoped-fmt/Cargo.toml --check -- --config skip_children=true
git diff --check
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R6/verify_evidence.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R6/verify_receipts.py
```

`verification.json` records outcomes, commands and source hashes; `format-manifest.toml.txt` preserves the scoped formatter manifest. Check reports only the pre-existing unused `FileOidMatch::as_bool` warning. `green-exhaustive-final.log` is the final target run; earlier GREEN/RED logs are retained as history. No goldens or snapshots were changed or used as inputs. Terminal empty lines in captured logs are normalized for Git whitespace checks; `log-normalization.json` retains original hashes and the exact reconstruction recipe. JSON requests/responses are unchanged.

The runtime regression matrix covers safe/changed Option guards, mutation x=3, shadowing x=8, checked-add x=255, plain addition with checks on/off, safe/bad index, short circuit/non-dominating guard, depth-0 helper frontier versus expanded/substituted witness, and loop prefix versus sufficient-bound witness. Additional controls cover ordered panics, wrapping values feeding later guards, helper early returns, singleton/narrowed u8 domains, all bool domains, source/build/limit identity changes, invalid models, state budget and malformed domains. Controlled runners are confined to otherwise inaccessible unknown/timeout/unavailable/parse-error/invalid-model branches; ordinary queries use real Z3.

The exhaustive test compares every x in **0..=255** against independent native Rust predicates for **17** controls, including helpers and loops: **4,352** checked inputs. It is enabled by `SPUR_R6_EXHAUST=1`; the documented final run enables it. The ordinary target invocation does not repeat that expensive loop. This is bounded acceptance evidence, not a claim about arbitrary Rust. R8 still owns full regression acceptance.

## SOLVE PRE → POST

Catalog discovery and the typed-language contract are in `catalog-language.json`. The implemented family rules selected are `workflow.initial_state_allowed`, `workflow.transition_allowed`, and `workflow.bounded_reachability`. They own the finite outcome reachability checks. No catalog family represents the actual bool/u8 SSA expressions, arithmetic, helper substitution or loop templates; those use the existing typed generic language. No family formula is duplicated as generic constraints.

Fresh PRE ran before test/production edits: five exactly preflighted generic feasibility/counterexample requests plus two family controls, all persisted and reloaded. Representative IDs:

- u8 feasibility: `sol_3ba855f3bc024a89` (SAT).
- checked-add witness x=255: `sol_3becefefb90443ea` (SAT).
- guarded checked-add: `sol_a1092e42fc9f45e2` (UNSAT).
- mutation/shadow witnesses: `sol_177d2d54f2104e8d`, `sol_168046eee3a74823`.
- family reachable/unreachable: `sol_3a2beb320ad44d50`, `sol_e0119f811cea436f`.

POST contains **47** persisted/reloaded receipts: **26** actual emitted typed panic queries, **4** feasibility controls derived by removing only the final panic assertion, and **17** catalog queries over the independently enumerated native outcomes. Feasibility remains SAT for checked-add safety, singleton false bool, constant helper arguments, and the complete safe loop. Incomplete helper/loop queries remain raw UNSAT plus report obligations; their classification is never promoted to complete safety.

`solve-manifest.json` lists all **54** PRE/POST IDs, statuses, exact request digests and local artifact paths. Every generic execution passed its exact `solve_constraint_check` first. Each artifact retains execution arguments, preflight, response and `get_solve_result` reload. Minimal PRE arguments reload with the service's published serde defaults. Family receipts reload the catalog compiler's typed request; the family envelope and native normalization are retained alongside it. No `.spur/solver` files were inspected.

The family normalization is an exact abstraction of all enumerated outcomes: one trace step means a complete function evaluation for the fixed finite input domain, with an enabled Panic/Return edge iff that outcome occurs among the 256 native inputs. It is not a claim that a function has a one-instruction source trace or unbounded reachability.

`exported.json` holds the current source/facts/reports; `post-requests.json` records the emitted and executed requests for equality checking. `operational-controls.json` retains the five controlled raw outcomes. Independent Python verification recomputes source OIDs, all report/request/receipt hashes, domains, candidate predicates and hard constraints, then checks the persisted/reloaded receipts and catalog model constraints. Library integration-test receipts use temporary stores and are reloaded through `SolverService`; durable task-level receipts are the IDs in the manifest.

## Retrieval, scope, and review

Both required graph-first attempts failed: orientation could not prepare the analyst query, and exact symbol search reported no graph artifact. `startup.json` records the failures. Raw fallback was bounded to the supplied R4/R5 modules, typed solver interfaces, scoped tests and retained local evidence. No graph publication/rebuild was attempted.

`unified.diff` is the source-only unified diff (zero context) against the dispatched baseline. Only the five allowed implementation/test/manifest files and this R6 evidence directory changed. No notebook, original CGA checkout/driver/launcher, global config/install, upstream message, historical evidence, or protected user file was changed. Root remains responsible for source/build authorization, notebook verification and final plan approval. Remaining source-model/authority assumptions and intentional unsupported frontiers are explicit in each report.
