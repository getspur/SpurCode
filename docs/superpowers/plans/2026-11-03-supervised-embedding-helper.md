# Supervised Embedding Helper Implementation Plan

> **For SPUR orchestrator:** This plan is designed for submit_plan with explicit task issues.
> Each task becomes a beads issue with spur:plan-task-id and spur:plan-id labels.

**Source spec:** docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md
**Formal @spec cells:** none; user-approved industry report is the source design.
**Design epic/approval record:** bd-16wv (closed on explicit user approval)
**Implementation source epic:** bd-2tan; master bd-hdi.
**Goal:** Preserve default embeddings and graph/notebook availability through a native-free host and separately supervised native inference executable.
**Architecture:** Native-free protocol/client crate; separate inference-only worker crate; migrate graph then analyst; package both; independent runtime/notebook acceptance.
**Tech Stack:** Rust2021, serde/JSON bounded framing, OS processes, existing FastEmbed5.8/ORT2rc10 with native1.22, existing DuckDB1.4.4/PGQ.
**Base:** ceed07a502a1dc805b66b09d0d807de89f58d7d8 plus approved report c765def46; root isolated branch spur/implementation/embedding-isolation.
**Routing:** codex/profilecodex/gpt-6.1-terra/xhigh for every task, honoring persistent user preference. Brain owns architectural coordination; worker scopes are bounded. Delegate output is unified diff plus one-sentence rationale and mandatory evidence.
**Ordering:** S1 → S2 → S3 → S4 → S5 → S6 → S7 → S8. Serial because public protocol/model/client interfaces, manifests and shared target are coupled. No broad parallel native builds.
**Approval:** User already approved implementation and the PRE/POST Jev plus TDD workflow. Submit and execute without another choice prompt.

## Shared discipline
CONTEXT: User-approved supervised embedding isolation, source epic bd-2tan/master bd-hdi, approved design bd-16wv. Agent codex, profile codex, model gpt-6.1-terra, effort xhigh are explicit persistent user preferences. Brain owns the architecture and serial DAG; honor this focused task despite codex's soft multi-file coordination avoidance. Native candidate base ceed07a502a1dc805b66b09d0d807de89f58d7d8 preserves production37727b282d4d69d250bcd92a5aa7be4ab387a750 and incomplete V1 evidence. It is NOT approved native-lifetime proof. The former plan5c42bcb3-b410-48a9-b752-373739af0ac1 is being superseded; all old bd IDs and old completion demands are historical lineage only. Read approved spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md and the industry REPORT.md/evidence.json. Do not restart the broad 48-owner audit. The installed inference-only worker still requires genuine clean native shutdown.
PROCESS PER TASK (mandatory): (1) verify actual assigned issue in_progress and correct dispatch binding, use that exact issue for durable audits/signals; never infer a delegation ID from branch components. (2) Fresh catalog discovery, Jev PRE compile with complete source-bound facts/subjects/bounds; inspect gate/family/mode/rule/parameters; execute through solve_rules, persist and reload full request/status. A Jev gate is interpretation only. Never lower a blocked gate or call unknown/timeout unsat. For a generic-only bound, Jev can compile a separate relevant family binding; discover/check the generic surface for the remaining theory and record both. (3) Commit test first, observe genuine RED before each production behavior change; new API compile RED is allowed but distinguish it from assertion RED. (4) Minimal implementation commit and meaningful GREEN/affected checks. (5) Fresh Jev POST compile from actual exported outputs/traces/source/build identity, inspect then catalog solve, persist/reload, plus negative controls. Original industry receipts are rationale, not this task's PRE/POST. A constant fact table or a boolean copied from the expected ledger is not evidence of actual implementation behavior. Keep independent expected policy and explain the bounded projection.
ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only, fmt local, one cargo writer in /private/tmp/rust-gap-g1-acceptance-target; alternatives/control targets separate. No bare cargo, recursive build-script cargo, compiling at runtime, edits to dirty main, global runtimes, live notebooks or ../spur-notebook graph/CURRENT. No AWS fallback loop. Scope signals early; unchanged inherited failures are retained, not hidden. No abort swallowing, _exit/process::exit(0) as cleanup, mem::forget, fake native frees or enabling embeddings only in tests. No in-process ORT fallback in the final host. Do not discard existing native tests: relocate ownership to the native worker when its dependency moves. Test-support features must stay out of product dependency trees.
OUTPUT: Unified diff + one-sentence rationale, test/fix commit IDs, exact RED/GREEN commands/results, compact verification table, full Jev PRE/POST provenance/requests and solver receipts/reloads bound to actual outputs in docs/research/artifacts/2026-11-03-embedding-isolation/<TASK>/. Keep evidence compact, preserve historical files. Commit finished work, completion audit on your actual issue, leave in_progress for root review; no self-approval, merge or downstream dispatch.

