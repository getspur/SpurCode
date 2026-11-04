# S6A independent root review

Recommendation: approve S6A only after the review transaction. Whole S6 remains incomplete.

Canonical issue bd-3jqy, task s6-owned-runtime, plan 805bc9eb-cc70-4d4c-8784-f35776e70a5d.
Reviewed delegation 62cdd2d088164674, attempt 3.
Authored source/evidence tip bffec0686421827f625e7e8bbee0dc0de34c4491 is protected on spur/review-evidence/embedding-s6a-final. The engine wrapper a45a3243e33e8bb9a3be8d364507eceaf1fa94e1 has exactly the same tree. The authored test/fix history remains reachable; no production edits were made by root.

## Source review

Completion now uses the same mutex as shutdown closure. Local results are checked after oneshot receipt; sidecar results are checked after readiness selection. The gate is the operation's completion point, with no later await before return. A result completed before closure stays valid. The admission/job/cancellation/cleanup ownership remains caller-scoped, with no new strong global owner.

The test-only local barriers cover closure after inference but before send and after a queued reply but before caller completion. Tests also cover readiness during closure with a real Unix socket response. Reap-error forwarding is gated behind test-support, enabled through a dev dependency; the error passes through the existing model/supervisor/CleanupOwner path. It is an injected reap-report failure, not a naturally observed OS wait failure or a mocked native release.

Mode semantics are preserved: Off starts no helper; Sidecar and reachable Auto stay external-only and skip local warm. InProcess and unavailable Auto perform internal protocol inference for warm and share the loaded session.

## Independent root verification

run_checks.py ran the same seven checks sequentially through scripts/spur-cargo with SPUR_REMOTE=0, TMPDIR=/private/tmp, the isolated acceptance target and pinned ORT archive. checks.json contains exact commands and exits; full output is compressed.

- Completion tests: 5 passed.
- Owned-runtime tests: 10 passed.
- Native-free client tests: 39 passed.
- Graph cancellation tests: 2 passed.
- Analyst embed and no-default library checks: exit 0.
- Workspace formatting: exit 0.

observe.py then ran 11 direct, isolated test-binary scenarios and archived their actual subprocess events, inputs and stdout. All assertions passed; every recorded helper/descriptor PID was absent after return. Clone reuse produced one service, one Hello and three Embed requests. Unavailable Auto produced one service, one Hello and two Embed requests. Off, Sidecar and reachable Auto produced no local helper events. Captured Nomic query inputs have exactly one search_query prefix. The reap-report error text was identical on initial and repeated shutdown.

These are protocol fault subprocesses. No neural model was loaded by the observations, and no claim is made about all possible schedules.

## Executed root POST

post.json.gz contains exact source-bound observed data, three OPEN Jev interpretations, catalog cards, executed requests, raw responses and pinned reloaded lowerings. Root added unique and exact cardinality bindings after inspecting Jev; facts were unchanged. All three positives passed; all fourteen adversarial models were UNSAT.

Positive IDs:
- reuse: sol_17b6cc558f544512
- completion/cleanup: sol_a5a33a06fe9042d7
- mode routing: sol_0696d5486ecc4402

Negatives cover descriptor-only warm, global owner, second clone session, cancelled discovery publication, both local delivery boundaries, sidecar publication, revoked prior result, hidden reap error, unjoined close, Off spawning, external modes falling back locally and skipped local warm. Each receipt was reloaded, pinned and matched to its result; SAT models matched as well. The finite verifier checks allowed tuples, identity uniqueness, row coverage, source hashes, actual observations and archive hashes. This verifies the recorded bounded evidence, not native correctness or untested host wiring.

## Risk disposition and remaining work

The earlier risk signal's unconditional warm concern was corrected in the approved mode contract; forcing local warm in external modes was never required. The concrete delivery and retained-error concerns now have deterministic source-bound evidence. The source diff whitespace check passes. Full historical diff checking still flags preserved raw logs/unified diff whitespace; this evidence-only limitation is accepted without rewriting historical observations.

Legacy facade/global warm paths and real application-host integration remain visibly unfinished for S6B. Three-model real analyst-path acceptance belongs to S6C. The eight legacy direct-native abort regressions remain separately baselined. No claim is made that ORT shutdown is fixed in those legacy direct-native programs, or that this is whole-S6 acceptance.

## Next task

../host-pre/ contains the freshly executed S6B contract PRE. The next task must import this docs-only root proof commit before source edits and verify A's source hashes. The task body supplies that commit ID. A preapproval preview showed only c778e3ce because the engine excludes unapproved A; verify the real dispatched B base includes the approved A overlay. Do not infer that preview alone includes A.

Run before changing frozen source:

```sh
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S6/replan/final-review/verify.py
```
