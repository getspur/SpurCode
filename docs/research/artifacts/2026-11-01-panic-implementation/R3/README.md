R3 adds a separately versioned source-bound call/body sidecar so repeated calls and deferred bodies retain precise identities without inferring execution from containment.

Assigned issue: **bd-2uw** (in_progress for root review). Plan: 04f4a253-7e5d-46f3-99ba-afcd3f016f85, task R3. The historical source issue bd-35k is lineage only. R2C was verified approved before editing; see retrieval.json. Dispatched base: fdf610a4495c4b436258538f9b337075cd8cb25d.

## Delivered contract

Public RustBodyEvidence, RustBodyKind, RustCallSiteEvidence, RustCallSourceBinding, RustCallSiteEvidenceSet and collect_rust_call_sites are exported from panic_evidence. The independent sidecar versions are panic-rust-call-sites-v1 / rust-direct-call-bodies-v1. The existing MethodCallSiteEvidenceSet API and graph v11 schema are unchanged.

Every intact direct identifier/path or method call in the supplied physical Rust files has an invocation identity, exact source OID/path/range/text and innermost syntax body identity. Functions, nested functions, async functions/blocks, closures, const/static initializers and type/item constant expressions have distinct contexts. Body ranges cover the executable block/expression, excluding function signatures and creation prefixes. Empty explicit bodies are retained. Body creation supplies no execution edge. Optional named definition IDs require exact file, name, kind, definition byte/line ranges and unique graph identity.

Identifiers hash the exact Git blob OID, file path, byte range and domain/kind using length framing and BLAKE3. They are snapshot-bound; even an edit after an unchanged call changes its identity. No cross-edit stability is promised.

Validation recollects and compares every field, including canonical ordering, versions, bindings, body ownership and optional graph definitions. It rejects stale inputs, forged same-file owners, omissions and duplicates. Parse errors and cfg/cfg_attr attributes (including raw identifiers) reject collection. Macro definitions/invocation token trees remain opaque. Identifier call syntax does not establish the callee's type, dispatch or execution.

## TDD and commits

- 8592cdbb3: failing API contract tests plus persisted/reloaded PRE evidence.
- 41847e8af: retained compile RED and runtime RED, with the temporary empty-collector API shape in runtime-red-api-shape.rs.txt.
- a5396564d: raw cfg regression assertion RED.
- 3fb472ac4d0c56163c4a36c79cc676f6cf31f871: minimal source-bound implementation and uncertainty-export instrumentation.

The first cold local build was interrupted with exit 130 to avoid duplicating a large target directory. This is not a RED result. The reused-target compile RED exited 101 with missing public Rust call/body API errors (E0432/E0425; cascading E0282), recorded honestly in red-compile-reused.log.

After adding only the API shape and an empty collector, the held-out test produced the intended runtime assertion failure: **left 0, right 14**, exit 101 (red-runtime.log). The exact temporary API shape is preserved locally. Implementation began after this runtime failure.

green-first.log records an implementation compile diagnostic (named_child index type), not a contract RED. It was corrected by using tree-sitter child iteration. green-second.log records the first 7/7 passing runtime suite.

The additional raw-attribute regression failed with **accepted uncertain source #[r#cfg(...)]**, exit 101 (red-raw-cfg.log). The fix strips raw identifier spelling before checking cfg/cfg_attr. Both raw cfg spellings are now covered.

The direct-syntax test's hand-counted total was corrected from 10 to 9 before the runtime RED/implementation, and committed in 41847e8af. No output snapshot was updated or used as an input.

## Exact commands and outcomes

All Cargo invocations used SPUR_REMOTE=0 scripts/spur-cargo. For compilation after the interrupted cold attempt, CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 reused an existing scratch cache. No original CGA checkout, driver, launcher or configuration was changed.

From the worktree root:

~~~sh
# Initial cold attempt: interrupted, exit 130; red-compile.log
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_call_sites

# Compile RED: exit 101; red-compile-reused.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites

# Runtime RED with API shape only: exit 101; red-runtime.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites held_out_calls_have_individual_ids_and_exact_executable_bodies -- --exact

# Raw cfg runtime RED: exit 101; red-raw-cfg.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites parse_and_cfg_uncertainty_cannot_validate_completeness -- --exact

# Final focused GREEN: exit 0, 7 tests; green-final.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites -- --nocapture

# Final affected GREEN and fresh exports: exit 0, 85 tests; green-affected.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end --test rust_lexical_calls --test rust_receiver_resolution -- --nocapture

# Crate check: exit 0; check.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph

# Scoped formatting: exits 0; fmt.log / fmt-check.log
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --config skip_children=true crates/spur-graph/src/panic_evidence/call_sites.rs crates/spur-graph/src/panic_evidence/mod.rs crates/spur-graph/tests/panic_call_sites.rs
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --check --config skip_children=true crates/spur-graph/src/panic_evidence/call_sites.rs crates/spur-graph/src/panic_evidence/mod.rs crates/spur-graph/tests/panic_call_sites.rs

git diff --check
git diff --cached --check
~~~

Affected suite counts: panic_call_sites 7, panic_method_sites 10, panic_compiler_evidence 23, panic_evidence_end_to_end 9, rust_lexical_calls 15, rust_receiver_resolution 21. All have zero failures. The existing unrelated dead_code warning for mcp::FileOidMatch::as_bool remains; checks exit successfully. No warning suppression was added.

