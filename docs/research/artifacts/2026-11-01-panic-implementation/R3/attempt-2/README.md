R3 attempt 2 repairs phase ownership so invoking an async closure cannot stand for polling its body, and array-repeat count calls cannot stand for runtime element evaluation.

Assigned issue **bd-2uw** remains in_progress for root review. R2C is approved; R4 remains pending. No acceptance or plan status is changed here.

Compact completion audit **1325** (`e19c1d92-47f8-4329-9320-f14b3d8d4805`) was persisted and idempotently reloaded on bd-2uw. Exact compact requests/responses are in `audit-completion.json`.

The first staged whitespace check found terminal blank lines in the two new GREEN logs. Their exact raw output and before/after hashes are retained in `log-normalization.json`; only terminal blank lines were trimmed in the readable log files. The complete committed diff check additionally found trailing whitespace in the copied root MIR hex dump and a terminal blank line in root's submitted log. `root-review/raw-outputs.json` preserves their exact original bytes and hashes; the readable mirrors are trimmed. `root-review/sha256.json` continues to identify the original inputs. Final staged, unstaged and complete committed diff checks pass. `check-final.log` repeats the successful crate check after the documentation-only clarification.

The public `RustBodyKind::AsyncClosure` variant distinguishes async and async move closures, including expression bodies. Their exact body ranges exclude the async/move/parameter creation prefix. `call_body` treats the `array_expression.length` field as a separate `ConstExpression` root while leaving the element in its actual enclosing body. Compound lengths share their complete length-expression context. Closure, async-block and async-closure element contexts, type/generic/item constant barriers, and ordinary runtime calls to const functions remain distinct. The collector records no execution edges. Canonical validation still recollects every field.

The existing method-site API, graph schema v11 and R3 sidecar v1 contract are preserved. This repairs the pre-acceptance R3 implementation; no schema-version migration is introduced. Body/site identifiers remain source-snapshot-bound, with no cross-edit stability promise.

The orchestrator supplied attempt-1 source/evidence as staged changes. Commit **8c7833bf0** preserves those bytes. An ancestry-only merge **3c9b25452** attaches the original authored chain `8592cdbb3 → 41847e8af → a5396564d → 3fb472ac4 → 2daca9480` while preserving the recovered tree. All original R3 evidence stays in the parent directory. R1/R2/R2C and protected files are untouched.

Fresh catalog discovery and 34 PRE receipts preceded new project tests or collector edits. The independent input fixtures are `async.rs.txt` and `repeats.rs.txt`; `oracle.py` derives their exact body/site ranges and explicitly authored ownership/kind expectations without reading collector output. `pre-oracle.json` is this source-derived input oracle, not a generated collector golden.

Commit **03bd8c8c2** adds the regression tests after PRE. The genuine assertion RED is preserved in **fdd525037** and `red.log`: `closure != async_closure`, `Function != ConstExpression`, and accepted forged ordinary-closure metadata. All seven existing tests passed in that RED run; three new tests failed. No compile failure is claimed as runtime RED.

Commit **22be68bbc** implements the minimal repair (19 added / 2 removed production lines). The final evidence commit adds only the evidence files and a clarifying `ConstExpression` doc comment. `repair.diff` is the unified Rust production/test diff against the recovered attempt-1 baseline. No original implementation diff or evidence was replaced.

All Cargo commands ran locally through the required wrapper. The existing scratch target is a build cache; Cargo freshly compiled this worktree. From the worktree root:

~~~sh
# Runtime RED: exit 101, 7 passed / 3 failed; red.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites -- --nocapture

# Focused GREEN: exit 0, 10 passed; green-focused.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites -- --nocapture

# Affected GREEN: exit 0, 88 passed; green-affected.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end --test rust_lexical_calls --test rust_receiver_resolution -- --nocapture

# Crate check: exit 0; check.log
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph

# Scoped formatting: exit 0; fmt.log / fmt-check.log
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --config skip_children=true crates/spur-graph/src/panic_evidence/call_sites.rs crates/spur-graph/src/panic_evidence/mod.rs crates/spur-graph/tests/panic_call_sites.rs
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --check --config skip_children=true crates/spur-graph/src/panic_evidence/call_sites.rs crates/spur-graph/src/panic_evidence/mod.rs crates/spur-graph/tests/panic_call_sites.rs

git diff --check
git diff --cached --check

# Independent ordinary Rust/runtime/MIR controls: exit 0; compiler-controls.log
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/compiler_controls.py

