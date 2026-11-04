# R5 attempt 2 — source authority rework

Primitive and lexical source authority now prevents default integers, expanded aliases and shadowed helpers from producing incorrect concrete panic facts.

Issue `bd-280` remains `in_progress` for root review. R6 remains pending. Graph schema v11, the sidecar schema and public API are preserved. No snapshots were updated. Full source and solver evidence remains local.

## Authored history and diff

- Preserved authored output: `spur/evidence/r5-attempt-1-authored`, `0c27271bfb43a58d5fd2b42c7a0c2db5ccd62389`.
- `06987cb00`: restore the pre-applied attempt-1 output, byte-identical to the authored source/tests and historical R5 evidence.
- `bc2615a94`: initial runtime RED tests and fresh PRE evidence.
- `552db9507`: separate assertions for both helper counterexamples and the range-comment counterexample; expanded runtime RED.
- `c0eba8c9a`: source authority fix.
- [implementation.patch](implementation.patch): unified diff against the recovered attempt-1 output. Source changes are restricted to `semantics.rs` and its integration test.

## Behavior

Raw identifiers are normalized at bindings and references, including helper and primitive namespace lookup. Imports and unexpanded items/macros cannot establish primitive authority. Numeric operations require a proven operand or a source context that establishes `u8`; an if condition does not inherit its result type. Arrays need explicit element authority and literal ranges need an actual `u8` endpoint. Generic type arguments and constructor paths are parsed from tokens excluding comments. Hoisted lexical items prevent selecting an outer helper. Unknown lexical values retain obligations on use. Comments cannot change an exclusive range to inclusive.

The eight exact root counterexamples and ten renamed/whitespace/namespace variants now retain non-LoopBound obligations. This includes the raw-function shadow that really panics; unknown is never a safe outcome. The four required positive programs and nine further typed/raw/comment controls remain supported. All 19 historical oracle programs retain their expected ordinary Rust behavior.

## Commands and results

All Cargo invocations use the project wrapper with `SPUR_REMOTE=0`. Test and check commands use `CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5`.

```sh
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics review_ -- --nocapture
```

Runtime RED exit **101**: initially nine failed test groups (`red-cached.log`); expanded eleven failures (`red-expanded.log`), including an independent assertion for every root counterexample. These are assertion failures, not compile failures. The first uncached build was interrupted during dependency compilation (exit 130; `red.log`) and is not RED evidence. `green-first.log` and `green-namespace.log` retain intermediate failures before unsupported lexical reads received obligations.

```sh
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end -- --nocapture
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r5-attempt2-fmt/Cargo.toml -- --check
git diff --check
```

GREEN exit **0**, **84 tests** (21 R5 and 63 affected existing tests), crate check, scoped fmt and diff checks. The temporary format manifest is preserved in `format-manifest.toml.txt`. The only check warning is the existing unused `FileOidMatch::as_bool`.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-2/verify_rework.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-2/project_ownership.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-2/project_arrays.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/attempt-2/verify_receipts.py
```

The finite IR interpreter and ordinary Rust agree on **17,420 inputs**: 12,544 historical and 4,876 rework inputs. Thirteen rework programs are supported and eighteen retain unknown obligations. Rust commands, full control source, stdout and compiler details are preserved in the runtime command JSON files. `verify_rework.py` reuses the previously reviewed independent interpreter and hand-authored finite truth function without modifying historical evidence; it consumes only fresh test exports. No generated facts are collector inputs.

## SOLVE PRE / POST

Fresh catalog discovery preceded RED. The implemented `data_integrity.mutually_consistent` rule owns source/site and lexical ownership tuples. Uncatalogued typed integer/bool equations use `solve_constraints` after exact `solve_constraint_check` preflight. No unknown/timeout result is treated as proof.

**7 PRE + 131 POST receipts** were persisted and reloaded: **49 SAT, 89 UNSAT**. [receipt-verification.json](receipt-verification.json) indexes every ID, expected result and reload check. PRE includes overflow/wider-integer/short-circuit and lexical positive/adversarial controls, plus exclusive/inclusive range membership controls. Representative PRE IDs: `sol_377e69496ff8465d` (overflow SAT), `sol_47472bb795944707` (wider UNSAT), `sol_451594d39d2b4edf` (lexical refusal SAT), `sol_9efaa653a88744ae` (wrong owner UNSAT).

POST consists of 123 generic equations derived from fresh exported origins, guards, operations and loop spans, plus eight catalog ownership checks. `post-requests.json` and the ownership request inventory retain the source OIDs/body IDs/span bindings. `project_arrays.py` lowers the two typed array selections to finite equations from the actual exported elements. Witness and oracle-equivalence checks include positive and adversarial controls. Every generic request has its exact preflight, response and `get_solve_result` reload under `post/`; catalog checks retain both high-level facts and the compiler's reloaded typed binding.

The first combined site-ownership request exceeded the live 64-variable compiler limit. Two validation errors, not solver verdicts, are retained in `post-ownership-initial.json` with their original requests. Each site row requires eleven variables, so five-row batches verify all 22 exported sites without weakening the allowed tuples. The forged-span and wrong-raw-outer-target controls are UNSAT. Final receipts are in `post-ownership-receipts.json`.

## Boundaries and remaining obligations

This remains a declared physical-source bool/u8 model, not compiler-attested Rust name resolution. Unsupported type authority, macros, imports, aliasing and lexical effects retain uncertainty. The independent finite controls cover bool/u8 domains and the supplied finite loop ranges; they do not establish behavior for arbitrary Rust. R6 owns bounded exploration and LoopBound discharge; R8 owns complete regression acceptance. Historical R1/R2/R2C evidence and all attempt-1 receipts remain unchanged.

Both graph orientation and exact symbol lookup failed; `discovery.json` records the bounded raw-source fallback. No notebook, graph publication, global configuration, upstream communication or unrelated user-file edits were made. `verification.json`, `build-environment.json` and `hashes.json` provide the machine-readable review index.

The final harness log had a blank EOF line rejected by staged diff checking. Its exact raw bytes and SHA-256 are preserved losslessly in `log-archives.json`; only trailing EOF blank lines are normalized in `green-affected.log`. Exported facts and test outcomes are unchanged.
