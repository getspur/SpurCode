# D1 worker handoff for ROOT review

D1 implements the standalone `RustEvidenceSet` contract, origins, checksum,
per-file call uncertainty, stored validation and canonical source recollection.
The implementation is ready for ROOT review. Fresh persisted/reloaded POST,
ROOT's independent mutations and final Jev interpretation remain ROOT-owned.
No worker approval, issue closure, merge, push or successor dispatch occurred.

Canonical issue **bd-1nsu** remains **in_progress**, assigned to
`spur-worker-9788a184983f459d`, delegation `9788a184983f459d`, plan
`787e2ce9-7a7d-4b7e-b6d5-950136bcb83b`, task
`D1-rust-evidence-contract`. `bd-2kfw` is lineage only. Dispatched base is
`de7747d7b4155cfdafa747ab2681d8fff581c1c8`. Final Rust/test source checkpoint is
`962862425`; later evidence commits do not change Rust/test source bytes.

## Entry and preserved PRE

`entry.json` retains the exact UUID INTENT request and persisted audit response.
Audit `431b289f-1f23-48ab-9221-e083ae220271` persisted as comment 3846; exact
replay returned `idempotent=true` and the same comment. Plan status confirmed D1
dispatched with no dependencies and the matching delegation/assignee.

Before any test/implementation edit, the required command actually exited 0:

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/root-pre/verify.py
```

It reported fingerprint
`1e6338133c7572de5625d6f18e24d030b47c88bc22cd2e8f39d8bc7230986dcc`, 63 bindings,
six fixtures, nine receipts (four SAT/five UNSAT), 27 predicates, exact generic
lowerings/models and open/inspected Jev. All nine IDs in ROOT's README were
reloaded through `get_solve_result`; every complete envelope was JSON-equal to
the corresponding `proof.json.gz` `reloaded` record. `pre-live.json.gz` stores
those exact live results. Audit comment 3847 records this comparison.

PRE is a prospective finite contract, not wrapper output. Its source hashes
intentionally precede implementation; it was not rerun or rewritten to certify
changed implementation. The diff from the dispatched base leaves all ROOT PRE
and historical artifact bytes unchanged.

## Reachable TDD and results

| Checkpoint | Actual observation |
| --- | --- |
| `cfdfbe5e6` tests, `fedee16b4` evidence | New-API **compile RED**, exit 101/E0432 missing D1 exports; zero runtime assertions executed. |
| `5e06b4184` implementation | First runtime run: eight passed, zero failed/ignored/filtered. |
| `3013194c9` regressions, `651dd883a` evidence | Runtime RED: nine passed/two failed. Blanket FFI range rejection dropped canonical signature constants; a nested body's definition could reference the existing outer function. |
| `144175327` fix | Eleven passed, zero failed/ignored/filtered. Preserves R3 signature constant bodies and checks the exact function-body end for definition provenance. |
| `bcd1e5742`, `3464b4539`, `4f397d5b7` | Full non-Rust historical origin inventory, multiline tuple/method-FFI parity, direct authentication cleanup and external JSON API replay. |
| `385777ad0`, `962862425` | Added rehashed source/owner controls. `final-green.log` honestly records a **probe expectation error**, 13 passed/one failed: the first adversary duplicated an owner ordinal and stored validation correctly rejected it. Corrected the adversary to swap both owners while retaining ordinals; no production relaxation. |
| `final-verified-contract` | **14 passed, zero failed/ignored/filtered**, including seven independent JSON replay cases. |
| `final-existing` | R3: 10 passed/zero ignored; declarations: six passed/one ignored; foreign: five passed/one ignored. Zero failures/filters. Both ignores are unchanged opt-in frozen-export tests. |
| `final-all-targets` | `scripts/spur-cargo check -p spur-graph --all-targets`, exit 0. |
| `final-fmt` | `scripts/spur-cargo fmt --package spur-graph -- --check`, exit 0. |

Each named run has its exact command, exit, environment, source SHA-256 values,
toolchain and available executable SHA-256 values in `*-command.json`; raw
stdout/stderr is in `*.log`. `final-verification.json` collects final checks.
The contract run additionally set `D1_REPLAY_INPUT` to this directory's
`independent-replay-input.json` (the command metadata's environment subset does
not include that optional variable). Rustc was 1.94.1, host aarch64-apple-darwin.

All builds used `scripts/spur-cargo`, `SPUR_REMOTE=0`, `TMPDIR=/private/tmp` and
`CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`, default features,
with `INSTA_UPDATE`/`SPUR_GRAPH_BLESS` unset. Process inspection found no active
Cargo/rustc writer before the first run; commands were serial. About 10 GiB
remained available. No cache deletion, native/ORT repair, snapshot blessing or
new ignore/skip was used. Existing unrelated dead-code/bench-import warnings
remain in logs. The eight historical opt-in native failures are separate.

## Actual output and independent projection

`actual/actual.json` contains the real current graph and wrapper collected from
the six exact hex-inventory fixture bytes, with production file/symbol/body/site
IDs and the envelope checksum. It retains six source entries, three complete
call files, three typed unknown call files, three main-file bodies and two
distinct marker-owned sites. Recovery retains its intact tuple row. FFI retains
one foreign function, one nonexecutable block and zero local executable bodies.

`project_actual.py` independently verifies fixture hex/SHA1/SHA256, expected
status/counts and marker/brace coordinates. It joins production IDs by trusted
graph path and unique source coordinates, then compares ownership. It never
uses PRE's prospective binding labels as production IDs. Its successful output
is retained in `projection-verification.json`.

`post-candidates.json` contains independent expected and actual source/site
relations, actual body/declaration/origin rows, FFI and empty controls, mutation
boundaries and suggested catalog rules. These are fresh POST **candidates**,
not fabricated solver requests, receipts or Jev results. ROOT must discover the
current exact encoding and persist/reload final POST before approval.

`actual/mutations.json` retains 17 complete changed envelopes and exact validator
errors. `actual/owner-mutations.json` retains four wrong-existing structural or
lexical owner mutations. `actual/rebound.json`, `actual/rehashed-forgeries.json`,
`actual/strict-decoding.json`, `actual/captured-empty.json` and
`actual/composed.json` retain the other controls. Every canonical mutation is
asserted to change the original value. The rehashed-owner control keeps stored
shape consistent, forges both graph parents/ranges and recomputes the graph
artifact body hash; source recollection still rejects it.

Stored checks reject missing/duplicate sources, stale bindings, invalid origins,
headers, counts, IDs, metadata bounds, actual graph owners and FFI body tags.
Stored-only success remains possible for recounted syntax omissions, an outer
existing body assignment, same-length ABI text and cfg Unknown forged as zero;
source validation rejects each. That boundary is intentional: stored shape
cannot authenticate unseen syntax. `actual/packet-replay.json` records seven
independently generated JSON cases through the actual current public APIs.

## Exact current-code replay

Run from the assigned checkout, with no other Cargo writer. The helper runs the
full contract suite, exports actual data and records source/executable identity:

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/worker/run.py root-replay
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/worker/project_actual.py
D1_REPLAY_INPUT="$PWD/docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/worker/independent-replay-input.json" \
  python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/worker/run.py root-independent
```