# Retained export/receipt and preservation checks: exit 0; verify*.log
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/verify.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/verify_preservation.py
~~~

Suite counts are 10 / 23 / 9 / 10 / 15 / 21, all zero failures. The pre-existing unused `FileOidMatch::as_bool` warning is retained. `compiler/commands.json` records exact ordinary rustc commands, toolchain and results. The two new source fixtures compile as ordinary Rust. Three async closures (including move and expression forms) return futures without polling or entering panicking bodies; the ordinary closure executes. Root's runtime control also succeeds. `compiler/repeats.mir` separately exposes runtime `length()` / `value()` calls and CTFE length contexts. Root review inputs and their exact SHA-256 hashes are copied under `root-review/`; the originals were not changed.

Fresh SOLVE uses the implemented `data_integrity` family, profile `finite_relational_snapshot`, with `unique`, `conditional_required`, `mutually_consistent`, and `cardinality`. `catalog.json` retains full selected-rule discovery. Family validation owns preflight/lowering; no generic constraints were authored, so generic `solve_constraint_check` is inapplicable.

There are **68 fresh receipts: 34 PRE + 34 POST**. Every solve used `persist=true`, `include_smt=true`, and a 20-second budget; every result was reloaded through `get_solve_result`. Full exact requests, returned envelopes/SMT and reloads are in `pre-*.json` / `post-*.json`; `receipt-index.json` lists every ID and receipt hash. No `.spur/solver` file was read. Each phase has 19 positive checks and 15 controls. Wrong async kind, wrong repeat kind, wrong same-file owners, forged source, missing body/site identity, missing owner, duplicates and omissions fail. NULLS DISTINCT-only missing-key controls intentionally pass, demonstrating why uniqueness alone cannot certify completeness. No unknown/timeout is interpreted as proof.

Representative PRE → POST receipts:

| Check | PRE | POST |
| --- | --- | --- |
| Async body tuples | sol_daa75cd40eb64dbe | sol_9426c0b85fd74681 |
| Repeat call tuples | sol_03aad4eccda74fc0 | sol_37fb10a6cfa043fd |
| Wrong async kind (fail) | sol_63d90fa420144d9b | sol_271068aaedfe4e0b |
| Wrong repeat owner (fail) | sol_24f3483ad1a043f1 | sol_54fac991a5fe46c0 |
| Missing site identity (fail) | sol_2ba4988c8fa8433c | sol_0ee7ef034d8a43d1 |
| NULLS DISTINCT only (pass) | sol_0337576ebde34148 | sol_9db0d1e7fdc34b1e |

POST uses the actual `R3_REWORK_OBSERVED` outputs from the affected GREEN run: 6 bodies/5 sites for async closures, 15 bodies/17 sites for repeat contexts. Expected source bytes/ranges/kinds/owners still come from the independent input oracle. Expected opaque IDs are joined by exact oracle ranges to the actual exported IDs. Observed rows retain actual exported source/range/kind/body/site/text values; they are not replaced by PRE fixtures. Identity inventories cover each complete relation; exact-tuple/presence batches cover every row within the 64-variable per-request limit. `verify.py` checks every export against the independent oracle, verifies POST row provenance and coverage, and checks all 68 receipt/reload pairs. It does not invoke a solver. The first verifier run exposed an absent `model` field in an UNSAT reload; `verify-first.log` preserves that helper error. The helper now compares optional models correctly; `verify.log` passes.

The original attempt-1 export check still passes. Its old receipt helper correctly reports current-source hash drift after the requested repairs; `historical-checks.json` records that result without rewriting historical hashes. `verify_preservation.py` checks the historical hashes against the recovered snapshot, all 34 retained receipt pairs, original authored commit ancestry, and scoped paths. The new source/export manifest is `source-hashes.json` in this directory.

Orientation and exact graph queries failed afresh; requests/errors, assigned issue status and R2C approval are retained in `retrieval.json`. Bounded raw fallback was limited to the assigned source/tests/evidence and provided root diagnostics. No notebook access, graph publication/CURRENT changes, global installs/configuration changes, upstream messages or original CGA edits occurred.

These are finite physical-source syntax checks under the supplied exact bytes and parser/toolchain, not typed execution reachability or universal semantic/collision proofs. Macro expansion and unevaluated cfg remain outside the evidence lane's guarantee; parse/cfg uncertainty rejects collection. Complete regression acceptance remains R8's responsibility; architecture, notebook verification and approval remain root's responsibility. The compact completion audit on bd-2uw links this directory and commits; full source/solver payloads remain local.
