# R4 attempt-3 recovery verification

The structural validator now requires one sort for each `(body_id, binding_id)` across all versions, preserving repeated versions, same-sort domain narrowing, distinct shadow identities and separate body scopes.

This is a bounded repair of the retained R4 implementation. Graph schema v11 is unchanged. `PanicBuildConfig` and `PanicSemanticFacts` remain the separately versioned, serializable public contract. R5 owns canonical automatic source translation, R6 consumption and independently expected build identity, and R8 complete regression acceptance. The authored fixture and structural oracle do not establish generic Rust semantic coverage.

## Recovery and TDD chronology

1. Confirmed actual issue `bd-1f8` was `in_progress`, assigned to `spur-worker-6d2c9e4564f14d6d`, and plan `04f4a253-7e5d-46f3-99ba-afcd3f016f85` had R4 attempt 3 dispatched to delegation `6d2c9e4564f14d6d`. The worktree was clean at approved R3 `4417055a4e48278a3d9a58b717cad4f32d789a91`; no R4 overlay was supplied. Assignment was checked again at the GREEN/POST boundaries.
2. Verified all 51 frozen input files against `/private/tmp/r4-lease-recovery-a30640d0/frozen/manifest.json`. The retained branch ended at `c30b497a7913d5f02d93f188831a0974a79a3e17`. Its sort-change tests were committed, but its eight-line validator fix was uncommitted; the preceding attempt expired before accepted completion. The frozen patch also corrects a body-span selector in the positive control (`{}` is the R3 executable-body range).
3. Before restoring or modifying source/tests, performed fresh catalog recovery PRE using `data_integrity.unique` and `data_integrity.mutually_consistent`. Eight exact requests were persisted and reloaded: four positive controls and four adversarial controls. `pre-results.json` contains the complete requests, responses and reloads. Cache-hit flags are retained; these were fresh tool invocations, not replayed archived responses.
4. Restored scoped retained history, preserving author/test-before-fix chronology: `e7e5c92c9 → 1308b018b`, `ba98f6a90 → 87e6bcea1`, `9ae8345e9 → ad0ad15ef`, `c30b497a → 055d80f65`. These restored files were verified byte-identical to the retained Git objects before RED. Historical evidence was not presented as newly executed verification.
5. Newly executed runtime RED against that unfixed snapshot: four assertion failures, exit 101. Both U8→Bool and Bool→U8 were wrongly accepted across expression, assignment and unknown origins; the parameter/phi control also failed. `red-source-sha256.json`, `red-outcome.json` and `red.log` bind the exact source and command. Recovery PRE and new RED evidence were committed as `97f71ed06` before restoring the fix.
6. Restored the three scoped Rust files from the immutable capture. The actual delta was the eight-line sort check plus the positive-control selector correction. Fresh focused GREEN passed 11 tests; affected GREEN passed 63 total tests (11 contract, 10 call-site, 23 compiler-evidence, 9 end-to-end, 10 method-site). Crate check and scoped formatting passed. Fix commit: `0b32c998e`.
7. Serialized fresh actual exports through the running shipped contract tests and projected 27 models into 81 bounded POST requests. Every request was executed, persisted and reloaded. Archived POST receipts were not counted as fresh verification. All 81 returned the expected pass/fail results; none returned unknown or timeout.
8. `verify_recovery.py` independently checked 89 PRE/POST receipts, 97 finite predicates, SAT models against the reloaded lowered constraints, every binding/version/sort projection against the fresh serialized facts, the separately authored expected-sort oracle, eight explicit-uncertainty completion controls, and all 51 archive hashes. It also verifies the shipped source hashes equal the frozen capture. See `solve-summary.json` and `verification.log`.

The original R1/R2/R2C/R3 and R4 evidence remains intact. `historical-partial/` is a separate, byte-identical copy of all frozen attempt-2 inputs, including its manifest, patch, source snapshot, logs and partial POST material. No old live worktree was accessed.

## Exact fresh commands

All commands ran in this assigned attempt-3 worktree. The existing target cache avoided a duplicate cold build.

