# M5 targeted packaging repair — evidence for ROOT review

Canonical issue `bd-2zan` remains **in_progress**, delegation `55ccb98358704cc1`,
assignee `spur-worker-55ccb98358704cc1`. Routing remains codex/profile codex/
gpt-6.1-sol/xhigh. This packet contains GREEN repair observations, not M5
approval. ROOT still owes the complete fresh final M5 Jev/SOLVE POST, including
the initial packaging gates and new negatives, and the M6 runtime/notebook PRE
before approval. All final runtime/performance/containment/Rust Notebook gates
remain in force. No downstream task was dispatched.

## Recovery, authorization and PRE

The clean worker started at the actual original base
`093a1461c5dde2abb2e1732d1b11af6ec0639596` and fast-forwarded the exact protected
root branch to `bfbefd87ca44f1f8c830e829d997dbf06e156a57` before edits/builds.
The first merge attempt hit sandbox Git-metadata EPERM; the authorized retry
fast-forwarded without reset/clean. All initial M5 work remains in ancestry.
Canonical assignment and the single delegation label were checked through the
worker service. INTENT `7bd98632-1151-43a4-8d36-957484c3549b` persisted as
comment 3503 and replayed idempotently; exact request/returns are in
`authorized-intent.json`.

The current 64-file repair PRE verifier and immutable root review verifier
passed before production changes. Live reloads `sol_30590f8b03b84f3f`,
`sol_7c7ea39c8f77452b`, `sol_bcf862aef5ff43ca` returned SAT/SAT/UNSAT and equal
ROOT's complete archived reloads exactly (`pre-worker-reloads.json.gz`).
No historical 62-file or source091 verifier was applied to changed source.
The production timeout reuses the approved startup allowance 120000 ms; the
observer guard reuses startup 120000 + shutdown 30000 = 150000 ms. No new capacity,
public setting, native architecture or owner mechanism was introduced.

## Repair and test history

The root regression/PRE commit `4e6ae0dc9a073374d301ed42beb2a8c23932f2f4`
precedes the observed failures and repair. The committed review tests ran
through `run_review.py`: only their evidence directory was remapped; their
bodies, actual production source and original saved RED records were preserved.
The later wrapper export observes every actual argv without changing results.

| Milestone | Observation |
|---|---|
| `a65053535abff3154bcf788cf96b37a70a3a3fba` | Focused wrapper regression plus fresh PRE/RED evidence committed before production edits. Root wrapper: 7 tests / 5 assertion failures. Root hung metadata: 1 test / 1 assertion failure after 150.012 s. Test-owned helper absence confirmed after external cleanup; failed production containment is explicit. |
| `103cc3e09611aa6c978af35b41ff0fc8877aad93` | Minimal wrapper/npm production repair and fixture path canonicalization. CLI role uses the resolved physical CLI directory, including trailing slash/dot/equals/workdir aliases; unrelated suffix packages retain their exact argv. npm uses spawnSync timeout 120000 and SIGKILL, with synchronous reaping. |
| Fresh GREEN | Root wrapper 8/8 including the new focused regression; production wrapper/release 7/7; npm 10/10; root real hung-metadata 1/1 in 120.070 s; xtask 95/95; unchanged default CLI gate 1/1; local formatting and syntax/diff checks exit 0. |

`repair-production.diff` contains only the two repaired production files;
`repair-tests.diff` contains the focused test delta. The inherited partial-reuse
and original production diffs remain unchanged in the parent packet.
The first wrapper GREEN attempt exposed a new fixture comparison between
`/var` and `/private/var`; its five failures remain in `wrapper-green` and
`packaging-green.log`. Canonicalizing the fixture root corrected that test-only
mistake. The actual missing-helper and unrelated-package RED observations remain
separate and unchanged.

`wrapper-export/actual-commands.json` captures 17 actual wrapper invocations.
`root-post-candidates.json` derives 13 install instances and nine required contract
rows from actual argv and probe observations, including exact original host
argument preservation and resolved sibling paths under space/Unicode roots.
The final source fingerprint is
`b633efdf819255ce175c3cf5dab318cae6908e4b07793cc5f43621882b9b643c` over 64 files.
Only `scripts/spur-cargo`, `npm/spur-cli/platform.js`, and the focused wrapper
test differ from the recovered source binding. All Rust/native/model/state,
xtask, release and original CLI-test source bytes remain unchanged.

