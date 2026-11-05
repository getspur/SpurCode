# D2 independent ROOT acceptance

D2 passes independent review for its optional in-memory artifact carrier, source convenience validator, canonical body hash coverage and capability digest. Persistence and automatic build capture remain later stages.

ROOT verified `bd-37hc` was in progress without unresolved signals and read INTENT/replay 3871, behavioral RED 3874, implementation 3875, retained infrastructure retries and handoff/completion 3888/3891. The original existing-API test compiled and failed on the unknown field before fix `f11face44`; broader compile RED is separate. Authored `1bbcbef8a` and normalized `748e1f1d4` have identical trees. Original TDD history is protected by `spur/review-evidence/g3-d2-authored`.

All 92 worker paths fit the authorized seam. ROOT independently checked all **64** mechanical `None` additions in **32** files: removing only those lines restores every nonsemantic file byte-for-byte. Semantic edits are limited to the graph schema, canonical hashing, evidence capability helper/origin-view clearing and contract tests. Existing collectors, versions, frozen canonical snapshot, dependencies and source graph-hash meaning are unchanged.

The remote workspace command finished with exit 0; raw output and all source hashes match the final source. The compiled revision `33a9222d7` has identical production/test/Cargo/wrapper bytes. Earlier AWS, rsync and checkout-revision failures occurred before a compiler result and remain explicit; no remote test failure was replaced with a local pass. ROOT reparsed the targeted canonical run: five passed, zero failed/ignored, 1,369 intentionally filtered across 110 summaries. The original snapshot passed without blessing.

## Fresh native replay

The frozen current executable/source bindings were authenticated before execution. A fresh full run passed **18 tests, zero failures/ignores/filters**, and regenerated all **265 typed mutations**. Every mutation changes canonical hash, whole-envelope checksum and capability digest, and source validation rejects it after self-header rehashing. Fresh populated output matches the worker's current producer output.

ROOT constructed **12 independent API cases** over fresh output: populated capture, unavailable legacy omission, explicit null, captured empty and empty absence, self-header-only changes, same-length ABI forgery, manifest identity change, strict envelope/artifact decoding, a genuinely rehashed ABI forgery and changed source bytes with an unchanged artifact. All native verdicts matched. The four focused packet runs each passed one replay test with 17 unrelated tests filtered; the preceding 18-test run was unfiltered.

The rehashed forgery's self-header equals its **actual native canonical hash**, yet exact source validation still rejects it. Changing only supplied source bytes leaves both hashes unchanged and causes source authentication to fail. `valid` in the finite projection means canonical source authentication succeeded; it does not mean complete semantics or panic safety.

ROOT independently packed the legacy body **values and top-level field order** from each input artifact and compared them with the native canonical JSON. The optional member is appended only when present; omitted legacy arrays retain their existing layout. Raw UTF-8 bytes/SHA-256 and native hashes are retained. Python does not reimplement BLAKE3 or claim an independent cryptographic proof. The existing frozen raw-byte snapshot and native BLAKE3 comparisons remain separate evidence.

## Fresh POST/Jev

Fingerprint `ff70836191357574f5afc1b4a854db5797d7118f3466dc543c8509ccbdaead50` binds **96** current source/evidence files. Jev opened at **0.99**; exact facts, verify mode, family, tuple subject and no unknowns were inspected. Catalogued uniqueness and cardinality bindings were added explicitly.

Seven fresh receipts were persisted, pinned and exactly reloaded. Complete lowered requests, models and **21 independent predicates** passed: three SAT/four UNSAT.

| Case | Result | Receipt |
| --- | --- | --- |
| actual_presence | SAT | sol_4e2111b936524e00 |
| actual_hash_source_policy | SAT | sol_6f165178a04d467b |
| actual_stale_source | SAT | sol_931c8f8aa2374c8d |
| absent_as_complete | UNSAT | sol_5937fc046eef4965 |
| present_omitted_from_hash | UNSAT | sol_c96fd43bfe76438e |
| self_header_in_hash | UNSAT | sol_84815d5c7ff34439 |
| rehash_as_authentication | UNSAT | sol_1588c695513c4430 |

Seven already-closed D1 POST records were independently checked and matched against full live archives before releasing their pins. Current D2 PRE/POST receipts remain protected. No solver-storage or quota edits occurred.

Offline verification:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/ROOT-review/verify_post.py
```

`run_actual.py full` replays the authenticated frozen executable; named packet JSON files replay the independent controls. ROOT preserved worker evidence, the five dirty main files, existing notebooks, live external graph/CURRENT and installed runtime/model state. No builder capture, Parquet transport, query/cache feature, cfg evaluation, macro expansion, compiler dispatch, ABI/linkage/execution or complete panic claim is added by D2.
