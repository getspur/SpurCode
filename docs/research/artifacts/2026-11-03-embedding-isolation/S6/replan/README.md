# S6 recovery through smaller tasks

The third S6 attempt remains incomplete. Full authored checkpoint `0e76c0faa8be270e5d65a9741955ecc4e3f04870` is protected at `spur/review-evidence/embedding-s6-attempt3`; no root production edits were made. The checkpoint includes cancellable discovery/query adapters, cleanup scopes, bounded cleanup waits/error retention and their TDD histories. Its latest client suite has 39 recorded passes. The two graph cancellation passes predate the last client cleanup fix and need rerunning. Warm-session and warm-inference have observed runtime REDs; the owned-runtime test has a missing-API compile RED. There is no analyst-path GREEN, host integration or complete POST.

The original task `bd-3cu4` reached its three-attempt limit. It will be superseded, without approval, by source epic `bd-1qmt` and the sequential tasks below. A dedicated Plan Engine submission retains explicit agent/profile codex, model gpt-6.1-terra and effort xhigh for each node.

| Task | Canonical issue | Acceptance |
|---|---|---|
| S6A owned runtime | bd-48r6 | Implement the new reusable owned API with real warm/cancel/shutdown behavior and scoped tests. |
| S6B host integration | bd-2f21 | Wire approved ownership through server, direct worker, socket and CLI lifetimes; retire legacy global paths. |
| S6C analyst acceptance | bd-4ha3 | Exercise real helper inference and all original analyst mode/socket/process/dependency gates. |

The new plan starts from a documentation-only descendant of the preserved checkpoint, so workers must not replay the existing history. The original packaging task `bd-3pcp` must be blocked on `bd-4ha3` before the old S6 task is cancelled as superseded. Original final acceptance remains downstream. Before releasing packaging, root must hand off the approved new source for explicit integration into the original plan's older base. No original user goal is cancelled or declared complete.

Only S6A is ready to start. Its fresh PRE has two inspected OPEN Jev interpretations, two SAT models and eight expected UNSAT corruptions, all persisted/reloaded/pinned. The finite metadata policy is not runtime or memory-safety proof. Full original inputs, provenance, current catalog and exact family/lowered requests/results are in `owned-pre.json.gz`. Root will execute fresh PRE for B before approving A, and C before approving B; each task needs actual-output POST before approval.

| Case | Status | Receipt |
|---|---|---|
| owned_runtime_1 | sat | sol_e762aa16c3b5444e |
| owned_runtime_2 | sat | sol_2015c6cc9a964052 |
| global_handle | unsat | sol_f6bc27c682ec4cf5 |
| descriptor_only_warm | unsat | sol_391319e7b97e48b3 |
| clone_opens_second_helper | unsat | sol_0b95f55c96164249 |
| cancelled_start_publishes | unsat | sol_143eeea1f79c404e |
| cancelled_query_publishes | unsat | sol_bd49b0fec10a45be |
| close_before_join | unsat | sol_92357b06cbf64854 |
| cleanup_error_hidden | unsat | sol_d6b0493b8a9c474a |
| off_spawns | unsat | sol_8ac53135e30d412f |

Run `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S6/replan/verify.py` before source edits. It checks the frozen bindings and finite PRE models. After source edits, old source hashes remain historical evidence.

Use `scripts/spur-cargo`, authorized local builds and the single target `/private/tmp/rust-gap-g1-acceptance-target`. The pinned archive remains `/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz` with SHA-256 `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`. Preserve global caches, active checkout, live notebooks and external notebook source/CURRENT. Commit evidence regularly and report progress during cold builds.

The eight direct-native legacy abort cases remain separately baselined and preserved. They are not passing tests or the problem these three tasks are intended to fix. Packaging and complete CLI/PGQ/notebook acceptance remain required.
