# D7B ROOT acceptance

ROOT reviewed canonical `bd-2bur`, retry `a0fcbcae99f14159`, normalized source `7dd41f0c9cb4d9851d2f1093cbc607ecaac2c718`. Authored TDD history remains protected at `spur/review-evidence/g3-d7b-attempt2-authored` (`bda42d6bbfb34ca6a663f828de95f3f6160c7a62`). The production repair changes only the shared reader: it removes untrusted footer row-group exclusion and retains actual-path filtering over frozen compressed bytes.

The exact worker diff matches 239 paths, 33,764 additions and 18 deletions against dispatch base. Relative to the recovery checkpoint, production changes are confined to `store/rust_evidence.rs`, plus one new test file. Original eight tests and the previous six tests are unchanged. ROOT independently verified packet consistency and Beads completion audit 4265, including successful persistence/replay; reconciler completion 4268 is also present.

## Native review

ROOT used independently supplied D5 artifacts, disks, path/OID filters and footer mutations, rather than only repeating worker-selected expected values. The replay checks 812 source/config/fixture hashes and seven native executable hashes. Builder configuration matched the recorded identity. All complete inputs, requests, actual outputs, logs, scripts and raw artifacts are retained under `native/`.

Nineteen native invocations executed **28 tests: zero failed, zero ignored, 666 filtered**. They include the original eight query tests, ROOT's zero-source regression, all three new nonzero regressions, an independent frozen-IO replay and arbitrary input hooks.

- Footer-only path-statistic changes preserve the complete source, body, site and declaration projections for path-only and matching-OID queries.
- Filtered-out body/source corruption is rejected before selection. Rehashed wrong-existing source and owner substitutions are rejected by semantic validation, rather than just a checksum comparison.
- Whole-file ordered groups and referenced whole origins match independent input-artifact projections. Healthy zero, cfg/recovery/invalid-source uncertainty, declaration-only uncertainty, legacy absence and captured-empty domains remain distinct.
- An actual bound physical notebook-like filename retains its known unavailable source entry. Unbound virtual spelling is unsupported. Custom backends retain an unknown domain.
- Warm contexts survive replacement, deletion and current-source edits. An additional D5 `main.rs` witness deletes all five evidence tables; the existing context retains its full result and a new context rejects the missing tables.
- Existing-context evidence-read and byte deltas after replacement and deletion are zero. Cold-client reads are accounted for separately.

The default replay and arbitrary-input records are normalized only for typed error categories whose diagnostic paths differ. Full raw diagnostics remain available. `prepare_post.py` independently derives exact expected file groups, source identities, availability and whole origin inventories from supplied artifacts and the frozen public contract. It also checks the D5 corpus against independent source bytes, Git blob OIDs and declared row counts. Equality is checked before hashing the finite solver observations.

## Fresh Jev POST

Fingerprint `496f2772bb55b35bb5c5e730f24678f925615be0c0b324be742a435f6d1e48b6` binds 2,524 paths, 31 positive observations and six changed negative claims. Jev returned an open gate for the intended tuple-membership verification; ROOT inspected its complete facts, mode and empty unknowns, then added catalog uniqueness and exact cardinality.

- `sol_e06ef3ba3f124ded`: SAT/pass, first native projection partition.
- `sol_a2cf50454f504fd8`: SAT/pass, second native projection partition.
- `sol_130a7a08361541c0`: SAT/pass, footer preservation.
- `sol_7abda142f628414e`: SAT/pass, domains, frozen contexts, integrity and IO.
- `sol_914b8a5a51e94c8e`: UNSAT/fail, false zero/complete/stale/integrity/freeze/IO claims.

All five complete receipts were persisted, pinned and reloaded. The independent evaluator checks all 15 fixed-input and lowered predicates. The consumed rework PRE and approved D7A POST were fully archived, exactly reloaded and independently checked (27 predicates) before releasing their pins. Current POST receipts remain pinned for D8.

Run `python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/ROOT-acceptance/verify_post.py`. Later phases may verify this consumed source epoch with `prepare_post.derive(source_revision=<accepted ROOT commit>)`; they must not claim that future changed source bytes match this snapshot.

## Explicit limitations and disposition

The worker required-suite evidence remains exact: query targets 18 passed; parity seven passed; query-client selection 26 passed with 1,432 filtered; all-target compilation, scoped formatting and diff checks passed. The macro target has **13 passed and one known baseline failure**: `parquet_macro_queries_reject_missing_source_manifest_rows`. Its stale-source fixture fails before the macro query on all 803 exact accepted predecessor inputs. ROOT accepts that demonstrated pre-existing fixture limitation for this bounded repair; no assertion was disabled or fixture blessed.

The default remote-sync compile failure and successful full-sync retry are both preserved. ROOT's earlier follow-up upload-guard failure is also preserved separately in `ROOT-review/final-regression-upload/`; this retry genuinely reproduced the final regression before its fix, so that operational failure is not substituted for TDD evidence.

D7B is ready for ROOT approval and local integration. D8 overlay composition, D9 capability-aware cache identity and D10 fresh Rust notebook validation remain subsequent work. This acceptance establishes the tested stored-query and provenance contract. It does not establish current-source syntax authentication, call-target correctness or complete panic detection.
