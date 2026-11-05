# Supervised embedding helper

Approved by the user on 2026-11-03 following the [industry evaluation](../../research/artifacts/2026-11-03-ort-industry-evaluation/REPORT.md). Design approval issue: bd-16wv. Implementation source epic: bd-2tan; master bd-hdi.

## Outcome

Default graph and analyst embedding functionality runs through a separately built supervised executable. The Spur host keeps DuckDB/DuckPGQ and notebook/graph behavior and has no native inference package in its normal/build dependency closure or loaded runtime. A native inference crash becomes a typed operation failure at the process boundary. Native inference still shuts down cleanly in ordinary operation.

This implements the user's approved architecture and per-task TDD with inline Jev PRE and POST catalog execution. It does not claim general native memory safety, complete panic detection or completion of the independent Rust declaration gaps.

## Source and lineage

Start from preserved candidate ceed07a502a1dc805b66b09d0d807de89f58d7d8, whose production code matches37727b282d4d69d250bcd92a5aa7be4ab387a750. It contains previously tested vendor fixes and incomplete verification evidence, not an approved broad ORT ownership proof. The industry report01ab4b440 is carried as c765def46. Old source/ref protection and evidence remain intact.

Old plan5c42bcb3-b410-48a9-b752-373739af0ac1 is superseded by this architecture. Cancellation does not mean acceptance or merge. Its broad V1/O2/N1 proof obligations are replaced by the concrete native resources exercised through the exposed inference API. G2 kind/capture/provenance and later G3–G5 remain approved work and resume after the new runtime gate.

## Architecture and ownership

Two new focused crates have different dependency boundaries:

- spur-embedding: pure Rust wire types, validation, model identities and supervisor/client. It has no FastEmbed, ORT, DuckDB or native inference build hook.
- spur-embedding-worker: separately executed binary; owns FastEmbed/ORT initialization, bounded model cache and native resources. It depends on the neutral crate, never graph/analyst/DuckDB.
- spur-graph and spur-analyst: depend only on the neutral client for embedding execution. Both current native model caches and all loaders must migrate.
- Existing analyst external Unix socket v1 and spur embed serve remain supported through a thin proxy/translation layer. Internal worker framing is a separate strict contract.
- Build/install tooling distributes the executable next to the host, with compatible platform/version and explicit executable override. The host never compiles or downloads executable code at runtime.

The helper owns all native work and closes it before environment teardown. The supported API is inference and model caching only. Editor/training/custom-op capability does not expand with this project. Existing broad native regression tests are relocated to the package that owns the dependency, rather than deleted or silently skipped. Final default product packages exclude owner-test-hooks and fault fixtures.

## Wire and completion contract

A big-endian u32 length precedes one UTF-8 JSON envelope. Validate size before body allocation. Types include model ID/fingerprint, protocol version, session generation UUID, monotonic request ID and typed Hello/Ready, Embed/Embeddings, Shutdown/Closed and Error messages. Unknown fields, invalid/duplicate required fields and version disagreement fail.

Model identity covers selected model, actual model/tokenizer artifacts, dimensions and inference configuration (prefix/pooling/normalization). The handshake pins this identity; every reply and replay must match it. There are finite bounds for frame bytes, text count, per-text/total bytes, queue/cache and pending requests with checked arithmetic. New capacities are explicit validated policy parameters until product defaults are grounded in caller constraints and measurements.

Validate exact response rows, dimensions and finite floats before publication. The helper returns vectors and owns no graph/database writes. Host publication is keyed to request/model/source identity and happens only after validation. A one-shot invocation requires a complete validated response and normal successful exit. For a long-lived process, validated committed replies complete their requests; later crashes fail still-pending work and report shutdown failure without erasing previous committed responses.

Client process/thread/pipe ownership must avoid leaked child processes and indefinite waits. Admission, start, request, cancel, restart and close operations use monotonic deadlines and finite restart budgets. A thread is not a native abort boundary. A helper using the same ORT-linked host binary is insufficient for the native-library separation goal.

## Resource shutdown

Stop accepting requests, finish or explicitly fail pending work, join owned jobs, drop retained outputs/inputs/model sessions/allocators according to their API owners, then release the environment. Checked close rejects premature release. Main returns normally. Do not use forced successful exit, forgotten objects, fake frees, native abort suppression or a result printout as proof of clean shutdown. OS termination after a bounded shutdown timeout is containment and must be recorded as failure.

## Compatibility

Preserve the three supported model selections and current vector dimensions, prefixes, normalization/pooling, CodeRank metadata, tokenizer/cache paths, batching and graph storage identities. Existing disabled-embedding flags must not launch the helper. Optional retrieval degradation remains explicit. Default embedding capability remains available through the distributed helper; success only with features disabled is insufficient.

