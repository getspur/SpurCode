# S6B host-integration PRE

Canonical execution issue bd-7gja, source tracking bd-2f21.
Task s6-host-integration; dependency is approved canonical bd-3jqy.
Routing remains agent/profile codex, model gpt-6.1-terra, effort xhigh.

pre.json.gz preserves a fresh OPEN Jev interpretation (jev-1.14.0, weakest confidence 0.97), inspected complete facts, catalog cards, one SAT design and ten expected UNSAT controls. Every receipt was reloaded and pinned; models match. These are PRE acceptance obligations, not claims of implemented host integration.

The five entry cases model local-backend ownership:
- Callback requests share a callback-server owner across per-request modules.
- Direct worker calls share a worker-service owner.
- Analyst socket requests share a socket-server owner.
- CLI calls share their command's owner.
- Compatibility free calls use an explicit host or a bounded operation scope, with no implicit global owner.

Every owner tracks work and joins cleanup while reporting failures. A compatibility scope is allowed to be shorter than a server's lifetime; server and worker paths must still reuse their genuine long-lived owners. Modes and formatting stay as reviewed in A: external-only modes never start a local helper.

SAT: sol_6adee4cacde84d57.
UNSAT: sol_4eef9afb32294e95, sol_a44d7c9290a44906, sol_908038a6985e414d,
sol_6ea4d3b49c9d4abb, sol_27a1bce2683c4b53, sol_018b1493180f4b7b,
sol_a1144ee8fbbc4648, sol_7a0d8f58b11f4cd9, sol_e4bc4f81f3304ab3,
sol_a0d9edcbbdf64184.

All requests compile to 55 variables, below the compiler's 64-variable bound. Controls reject request-scoped callback ownership, missing direct-worker owner, socket/compat globals, detached work, unjoined shutdown, hidden cleanup errors, per-request CLI recreation and incomplete/duplicate host coverage.

## Grounded seams and scope

Exact reviewed source bffec0686421827f625e7e8bbee0dc0de34c4491:
- core/server/handlers/mod.rs: handle_analyst_tool constructs AnalystMcpModule for each request.
- core/worker_server.rs: invoke_knowledge_context_for_worker calls knowledge_context_pack_2 directly.
- analyst/pack/service.rs still uses EmbeddingRuntime::global in both pack variants.
- AnalystMcpModule currently contains project/followup fields, without a passed owned embedding context.

Therefore a cache only on the per-request module is insufficient. Propagate explicit owned handles from actual long-lived host state, including worker direct calls, spawned/blocking jobs, socket lifetime and applicable CLI/service paths. Retire strong global/detached loaders. Preserve external socket v1 and query/model formatting, and make old warm/session regression tests meaningful and passing.

No packaging, UI, lease-engine or unrelated orchestration changes. No reimplementation of A. Use TDD and existing helpers/fault subprocesses. Provide actual host reuse, cancellation, shutdown-error, socket translation/mode and dependency-closure evidence for root POST. C retains three-model real analyst-path acceptance. Do not self-approve.

## Preflight and lineage

The dispatched base must include A's approved source. Import only the new docs-only root review/PRE commit supplied in the task body; do not replay A's source into an engine base that already contains its overlay. Authored history is separately protected; the engine normalizes worker commits.

Before edits, run ../final-review/verify.py and reload/compare every B receipt using get_solve_result. The verifier intentionally binds the frozen pre-edit source. Do not rewrite it to make changed source pass; create new final-source evidence for POST. Host source hashes are included here.

A's mode POST remains prerequisite evidence, not proof of B's new wiring. Root will run B POST from actual outputs before approval. Then C needs its own fresh PRE before release. Original packaging bd-3pcp stays blocked on C's canonical bd-1ktd.
