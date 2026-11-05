# D1 independent ROOT review

D1 passes review of its standalone per-file envelope and source/stored validation contract. This is separate from D2's artifact carrier and later durable transport.

ROOT verified live canonical issue `bd-1nsu` was in progress without unresolved signal labels. INTENT 3846, exact PRE replay 3847, compile RED 3848, verification/handoff 3849/3851 and completion 3853 match the implementation. The normalized tree `123588093` equals authored `36d8690ab`; the original test-before-implementation and regression-before-fix histories remain protected by `spur/review-evidence/g3-d1-authored`.

The worker changed exactly four Rust/test paths plus its permitted evidence. ROOT inspected all wrapper code and tests, checked final source hashes and raw runner summaries, and confirmed the declaration collectors and snapshot ID algorithm are unchanged. The helper's visibility changes only within the crate. Worker evidence reports 14 D1 cases, 21 existing cases, two named pre-existing export ignores, all-target check and scoped formatting passed. New-API compile RED and two later behavioral assertion failures remain distinct. The first rehashed-owner adversary had an expectation error; it was corrected without weakening production validation.

## Fresh current Rust replay

ROOT appended the archived `compatibility-probe.rs` only in the isolated review checkout, built it through `scripts/spur-cargo` using the existing authorized local target, and restored the worker test bytes afterward. All 19 controlled malformed-tuple/cfg/constant/FFI/deferred/Unicode cases were accepted by the original declaration collector and preserved exactly by the wrapper. Healthy R3 sets matched unchanged. No compatibility case was skipped.

`combined-review-test.rs` retains the exact compiler input including that extra test. The frozen native executable and eight original source hashes are in `native-identity.json`. A fresh complete run passed **15 tests, zero failures/ignores/filters**. Its actual wrapper and graph are value-equal to the worker's output. The replay writes only `actual-current/`, leaving worker evidence immutable.

ROOT separately mutated the fresh actual output and ran **13 cases** through the real public Rust APIs. They cover omitted zero sources, duplicate records, another existing source OID, same-file existing outer-body assignment, cfg Unknown forged as complete zero, recounted unused-body omission, consistently rebound headers with an invalid original inventory hash, same-length ABI substitution, a wrong typed uncertainty reason, foreign body category, strict enum payload decoding and forged body identity. `mutations-execution.json` records the focused runner's one passed test and fourteen filtered unrelated tests; the preceding complete run remains unfiltered.

Stored acceptance for same-length syntax changes, a recounted omission or a containing existing body is an intentional boundary. Exact source recollection rejects them. Checksums and graph membership do not authenticate unseen syntax. The runtime packets verify these boundaries rather than treating every shape-valid record as canonical.

## Fresh POST/Jev

Fingerprint `7181c8fa84b166d967676909b95dd05dcc63965187bb7d38eab441cbbe8d6abc` binds 83 current source/evidence files. The projection independently derives expected source OIDs/counts/statuses, marked call/body coordinates, exact FFI fields and raw metadata, original source inventories and runtime verdicts. Actual IDs are joined through authoritative graph/source coordinates; expected ownership never copies an observed site's body assignment. Oracle labels from PRE are not production IDs.

Jev opened at **0.99**. ROOT inspected exact facts/family/verify mode/tuple subjects/no unknowns, then added catalogued uniqueness and cardinality bindings. Nine receipts were persisted, pinned and exactly reloaded; independent complete lowerings, models and finite predicates passed: **four SAT, five UNSAT, 27 predicates**.

| Case | Result | Receipt |
| --- | --- | --- |
| actual_sources_0 | SAT | sol_6f78f04110af412b |
| actual_sources_1 | SAT | sol_37f8dc34ebe94b93 |
| actual_sites | SAT | sol_4caf616b341a4ce3 |
| actual_header | SAT | sol_c22c20ae1999463a |
| wrong_existing_body | UNSAT | sol_cb43651ea1c24af8 |
| unknown_as_zero | UNSAT | sol_d0a19721a0ce4281 |
| missing_zero | UNSAT | sol_eb438dbb74bf4b5a |
| duplicate_source | UNSAT | sol_870525bc2ee24835 |
| rebound_origin | UNSAT | sol_63dff164ce3640df |

Before releasing retention, ROOT exactly matched nine already-closed C PRE receipts against live envelopes and independently checked their 27 predicates. Only their pins were released; complete archives and operations are retained. Both D1 PRE and POST remain pinned. Solver storage and quotas were not edited.

Reproduce the frozen actual API run with `python3 -B .../D1/ROOT-review/run_actual.py normal`; regenerate and run independent packets with `make_packets.py` and `run_actual.py mutations`. The existing frozen binary must still match its recorded hash. Offline verification is:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/ROOT-review/verify_post.py
```

The six exact source entries retain a healthy zero, three typed unknown call lanes, three main-file bodies, two distinct call sites and three declaration records. Complete-empty capture is distinct from absent capability. The real parser-unavailable fault path was not induced. No performance, compiler cfg/expansion/dispatch, resolved ABI/linkage, execution or complete panic claim follows. D1 adds no artifact field, persisted tables, query/cache behavior or dependency change. ROOT approval/integration and D2 PRE are recorded separately in Beads.