ROOT can replace that input with independently mutated JSON. The packet is
`{"cases":[{"name":"...","graph":<actual graph>,"set":<wrapper>,
"decode_ok":true,"stored_ok":false,"sources_ok":false}]}`. The test always
uses the six frozen exact source bytes. Direct focused probe command:

```sh
env -u INSTA_UPDATE -u SPUR_GRAPH_BLESS SPUR_REMOTE=0 TMPDIR=/private/tmp \
  CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target \
  D1_REPLAY_INPUT=/absolute/path/to/root-mutated-packet.json \
  D1_EXPORT_DIR="$PWD/docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/worker/root-replay" \
  scripts/spur-cargo test -p spur-graph --test rust_evidence_contract \
  replay_wrapper_packets_through_current_public_validators -- --exact --nocapture
```

The focused probe intentionally filters the other 13 tests; the recorded final
GREEN ran all 14 without filters. Notebook/Parquet replay belongs to D10.

## Scope and remaining boundary

Production changes are only `src/rust_evidence.rs`, `src/lib.rs` and crate-private
visibility of the unchanged `snapshot_id` algorithm in `panic_evidence/call_sites.rs`.
Tests are only `tests/rust_evidence_contract.rs`. Original declaration collectors
remain byte-identical; the body/site hash algorithm body is byte-identical.
`scoped.diff.gz` is the complete unified Rust/test diff from the dispatched base.
Scoped source `git diff --check` passed. Raw evidence logs preserve their exact
trailing blank lines and are excluded from the source whitespace check.

No GraphIndexArtifact member, artifact capability digest, builder, persistence,
query/cache or Cargo change is included. Other checkouts, five dirty main files,
old notebooks/evidence, the notebook graph/CURRENT and installed runtimes/models
were not edited. No cfg evaluation, expansion, compiler/ABI linkage, execution
or whole-panic completeness claim is made. `ParseUnavailable` has a typed
no-tree path; no parser-unavailable fault was induced in these runs. ROOT's fresh
POST/Jev and independent final review remain outstanding.