## Task DAG
### S1 Add the native-free embedding wire contract

**Task ID:** embedding-protocol
**Depends on:** none
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `Cargo.toml`, `Cargo.lock`, `crates/spur-embedding/Cargo.toml`, `crates/spur-embedding/src/lib.rs`, `crates/spur-embedding/src/protocol.rs`, `crates/spur-embedding/tests/protocol.rs`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S1/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Create a host-safe spur-embedding library and a strict, bounded versioned stdio protocol, without fastembed, ort, ort-sys, DuckDB or inference initialization in normal/build dependencies.
CONTRACT: Define public ModelId for the three currently supported selections (NomicEmbedTextV15, CodeRankEmbed, JinaEmbeddingsV2BaseCode), ModelIdentity, ProtocolLimits, typed request/reply/error envelopes and pure validation. Internal framing is a big-endian u32 byte length followed by one UTF-8 JSON envelope; reject an oversized length BEFORE allocating/reading its body. Deny unknown/duplicate required fields and wrong protocol versions. Bind session generation UUID and monotonic request ID. Hello negotiates the exact selected model identity, expected dimensions and complete model/tokenizer/pooling/normalization fingerprint; Embed carries this identity plus bounded texts; Embeddings echoes it and exact rows/dimensions; Shutdown returns Closed only after cleanup. Inputs/rows and per-text/total/frame sizes are bounded with checked arithmetic; limits are explicit validated inputs, not arbitrarily optimized constants. Preserve the external legacy analyst socket v1 separately; this is a new internal protocol.
TEST FIRST: roundtrip all variants, partial/truncated header/body, oversized declared frame, trailing/invalid JSON, unsupported version, stale generation, wrong existing request/model/fingerprint, row/dimension mismatch, nonfinite vector, empty/over-limit fields and arithmetic boundary. Use a Read probe proving an oversized header does not trigger body read/allocation. Tests assert predicates at configured limit boundaries, not lucky defaults.
GREEN: local spur-embedding tests and normal/build dependency-tree check show no native inference/DB packages. Publish small stable APIs (read_frame/write_frame and reply validation) usable by S2/S3; no subprocess or native worker implementation in S1. Jev/catalog POST uses actual decoded/rejected case exports, not manually labeled pass booleans.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-embedding`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S2 Supervise real embedding child processes

**Task ID:** embedding-supervisor
**Depends on:** embedding-protocol
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-embedding/Cargo.toml`, `crates/spur-embedding/src/lib.rs`, `crates/spur-embedding/src/client.rs`, `crates/spur-embedding/tests/supervisor.rs`, `crates/spur-embedding/tests/support/fault_worker.rs`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S2/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Implement the native-free reusable client/supervisor against the approved S1 protocol, using a separately executed program and real OS child processes.
CONTRACT: EmbeddingClient owns a bounded request queue, generation/request correlation, explicit startup/request/shutdown deadlines, bounded restart policy and process/pipe/thread ownership. Spawn via Command, never shell interpolation or fork-only inheritance of initialized native state. Make all waits, queue admission and retries account for the caller's monotonic deadline. Validate replies before completing requests. In-flight abort/EOF/malformed frame/mismatched reply/timeout produces a typed failure, never a successful empty vector. Reap terminated children and close pipes; cancellation must not deadlock or leave blocked callers/reader threads. Serialize or bound concurrency; do not mix generations or replay unacknowledged work without request/model identity. Budget exhaustion stops restart; deterministic errors are not endlessly retried. Long-lived valid committed responses remain completed if the helper later crashes; shutdown failure remains visible. Use explicit validated policy parameters; product defaults are selected at S7 from caller requirements/measurements.
TEST FIRST: a native-free test-only helper executable under required test-support features exercises actual stdio framing and actual std::process::abort, before reply, mid-frame, after a valid reply, and on shutdown; also hang, wrong generation/model/dimension, truncated output, bounded-queue pressure and delayed cancellation. It is a protocol/fault fixture, never a fake ORT result used as inference acceptance. Verify parent stays alive, pending outcomes correct, retries bounded, children reaped and handles released. Keep fixture features out of normal deps. POST uses independently recorded child statuses/request lifecycle traces. No native inference or changes to graph/analyst callers in this task.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-embedding`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S3 Create the separate native worker and scoped runtime

**Task ID:** embedding-worker-lifecycle
**Depends on:** embedding-supervisor
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `Cargo.toml`, `Cargo.lock`, `crates/spur-embedding-worker/Cargo.toml`, `crates/spur-embedding-worker/build.rs`, `crates/spur-embedding-worker/build_support/ort_runtime.rs`, `crates/spur-embedding-worker/src/main.rs`, `crates/spur-embedding-worker/src/runtime.rs`, `crates/spur-embedding-worker/tests/native_lifecycle.rs`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S3/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Add a separately built spur-embedding-worker executable depending on spur-embedding and native inference packages, with scoped native ownership and real clean exits.
CONTRACT: The worker must not depend on spur-graph/spur-analyst/DuckDB/DuckPGQ. Reuse the exact pinned official macOS ORT1.22.0 bundle/checksum and existing materialization behavior, moved or copied into worker-owned build support for now; other platforms retain their supported native packaging. Use the existing managed environment capability narrowly for sessions/owned inputs/outputs used by inference; do not widen unsupported editor/training/custom-op APIs or make broad O1 audit acceptance a prerequisite. Runtime owner permits no new work after closing, joins its own work, drops native values/sessions before checked environment shutdown, then lets main return normally. stdout is framing only; logs to stderr. Hello/Shutdown framing comes from S1. Native initialization failure is a typed error. S4 adds product model loading, so do not fake production Embed success in this task.
TEST FIRST: use the existing tiny real ONNX fixture for deterministic real inference; repeated start/run/drop/close normal exits, retained owned output prevents premature environment close, native init error, idle close, repeated close, active-request shutdown and real signal termination observation. Assert actual native release/clean exit without replacing C API tables. Native samples and test-support stay in the worker crate; normal helper build excludes owner-test-hooks. Check worker normal/build tree is free of DuckDB/PGQ and client normal/build tree free of ORT. Do not yet remove graph native code, since S5 owns that migration.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-embedding-worker`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S4 Move real embedding model loading and caching into the worker