```sh
# Newly executed RED, exit 101: four runtime assertion failures.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract rejects_binding_sort_changes -- --nocapture

# Focused GREEN, exit 0: 11 passed.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract -- --nocapture --test-threads=1

# Affected GREEN, exit 0: 63 passed.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end -- --nocapture --test-threads=1

# Check and scoped fmt, both exit 0.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r4-attempt3-scoped-fmt/Cargo.toml --check -- --config skip_children=true

python3 docs/research/artifacts/2026-11-01-panic-implementation/R4/attempt-3/project_post.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R4/attempt-3/verify_recovery.py
```

The isolated formatter manifest is retained as `format-manifest.toml.txt`. The only build warning was the existing unused `FileOidMatch::as_bool` at `crates/spur-graph/src/mcp/mod.rs:908`. Local toolchain: rustc 1.94.1, host `aarch64-apple-darwin`, wrapper-reported Cargo 1.94.1.

Fresh raw command output is retained losslessly in `command-output-raw.json`; readable log copies only trim terminal blank lines as recorded in `log-normalization.json`. Captured historical bytes are unchanged. The default full `git diff --check 4417055a4` identifies two preserved historical logs with blank final lines (`historical-partial/.../attempt-2/green.log` and `green-affected.log`). The source diff and new recovery artifacts are checked separately; this archival lint exception is explicit. `unified.diff` is a context-free unified diff of the three scoped Rust files against approved R3.

## Solver binding and limits

Selected catalog rules are `data_integrity.unique`, `data_integrity.foreign_key`, `data_integrity.mutually_consistent`, `data_integrity.conditional_required` and `data_integrity.value_range`. Catalog responses are retained in `catalog.json`; fresh PRE is in `pre-requests.json` / `pre-results.json`; fresh POST is in `post-requests.json` / `post-results-0.json` through `post-results-3.json`. All receipt IDs and cache flags are indexed in `solve-summary.json`. No fresh generic encoding was needed, so a generic `solve_constraint_check` preflight was not applicable.

Representative reloaded PRE IDs: repeated U8 versions `sol_9b2b2a61984c4a58`, distinct shadows `sol_f3a63249399b424f`, body-local namespace `sol_818fc380039d452d`, U8→Bool rejection `sol_c85d8269214c43f9`, Bool→U8 rejection `sol_f4166537f42d484c`, duplicate version `sol_c7eba25071b048c2`. The complete eight-ID PRE list and all 81 POST IDs are in the summary; positive actual-export binding-sort POST is `sol_7fbaa9e639334cbe`.

The binding-sort POST projection retains body, binding, version and sort together, even when one predicate consumes fewer columns. Expected sorts come from `binding-sort-oracle.json`, authored from the fixture declarations and named structural-control intent. Body IDs are resolved through canonical R3 body/source evidence, never inferred from the value rows under verification. Repeated same-binding versions are legal; uniqueness is enforced on `(body, binding, version)`, not binding alone. The independent verifier also groups actual rows by `(body, binding)` to check stable sorts directly.

The fixture source SHA-256 is `d6fc88283b07c214a934520c58d256b18291d69aa27c45cdda321b7076db7dcb`; its semantic build hash is `sha256:7011a67078f7428f17d810b1faf3e194f1bb8b75aca4c71eb33858fb140e5ad3`. Shipped source hashes are in `green-source-sha256.json`. The fixture declares target `aarch64-apple-darwin`, edition 2021, explicit empty cfg/features/flags, overflow checks and debug assertions enabled, and unwind panic strategy. This is a structural fixture build declaration rather than a real-build attestation. Domains are bool/u8; fixture exploration limits remain call depth 1, loop iterations 1 and 32 states. The finite test projection uses integer schema bounds 0..512 and 20-second solver budgets; it makes no unbounded semantic claim.

Fresh orientation and exact symbol retrieval both failed (analyst query failure and missing graph artifact respectively), recorded in `recovery-start.json`. The bounded raw fallback used only scoped source/tests, immutable Git/archive inputs, the Cargo wrapper and local evidence. No graph CURRENT/publication, notebook, original CGA or unrelated user file was changed.

## Tracker obligation

`report_audit(task_id=bd-1f8)` and the corrected compact `report_signal` request were rejected with `target is not assigned to this delegation`, despite repeated authoritative issue/plan checks confirming the current assignment. Compact exact requests/errors are retained in `recovery-start.json` and `tracker-errors.json`; accepted `report_progress` events notified the root. No full source/solver payload was retried through another channel. Root must record the durable review from this committed local evidence. Leave `bd-1f8` in progress and R5 gated pending root approval.