SPUR_ANALYST_EMBED_MODE=inprocess may become a documented alias for the local supervised helper or an explicit deprecation error, but cannot re-enable native inference in the host. Existing Auto/Sidecar/Off and SPUR_EMBED_SOCKET usage must have tests.

## Per-task Jev, SOLVE and TDD gates

Before any production change: navigate the current catalog, run a fresh Jev PRE compile with grounded inputs, inspect its gate and exact binding, execute the matching family request through solve_rules, persist/reload its exact request/results. Jev confidence alone proves nothing. Explicit generic-only bounds follow typed-language discovery/preflight; Jev can additionally compile a relevant family binding and the evidence must state the distinction.

Commit a failing test before implementation, observe its actual RED, implement minimally, run GREEN and affected checks, then run fresh Jev POST on actual exported state/observations, inspect and solve/persist/reload again. Include independent negative controls and bind source/build identity. Python booleans or an expected table copied to actual data cannot certify resource ownership. Preserve compile RED honestly as compile-only.

Original industry receipts (including sol_e5e18e0275d54fb2 for the proposed helper and sol_332c51ffdca940ba for a hypothetical in-process deployment without containment goals) justify the declared architecture; they are not substitutes for fresh per-task checks. No claim of globally optimal timeouts or complete Rust/FFI memory safety follows from these finite policy checks.

## Acceptance

The unchanged default CLI DuckPGQ test returns five rows including the repeated serde::Serialize. The host artifact is inspected, not just its Cargo manifest: no ORT static objects, symbols or runtime loading; the helper is free of DuckDB/PGQ. Real product embeddings match a reference and repeated native shutdown exits normally.

Actual child abort, malformed/mismatched/truncated responses, nonfinite values, hangs, queue pressure, cancellation, restart exhaustion and shutdown failure preserve host operations and reject uncommitted output. Dependencies, runtime/model hashes, features and raw process statuses are recorded. Normal no-embed behavior, external socket compatibility and deployment failure messages are tested.

Measure cold/warm request latency, throughput and peak RSS for the actual supported host. Any absent control platform/model asset remains an explicit unresolved gate. Use a fresh Notebook MCP notebook with Rust cells to show real helper operations and host continuity; old/live notebooks and ../spur-notebook sources/CURRENT are untouched.

## Scope and execution

Cross-crate changes are necessary to remove both existing native loaders and ship the companion executable. Workers are sequentially reviewed through Plan Engine. The user's latest routing on2026-11-05 is explicit codex/profilecodex/gpt-6.1-sol/xhigh, superseding Terra. User approval covers implementation and local verification; no repeated confirmation is required.

Use SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo, one writer to the shared target. Preserve dirty main and all historical evidence. A real new defect is signaled with source/runtime evidence and addressed within the relevant scope; it does not restart the broad upstream ownership audit automatically.

## Current shipped-packaging rework entry gate (2026-11-05)

M1–M4 metadata/policy/discovery/cache/batching and complete local stage-A acceptance are independently reviewed in [root M4 evidence](../../research/artifacts/2026-11-05-embedding-sol/root-M4-review/README.md). Initial M5 installed/native evidence passed replay, but `shipped-packaging` (`bd-2zan`, plan`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`) is not approved: equivalent CLI install paths omit the helper, unrelated suffix-matching packages trigger helper installation, and the npm compatibility probe can hang indefinitely. See [root M5 review and current narrow PRE](../../research/artifacts/2026-11-05-embedding-sol/root-M5-review/README.md).

On a clean retry, fast-forward merge protected `spur/review-evidence/embedding-sol-m5-root` to recover the complete implementation plus committed PRE/tests/evidence. Before edits/builds run its `rework-pre/verify_pre.py` and reload `sol_30590f8b03b84f3f`, `sol_7c7ea39c8f77452b`, and `sol_bcf862aef5ff43ca` (2 SAT, 1 UNSAT). The gate binds64 source files, nine repair requirements and seven compound predicates to fingerprint `7eca8eb22d7eaa73296b6a7c5a8bafe8b639a3ec7cdc5eedb0b8758efd681033`. The old62-file M5 PRE is historical. Preserve regression commit `4e6ae0dc9a073374d301ed42beb2a8c23932f2f4` and observed runtime REDs. Repair path-role selection and bound/reap metadata probes using inherited startup/cleanup allowances; do not invent new capacities. Keep codex/profilecodex/gpt-6.1-sol/xhigh routing.

After focused GREEN and affected packaging checks, ROOT must execute fresh complete M5 POST and supply the final runtime/notebook PRE before approval. Preserve initial evidence and all test/fix/evidence commits. No numeric/native/owner/profile redesign, publication, service restart, main merge or external notebook source edit is authorized by this rework. Final runtime/performance/containment and fresh Rust Notebook acceptance remain separate.
