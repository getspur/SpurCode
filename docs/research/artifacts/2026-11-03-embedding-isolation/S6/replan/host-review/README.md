# Root S6B review: request changes

Canonical issue bd-7gja; task s6-host-integration; plan805bc9eb-cc70-4d4c-8784-f35776e70a5d.
Delegation8afdd2e660dd4ad3, attempt1.
Protected authored tip0782df09713227be53b7161e2157dfa85d046960 on spur/review-evidence/embedding-s6b-attempt1.
Production snapshot dff7474c99fbf2c9bac1d693f8df9a2ba6473e03.
Engine wrapper5b6da9220ee4285bac60c0a65bb6ef57807b11bc has the same full tree.
No production edits by root. No approval, whole-S6 success or release of C.

## Concrete blocker

The exact built CLI SHA-256 f851b1543afaf1d4f9db4ff49b7cdb8c708f952b559c390957f87ccd39a5250e was used with a protocol-only Python helper.

reproduce_termination.py reached the setup milestones and then signalled the newly created CLI process:
- analyst MCP, descriptor warm-up, SIGTERM: raw wait -15, descriptor PID still existed after parent exit.
- bundled MCP, descriptor warm-up, SIGTERM: raw wait -15, descriptor PID still existed.
- analyst MCP, descriptor warm-up, SIGINT: raw wait -2, descriptor PID still existed.
- analyst MCP, active query, SIGTERM: raw wait -15, helper PID still existed.

The observer recorded events, stdout, stderr and PIDs before cleanup. It then killed only its own experimental process groups. These forced cleanup actions did not turn the failed observations into passing application cleanup. A subsequent read-only probe confirmed none of those remaining fixture PIDs still existed.

Idle analyst SIGTERM was also observed: raw wait -15 with no remaining helper. That case is diagnostic and is not counted as an orphan counterexample. A conventional signal exit code after genuine joined cleanup can be valid; acceptance must examine cleanup evidence, not require exit0.

Source explains the failure: with_command_embedding_runtime awaits warm and the stdio operation before invoking shutdown, without an enclosing ordinary termination-signal path. A process terminated by the default SIGTERM/SIGINT action skips that explicit shutdown. This is a parent CLI cancellation problem. It does not justify catching native aborts or manufacturing a successful exit.

## Unaffected controls and reviewed work

Root independently replayed the existing built-CLI observer with a separate output directory:
- normal analyst stdio EOF: exit0, one helper reused for warm and request, joined shutdown.
- public embed socket SIGTERM after response: exit0, v1 result, formatted input, absent helper, removed socket.

known-good-cli contains those traces. The analyst stdio fixture deliberately has an invalid database, so its query produces the expected database error after embedding. Neither control loads a real model.

The worker's host reuse, socket lifecycle, dependency and test evidence is retained. Root verified all27 source hashes in the worker observation packet. Root did not rerun the whole reported suite after finding this blocker and makes no independent full-suite-pass claim.

The callback/worker/socket/command ownership wiring and stateless bounded compatibility facade should be preserved. No broad rewrite, native-owner audit or packaging work is requested.

## Executed POST and supplemental PRE

signal-solve.json.gz contains two OPEN Jev interpretations, catalog cards, exact family requests, responses and eight pinned reloaded lowerings. Root added uniqueness and cardinality; facts were unchanged. Each lowering has44 variables.

Observed POST: UNSAT/fail, sol_dd6181fdebd542a8. This is a failed runtime contract for the four observed orphan cases, not a proof of successful shutdown.

Rework PRE: SAT/pass, sol_0d7ea86bb84b41db. Its handled/cancelled-and-joined state is a proposed target, not implemented behavior.

Six expected PRE rejections:
sol_1e49b51bd1894cef, sol_7a10bb79bd1d4c2d, sol_8e3feb95ed2d4d34,
sol_28020d6aae2941a2, sol_ec34aa664d9d479c, sol_cbedaf22823c45cc.

Controls reject a live child after completion, unhandled startup signal, hidden query cleanup failure, pretending success without joining, omitted query coverage and duplicated case coverage. All statuses and SAT models matched reloads. These finite models validate declared/observed tuples; they do not execute Rust or prove all schedules.

## Required targeted retry

Preserve all authored a45a3243e33e8bb9a3be8d364507eceaf1fa94e1..0782df09713227be53b7161e2157dfa85d046960 commits in order and import the new docs-only root proof commit named in task feedback. Do not replay approved A history already in the base.

Add actual built-CLI subprocess regression tests before production changes and record assertion RED. Install ordinary termination handling before warm-up can start; keep it active through warm, idle serving and query work. On termination stop admission, cancel owned work and join/reap before command completion; retain visible cleanup errors and prior completed results. Preserve conventional exit semantics as appropriate; do not force success, intercept native SIGABRT as success, forget owners or detach cleanup.

Cover the four reproduced cases plus orderly idle termination and both commands' shared path. Test SIGTERM/SIGINT as applicable. Use fixture readiness/barrier events, not lucky sleep ordering, and always clean only experiment-owned children after recording a failure. The existing limits/deadlines remain authoritative; no new shipping timeout guesses. The CLI may retain bounded awaited warm if termination stays responsive; concurrent tracked warm is also an implementation choice.

Document that injected blocking callbacks must eventually return; arbitrary non-returning Rust callbacks cannot be safely preempted. Do not solve that unrelated API limitation with a new native lifetime redesign.

Keep existing tests and observations. Re-run focused new regressions, full embedding/pack suites and affected callback/worker/CLI checks, normal/build dependency boundaries, library compilation and formatting. Preserve the separately baselined native test exclusion transparently, rather than claiming it passed. Root will independently validate and execute fresh B POST from the fixed outputs before approval. C remains pending and packaging remains blocked on bd-1ktd.
