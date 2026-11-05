# D2 artifact carrier handoff

Canonical issue `bd-37hc`, delegation `bde831461a9b443d`, exact base `67a08c435651f450982c66b837794c277bcda043`. Leave the canonical issue in progress. ROOT owns independent fresh POST/Jev, approval and integration.

The pre-edit ROOT verifier exited 0 with fingerprint `fa6d0778fc9d69cb0d44c2004949d2e42a435a7efae77d1de039c15625a9f03e`, 68 bindings, six receipts (two SAT/four UNSAT), 18 independent predicates, 64 real literals in 32 files and Jev open/inspected. All six complete live envelopes exactly matched the saved proof, including lowered requests/status/models. `entry-audit.json` records INTENT persistence and identical replay (`3871`, idempotent true). `PRE-verification.json` preserves the initial verification. The prospective predecessor verifier is not rerun or rewritten for the changed source and is not implementation POST.

History:

- `8a16e2aa5`: existing-API JSON regression. The authorized scoped wrapper run compiled and failed the assertion because the strict decoder rejected unknown `rust_evidence`: 14 passed, one failed, zero ignored/filtered, exit 101 (`RED-local.txt`). No new API import was needed for this failure.
- `01c6c4bb2`: broader new-API tests, separately observed compile RED (21 missing-field/helper/method diagnostics, exit 101; `API-RED-local.txt`). This does not replace the behavioral RED.
- `f11face44`: minimal optional field, actionable unavailable/source validator, full-envelope canonical hashing, documented capability digest and origin-view clearing, plus exactly 64 mechanical `None` additions. Builders remain unavailable until D3.

`scope-verification.json` accounts for all original literal sites. Removing only the inserted `rust_evidence: None,` lines restores every nonsemantic file byte-for-byte to the dispatched base. Semantic changes are confined to schema, canonical hashing, Rust evidence helper/view clearing and the contract integration test. The frozen canonical snapshot, schema/extractor/resolver, dependencies, collectors, source graph-hash semantics, protected main files, old evidence/notebooks and live external graph/CURRENT are unchanged. `D2-scoped.patch` is the unified Rust diff from the exact base.

Verification:

- `scripts/spur-cargo test -p spur-graph --test rust_evidence_contract`: 18 passed, zero failed/ignored/filtered, exit 0 (`GREEN-local.txt`).
- `scripts/spur-cargo test -p spur-graph store::canonical_hash::tests`: five passed, zero failed/ignored, 1369 filtered across 110 test-binary summaries, exit 0 (`canonical-hash-local.txt`). The original canonical snapshot passed without blessing.
- `scripts/spur-cargo fmt --all -- --check`: exit 0 (`fmt.txt`).
- `scripts/spur-cargo check --workspace --all-targets`: the first attempt exited 30 during rsync before compilation. The next upload completed but the wrapper rejected a changed checkout revision (exit 1, before compiler) because this evidence checkpoint was committed during upload. Both failures are retained. The final retry held the checkout revision stable and **passed, exit 0**, on the default remote provider (`workspace-all-targets-final.txt`). Cargo finished all targets in 5m 37s; total wrapper time including replacement-builder startup/upload was about 910 seconds. No local workspace result substitutes for this remote pass. Preserve both logs and see `verification.json`/the retry command record for final status.

Scoped local tests used the explicitly authorized existing target with `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`, after confirming no other local Cargo/rustc writer. `INSTA_UPDATE` and `SPUR_GRAPH_BLESS` were unset. Existing graph dead-code warnings are retained verbatim. The remote attempts use normal cloud defaults with local fallback disabled. The first unprivileged RED launch could not reach AWS (exit 200); its log is retained. The network-enabled duplicate was stopped during source sync after the scoped behavioral RED was obtained; its abandoned exact-task receivers were diagnosed/terminated before the required workspace retry. No remote test failure was replaced with a local pass; no remote test compiler ran in those launch/sync attempts. Recovery records name only the exact task receivers. No cache cleanup, runtime/dependency/native-link change or infrastructure source edit was made.

Actual current Rust output is under `actual-current/`. Fresh collection from the accepted D1 producer inputs exactly matches ROOT's accepted D1 graph/envelope and complete-empty capture. The exports contain actual None, Some(empty), populated and self-header-only artifacts, full-envelope checksums, canonical hashes, capability digests and source verdicts. Five `*.canonical.json` files retain the exact independently encoded body bytes (no added newline); the production canonical hash equals BLAKE3 of each byte sequence. The independent legacy layout preserves every existing field and its order. `actual-body-bindings.json` binds lengths/SHA-256/order/value checks. The normal suite tests absent versus captured empty, unchanged legacy bytes, self-header invariance, manifest/version identity, actionable unavailable errors, and a manifest-reproducible capability encoding.

`actual-current/mutations.json` records 265 current typed envelope mutations with exact pointer, before/after value, actual canonical hash/checksum/capability digest and source error after rehashing the artifact header. The mutations cover wrapper/nested headers/versions, original source inventories, availability/uncertainty, source IDs/lengths, structural/lexical owners, declarations, body/site identities/text/ranges, FFI/body/ABI/qualifier/visibility/raw metadata and complete group omissions. Each changes all three identities and source validation rejects it. The one-variant physical-source scope enums have no alternate typed variant. Macro fields and source graph-content hashes remain unchanged. These are concrete runtime cases, not a claim of whole-program syntax, execution or panic completeness.

`native-identity.json` binds exact current source/test/Cargo/wrapper SHA-256, the actual compiled executable SHA-256, toolchain and compile command/environment. The executable is frozen in a private temporary directory, not installed. The test source contains the producer and public-API packet replay. To rebuild exports in an isolated checkout with the already authorized local target:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/worker/run_check.py GREEN-rebuilt --local test -p spur-graph --test rust_evidence_contract
```

ROOT can independently edit `actual-current/api-replay-input.json` (sources plus named artifact cases), then pass its new path. Optional `decode_ok`/`sources_ok` booleans assert the independent expected verdicts; actual hashes/checksums/digests/body bytes/source errors are exported regardless of expected source acceptance:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/worker/replay.py /absolute/path/to/ROOT-packets.json
```

The supplied canonical packet was actually replayed through the frozen current Rust binary: one passed, zero failed/ignored, 17 filtered, exit 0. `replay-command.json` and `replay-current/api-replay-output.json` retain that run. A packet replay is distinct from the complete unfiltered 18-test contract run.

Offline checks:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/worker/verify.py
```

ROOT still must independently mutate current output and persist/reload fresh actual-output POST receipts and Jev. No D2 worker POST/Jev approval, persisted tables, transport, query or notebook result is asserted. D3-D9 remain untouched.
