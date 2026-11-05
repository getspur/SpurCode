# Shipped embedding packaging: worker evidence for ROOT review

Canonical issue **bd-2zan**, delegation **1f0efa4664fe4ce2**, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`, task `shipped-packaging`.
Entry is `093a1461c5dde2abb2e1732d1b11af6ec0639596`, carrying the four approved
predecessors. Routing remains codex / gpt-6.1-sol / xhigh.
The issue remains **in_progress**. This is implementation and observed evidence,
awaiting ROOT's fresh Jev POST/catalog execution and the final node's fresh PRE.
Final runtime, performance, containment and Rust Notebook gates remain separate.

## Entry authorization and PRE

The dispatch/assignee/single delegation label and four approved predecessor states
were read back before edits or builds. INTENT UUID
`69683cdd-745d-4187-9c74-2db932935d91` persisted as comment3491 and replayed
idempotently. `authorized-intent.json` preserves the exact request and returns.
The initial use of the plan task key instead of canonical issue ID returned
issue-not-found, **not an authorization rejection**; its exact request/error was
saved, reported and corrected to bd-2zan before implementation. No guard or role
was bypassed. The original failed request is in `intent-identifier-error.json`.

Current [M5 PRE](../2026-11-05-embedding-sol/M5-pre/README.md) and root M4 verifiers
both passed before source changes. All three receipts were reloaded before edits:
`sol_f5632ddd6f8147a5`, `sol_7381ebd0b1984d1b` SAT;
`sol_adb5223797974d16` expected UNSAT. `pre-worker-reloads.json.gz` contains the
complete actual worker reloads and compares equal to ROOT's complete retained
receipts. No old source091 verifier was run against changed production source.

## Changes and TDD milestones

| Commit | Purpose and observed RED/GREEN |
|---|---|
| `396034d1f` | Exact old npm test726ea207 reused; historical xtask tests and wrapper fixtures committed. Missing npm sibling, missing wrapper build/run sibling, missing xtask worker assertion and missing dist sibling observed. |
| `af66aaf98` | npm missing/corrupt helper preservation and lazy repair tests; existing installer incorrectly succeeded or ran old host. |
| `76e2c9c6c` | Private local install requires sibling; observed missing installed worker. Includes explicit missing-helper preflight coverage. |
| `c660c6684` | Active release requires helper, metadata and checksums; assertion failed on unchanged workflow. Bundle transport fixture already passed. |
| `e10caee6f` | Initial minimal wrapper/npm/xtask/active release fix. npm, wrapper and all95xtask tests GREEN. |
| `3ea7d866f` → `b2e724845` | Four staged npm metadata-field assertions RED, then validation before replacing installed files; all10npm tests GREEN. Historical layout fixture now emits executable compatibility metadata while retaining its layout assertions. |
| `29b92cded` → `d2f615a8b` | Source install under a space/Unicode prefix lacked companion; wrapper now installs ordinary helper under the same prefix; all6wrapper/release fixtures GREEN. |
| `a9013d215` → `826671c13` | Remote helper fetch failure replaced old host; observed assertion RED, then helper fetch precedes host replacement and both modes are executable; all95xtask tests GREEN again. |
| `dd39d77e3` → `ac2d1de53` | Final review found wrapper remote defaults could change native local-install routing. Observed local-routing assertion RED, then explicit SPUR_REMOTE=0 on native install commands; all95xtask tests and local formatting GREEN. |

`historical-partial.diff.gz` is byte-identical to ROOT PRE's preserved119-add/
8-delete c1596d8c0-relative xtask delta from3406dc24. Only its production hunks
were applied with `git apply --check`, after RED. The normalized commit was never
cherry-picked. `reused-partial-production.diff` is that exact subset;
`new-production-after-partial.diff` separates subsequent production changes;
`all-production.diff` includes the complete seven-file production change.
Existing independent xtask/TUI behavior and predecessor ownership code remain
preserved. The final64-file source fingerprint is
`7eca8eb22d7eaa73296b6a7c5a8bafe8b639a3ec7cdc5eedb0b8758efd681033`.
All55 PRE files outside the seven production paths, including the original CLI
test and all A owner/state/native/model implementations, remain byte-identical.

The wrapper builds the normal companion for selected CLI builds and runs and
installs it for local CLI source installs. xtask local/macOS/remote install and
all five dist targets build/fetch/install the pair, preserving namespaces,
target names, default features and existing skill/extension assets. npm verifies
both executable checksums and staged helper version/protocol/OS/architecture
before replacement; lazy launch repairs missing/incompatible pairs. No native
inference package is added to the host closure. No build.rs recursion, host
runtime compiler or runtime executable download was introduced.

## Actual artifacts and installed execution

Normal default artifacts were built with `scripts/spur-cargo build -p spur-cli
--bin spur --message-format=json`; the wrapper adds the ordinary worker.
`normal-build.jsonl.gz` preserves exact Cargo artifacts/features. Host features
are default/embed/interactive-default/tui-default, graph/analyst have embed,
neutral and worker have no extra features, and no artifact is a test profile.
These are **debug normal-feature macOS ARM64** artifacts; optimized release
and other-platform executable acceptance is not claimed.

| Installed artifact | SHA256 |
|---|---|
| host415001920bytes | `06d245278b0298e8ce1f02ca5373fb3f3c0d9b10094e129b2870dca19accb033` |
| helper57618176bytes | `e6f16fe0d1c4727a1f4172d7e21c11e6e5e03fc3456fb89e046d91962949bbf7` |
| embedded/materialized ONNX Runtime1.22.0 dylib | `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a` |
| approved runtime archive | `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07` |

The temporary release fixture contains normal names
`spur-1.24.0-aarch64-apple-darwin`,
`spur-embedding-worker-1.24.0-aarch64-apple-darwin`, skills tar and SHA256SUMS.
Controlled fetch executes the real npm installer. Actual installed siblings are
under `/private/tmp/M5 installed ü bhdpf_fl/npm package with spaces/bin`.
`final-install.json` records final JS source hashes, successful native
postinstall and native npm shim `--version`. Model snapshots are private APFS
clones of all15 pinned files; shared and private SHA256s were compared before
and after. Runtime materialization stays under private HOME or a fresh explicit
private runtime cache. No loose sibling dylib is needed: the approved macOS
worker embeds the verified bytes and materializes them itself, unchanged.

`run_installed.py` executes the actual installed default CLI's external v1
`embed serve` socket with ordinary sibling discovery, without helper config or
executable overrides. Nomic uses the default model selection; CodeRank and Jina
select their existing product models. There are two independent lifecycles per
model, two repeated two-vector requests per lifecycle: **24 vectors**, finite
and768-dimensional, checked against the existing S4C pinned query references.
Maximum component difference is **3.706607812348217e-9**, below1e-4.
All six CLI processes exited0 after SIGTERM and checked owned shutdown; sockets
were removed and each observed service helper PID was gone. These captures
record Python returncodes and descendant absence; they do not invent a directly
observed raw worker waitpid status.

During live inference, `vmmap -w` returned0 for every host and helper. Hosts
never mapped libonnxruntime; workers mapped the pinned private dylib and never
mapped DuckDB. Normal/build trees, complete symbol tables and linked-library
lists independently confirm no fastembed/ort/ort-sys inference in the normal
host, and no graph/analyst/DuckDB/PGQ in the helper. Test/development feature
unification is archived separately in `graph-dev-feature-tree.txt` and is not
used as normal-host artifact evidence. `compressed-streams.json` preserves raw
stream hashes and sizes for compressed symbol/loader/vmmap/build outputs.

`run_reinstall.py` executes the final installer and ordinary native helper,
then embeds default Nomic using a fresh private runtime with cargo/rustc/C/C++
compiler/curl/wget sentinel commands. No sentinel ran, vectors matched the
reference, and the final actual host `waitpid` raw status was0.
`run_negatives.py` exercises actual installed host copies with missing,
version-mismatched and SIGKILL descriptor helpers. All reject vectors with the
existing typed diagnostic chain, still answer ping and exit0 with socket
cleanup; compiler/download sentinels remain unused. The first evidence assertion
expected the word "helper" for a crash; actual source returns "process or
protocol failure". That assertion and logs remain preserved; only the evidence
expectation was corrected, with no production error/owner change.

## Verification, failures and limits

All Cargo commands use the approved local wrapper/environment and one writer.
Initial installed socket and neutral process-inspection checks recorded EPERM;
their logs are preserved and were never treated as GREEN. Authorized reruns
completed. No installed user binary, service, live/old notebook, graph/CURRENT,
external notebook source or shared model/runtime cache was modified. Only the
abandoned first task-private EPERM install fixture was removed for disk space.

| Fresh check | Result |
|---|---|
| neutral protocol/policy/discovery/batching/supervision |48passed |
| graph discovery/compatibility/cancellation |12passed |
| graph store/model/publication/progress/batching |66passed,3existing ignored |
| analyst mode/socket/owned-runtime integration |29passed |
| analyst embedding units |9passed plus nested subprocess assertion;1existing real test ignored |
| original default CLI DuckPGQ regression |1passed twice, no ignores; unchanged query and five rows including repeated serde::Serialize |
| xtask entire suite |95passed after final change |
| npm platform/install/layout |10passed |
| wrapper/source-install/release/bundle fixtures |6passed |
| affected client/graph/analyst/CLI all-target check and local formatting |exit0 |

Existing graph3/analyst1 ignored native/model tests remain explicitly ignored;
actual ordinary installed CLI testing above covers all three models separately.
Warnings already present in graph dead-code paths are retained in raw streams.
`check` is not claimed as warning-free clippy. No broad tests were weakened.

The active tag publisher is `release-dist.yml`: its xtask matrix now produces
both artifacts and its smoke checks require pair files, checksums and native
Linux helper compatibility before upload. The existing real cloud-build bundle
publisher was exercised against the exact external scripts with an intercepted
AWS command: `actual-bundle.json` hashes all68 captured files, excludes local.env
and records the local archive. No S3 upload or external source edit occurred.
The generic transport already forwards the package arguments and needs no edit.

`release.yml` and cargo-dist configuration are retained PR planning machinery
since the existing cutover; neither publishes tag releases. Direct planning
attempts encountered sandbox sccache/network/uncached target dependencies.
Automatic approval review rejected the optional elevated `dist plan` download
check because it bypassed the required wrapper and was unnecessary for the
authorized checks; the exact request/reason is preserved in
`cargo-dist-auto-review-rejection.json`, with no retry or bypass. This retained
plan is **unverified**. Static/fake target-matrix success does not establish
Linux/Windows/x86 or optimized release execution. Nothing was pushed/published.

Replay `python3 -B docs/research/artifacts/2026-11-05-embedding-shipped-packaging/verify.py`.
It rehashes current sources and installed bytes, compares complete PRE receipts,
recomputes all24vector comparisons, checks raw maps/symbols/dependencies,
negative captures, command exits and compressed-stream identities. It verifies
finite observations, not a general FFI or ownership proof. `root-post-candidates.json`
provides concrete grounding for ROOT's fresh Jev POST and negative controls.
ROOT must supply the final runtime/notebook node PRE before approving B.
