# S6B lease recovery

Canonical issue bd-7gja, plan805bc9eb-cc70-4d4c-8784-f35776e70a5d, task s6-host-integration.
Expired delegation7197242484c44f5f, attempt2. No approval or release of C.

Protected checkpoint69d7f9ae269aa13c5d95d3de3e20893440a11014 on spur/review-evidence/embedding-s6b-attempt2-recovery contains all authored history. The production fix is fc45253d728c4db3b393bf1f5ae3fed615006694; feature-gating test refinement e1b07433d5ed98b97a2bf995371b8592fe9a36d6; signal evidence69d7f9ae. Earlier tests4660d4f35/5e6ccfa43 and RED evidence remain reachable.

The plan engine reported dispatch lease expiry, without a worker result branch. Recovery found the worktree and in-flight check. The old worker continued saving work after expiry; two guarded copies detected HEAD advancement and did not copy a mismatched source. Root protected the eventual checkpoint, then copied30 additional stable, complete evidence files from the expired worktree into this isolated recovery tree. No production files differed from the protected checkpoint. The old worktree and branch are retained for any late evidence.

cancel_delegation returned Unknown delegation after the engine cleared its orphan record. Root did not kill an unrelated process or start another Cargo build. The identified old Cargo/check shell processes had exited before the recovery handoff; the next worker must still check for active shared-target writers before building.

## Scope signal resolution

Signal4f277590-3e74-4224-8b8f-3487569382a3 proposed a shared spur-mcp seam because cancelling Tokio stdin could leave a blocking read alive. The implemented alternative is CLI-local: commands/mcp_stdin.rs uses safe nix nonblocking flags and Tokio readiness for Unix pipe/socket/terminal input. The command owns the RMCP cancellation token and joins shutdown. No shared spur-mcp production file changed.

Root accepts this local scope for continued verification. Do not expand into shared transport or native-owner redesign. The flag restoration, EOF and zero-length AsyncRead behavior deserve focused validation; any correction must begin with a failing test. Unix behavior is the observed scope; do not infer independently verified non-Unix behavior.

## Recovered evidence

host-signal-rework now includes the source diff, source identity, original failures, first-fix timeout observations, rejected unsafe build, signal GREEN, normal controls, feature/dependency/format checks and POST candidates.

Root verified:
- All31 source hashes in the worker's typed source manifest match.
- Eight signal cases reached setup, exited with status1 and had no recorded fixture PID alive after return.
- The injected shutdown failure was reported, rather than converted into success.
- Two normal built-CLI controls report exit0 and no errors.
- The recovered command-status sets contain successful CLI, embedding, pack, core, feature, dependency and formatting checks. Original sandbox socket failures are retained alongside socket-enabled passing reruns.

These are worker observations and archive/hash verification, not independent root reruns or root POST. No neural model was loaded.

recovered-evidence-sha256.json covers committed signal evidence; rescued-late-evidence-sha256.json covers the30 rescued files. source-sha256.json binds the seven source/test files changed during the signal retry. verify.py checks all of these plus the complete typed worker source manifest.

## Resume contract

Retry the escalated task through submit_plan_mutation, preserving agent/profile codex, model gpt-6.1-terra, effort xhigh, and dependencies. Continue from the protected checkpoint plus this docs/evidence-only recovery commit; do not replay A or reimplement B.

The original host PRE1SAT+10UNSAT and signal PRE1SAT+6UNSAT remain the unchanged contract. Reload from host-pre/pre.json.gz and host-review/signal-solve.json.gz; old observed failing POST stays historical. Root will run fresh whole-B POST from final actual output before approval, and prepare C PRE before releasing C.

First run this verifier on the frozen recovered source. Review the CLI-local stdin adapter and close any concrete edge-case gap with test-first minimal changes. Re-run relevant checks only for new changes or unresolved failures; do not discard already-passing observations or start a cold target. If no production change is needed, finish the missing evidence/handoff promptly. Commit evidence before any long check and keep progress updates bounded.

C remains pending. Original packaging bd-3pcp remains blocked on canonical C bd-1ktd. No whole-S6, real-model, packaging or notebook acceptance is claimed.
