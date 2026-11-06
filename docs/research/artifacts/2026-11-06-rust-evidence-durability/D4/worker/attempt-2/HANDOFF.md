# D4 worker handoff — ROOT review required

D4 transports authenticated whole-file Rust payloads and complete retained origin inventories through in-memory composition; changed files retain their selected capture origin and unused origins are pruned whole.

Canonical issue `bd-1qn9` remains `in_progress`, assigned to `spur-worker-0a1558864da542c1` with the matching delegation label. Source issue `bd-38ls` and the branch UUID were not used for audits. ROOT owns fresh actual-output POST/Jev and independent review; this packet is not approval or G3/panic-completeness acceptance.

Base: `4d4ef0fbdf3a28a9e02e7e44f0b2af3bd5872bcc`. Required first test commit: `cd2fe87bfa0b58f859b7de64d70011981fcb5ed7`. Additional test commit: `a4dc8cf467d8ae676b21ff615e28f969d3277930`. Implementation: `002fd94a7b4885cd32d88ae5109bd0694244ebd6`. The evidence commit is the following task-only documentation commit in branch history. [Scoped unified diff](source.patch), SHA-256 `c5c7ac394b2aa45d17069a56566e16a6e4b7f2245e12f83e5267f65fc35fb371` (42,040 bytes), covers exactly the five authorized source/test files. It uses zero context (`git apply --unidiff-zero`); the complete context diff is preserved losslessly in [source-context.patch.gz](source-context.patch.gz), uncompressed SHA-256 `83a26cc4c0d1b78a36f3ee008cdba5e2950d7344f5600f2f566de95c37bb4018`.

PRE was verified before any test/production edit with `python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/root-pre/verify.py`: fingerprint `ab01fe1d8b738241a3c4a6896974aa102eea6887de3a41f218575f8279293f0d`, 70 bindings, seven receipts (3 SAT/4 UNSAT), 21 independent predicates, exit 0. All seven complete live `get_solve_result` envelopes exactly equalled archived `reloaded` values, including lowerings/models/cores/durations/reasons. Jev's archived gate was open; family `data_integrity`, mode `verify`, exact facts and empty unknowns were inspected; unique/cardinality bindings were explicit. [Complete reload/dispatch record](entry-gate/pre-reload.json.gz). The PRE is prospective; its selected markers are not native hashes. Original D3 POST and D4 PRE were not rewritten after authorized source edits.

The seven reloaded receipts are `sol_e0de0ffd014448fc`, `sol_770b78e5109a4db9`, `sol_2e6e35e7d9614032`, `sol_539a39d436ff4e7f`, `sol_f166341dee8f40d9`, `sol_347606b973e34fa7`, `sol_3530532594b74560`.

| Execution | Passed | Failed | Ignored | Filtered | Exit |
|---|---:|---:|---:|---:|---:|
| Required pre-implementation assertion RED, local | 0 | 1 | 0 | 17 | 101 |
| Additional pre-implementation evidence RED | 14 | 4 | 0 | 0 | 101 |
| Final AWS evidence tests | 19 | 0 | 0 | 0 | 0 |
| Final AWS macro tests | 8 | 0 | 0 | 0 | 0 |
| Final AWS store build unit tests | 38 | 0 | 0 | 544 | 0 |
| Final AWS full extractor suite | 53 | 1 | 3 | 0 | 101 |
| Final AWS Git incremental suite | 26 | 2 | 1 | 0 | 101 |
| Final AWS graph all-target check | — | — | — | — | 0 |
| Scoped local format check / full diff check | — | — | — | — | 0 |