## Forced termination and reap

The unchanged root regression returned compatible=false and Node exit 0 before
its 150 s observer guard. Its own final cleanup is not used as reap evidence.
`run_reap.py` independently observes the actual default implementation against
a real SIGTERM-ignoring executable. It delegates unchanged arguments to the
real spawnSync, records its result, then checks the helper PID before any
external cleanup. In 120.071697 s it observed `ETIMEDOUT`, signal `SIGKILL`,
compatible=false and helper PID absent. `observer_cleanup_needed=false`.
The measured wall time includes about 72 ms beyond the nominal timeout and fits
the inherited observer guard. This is failed-probe containment, not successful
helper compatibility or ordinary inference shutdown.

## Refreshed actual npm installation

`refresh_install.py` APFS-clones the preserved task-private release/package/model
fixture into a new temporary root, loads current JS and executes the unchanged
installed-runtime harness with output remapping. It does not replace the prior
package or its evidence. The sandboxed socket bind failed with EPERM; its
artifacts/logs remain under `install-initial`, and are not counted as GREEN.
The authorized run passed in
`/private/tmp/M5 rework install ü a4ibq7n6/npm package with spaces`.

| Artifact | SHA256 |
|---|---|
| Ordinary default host, 415001920 bytes | `06d245278b0298e8ce1f02ca5373fb3f3c0d9b10094e129b2870dca19accb033` |
| Ordinary worker, 57618176 bytes | `e6f16fe0d1c4727a1f4172d7e21c11e6e5e03fc3456fb89e046d91962949bbf7` |
| Materialized ONNX Runtime 1.22.0 | `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a` |
| Repaired npm platform source | `365fe08cefc8b41f2ff54f656331663592dc710d1c5461d6b4b908be66c12fb7` |

The current installer and lazy native shim returned 0. The actual installed
default host returned a finite 768-dimensional Nomic vector with maximum delta
`3.706607812348217e-9` against the unchanged pinned reference. Normal shutdown
returned raw host wait status 0 and removed its socket. Runtime compiler/download
sentinels were unused. `final-install.json` preserves exact response, source
hashes, artifact paths/hashes and runtime identity; `model-preservation.json`
verifies all 15 shared/preserved-private/refreshed-private model assets unchanged.

Initial six lifecycles / 24 vectors / three-model/native-boundary/typed-error evidence
is retained, with identical native artifacts and Rust source. It remains the
historical accepted observation, not a newly repeated three-model experiment.
The verifier checks 167 inherited packet/spec Git blobs remain exact. The old
current-source verifier is intentionally not rerun against the repaired source;
ROOT's `historical-replay.json` remains the source-bound historical replay.

## Reproduction and limits

Run `python3 -B docs/research/artifacts/2026-11-05-embedding-shipped-packaging/rework/verify.py`.
It checks actual saved RED/GREEN, argv exports, source identities, complete PRE
reloads, preserved Git blobs, private installed bytes/models/vector comparison,
probe termination/reap and raw check streams. It is not a solver POST.

Cargo commands and environments are retained in `xtask.json`, `cli-default.json`
and `format.json`, with complete compressed streams. All use `scripts/spur-cargo`,
approved SPUR_REMOTE=0/TMPDIR/private target/ORT settings, serially with one
Cargo writer. The original default CLI test is unmodified, has no feature
disablement/ignores, and preserves its five-row equality including repeated
serde::Serialize. The earlier client/graph/analyst/native checks and their
existing ignores remain in the preserved parent packet.

Execution here is macOS ARM64 with the retained ordinary debug artifacts.
Optimized release/Linux/Windows/x86 process execution was not rerun. The optional
cargo-dist PR-plan remains unverified after the earlier automatic-review
rejection; no bare Cargo bypass or retry occurred. No push/publication, installed
user binary overwrite, service restart, main merge, shared-cache modification,
external notebook source change or graph/CURRENT edit occurred.
