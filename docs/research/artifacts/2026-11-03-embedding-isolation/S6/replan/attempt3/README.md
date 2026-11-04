# S6A attempt 3 — worker observations for review

Canonical execution issue `bd-3jqy`, task `s6-owned-runtime`, delegation `62cdd2d088164674`; issue was `in_progress` and plan status `dispatched` at preflight. Source issue `bd-48r6` is historical tracking only. No approval or POST is claimed here. B host integration, C real acceptance, legacy global/facade warm regressions, and the eight baselined native aborts remain unfinished or separately baselined.

Rationale: serialize a public result's completion with owner closure so private helper or sidecar responses cannot escape after shutdown wins, while preserving results already committed before closure.

History restored: authored `c778e3ce..b5e839e89` in original order, then root proof `10da0ebcb` (new corresponding tip `368f60df9`). `S6/replan/verify.py` exited 0 before replay; `review-attempt2/verify.py` exited 0 immediately after replay and before edits. Both verified frozen source bindings. All 18 supplied receipt IDs were reloaded from the solver store; statuses and FNV-1a comparisons of canonical lowered requests and models matched their archive copies (original 2 SAT / 8 UNSAT; supplemental 1 SAT / 7 UNSAT). These are finite declared scenarios, not proofs of actual network routing or race freedom. The first receipt-comparison script failed with `ReferenceError: TextEncoder is not defined`; the subsequent complete comparison used UTF-8 bytes and matched. The initial unprivileged `git cherry-pick` exited 128 because the worktree sequencer is outside the writable sandbox; the authorized process-inspection/git escalation restored all commits successfully.

## Test-first changes and observations

- `420156e47`, `183c0fa30`: added test-only barriers and retained RED logs. The draft test command exited 101: after inference and before send, shutdown won and the draft returned a result; the same occurred when the oneshot reply was queued before closure but the public future was not polled until afterwards. A sidecar future that initiated shutdown during the same readiness poll likewise returned `Some(42)` after closure. Both local event orders ran independently (`s6a-attempt3-races-red2.log`).
- `a896822f6`: gate local and sidecar completion using the same mutex as shutdown closure; no await separates the gate from result completion. The exact regressions exited 0 (`s6a-attempt3-races-green.log`).
- `9de84a2b1`: mode routing tests and test-support-only forwarding from the actual graph model to the existing neutral supervisor injection. The owner test was immediately GREEN, without a manufactured failure. A real hung helper query is cancelled; `CleanupOwner` reports `process or protocol failure: injected embedding child wait failure` on both first and repeated shutdown (`completion.log`).
- `a6320d398`: verifies the sidecar completion gate against a valid response on a real Unix socket, with shutdown initiated in the poll that receives the completed transport response. Also formats the two authorized inherited files. A result completed before closure remains observable in the dedicated test.

The mode subprocess test observed: Sidecar warm and reachable Auto warm did not start a local helper (Auto performed a real socket ping); unavailable Auto performed warm inference plus one query on the same helper, then shutdown. Existing InProcess test performed warm plus two queries on one helper; Off started none. These are protocol fault subprocesses, not real model inference. The requested bounded local inference uses the existing protocol and limits; no new default was chosen.

## Final commands and exits

All builds were sequential, with process-inspection access, and used the following common prefix:

```
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo
```

| Suffix | Exit | Observation |
|---|---:|---|
| `test -p spur-analyst --lib embedding::owned_runtime::completion_tests:: -- --nocapture` | 0 | 5 passed, including real transport, both local boundaries, retained reap report |
| `test -p spur-analyst --test embedding owned_runtime:: -- --nocapture` | 0 | 10 passed |
| `test -p spur-embedding --features test-support` | 0 | 39 passed; output includes `kill: illegal process id:` |
| `test -p spur-graph --test embedding_cancellation` | 0 | 2 passed; output includes `kill: ... No such process` |
| `check -p spur-analyst --features embed --lib` | 0 | library check |
| `check -p spur-analyst --no-default-features --lib` | 0 | library check |
| `fmt --all -- --check` | 0 | workspace format |

Full logs and `verify-status.txt` are alongside this file. Two pre-existing graph dead-code warnings remain. `git diff --check c778e3ce..HEAD -- crates/` exited 0; the full-history diff check exited 2 because preserved raw logs and the historical unified diff contain whitespace and blank lines (see `full-diff-check.log`). Those artifacts were not rewritten. `source.diff` is the unified crate diff from the dispatched base to source tip `a6320d398`; hashes of source, fault fixture, test executables and diff are in `SHA256SUMS.txt`.

The pinned archive SHA-256 was `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`; its contained dylib was `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`. No model was loaded or hashed in these tests.

## Candidate observations for root's independent POST

Positive candidates: local warm sends an Embed and reuses the same owned helper; two clones reuse one session; cancellation and shutdown join helper work; queued and pre-send local replies and a real sidecar reply do not complete after closure wins; a result already completed before closure remains valid; injected cleanup reap-report failure persists on repeated shutdown; Off and external warm modes do not spawn local helpers; Auto unavailable selects a local inferred warm.

Adversarial candidates to challenge: descriptor-only local warm, second helper on clone, result completed after closure, sidecar/Off starting local helper, local warm skipped but reported locally warmed, cleanup failure hidden, and close acknowledged before tracked work/cleanup finishes. These are candidate negatives, not fresh solver POST results. Root must ground and execute POST independently before approval. Production neural model execution, end-to-end external sidecar acceptance, host integration and complete S6 remain unverified here.
