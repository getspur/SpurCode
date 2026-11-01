# R5 attempt 3 — lexical and trait dispatch authority

The collector now shares one lexical helper gate between calls and argument inference, and retains a caller/site obligation when Option autoref dispatch is uncertain.

Issue **bd-280** remains **in_progress** for root review. R6 remains dependent on accepted R5. Graph schema v11, the existing sidecar schema and public API are unchanged. No snapshots were updated.

## Source and history

- Preserved attempt-2 authored tip: `spur/evidence/r5-attempt-2-authored`, `e4ad738d81fd0560bd3208d65c951d39e6ca68cd`.
- `40d29971b`: recover the pre-applied attempt-2 source/tests and historical evidence byte-for-byte.
- `faae96af8`: committed runtime RED for all five exact root controls, with module/trait variants and positives; fresh PRE evidence.
- `5a8a18170`: committed runtime RED for incorrect contextual helper argument typing.
- `0e5c55565`: minimal dispatch authority fix.
- `0c0064432`: ordinary-Rust-valid positive helper control (`254+1`).
- [implementation.patch](implementation.patch): unified diff against recovered attempt 2. Only `semantics.rs` and `panic_semantics.rs` have new source changes. `mod.rs` and `semantic_facts.rs` retain the reviewed attempt-2 bytes.

All historical R1/R2/R2C and R5 attempt-1/2 evidence is preserved. The exact five review programs are retained in `review-module-cases.json` and `review-trait-cases.json` and checked against fresh collector inputs.

## Behavior and coverage

The helper catalog owns root functions in each physical source. `root_helper` requires the caller function's AST parent to be `source_file`, an unqualified identifier, one catalog target and no intervening lexical binding. Both `call` and `contextual_u8` use this predicate. An inline/nested module or nested function cannot borrow a root helper's signature or body. Unsupported calls retain invocation-local Callee obligations.

`Option::is_some` takes `&self`; a trait method taking `self` can win first. This source lane conservatively refuses the presence shortcut when the source contains a trait declaration or trait implementation. The opaque call taints live values and retains uncertainty in the affected caller at the exact invocation span. Imports and unexpanded macros already prevent primitive/prelude authority. Compiler dispatch evidence remains R7's separate lane.

Tests cover all five exact review programs, renamed/raw/nested module controls, a qualified module call, module/nested-function argument typing, and default, Option-blanket, all-type-blanket, raw, local and nested-module traits. Root helpers, ordinary Option presence, scalar arithmetic and explicitly typed arrays remain supported, along with all 50 previous exports and their independent oracles.

## Runtime RED and GREEN

All Cargo commands use the project wrapper with `SPUR_REMOTE=0`; tests/check use `CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5`.

```sh
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics dispatch_ -- --nocapture
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics dispatch_contextual_argument -- --nocapture
```

RED exit **101** in `red.log`: **7 assertion failures, 2 passes**. Each of the five exact review programs independently fails the missing caller/site obligation assertion. RED exit **101** in `red-contextual.log`: the unsupported lexical helper supplied a false u8 argument type. These are runtime assertion failures, not compile failures.

```sh
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end -- --nocapture
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r5-attempt3-fmt/Cargo.toml -- --check
git diff --check
git diff --cached --check
```

Final GREEN exit **0**, **93 tests**: 30 collector tests and 63 affected existing tests. Crate check and scoped fmt pass. Check reports only the pre-existing unused `FileOidMatch::as_bool` warning. `format-manifest.toml.txt` preserves the temporary format manifest. Final staged/unstaged diff checks and scope/history checks are indexed in `verification.json`.

