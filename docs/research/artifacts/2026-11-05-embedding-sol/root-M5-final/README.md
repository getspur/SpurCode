# M5 final ROOT review

The recovered packaging implementation meets the finite M5 acceptance gates. ROOT completed the remaining independent review and POST checks; the final runtime/performance/Rust-notebook stage remains unverified. No main merge or publication is included.

## Source, authorization and lineage

Source fingerprint `b633efdf819255ce175c3cf5dab318cae6908e4b07793cc5f43621882b9b643c` binds 64 files. Authored attempt-3 checkpoint `bb67850a10a2f435f7c398d8b3cc3fa2e6af68e0` and normalized result `40a2974084648f7cc9c0efc712ba6d968be81bcc` have identical trees. ROOT reviewed the authored lineage so the failing test `a65053535abff3154bcf788cf96b37a70a3a3fba` precedes fix `103cc3e09611aa6c978af35b41ff0fc8877aad93` in ancestry. Original root regression/PRE commit `4e6ae0dc9` and its actual assertion REDs are preserved.

Attempt 2 persisted an authorized, idempotently replayed INTENT before the fix. Attempt 3 added only the six-file audit-rejection record; it correctly stopped when the worker service rejected its new INTENT and replay. ROOT did not impersonate the worker, forge a successful audit, weaken authorization, or add a production fix. The independent ROOT review actions and their intent/results are recorded on `bd-2zan`; the separate dispatch-evidence problem remains documented on `bd-mbta`. A matching assignee alone does not satisfy the delegation-label guard, and the precise cause of this occurrence is not established by the saved record.

## Implementation and validation

The wrapper now compares physical CLI directory identity, covering equivalent slash/dot paths without matching unrelated suffixes. npm metadata probes use the inherited 120,000 ms startup allowance with `SIGKILL` and synchronous reap.

Fresh ROOT checks passed: eight wrapper regressions, ten npm tests, and both recovery/evidence verifiers. The saved real SIGTERM-ignoring helper returned a failed compatibility result after 120.071697 seconds, with `ETIMEDOUT`/`SIGKILL` and helper absence observed before external cleanup. The original external 150-second RED and test-owned cleanup remain distinct from this passing containment observation.

ROOT rechecked the original native observation bytes, normal build/features, dependency/link/symbol/load evidence, six installed lifecycles and 24 reference vectors across all three models. The maximum reference difference is `3.706607812348217e-9`. The refreshed private npm layout uses repaired JS and identical native binaries, returns a finite Nomic vector and raw host wait status 0, and removes its socket. Missing/mismatched/crashed helper controls return errors without vectors and preserve host ping/close. The unchanged default CLI five-row regression, 95 xtask tests and the affected graph/analyst/client checks retain their recorded successful results and named existing ignores.

Normal executed artifact identities:

| Artifact | SHA256 |
|---|---|
| Default host | `06d245278b0298e8ce1f02ca5373fb3f3c0d9b10094e129b2870dca19accb033` |
| Worker | `e6f16fe0d1c4727a1f4172d7e21c11e6e5e03fc3456fb89e046d91962949bbf7` |
| ONNX Runtime 1.22.0 dylib | `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a` |

These are ordinary debug macOS ARM64 artifacts. Other platform/release mappings were tested with fixtures; optimized release/Linux/Windows execution is not claimed. Python host return codes and observed child absence are distinguished from raw helper waitpid status, which the ordinary CLI does not expose. The initial socket-bind EPERM is retained as failure, not GREEN.

## Fresh full POST

`prepare_post.py` separates 23 required results from actual observations, freshly derives the repaired path results from subprocess argv, and binds everything to current source and actual native identities. Four fresh Jev gates opened with exact inspected bindings. The executed catalog requests additionally include explicit identity/cardinality checks and numeric limits where applicable.

| Request | Result | Receipt |
|---|---|---|
| Original requirements 1–7 | SAT | `sol_a7d631a2a626445f` |
| Original requirements 8–14 | SAT | `sol_06c08fbdf4a64f2f` |
| Nine repair requirements | SAT | `sol_56dc8af447e5456d` |
| Seven independent original tuple forgeries | UNSAT; seven failing bindings | `sol_a6c7089ee3244604` |
| Seven further original tuple forgeries | UNSAT; seven failing bindings | `sol_fe7d37be64d64efe` |
| Nine independent repair tuple forgeries | UNSAT; nine failing bindings | `sol_5c7e875b96f24046` |
| Stale source/build binding | UNSAT | `sol_41f394234b454c64` |
| Duplicate identity | UNSAT | `sol_905c7eba743c42e3` |
| Missing required observation | UNSAT | `sol_45b88cba36b2456f` |
| Configured and observed probe budgets | SAT | `sol_ae9272ada60f4be3` |
| Two exceeded numeric limits | UNSAT; two failing bindings | `sol_c40478a4c40e42b3` |

All 11 receipts were persisted, reloaded and pinned. The independent oracle checked exact fixed facts, complete generic lowerings/models and 45 compound predicates. The grouped tuple-negative requests use separate one-row relations and separate failing bindings; combined UNSAT is not presented as 23 separate universal program proofs. Observed duration is conservatively rounded upward to integer milliseconds and compared with the inherited 150,000 ms observer allowance, not a fabricated strict nominal-time equality.

```sh
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M5-final/verify_post.py
```

The verifier exits 0. Source-bound native measurements are preserved historical observations; only explicitly identified checks and the POST are fresh. Finite SAT/UNSAT does not prove arbitrary native memory safety, complete panic detection, runtime performance optimality or the next stage's execution.

## Next gate and retained limitations

The [fresh M6 PRE](../../2026-11-05-embedding-runtime-acceptance/PRE/README.md) was executed and independently verified before packaging approval. It preserves the complete canonical task. Actual Rust-cell execution, full runtime fault coverage and performance measurements are still required. The notebook MCP context was inspected read-only; no existing notebook was modified and no new notebook success is claimed.

Twelve completed/approved M4 POST retention pins were released only after complete live-result equality with the immutable `093a1461` archive and independent validation. Their full evidence remains archived. Current M5 PRE and new M5 POST/M6 PRE receipts remain protected; no solver quota or solver files were modified directly.

Automatic approval review previously rejected the optional direct cargo-dist PR-plan check for bypassing `scripts/spur-cargo`. That check remains unverified; it was not bypassed or retried. Active release pairing, local install and native execution evidence are separate.

ROOT changed review/acceptance evidence only. Local main, its five existing dirty tracked files, external notebook source, old notebooks, shared caches, installed user binaries and services are preserved. The final manifest is checked against committed blobs before handoff.

The first downstream preview could not create its transient worktree. Disk inspection found only about 0.3 GB available. ROOT removed six redundant, clean, task-created review worktrees after checking their protected branches, HEADs and lack of untracked data; the only ignored file was generated Python bytecode. All commits/evidence branches remain intact. Available space increased to about 4.0 GB and the preview tool then succeeded. The pre-approval preview excludes the still-unapproved M5 overlay, so actual downstream dispatch-base verification remains required after approval. Independent Git ancestry already confirms the reviewed candidate includes the plan snapshot and every approved predecessor. Exact cleanup and both preview outcomes are retained without presenting the initial failure as a pass.
