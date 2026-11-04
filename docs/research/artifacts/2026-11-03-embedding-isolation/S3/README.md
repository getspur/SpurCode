# S3 — embedding worker lifecycle (bd-2hdm)

A separate native worker confines ORT ownership and performs checked environment shutdown before acknowledging a framed shutdown, while product embeddings remain explicitly unavailable until S4.

**Status: GREEN, root Jev POST review pending.** The issue remains in progress. This does not claim the later host migration, S4 product inference, other platforms' native execution, or general ORT safety.

Source base `251a1fb0627ed10849077a7424648cee0c8ae32c`; dispatch authenticated as `8127590442aa42f6`. Test commits: `47b37c2e5`, `c17886483`, `fd8d12560`, `1ad94008c`, `fdee55059`, `1c93b535e`, `224b76533`. Implementation commits: `420b44181`, `3380bde6d`. [Unified patch](implementation.diff) covers the workspace and new worker. S2 final-review evidence was imported without other history.

## Verification

Commands below ran on Darwin arm64. Set these environment variables for all cargo commands unless otherwise stated:

```sh
export SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_NET_OFFLINE=true
export CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target
export SPUR_ORT_ARCHIVE=/private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz
```

| Check | Result and evidence |
|---|---|
| Test-first initial scaffold | `scripts/spur-cargo test -p spur-embedding-worker --features test-support --test native_lifecycle`: exit 101, missing test probe (compile RED); this first invocation did not set `CARGO_NET_OFFLINE`. A second scaffold RED also showed the absent product binary. |
| Synchronized active-close test | Same command: exit 101, missing `close_with_hook` method (compile RED), [log](embedding-s3-active-red.log.gz). |
| Reopen after shutdown | `scripts/spur-cargo test -p spur-embedding-worker --features test-support --test native_lifecycle native_inference_and_checked_shutdown_exit_normally -- --nocapture`: exit 101, assertion `closed managed environment reopened` (runtime RED), [log](embedding-s3-reopen-red.log.gz). |
| Small caller limit | `scripts/spur-cargo test -p spur-embedding-worker --features test-support --test native_lifecycle unavailable_reply_fits_the_callers_small_identity_limit -- --nocapture`: exit 101, invalid error-message length and EOF (runtime RED), [log](embedding-s3-small-limit-red.log.gz). |
| Native loader failure | The focused test first produced a missing test API compile RED, [log](embedding-s3-loader-red.log.gz), then an actual missing-dylib Rust panic RED, [log](embedding-s3-loader-panic-red.log.gz). The final probe uses the same worker initialization helper and observes typed `Initialization`; the narrow catch handles wrapper Rust panics only, before environment creation, and cannot catch native aborts. |
| Native and protocol tests | `scripts/spur-cargo test -p spur-embedding-worker --features test-support --test native_lifecycle -- --nocapture`: exit 0, 4 passed, [log](embedding-s3-final-test.log.gz). The feature-only probe executes the same owner shutdown implementation. |
| Normal helper build | `scripts/spur-cargo build -p spur-embedding-worker`: exit 0, [log](embedding-s3-build.log.gz). |
| Normal binary wire/exit | `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S3/observe.py`: normal build, explicit one-byte and 256-byte identity limits; framed Ready, Unavailable, Closed; no trailing stdout bytes; raw wait status 0. Results and source/build hashes in [observations](native-observations.json). |
| Native subprocess observations | Same script: three fixture runs output `[1,4,9,16,25,36]` with raw wait status 0; retained output causes checked Busy until dropped; idle, repeated close, reopen rejection, real missing-library typed failure and synchronized active close exit normally. A separately initialized signal control records raw wait status 9 / SIGKILL, not clean shutdown. |
| Dependency boundaries | `scripts/spur-cargo tree -p spur-embedding-worker -e normal,build`, `scripts/spur-cargo tree -p spur-embedding -e normal,build`, and `scripts/spur-cargo tree -p spur-embedding-worker -e features -i ort`: [worker tree](embedding-s3-worker-tree.txt.gz), [client tree](embedding-s3-client-tree.txt.gz), [ORT features](embedding-s3-ort-features.txt.gz). Worker has no graph, analyst, DuckDB, PGQ or owner-test-hooks; client has no ORT/FastEmbed. |
| Existing client tests | `scripts/spur-cargo test -p spur-embedding --features test-support`: sandbox run exit 101 because two unchanged `root_close_review` tests could not spawn `ps` (EPERM), [log](embedding-s3-client-tests.log.gz). The same command with approved process-inspection escalation exited 0, 32 tests passed, [log](embedding-s3-client-tests-unsandboxed.log.gz). |
| Formatting | `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo fmt -p spur-embedding-worker -- --check`: exit 0. |

The first post-implementation build attempt without offline mode hit restricted DNS fetching the crates.io index; subsequent local-cache builds used `CARGO_NET_OFFLINE=true`. Vendor ORT emitted inherited warnings; they are retained in logs. Tests use an explicit isolated `/private/tmp/spur-embedding-worker-native-test-cache` and the pinned archive. No global runtime installation or live notebook was used.

## PRE and POST handoff

[Root PRE manifest](README-root-pre.md), [compact PRE](root-pre.json), and [full provenance](root-pre.full.json.gz) are the supplied root artifacts, not a worker Jev invocation. Full archive SHA-256 `363246d4464a96d8b3c0fcf7fa49aaffd0b9c48fbdfc810d0f9bb4ae1b0becc0`. All four source bindings match both candidate and worktree. The eight `get_solve_result` reloads matched the archived complete lowered requests and statuses: two SAT and six UNSAT negative controls. The catalog workflow and resource rule specs were independently inspected. PRE is prospective only.

[POST candidate](post-candidates.json) contains exact proposed Jev intent/data and independent bounded ordering policy, plus negative controls. [Actual observations](native-observations.json) include raw phase events, values, wait statuses, and source/runtime/executable SHA-256. The emitted test-support phases describe events reached by the same owner, while the checked ORT shutdown and process status provide independent runtime evidence; a session-handle drop is not itself proof that its backing native session has been freed. The observed checked Busy/success does not directly measure four numerical owner counts, so no synthetic zero-count fact table is asserted. Root must perform fresh POST Jev compile, inspect, solve, persist and reload against actual exports before approval.
