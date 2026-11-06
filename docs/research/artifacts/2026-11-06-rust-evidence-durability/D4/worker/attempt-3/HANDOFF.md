D4 now treats explicit Git commit pointers as manifest inventory, while authenticating and retaining physical Rust payloads and complete original inventories through composition.

[Complete task unified diff](source.patch) · [Narrow attempt-3 repair](repair.patch) · [Native measurements](measurement-manifest.json) · [Actual native packets](actual-final/).
Both patches use zero context (`git apply --unidiff-zero`); lossless full-context copies are in `source-context.patch.gz` and `repair-context.patch.gz`.
Complete task patch SHA-256: `70d43e48033ff8c6eed958446b55b7c9eb0a31fa6a592ffb2d50378db21719f5`.
Repair patch SHA-256: `7e620d7909024703a6612131a90afa90a68d3a908c76e8bc3c65a6fe9d90d061`.

Canonical `bd-1qn9` remains `in_progress`, assigned to `spur-worker-51a607ef93514249` with exactly matching delegation label and no supersession. The branch UUID is not delegation identity. ROOT owns fresh actual-output POST/Jev, independent review and any approval/integration. This handoff makes no whole-workspace, all-evidence or panic-completeness GREEN claim: the supplemental contract suite below remains RED outside the authorized scope.

Base: `4d4ef0fbdf3a28a9e02e7e44f0b2af3bd5872bcc`. Authorized clean fast-forward recovery: `9c4ff5a61f6c682c6ef326400a75e843fb3b345c`. That checkpoint retains the full reviewed attempt-2 implementation/evidence and ROOT's review/PRE. Protected authored history remains at `spur/review-evidence/g3-d4-attempt2-authored`; no patch reconstruction, cherry-pick, stash or reset was used.

New gitlink regression: `323b10784`. Production/domain repair plus sentinel contract correction: `686333a38`. The new test's extraction-statistics assumption was corrected in `613b884b4`, the final compiled source tip. Entry evidence is `b07d8d326`; initial RED/capture tooling is `b7fa50db1`. Original authored TDD commits `cd2fe87bf`, `a4dc8cf46` and fix `002fd94a7` remain protected in prior history. Delivery evidence is committed after the final source tip.

Before any new test/source edit, `python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/ROOT-review/rework-pre/verify.py` exited 0: fingerprint `af813bd7958ee77eca5b6702ad2c7d2cbf77f2bef432361fe978caf0ceb710d2`, 1,694 source bindings, two receipts (one SAT/one UNSAT), nine independent finite/lowered predicates, including two independently failing negative bindings. Complete live `get_solve_result` envelopes for `sol_db8b59ff16db461c` and `sol_c367b84283bc464a` exactly matched archived `reloaded` records, including requests, models/cores, durations and reasons. See [live reloads](entry-gate/live-receipts.json) and [entry verification](entry-gate/pre-verification.json). Jev's archived gate is open; exact family `data_integrity`, mode `verify`, facts, empty unknowns and exact-boundary rule were checked; unique/cardinality checks are explicit in the receipts. This PRE is prospective, not native GREEN or POST. Original D4 PRE and old D3 POST were preserved without rerunning obsolete fingerprints or rewriting them. This attempt unpinned no receipts.

The real mixed Git regression was committed before production edits and compiled successfully on AWS. It failed at the attempted read of absent `vendor/submodule.rs`: cargo exit 101, 0 passed/1 failed/0 ignored/19 filtered. [Exact runtime RED](runs/RED-gitlink-remote.txt). The only production additions skip explicit `gitlink:` markers in the disk reader and authoritative physical Rust domain, preserving manifest rows, hashes and whole origin inventories. True physical read/OID/owner validation remains active. The original mixed symlink/gitlink/sparse and submodule pointer-change integration assertions were unchanged.

The first complete post-fix run passed extractor/Git/macro but failed the new test's assertion that a pointer change counts as one extracted source. Source inspection showed `ChangedPathDiff` fills both changed-path fields only for `current.extractable`; these fields feed source extraction. `613b884b4` corrects the new assertion to zero/empty while retaining reduced reused-bucket count, changed exact commit pointer/hash, exact unchanged payload/origin, deletion and pruning checks. No further production repair or filtering was used. The exact failed run remains in `runs/GREEN-suites.txt` (exit 101).