**Task ID:** embedding-worker-models
**Depends on:** embedding-worker-lifecycle
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-embedding-worker/Cargo.toml`, `crates/spur-embedding-worker/src/main.rs`, `crates/spur-embedding-worker/src/models.rs`, `crates/spur-embedding-worker/src/runtime.rs`, `crates/spur-embedding-worker/tests/embedding_models.rs`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S4/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Serve actual product embeddings over the protocol with a finite, droppable model cache owned by the worker.
CONTRACT: Move the loader behavior from spur-graph/src/store/lance_sections.rs into worker-local models code without importing spur-graph. Preserve all existing supported model IDs, output dimensions, text prefixes, pooling, normalization, CodeRank custom output/metadata, tokenizer/cache paths and hf-hub behavior. Identify actual model/tokenizer artifacts and inference configuration in the handshake fingerprint; pin it for every batch and replay. Bound model cache/queue memory using declared policy, reject unsupported combinations. Reuse loaded models across batches; no static immortal TextEmbedding cache. Drop all model sessions and outputs before S3 environment shutdown. Traces must not leak input content or credentials. Preserve root vendor fixes, but restrict actual worker execution to supported inference APIs.
TEST FIRST: real supported cached model inference through a real worker/client verifies exact row count, finite dimensions and agreement with an independent existing reference embedding (including normalization/prefix behavior), wrong identity rejected and a second batch uses the same generation/cache. Tiny real ONNX remains a deterministic release/control fixture, not a substitute for product-model acceptance. Check repeated model load/infer/close exits normally. If model assets are unavailable, record the exact missing artifact and continue unaffected checks; do not label a skipped real-model gate as pass. POST models actual response/cache/ownership observations. No host integration yet.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-embedding-worker`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S5 Route graph embeddings through the native-free client