Required RED command (after committing the existing-API regression, before production edits):

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo test -p spur-graph --test rust_evidence_build reuse_keeps_original_snapshot_and_deletion_removes_active_file -- --exact --nocapture
```

It compiled and panicked at `expect("incremental carrier")`; [raw assertion RED](RED-local-assertion.txt). An earlier extra-test `BuildStats` import compile failure is preserved separately in `RED-local.txt` and is not the required RED. No production changes preceded the assertion RED.

Final remote commands (the test-only config sets `D4_EXPORT_DIR`; no workspace Cargo/dependency configuration changed):

```sh
scripts/spur-cargo test -p spur-graph --test rust_evidence_build --test macro_invocation_artifacts --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-2/native-export-env.toml
scripts/spur-cargo test -p spur-graph --lib store::build::tests --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-2/native-export-env.toml
scripts/spur-cargo test -p spur-graph --test rust_evidence_build --test integration_git_incremental --test macro_invocation_artifacts --test extractor --no-fail-fast --config docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-2/native-export-env.toml
scripts/spur-cargo check -p spur-graph --all-targets
scripts/spur-cargo fmt -p spur-graph -- --check
git diff --check 4d4ef0fbdf3a28a9e02e7e44f0b2af3bd5872bcc
```

[Remote command identities and raw logs](remote-runs/) retain launcher HEAD, dirty source bindings, environment, timestamps and actual exit codes. `run.py`'s toolchain field describes the local launcher; the actual remote compiler is separately measured in [AWS native identities](aws-native-identities.json): Rust 1.94.1/e408947bf, aarch64-unknown-linux-gnu, LLVM 21.1.8. Final code source binding digest is `714579d99d26e9e4eafa253028269cf092f81971dee0706696586197ffb227e6`. Existing dead-code/benchmark warnings were not changed.

The broader suites remain red for three baseline failures: extractor `artifact_uses_sentinel_anchor_when_source_drifts_after_extraction` rejects out-of-bounds stale facts; Git `discovery_filters_symlink_gitlink_sparse` and `submodule_pointer_change_invalidates` attempt to read an absent `.rs` gitlink. An actual native BASE comparison reproduced these using exact base production/test bytes; [raw baseline](BASELINE-core.txt) and [guaranteed restoration check](baseline-restoration.json). Baseline extractor: 52 passed/2 failed/3 ignored (the other failure was the now-fixed missing no-op carrier); baseline Git: 25 passed/3 failed/1 ignored (its additional timing-sensitive MCP freshness case passed on final AWS). Five v12 disk-seed tests first prove `rust_evidence.is_none()`, then require Full/default stats and fresh files/symbols/edges/evidence parity. No ignores were added. Previously ignored core-edge parity/edit-sequence tests were also exercised and still fail; core comparisons were preserved. Full GREEN acceptance therefore remains pending ROOT disposition of the reproduced baseline failures.

[Fresh AWS capture](aws-native-capture.json.gz), SHA-256 `d9b9bdb4a794f1aef1e5ce8946a66bc1fd9ecb702cee2392abfda6c003310c77`, contains exact text/hashes for 13 native export files, 1,682 source bindings and five executable hashes. [Exact SSM requests/responses](aws-fetch-trace.json.gz) document transfer without semantic rewriting. [Native exports](actual-final/) include full/no-op/edit/delete/readd/pruned-empty, changed/empty/deletion-only private deltas, eight compatibility cases, six mutation controls and the reused index-OID/source-byte race. [Independent source inventories](source-inventories.json) include actual source bytes, paths and independently computed Git blob OIDs; [finite observations/digests](independent-native-observations.json) retain original file and whole-origin digests.

The real selected-origin hash is `5b481c13812750ff507cb15beac54883614b9e9ebac9b142e49d7f28967fc530`; the aggregate edit hash is `b3b8fb772a25d8736dea076fec4dcf7e380a5dcd689f4aa701e6e67ea71bccfe`. The retained original whole-origin canonical-JSON SHA-256 is `e1f0bcd529a5e21842214162560c875e37ac5b1723a30fab284bca5aca2a399f`. These are measured native outputs/independent JSON digests, not PRE prospective markers.

All six mutations are rejected by source validation and the incremental builder before and after genuine native `artifact_content_hash_blake3_hex` rehash/stamping: wrong reused OID, swapped existing owners, omitted site, nested header rebound, deleted-site reinsertion and Unknown relabelled complete-zero. The omission and Unknown controls pass stored shape/FK validation, demonstrating why exact source correspondence is required. Every mutation carries its own exact source bytes/inventory. Full native verdicts and mutated artifacts are retained.

Recheck observations with:

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-2/verify-native.py docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/worker/attempt-2
```

This observer verifies all 1,682 local/remote source hashes, source OIDs, exact retained objects, selected-versus-aggregate origins, empty domains and native mutation verdicts. It does not implement Rust collection or replace ROOT's solver/native replay. The existing `replay_packet_through_current_api` adapter accepts a standard packet through `D3_PACKET` and optional fresh `D3_REPLAY_OUTPUT`; leave `D3_EXPORT_DIR` unset to preserve old evidence. That adapter is conditional when no packet is supplied.

Attempt-1 ENOSPC history is preserved in the entry packet; its failed UUID `9dca6e4a-59f4-4958-b104-3fab8a691d8d` was not replayed under this delegation. Attempt-2 remote endpoint failure, upload source-state drift, inline Cargo quote failure and the corrected remote runs are all retained. The shared local target disappeared externally after baseline execution; this worker performed no deletion and preserved then-existing native test executables by hardlink at `/private/tmp/d4-attempt2-preserved`. Supplementary [preserved-binary exports](actual-preserved/) predate final export-only test additions and are not the final source-bound capture. No cold local workspace build was run.

INTENT audit `ffd6ddfe-f8d3-480e-8d73-510cbfea2e34` recorded as comment 4012 and replayed identically. Checkpoint audit `43ce7d27-f939-448b-b6c8-080409b4fec4` recorded as 4015. Risk signal `5eb8604a-e245-4220-b38c-c5d68096b242` is on the canonical issue. Completion audit `de3c277b-71ab-42ca-90c4-30dc934dff7d` recorded as comment 4017 and its identical idempotent replay are stored in `completion-record.json`. Status remains in progress for ROOT review; no approval, closure, merge, push or downstream dispatch is performed.

No storage/Parquet/schema/query/cache/ORT implementation, public Rust delta disk transport, other-crate source or broad golden change is included. D4 deltas here are private in-memory artifacts; v12 disk caches still omit Rust evidence until D5. Edited/deleted old bytes are not falsely authenticated. Protected user files, external spur-notebook, old notebooks/evidence, live graph/CURRENT and native installations were not edited by this worker.