| Final execution | Passed | Failed | Ignored | Filtered | Exit |
|---|---:|---:|---:|---:|---:|
| Required evidence build suite | 20 | 0 | 0 | 0 | 0 |
| Required full extractor/core parity suite | 55 | 0 | 3 | 0 | 0 |
| Required full Git incremental suite | 28 | 0 | 1 | 0 | 0 |
| Required macro artifact suite | 8 | 0 | 0 | 0 | 0 |
| Required store build units, complete streamed log | 38 | 0 | 0 | 544 | 0 |
| Supplemental Rust evidence contract suite | 16 | 2 | 0 | 0 | 101 |
| Graph all-target check | — | — | — | — | 0 |
| Scoped format / full diff check | — | — | — | — | 0 |
| Independent native / boundary observers | — | — | — | — | 0 |

The only ignored tests are the three pre-existing extractor cases `full_and_incremental_emit_byte_identical_edges_for_same_state`, `incremental_matches_full_under_edit_sequence`, `incremental_rebinds_call_edge_after_callee_file_changed`, and Git's pre-existing `cross_fs_write_falls_back_to_copy`. No ignores were added or changed. Store was replayed once with streaming because the wrapper's default successful-output tail omitted its executable identity; both 38-test passes are preserved.

All build/test/check commands used `scripts/spur-cargo`, AWS remote-only execution and no local fallback. Exact argv, environment, timestamps, source bindings, statuses and raw output are in [runs](runs/). The final source-binding digest recorded by the launcher is `260504fe192fed3ee18960abbe4909856897a6ebb6218f342f41deada25dcbc5`. The launcher's Rust identity is macOS; the actual remote compiler identity is separately captured: Rust 1.94.1/e408947bf, `aarch64-unknown-linux-gnu`, LLVM 21.1.8. Existing dead-code/benchmark warnings remain.

Remote commands used `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CLOUD_BUILD_SH=/Users/kevintruong/project/SpurCode/scripts/cloud-build/build.sh`, with logs in `/private/tmp/d4-attempt3-logs` until uploads finished. `SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0` retained complete later output. The capture config is an evidence-only file; no workspace Cargo implementation/configuration was changed.

```sh
scripts/spur-cargo test -p spur-graph --test rust_evidence_build gitlink_is_manifest_only_and_preserves_original_rust_inventory -- --exact --nocapture
scripts/spur-cargo test -p spur-graph --test rust_evidence_build --test integration_git_incremental --test macro_invocation_artifacts --test extractor --no-fail-fast --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3/native-export-env.toml
scripts/spur-cargo test -p spur-graph --lib store::build::tests --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3/native-export-env.toml
scripts/spur-cargo test -p spur-graph --test rust_evidence_contract --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3/native-export-env.toml
scripts/spur-cargo check -p spur-graph --all-targets
scripts/spur-cargo fmt -p spur-graph -- --check
git diff --check 4d4ef0fbdf3a28a9e02e7e44f0b2af3bd5872bcc
```

The supplemental contract failures are `d2_absence_empty_legacy_bytes_and_capability_encoding` at line 799 (expects builders unavailable until D3) and `d2_entire_typed_envelope_hashes_and_rehashed_forgeries_reject` at line 929 (compares current automatic-carrier graph with a pre-D3 carrier-absent graph). The test file has identical Git blob `06baf8afb502d38b1538ae60eb290fc94a7d069f` at base, recovery and final source. This is a source diagnosis; its baseline runtime was not rerun and is not claimed GREEN. `rust_evidence_contract.rs` is outside the five authorized paths and was not changed, ignored, filtered or rerun locally. ROOT disposition is required. [Exact risk and evidence](supplemental-contract-risk.json), audit `11b57cb9-0e19-48d3-a891-e99ec9670991`/comment 4031, signal `f42b2352-472a-4b64-94b0-283e2daa0a59`.

The [fresh AWS capture](aws-native-capture.json.gz), SHA-256 `c3bf8003733911740d2bbe86deebefa05b968c61ff6fcbdf0a5f6a7c474519f2`, contains exact text/hashes for 16 native packets, 1,682 source bindings and executable identities. Five executables are explicitly marked as executed in final required suites; one preserved prior RED executable and the supplemental contract executable are separately marked. They are not all represented as final passing binaries. Exact SSM transfer records are in `aws-fetch-trace.json.gz`; [compiler/executable measurements](aws-native-identities.json) and [measurement manifest](measurement-manifest.json) bind the capture.

