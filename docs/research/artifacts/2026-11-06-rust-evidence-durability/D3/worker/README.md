# D3 current full and selected Rust capture

Canonical assignment: `bd-mq2h`, delegation `facbbe046c5e44e3`, dispatched base `e30cc8b266d93f208b42223679e32e79b4f83277`. The canonical issue stays `in_progress` for ROOT's independent POST/Jev and review. No approval, closure, merge, push or successor is performed here.

History: assertion RED commit `40ff0aea5` precedes implementation `62d494f26`; native evidence is committed at `4320dee73`. `implementation.diff` is the zero-context unified production/test diff against the dispatched base (`git diff --unified=0 BASE HEAD --` the four graph paths). Semantic changes occupy only `store/build.rs`, its reexport, the overlay builder call, and the new integration test. Existing collectors, versions, dependencies, canonical hashing and origin validation remain authoritative.

The full builder attaches `collect_rust_evidence` after composing real owners/manifests. It reads exact eligible physical Rust bytes and checks every source blob OID. Errors propagate. The selected builder normalizes and checks requested paths, discovers only their current entries, restricts macro carriers and graph composition, then collects. Overlay extraction routes to that API. No whole-root discovery is used by the selected builder.

## Pre-edit transaction

ROOT's unchanged PRE verifier exited 0 with fingerprint `9ea37e195982bf582303ffdb83eb1fbeaba44dd69f4ed8483b4bfe39ca8d6ae8`, 68 bindings, 8 receipts (3 SAT/5 UNSAT), 24 independent predicates, Jev open/inspected. Every current receipt was retrieved through `get_solve_result`; complete envelopes equal `root-pre/proof.json.gz`. The tool responses are retained in `entry.json`. `verify_entry.py` replays those captured envelopes; it does not claim another live lookup. The prospective PRE source fingerprint is intentionally stale after implementation and is never presented as POST.

INTENT, RED and implementation audits are persisted and exact-replayed on the canonical assignment: comments 3909, 3910 and 3911. Requests and idempotent service responses appear in `audits.json`. Native packet progress was persisted and exact-replayed as comment 3913. Final handoff comment 3914 is retained with both complete service responses in `handoff-audit.json`; the final canonical status was independently reloaded as `in_progress`, with the matching assignee/delegation and no signal labels.

## Native behavior and verification

`RED.txt`: the existing-API test compiled, then failed exactly at `value["rust_evidence"].is_object()`: 0 passed, 1 failed, 0 ignored/filtered, exit 101. `API-compile-RED.txt` is the later new-API compile failure, kept separate.

`GREEN.txt`: 14 passed, 0 failed/ignored/filtered, exit 0. This includes 13 behavior tests and the configurable packet replay entry (which returns when `D3_PACKET` is unset). `GREEN-regressions-final.txt`: 8 macro tests passed; 5 foreign-evidence tests passed and the existing opt-in frozen export test was ignored; zero failures/filters, exit 0. There are two existing dead-code warnings. `FORMAT.txt` and `DIFFCHECK.txt` both exit 0.

| Source | Native declaration lane/rows | Native call lane/bodies/sites |
| --- | --- | --- |
| cfg.rs | complete / 0 | Unknown unevaluated_cfg |
| empty.rs | complete / 0 | complete / 0 / 0 |
| ffi.rs | complete / 2 (foreign function + extern block) | complete / 0 / 0 |
| invalid.rs | Unknown / 0 | Unknown invalid_utf8 |
| main.rs | complete / 0 | complete / 3 / 2 |
| recovery.rs | Unknown / 1 intact tuple | Unknown parse_recovery |

Automatic full evidence equals the complete accepted D2 manual wrapper, including original inventories, IDs, ABI, coordinates, bodies and sites. Selected and overlay artifacts have exactly `empty.rs` and `main.rs` in both manifests and evidence. Empty workspace is `Some` with zero files/origins. Separate cases cover partial facts without fabricated unselected zero coverage, normalized duplicates and absolute paths, mixed notebook/Python sources, macro-carrier restriction, directory I/O errors, symlink escape rejection and missing owners. A tracked assume-unchanged fixture forces a stale index OID versus freshly read source bytes; the builder rejects it with the actual error in `actual/oid-race-error.json`.