**Task ID:** embedding-graph-routing
**Depends on:** embedding-worker-models
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-graph/Cargo.toml`, `crates/spur-graph/build.rs`, `crates/spur-graph/build_support/ort_runtime.rs`, `crates/spur-graph/src/store/lance_sections.rs`, `crates/spur-graph/src/store/mod.rs`, `crates/spur-graph/src/store/ort_runtime.rs`, `crates/spur-graph/tests`, `crates/spur-embedding-worker/tests`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S5/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Graph section/code-symbol embedding production paths use spur-embedding client; spur-graph normal/build dependencies cease linking/loading native inference.
CONTRACT: Keep existing stored model selection, dimensions, vector identity, batching semantics and skip flags. Replace raw TextEmbedding caches/loader returns with client/model handles and migrate callers; there is no silent native in-process fallback. Remove fastembed/ort/hf-hub inference imports and macOS native bundle build hook from graph production; move their materialization ownership to the worker. The native tests currently in graph (ort_runtime_cache, ort_lifecycle, ort_shutdown, ort_owner_constructors, ort_custom_allocator, o1_node_attribute_view_review, o1_initializer_names and required ONNX fixtures) must be relocated to worker-owned tests or explicit standalone wrapper regression targets while retaining assertions, real behavior and historical evidence. Do not delete, ignore or weaken them to remove a dependency; keep broad wrapper test features out of normal CLI/helper trees. Adjust graph source references only for the relocation and neutral client migration.
TEST FIRST: graph-level embedding invocation, disabled flags never start helper, correct model/batch/vector artifact persistence, worker error/death rejects pending publication and no source identity drift; use actual client/protocol. Add dependency boundary regression. Run affected embedding/store/artifact/incremental tests, native relocated tests and graph all-target checks. Existing native-broad tests may expose a real regression: signal with evidence rather than restart the full 48-row research audit. S6 still removes analyst native deps, so do not claim the whole CLI is ORT-free yet.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S6 Route analyst and legacy socket embeddings through the helper

**Task ID:** embedding-analyst-routing
**Depends on:** embedding-graph-routing
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-analyst/Cargo.toml`, `crates/spur-analyst/src/embedding/mod.rs`, `crates/spur-analyst/src/embedding/in_process.rs`, `crates/spur-analyst/src/embedding/model_cache.rs`, `crates/spur-analyst/src/embedding/runtime.rs`, `crates/spur-analyst/src/embedding/mode.rs`, `crates/spur-analyst/src/embedding/sidecar_service.rs`, `crates/spur-analyst/tests`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S6/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Analyst query embeddings, warm-up/cache/jobs and the public socket service retain behavior through the separate helper, with no FastEmbed/ORT normal/build dependency in analyst or host.
CONTRACT: Remove native TextEmbedding caches/loaders and analyst fastembed dependency; reuse neutral model/client handle and bounded supervisor. Preserve SPUR_EMBED_SOCKET and external socket v1 request compatibility via translation to internal S1 protocol. spur embed serve must proxy to the separate helper, not initialize ORT. Preserve Auto/Sidecar/Off behavior and test override isolation. The old SPUR_ANALYST_EMBED_MODE=inprocess selection cannot restore native execution: keep a documented compatibility alias to the local supervised helper or an explicit deprecation error, never silent fallback. Warm-up jobs and cancellations are owned/joined and have bounded queue/deadline semantics; avoid orphaned detached loaders and clone/handle cycles. Optional retrieval degradation must be explicit; do not count missing embeddings as successful embedding computation.
TEST FIRST: existing analyst embedding/socket tests plus local-helper selection, external-v1 translation, no-embed mode no process, missing helper actionable error, worker exit/error/cancel during warm and query, shared client reuse and teardown without leaks. Use actual request/response traces for POST, including wrong model/generation negatives. Check normal/build dependency closure of spur-cli, spur-core, spur-graph and spur-analyst excludes ort/ort-sys/fastembed even with default product embed features. No release packaging changes except tests needed to demonstrate this contract; S7 owns deployment.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-analyst`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S7 Package and discover the helper for normal CLI workflows

**Task ID:** embedding-packaging
**Depends on:** embedding-analyst-routing
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-embedding/src/client.rs`, `crates/spur-cli/src/commands/embed.rs`, `crates/spur-cli/tests/embedding_worker_cli.rs`, `scripts/spur-cargo`, `scripts/cloud-build`, `xtask/src/main.rs`, `docs`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S7/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Default product CLI builds/runs/tests/distribution can find the separately built helper and preserve embedding capability without ORT in the host artifact.
CONTRACT: Use existing build/release layout conventions, discover a platform/version/protocol-compatible sibling spur-embedding-worker executable with an explicit executable-path override for controlled deployments/tests; execute it directly without shell parsing or cwd-relative surprise. Missing/mismatched helper returns actionable typed error. Build/install both required artifacts through repository wrappers/release packaging, no recursive cargo in build.rs or runtime compilation. Inspect actual release/install entry points before editing; planned scripts/xtask paths are scope roots, not permission to rewrite unrelated release systems. Preserve CLI command syntax and embed flags; SPUR_EMBED_SOCKET can remain an external service choice. Set product capacity/deadline/restart defaults only from existing caller contracts and measured cold/warm startup/inference behavior, with fresh catalog/resource bounds and user-configurable validated policy. Do not invent performance optimality from a SAT model.
TEST FIRST: helper lookup with spaces, override and version mismatch, default CLI with actual helper, negative missing-helper, helper normal exit/crash, no on-demand model compiler, and package manifests/install layout. Run the unchanged analyst_temporal_views::analyst_build_emits_external_dependency_surface_views against the actual default embedding-enabled host artifact, expecting the original five rows including duplicate serde::Serialize. Inspect normal/build deps and Mach-O linked/exported symbols/runtime loaded libraries to confirm host has no ORT and helper has no DuckDB/PGQ. Handle dev-feature unification honestly by inspecting the actual host artifact, not only cargo tree text. Preserve default capability, not just --no-default-features success. Record actual packaging limitations per platform rather than claim untested cross-platform execution.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-cli`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

### S8 Verify crash containment, real embeddings and notebook evidence

**Task ID:** embedding-acceptance
**Depends on:** embedding-packaging
**Suggested worker:** codex / codex / gpt-6.1-terra / xhigh.
**Planned file scope:** `crates/spur-cli/tests/embedding_worker_cli.rs`, `crates/spur-cli/tests/analyst_temporal_views.rs`, `crates/spur-embedding/tests/supervisor.rs`, `docs/research/artifacts/2026-11-03-embedding-isolation/acceptance`, `docs/superpowers/plans/2026-11-03-supervised-embedding-helper.md`.
**Evidence scope:** docs/research/artifacts/2026-11-03-embedding-isolation/S8/.
**Scope boundary:** Only this unit and the precise glue needed to compile it. Emit scope_drift before changing predecessor contracts or unrelated files. No graph/native-inference claims ahead of their acceptance gate.

GOAL: Complete independent runtime, dependency, regression and notebook acceptance for the shipped narrow inference architecture.
REQUIRED: Preserve the original PGQ assertion/query; run the complete relevant graph suite (all targets/doc tests), analyst and CLI embedding/temporal regressions with actual default capability. Repeated real product-model embeddings and native clean exits must pass. Inject actual child abort before reply, mid-frame, during work and at shutdown; reject wrong generation/model/row/dimension/nonfinite/truncated responses; exercise timeouts, bounded queue/restart, cancellation and pending publication. Assert host graph/notebook query continues and no pending results are silently certified. Test old caches/model identities and public socket compatibility. Inspect exact host/helper artifacts, pin runtime/model/tokenizer/build hashes, confirm no test-support/owner-test-hooks in normal packages.
Measure cold/warm latency, batch throughput, peak RSS and shutdown time on the actual supported host; keep workload/methodology and distinguish per-process from combined memory. Compare to a preserved supported in-process control when runnable; if unavailable, report missing comparison, not fabricated improvements. Defaults must satisfy declared constraints; performance is measured, not proved by family SAT.
Use notebook MCP (navigate/read skill before any notebook operation) to create a NEW isolated acceptance notebook, run Rust cells that exercise the built client/helper and query continuity around injected child failure, and display measured outcomes. Do not edit prior notebooks, live ../spur-notebook project or its graph/CURRENT. Notebook tools are available in current session; if actual runtime cannot execute Rust, record that distinct blocking gate and continue the unaffected acceptance instead of silently substituting a different runtime. Native Rust experiments must run the actual separately built helper and parent, not catch_unwind pretending to intercept SIGABRT.
Each new discovered regression receives a fresh Jev PRE→test RED→fix GREEN→Jev POST cycle. Final POST checks independent expected policy against actual exported process/status/publication/dependency rows and source/build digests, with stale/forged/missing/duplicate negatives. Old broad O1 verification remains historical and unapproved; this acceptance covers only the actual exposed inference surface and real resources it uses. Provide compact durable evidence and leave review pending. Do not claim the G2 declaration gap or G3–G5 complete; root resumes those approved objectives after this runtime gate.

**Ordered implementation and checks:**

1. Verify actual assignment, then fresh Jev PRE compile → inspect → catalog solve → persist/reload.
2. Commit semantic tests and run them before implementation:
   `SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-cli`.
   Select actual tests from this task's explicit contract, retain runtime/compile RED distinction and command output.
3. Minimal fix; run the focused test target without arbitrary blessing or default-feature suppression, then affected checks.
4. Fresh actual-export Jev POST compile → inspect → solve → persist/reload, independent negative controls and source/build digest binding.
5. Commit finished code/evidence and durable completion audit; leave issue in_progress pending root review.

## Root self-review

All approved requirements map to tasks: dependency separation S1/S5/S6/S7; framing/identity S1; abort/deadline/restart/publication S2/S8; actual native owner lifecycle S3/S4; product model parity S4/S5/S6; CLI/socket/distribution compatibility S6/S7; unchanged PGQ/default feature/real output/native close S7/S8; measured tradeoffs and Notebook Rust evidence S8. All tasks repeat the required fresh per-task Jev and TDD gates. Bounds are explicit inputs until measured integration defaults. Serial DAG avoids overlapping Cargo/manifest changes and a shared-target race.

The task list specifies behavior-level tests and public protocol contracts rather than speculative implementation bodies. Workers choose the smallest code matching the witnessed RED; no production code is written in this planning document. Any scoped API adjustment is reviewed before downstream dispatch.

## After this runtime plan

Root resumes source phase bd-3nv (G2 kind/schema acceptance, physical foreign declarations, source-bound evidence), then approved G3–G5 in their original order. Those objectives remain open; cancellation of stale dispatched-plan units is recorded as supersession, never as acceptance of graph coverage or panic freedom.