All original 13 D4 packets were refreshed on final compiled source: full/no-op/edit/delete/readd/pruned-empty, changed/empty/deletion-only private deltas, eight version/absence cases, six forgery controls and the reused-source race. The three additional packets are `gitlink.json`, `truncated-rust.json` and `nonrust-sentinel.json`. The Git packet records actual target commit bytes/OIDs, full/no-op/pointer-change, deletion of the gitlink while its whole original inventory remains referenced, and pruning when the last physical Rust member disappears. Commit object SHA-1 framing is independently checked. Direct validators reject gitlink bytes as physical source. The Rust drift control retains the exact historical `pub fn run() {}\n` → `pub fn` rejection, with a fresh authenticated control; Python retains the generic zero-anchor sentinel behavior.

[Independent observations](independent-native-observations.json), [exact physical source inventories](source-inventories.json) and [boundary observations](boundary-observations.json) verify actual finite output. Native selected origin `5b481c13812750ff507cb15beac54883614b9e9ebac9b142e49d7f28967fc530` differs from aggregate edit hash `b3b8fb772a25d8736dea076fec4dcf7e380a5dcd689f4aa701e6e67ea71bccfe`. Retained whole original inventory canonical-JSON SHA-256 is `e1f0bcd529a5e21842214162560c875e37ac5b1723a30fab284bca5aca2a399f`. The mixed Git payload and whole-origin digests are `c4259339512f32ced619e01441f1ef20cd559454e08bd6ffb86a4c447936d2d7` and `329843319426661a2218c9c9504367bd484f4ebd31c6ca78bb5014b1802b666a`.

All six mutation controls reject source validation and incremental publication both before and after genuine native canonical rehash/stamping: wrong reused OID, swapped existing owners, omitted site, rebound nested header, deleted-site reinsertion and Unknown relabelled complete-zero. The omission and Unknown controls remain stored-shape/FK-valid, demonstrating the need for exact source correspondence. No prospective PRE marker is used as a native hash.

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3/verify-native.py docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3/verify-boundaries.py docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-3
```

Infrastructure history is preserved. Attempt 1's ENOSPC INTENT UUID was not replayed under this delegation. This attempt's first ff hit sandbox `ORIG_HEAD.lock` EPERM; the identical explicitly authorized ff succeeded with metadata permission. The sandbox remote RED attempt exited 200 before compilation because AWS endpoints were blocked. Its network-enabled retry launched `i-01d560dbad5bbb79c`, synced the stable committed tree and produced the genuine exit-101 runtime RED. No remote test failure was relabelled by a local rerun. An invalid initial `fmt --test` invocation exited 2 without edits; proper scoped formatting followed. The initial risk-signal request exposed a tool schema omission (`signal_id`); the complete UUID-bearing request then persisted. No authorization guard was changed or repeatedly probed.

INTENT UUID `853ac42e-05a7-48a9-a17e-32fa8bc5f63f` is comment 4028 and was replayed identically once, idempotently. Checkpoint UUID `bca08c18-f5c8-4c77-8298-1801beae20c3` is comment 4030 with identical replay. Completion UUID `a3f3f86b-0bb4-4b63-9b3b-34b4e68d47e5` is comment 4035; the complete request and identical idempotent replay are in `completion-request.json` and `completion-record.json`. Evidence commit `f77d5c623` binds the full handoff before this audit-record commit. Canonical status stays in progress for ROOT review.

The complete code diff remains exactly the five authorized paths; attempt 3 adds changes only to four of them. All prior evidence, original attempt-2 `actual-final`, D3 POST, protected user files/notebooks, external notebook checkout, live graph/CURRENT and native installations were preserved. Disk preflight had about 70 GiB free; no local cold compile or cache deletion was performed. No storage/Parquet/schema/query/cache/ORT/extractor-version change or public Rust delta disk transport is included. Deltas are private in-memory artifacts; v12 disk caches still omit Rust evidence until D5. Unavailable old bytes for changed/deleted sources are not falsely authenticated. No self-approval, closure, integration merge, push or downstream dispatch was performed.