The default initial test command could not connect to either AWS endpoint inside the sandbox and fell back to a fresh local target. That fallback was interrupted (exit -2) before a graph compiler/test result. `RED-remote.txt` preserves the entire output. Focused tests then used the explicitly authorized `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`; disk and single-writer checks preceded execution. A first formatting invocation supplied `--edition` twice and exited 1 with `Option 'edition' given more than once`; the corrected crate-scoped formatting/check commands passed. `GREEN-attempt-1.txt` used a relative export path resolved by Cargo beneath the crate; those generated outputs were relocated into this worker directory, and the final export helper resolves relative destinations against the repository root.

`CHECK-remote.txt` is the first network-enabled all-targets check attempt. It spun up the AWS builder and completed sync, then stopped before compilation because evidence commits changed the source revision/state during upload (exit 1). This is an upload-integrity rejection, not a compiler result. The retry uses a stable committed tree and an out-of-worktree live log; guards are unchanged.

`CHECK-remote-stable.txt`: the same remote `scripts/spur-cargo check -p spur-graph --all-targets` passes, exit 0, against stable checkout `b454cd7a5`. Production/test/query/manifest/wrapper source bytes equal the native producer bindings. The cold AWS compiler check finished in 1m 50s. Existing library/test/bench warnings and aggregate sccache statistics remain in the raw log. No remote compiler/test failure was replaced by a local pass.

The complete base-to-HEAD diffcheck found two whitespace-only context lines inside the stored three-context unified diff (not production source). Its exact stdout/status are archived in `diffcheck-context-artifact-failure.json.gz`. Regenerating that evidence artifact with `--unified=0` preserves a valid unified patch without whitespace-only context lines; the final complete scoped diffcheck passes. No ignore, fixture change or check suppression is used.

## Actual outputs and arbitrary packet replay

`actual/{full,selected,empty,overlay,mixed}.json` contain complete native artifacts, source hex/SHA-256/Git blob OIDs/lengths, native canonical body hashes, envelope checksums, capability digests and source-validator results. The builder's body-hash header remains writer-stamped as before; exported canonical hashes come from the current native API. `actual-rows.json` and `source-inventory.json` are derived tables over those outputs. `native-identity.json` binds the exact executable SHA-256, Rust/Cargo toolchains, compiled source commit and 364 current source/query/manifest/wrapper files.

Run from repository root:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/worker/replay.py \
  docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/worker/actual/full.json \
  --output docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/worker/root-replay.json
```

The input may be any original or forged packet with `artifact` and `sources` (each source has `path` and `hex`). The adapter authenticates current source/executable/toolchain bytes, invokes the compiled Rust test's current APIs, and saves the full result and raw command/output/status log. `source_error: null` means native source validation accepted. Strict decoding failures exit nonzero and retain raw output. Each focused replay runs 1 test with 13 unrelated tests explicitly filtered.

`native-controls.json` records 5 accepted originals and 11 rejected mutations. Every typed mutation was replayed after stamping the actual native canonical body hash into its self-header; the hash equals the supplied header and source validation still rejects. Controls remove/duplicate healthy zero, fabricate cfg as zero, leak an unselected source, change supplied bytes/OID, remove tuple/foreign owners, omit a site with a consistent count, choose a wrong existing body and forge the ABI literal. Graph-owner/source-only mutations need not change envelope checksum/capability; their exact native values are retained. No hash is represented as source authentication. The malformed first ABI control used an invalid enum shape; `controls/malformed-abi_forgery.*` preserves its exact input and decode failure before correction.

Regenerate derived actual-output candidate data:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/worker/prepare_post_candidate.py
```

`post-candidate.json` is grounded in the actual outputs and independent fixture hash joins. It is explicitly a worker candidate for ROOT's fresh source-bound catalog POST/Jev, not a solver receipt or independent approval.

## Limits

Full facts must go to the full builder; selected facts use the selected builder. `build_facts_for_paths` already skips nonexistent requested paths, and the selected builder likewise emits no active source for them. This does not implement deletion tombstones or change D8 behavior. The tests do not infer compiler-grade named-definition completeness: optional R3 references remain optional, while tuple/foreign structural owners remain mandatory.

This stage claims current full/selected capture only. Incremental original-bucket reuse, persistence/publication, queries, caches, Cargo cfg evaluation, macro expansion, compiler dispatch, execution/ABI guarantees and complete panic detection remain downstream or outside scope. Protected main files, old evidence/notebooks and the live external graph/CURRENT were not edited.
