# M5 review: targeted rework required

`shipped-packaging` / `bd-2zan` is **not approved**. The initial installed execution and native-boundary evidence replayed successfully, but two independently reproduced packaging defects block acceptance. The final Rust notebook task remains pending.

The reviewed checkpoint is `3f1e883584a6b92e6b7a526eed1c148bebe9664e`, tree-identical to authored evidence commit `52dedab4b`. Its actual dispatch base is `093a1461c5dde2abb2e1732d1b11af6ec0639596`. The protected branch `spur/review-evidence/embedding-sol-m5-root` contains the complete normalized M5 implementation followed by the root PRE, regression tests and review evidence. The raw authored test/fix/evidence history is separately protected under `spur/review-evidence/embedding-sol-m5-authored`.

## Observed failures

The wrapper regression ran seven tests, including the four existing controls, and exited 1 with five assertion failures. `crates/spur-cli/`, `crates/spur-cli/.`, `--path=crates/spur-cli/`, and `--path ./` from the CLI work directory omit the companion. Conversely, an unrelated directory `not-spur-cli`, whose manifest names `spur-cost`, incorrectly triggers installation from `not-spur-embedding-worker`. The fake Cargo executable records actual wrapper argv; no real installation or build is performed. The original host argv and unrelated-package behavior are acceptance requirements.

The npm regression invokes the actual `helperIsCompatible` implementation through Node against a real, test-owned helper that ignores SIGTERM. It exited 1 at a runtime assertion after the external 150,000 ms observer guard expired. The helper entered the compatibility request, but the implementation never returned. The observer killed and waited for its own process group; the helper PID was subsequently confirmed absent. This is failed production containment, not successful npm cleanup. Both postinstall and lazy CLI launch use this unbounded probe.

The regression/PRE commit `4e6ae0dc9a073374d301ed42beb2a8c23932f2f4` precedes both observed REDs and any repair. Exact commands, stream hashes, timings and actual argv are in `wrapper-red.json`, `npm-hang-red.json`, their compressed streams, `wrapper-observations.json` and `hung-metadata-observation.json`.

## Fresh narrow PRE

Run before repair edits:

```sh
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M5-review/rework-pre/verify_pre.py
```

The current checkpoint binds 64 files with fingerprint `7eca8eb22d7eaa73296b6a7c5a8bafe8b639a3ec7cdc5eedb0b8758efd681033`. Nine declared repair requirements and seven compound catalog predicates were checked, persisted, reloaded and pinned:

| Receipt | Expected result | Meaning |
|---|---|---|
| `sol_30590f8b03b84f3f` | SAT | Declared path-role and bounded-probe repair contract |
| `sol_7c7ea39c8f77452b` | SAT | Inherited startup/cleanup resource allowances |
| `sol_bcf862aef5ff43ca` | UNSAT | Unbounded-probe negative control |

The Jev gates are open and exact requests, lowerings, fixed facts, models and per-rule outcomes were independently checked. This is a repair PRE, not a runtime pass or full M5 POST. Reload these three live receipt IDs before proceeding. The older 62-file M5 PRE remains historical evidence and must not be applied to the recovered 64-file checkpoint.

The metadata allowance is the approved M2 startup budget, 120,000 ms. The external fault-test guard is startup plus the existing 30,000 ms shutdown allowance, totaling 150,000 ms. A repair must terminate and reap even when SIGTERM is ignored. Do not add guessed capacities or silently weaken the observer guard.

## Recovery and repair

On a clean retry, fast-forward merge `spur/review-evidence/embedding-sol-m5-root` before builds or edits. This restores the complete M5 checkpoint and its current PRE/tests/evidence. If ancestry prevents a fast-forward, report progress and preserve the work; do not reset, clean or drop prior commits. Recheck canonical assignment/INTENT through the normal worker service and replay the new PRE before production changes. Preserve routing as codex/profile codex/gpt-6.1-sol/xhigh.

Fix actual CLI path-role selection for equivalent spellings without reclassifying suffix-matching unrelated packages. Bound npm metadata execution and ensure forcible termination and reap. Keep both changes within the existing packaging scope. Preserve current host arguments, default behavior, checksum validation, cache propagation, model semantics, ownership and native boundaries.

Reproduce the committed tests before fixing and run them GREEN afterward:

```sh
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M5-review/review_regressions.py ReviewWrapper
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M5-review/review_regressions.py ReviewMetadata
```

The real default-budget fault test can take two minutes on GREEN and 150 seconds on RED. Keep tool calls yielded and send progress while it runs; do not repeat it merely for status. Add focused cases to production regression suites as needed. Run affected npm/wrapper/xtask checks and the unchanged default CLI gate. Preserve prior installed three-model observations; regenerate build-bound evidence if a relevant artifact changes. Use `scripts/spur-cargo` exclusively, with the already approved local settings and one Cargo writer.

After GREEN, ROOT still owes a fresh full M5 Jev/SOLVE POST over actual final exports and negative controls, then the M6 runtime/notebook PRE before approval. No local-main merge, publish, existing install overwrite, service restart or external notebook source edit is part of this rework.

## Preserved evidence and limits

`historical-replay.json` records the successful initial M5 verifier replay. References to the retired worker directory were mapped only to exact committed checkpoint blobs, including compressed-log fallbacks; the original verifier was unchanged. That evidence includes the unchanged default CLI five-row test, six private installed lifecycles, 24 vectors across three models, maximum reference difference `3.706607812348217e-9`, normal exits, and observed host/helper native separation. These observations do not cover the two newly failing cases.

Automatic approval review rejected the worker's optional direct cargo-dist plan because it bypassed `scripts/spur-cargo`. That optional PR-plan check remains unverified. The active release pairing and native installed observations are separate evidence; no bypass or retry with bare Cargo is authorized.

Root changed only regression/review/spec evidence. The 64 implementation files, local main HEAD, five pre-existing dirty tracked files, external notebook sources and running services were preserved. `preservation.json`, `verification.json` and `SHA256SUMS.json` bind the review packet. Run `python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M5-review/verify_review.py` to check the immutable RED records without rerunning the hung process.