## Independent finite and ordinary Rust oracles

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/verify_rework_base.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/project_arrays.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/verify_dispatch.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/merge_dispatch_queries.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/project_ownership_base.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/project_dispatch_ownership.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/verify_receipts.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-3/verify_artifacts.py
```

Finite and ordinary Rust results agree on **22,285 inputs**: 12,544 historical, 4,876 attempt-2 and 4,865 new dispatch inputs. Emitted concrete semantics match on **15,876 inputs**; **6,409 inputs** belong to refused controls whose obligations remain explicit. The 20 new programs comprise four concrete positives and sixteen unknown controls. Unknown is never counted as a safe execution or an exact value prediction.

The independent finite IR interpreter and hand-authored oracle functions are reused without changing historical evidence. `verify_rework_base.py`, `project_arrays.py` and `project_ownership_base.py` are copies of the reviewed attempt-2 scripts, writing only here. `verify_dispatch.py` replays every new program through ordinary rustc with edition 2021, unwind and overflow checks. All exact programs, commands, compiler output, runtime output and hashes are retained in the three runtime-command inventories. `export-stability.json` confirms all **70** saved exports equal the latest test exports. Generated facts are never collector inputs.

The initial added positive `work(255+1)` is rejected by ordinary Rust's constant-overflow lint. The diagnostic remains in `oracle-dispatch-initial.log`; the corrected `work(254+1)` has fresh passing tests, export, Rust replay and POST equations. This is a source correction, not a blessed snapshot.

## Fresh SOLVE PRE / POST

Catalog discovery preceded RED and production edits. The implemented **data_integrity.mutually_consistent** rule owns finite source/site/lexical ownership tuples. Typed bool/u8 evaluation equations have no matching catalog family and use `solve_constraints` after exact `solve_constraint_check` preflight.

**6 PRE + 154 POST accepted receipts**, persisted and reloaded: **60 SAT, 100 UNSAT**, no unknown or timeout verdicts. [receipt-verification.json](receipt-verification.json) indexes every ID and checks all reloads. Generic persisted requests equal the exact preflighted requests; catalog receipts retain high-level facts and the compiler's named lowered binding.

PRE examples: `sol_fd82f6e491b34df8` (ownership/refusal SAT), `sol_4ca536ca15cd4883` (wrong module owner UNSAT), `sol_9adb7600ed7d4272` (trait presence substitution UNSAT), `sol_9fec495eef6549cb` (by-value hidden panic SAT).

POST comprises **131 generic queries** expanded from fresh exported origins, guards, operations and loop spans, plus **23 catalog checks**. Request inventories retain exact source OIDs, body/site identities and span bindings. The new ownership lane verifies all **40** new exported site tuples and all 20 affected dispatch decisions. Five-column site rows use five-row batches; four-column decisions use seven-row batches, staying within the live 64-variable compiler limit.

Representative POST IDs: `sol_fd3ef9f2d0c24eab` (root helper oracle equivalence UNSAT), `sol_0b5e5c24eb9e4c0a` (caller/site obligations SAT), `sol_6ad78d5963614c04` (exact wrong root-helper target UNSAT), `sol_bb8d79279a1f4eb1` (trait presence substitution UNSAT), `sol_90a6526c32714e9e` (detached obligation UNSAT), `sol_d7ed8512172f4f4d` (forged span UNSAT).

`post/` retains accepted requests, preflights, responses and complete `get_solve_result` reloads. `solver-batch.js` records the tool orchestration. The initial inventory read was truncated before any solve; transport subsequently omits redundant span bindings while the inventories retain them. A first four-request catalog batch reached a shell argument limit when writing envelopes locally. Its unsaved preliminary envelopes are not counted as accepted evidence; the exact requests were replayed, persisted, reloaded and saved through `apply_patch`. `operational-diagnostics.json` records both failures. No failed write or validation error is treated as a solver verdict.

## Assumptions and unresolved obligations

This is a physical-source bool/u8 model with declared build settings, not compiler-attested Rust resolution. rustc 1.94.1 on aarch64-apple-darwin replayed the controls. Call depth is below the independent oracle's bound of eight; finite loop controls use their literal source ranges. These oracle bounds do not discharge R6 exploration limits. Unsupported module/nested-function callees, competing trait dispatch, aliases, opaque effects, unsupported types/macros/cfg and LoopBound obligations remain explicit. R6 owns exploration, R7 compiler evidence and R8 complete regression acceptance.

Graph orientation and exact symbol lookup failed (`discovery.json`), so bounded raw fallback was confined to scoped R5 files and local review/evidence inputs. No notebook, graph CURRENT/publication, original CGA producer/driver/launcher, global configuration, upstream messages or unrelated user files were changed. Logs with trailing whitespace or EOF blanks are normalized only after lossless archival of their exact raw bytes and SHA-256 in `raw-log-archives.json`.