Build toolchain: rustc 1.94.1 (e408947bfd200af42db322daf0fadfe7e26d3bd1, 2026-03-25), aarch64-apple-darwin, LLVM 21.1.8. Source hashes are in source-hashes.json. The pre-existing scratch target is a build cache; Cargo recompiled spur-graph from this worktree before the tests/check.

## SOLVE PRE and POST

Selected implemented catalog rules, all in data_integrity / finite_relational_snapshot:

- data_integrity.unique: invocation and body identity uniqueness with NULLS DISTINCT.
- data_integrity.conditional_required: required identity/source/path/range/body/text fields, plus refusal of cfg sources without evaluation evidence.
- data_integrity.mutually_consistent: exact source/body/site tuples.
- data_integrity.cardinality: exact finite site/body inventory.

No generic constraints or SMT requests were authored. Family validation owns the lowered encoding, so solve_constraint_check is not applicable. pre-oversize.json retains the first request's 64-variable validation rejection; it has no solver verdict. Identity inventories and three-row tuple batches respect that limit without dropping rows or fields.

**34 receipts (16 PRE, 18 POST)** are retained in pre-*.json / post-*.json. Every executed request uses persist=true and include_smt=true; every result was reloaded via get_solve_result. Each file includes the exact family request, complete returned envelope/SMT and complete reload (including the compiler-lowered typed request). No .spur/solver files were read. receipt-index.json lists every ID, selected rules, raw status/outcome, variable count and receipt hash.

Representative IDs:

| Check | PRE | POST |
| --- | --- | --- |
| Body identities | sol_1e45dd671e674b64 | sol_8c41c8ca0f1140f2 |
| Site identities | sol_61aa54105a274e61 | sol_7be28a3fea824955 |
| Missing required identity: UNSAT/fail | sol_13e4fa05ec80415f | sol_139d69da5acc404c |
| NULLS DISTINCT alone: SAT/pass with missing identities | sol_bc0f95a965c64a86 | sol_32e0d3dce88249da |
| Wrong parent body: UNSAT/fail | sol_57b6d9b3d07f42da | sol_ef7ec015156c428a |
| Omission: UNSAT/fail | sol_00b529c52ea743e0 | sol_e60e8a40fcef44f7 |
| Duplicate identity: UNSAT/fail | sol_35faee92c3974e7c | sol_1ad447371eab4774 |
| Unevaluated cfg acceptance: UNSAT/fail | sol_95bc307021e34502 | sol_d061af4095e940e3 |

POST's stale-source control is sol_9d21956f2ea44c10 (UNSAT/fail). Actual cfg refusals are sol_08a787fe0a5f4758 (SAT/pass), using the API's exported accepted=false decisions as inactive evidence rows. That check proves refusal for the six supplied cfg cases, not cfg evaluation or whole-project activation.

PRE used the independent marker/delimiter source oracle in fixture_oracle.py / pre-oracle.json. POST used the actual fresh R3_OBSERVED collector result and R3_UNCERTAINTY decisions from green-affected.log, extracted into exported*.json. Expected body ranges/kinds, invocation ranges/text and source bindings remain independently derived from the held-out source. Hashed IDs are joined to oracle ranges; a wrong collected owner would fail that independent join. request_projection.js documents the finite projection and batching. These are explicit fixture bounds, not universal semantic or collision proofs.

The positive 14-site/8-body inventory passes. Duplicate, omission, missing identity, forged source and wrong same-file owner controls fail. The NULLS DISTINCT-only missing-key request intentionally passes, demonstrating why unique alone cannot establish identity completeness. Combining required fields and exact tuples fails the missing-key control. No unknown/timeout was treated as UNSAT or safety.

Retained-evidence checks (standard library only; do not read notebooks or run a solver/compiler):

~~~sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R3/verify_export.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R3/verify_receipts.py
~~~

Both exit 0. The first checks every source/body/site tuple against the independent input oracle and extracts output from the actual test log. The second checks 34 receipt/reload pairs, bounded variable counts, expected statuses/outcomes and source hashes. It does not rerun Z3 or claim an independent solver proof.

## Review boundary and remaining obligations

implementation.diff is the unified production/test diff against the dispatched base. All edits are limited to the three assigned Rust files and this evidence directory. Historical R1/R2/R2C source/evidence and protected user files are unchanged. No notebook access, graph CURRENT/publication changes, global installs/config changes, upstream messages or original CGA changes occurred. Graph-first failures and bounded raw fallback are retained in retrieval.json.

This is physical-source syntax evidence, not a typed execution graph. Macro expansion, receiver/callee dispatch, cfg/build-context evaluation, complete runtime reachability and cross-edit identity are outside this collector's guarantee. Missing/ambiguous graph definitions yield no optional definition identity. R8 owns complete regression acceptance; root owns architecture, acceptance and notebook verification. This issue remains in_progress for root review; no merge or close was performed.

The compact completion audit was persisted and idempotently reloaded on bd-2uw as comment 1280 (audit c7112504-66bf-4f26-878e-917326fa798a). Its exact compact request and responses are retained in audit-completion.json.
