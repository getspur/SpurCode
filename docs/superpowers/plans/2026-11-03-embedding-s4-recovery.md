# Supervised Embedding S4 Recovery Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md`
**Formal @spec cells:** none; existing approved Markdown spec
**Design epic:** bd-16wv (closed)
**Source epic/master:** bd-2tan / bd-hdi

**Goal:** Finish actual embedding implementation and native-free host integration after two S4 attempts produced only tests/reference evidence.

**Architecture:** Preserve approved S1–S3 at base c6dbb22a1de4308e1070e8742786762d54ea7a39. Three focused S4 gates implement artifacts/strict identity, Nomic inference/cache, then all-three-model acceptance. The original S5–S8 behavior and acceptance follow serially through the plan engine.

**Tech stack:** Rust2021, SHA256, shared framed protocolv2, FastEmbed5.8.0, pinned ONNX Runtime1.22.0, DuckDB/DuckPGQ only in host, Jev/catalog SOLVE, notebook MCP Rust cells.

The previous engine plan's unfinished S4–S8 will be superseded, not approved. Preserve authored histories `spur/review-evidence/embedding-s4-attempt1-authored` and `spur/review-evidence/embedding-s4-attempt2-authored`. Canonical dispatch issue IDs replace all historical source IDs. No source or evidence deletion, dirty-main edits or runtime installation.

User already authorized implementation and local verification; continue that authorization without another execution question. A single replacement tail avoids duplicate dispatch and preserves explicit worker model/effort, which the current split-task draft schema does not carry.

S4A PRE is executed: two open Jev interpretations, two SAT and six expected UNSAT receipts, full requests/provenance/reloads and actual source/assets in `docs/research/artifacts/2026-11-03-embedding-isolation/S4A/root-pre/`. The catalog's one-subject arity correction is recorded. Later nodes receive fresh PRE from approved predecessor exports before root releases them. Root performs actual-output POST before each approval; no activation on guessed numeric defaults.

The exact `submit_plan` payload is in `docs/research/artifacts/2026-11-03-embedding-isolation/S4/recovery/plan-request.json`.

### Task 1: embedding-model-artifacts

**Task ID:** `embedding-model-artifacts`

**Depends on:** none

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `Cargo.lock`
- `crates/spur-embedding-worker/Cargo.toml`
- `crates/spur-embedding-worker/src/main.rs`
- `crates/spur-embedding-worker/src/model_artifacts.rs`
- `crates/spur-embedding-worker/tests/embedding_metadata.rs`
- `crates/spur-embedding-worker/tests/embedding_models.rs`
- `crates/spur-embedding-worker/tests/native_lifecycle.rs`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S4A/`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL S4A: Implement the real cached-Nomic descriptor, checked artifact-byte budget and strict artifact-bound Hello before native startup. This is a production metadata/security unit; product Embed remains explicitly Unavailable until S4B.

FILES: Create src/model_artifacts.rs in crates/spur-embedding-worker; modify that crate's src/main.rs and Cargo.toml only as required for neutral Rust dependencies; Cargo.lock; tests/embedding_metadata.rs; existing tests/embedding_models.rs and tests/native_lifecycle.rs for interface adaptation; evidence S4A/. Do not change graph/analyst/client production code.

ROOT PRE IS ALREADY COMPLETE in docs/research/artifacts/2026-11-03-embedding-isolation/S4A/root-pre/: two fresh OPEN interpretations, 2 SAT +6 expected UNSAT, all persisted/reloaded. Positive IDs sol_a10f258cdf7846fc (artifact/profile tuples) and sol_f2a426003d4a42a7 (artifact byte limit). Read README and pre-proof archive and run verify.py; reloaded results are lowered generic requests, so compare with the saved reloads, not the higher-level family envelope. Original S4 archives/15 receipts remain historical context, not missing prerequisites.

IMPLEMENT:
1. Start from the actual inherited assertion REDs: descriptor_returns_actual_nomic_identity_with_bounded_execution rejects --describe-model; hello_rejects_unverified_artifact_identity accepts forged zero hashes. Preserve the real Nomic reference/end-to-end test for S4B/C; it is expected to remain pending after this metadata-only stage, not ignored or deleted.
2. Add a ResolvedArtifacts record carrying actual source files, total bytes, inference profile and ModelIdentity. Nomic has five files in S4A/root-pre/grounding.json at /Users/kevintruong/.spur/cache/fastembed/models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab. Resolve cache from explicit SPUR_MODEL_CACHE, otherwise existing HOME/.spur/cache/fastembed then XDG_CACHE_HOME/spur/fastembed fallback when HOME is absent. Resolve the actual snapshot and validate ref/path shape; no hardcoded expected fingerprints as implementation.
3. Account actual u64 file lengths with checked_add/checked conversions and reject sum overflow or total > caller limit before loading/materializing model bytes. The known five-file sum548026095 and one-byte-lower548026094 are test witnesses. Artifact bytes do not bound RSS. Hash actual used model and tokenizer/config bytes with SHA256. Define/document a deterministic versioned fingerprint encoding with unambiguous named/length-prefixed parts; bind actual files plus max_length8192, dimensions768, verified pooling/normalization and legacy preformatted-input profile. Use dependency/source evidence for those profile details. Descriptor hashes must change after artifact/config/profile change.
4. Implement --describe-model NomicEmbedTextV15 <artifact_budget> <generation_uuid> <request_id> <seven_protocol_limits> matching the inherited test. Return existing correlated framed Reply::Ready identity and exit normally; stdout framing only, byte limits enforced, no native RuntimeOwner initialization in descriptor mode. Test success even when native runtime path/cache is unusable. Missing files, undersized budget and invalid policy fail; send bounded correlated errors whenever a valid correlation context exists.
5. Normal mode accepts the seven protocol limits plus an explicit artifact budget. Validate wire Hello, recompute/verify the resolved actual ModelIdentity and reject forged/stale hashes or mismatched profile before RuntimeOwner::start. Adapt the inherited forged-Hello test to the required budget argument, so it still exercises identity rejection rather than parse/EOF. Never echo caller fingerprints as proof. Other model IDs can return explicit Unavailable in this intermediate S4A stage; S4C restores their production support. Valid Nomic Hello may still return Ready then product Unavailable, with the actual identity pinned.
6. Preserve S3 typed native-startup errors and checked close behavior. Update old arbitrary-identity scaffold test assumptions where strict verification changes them; retain real tiny-model lifecycle/control tests. Byte-limit1 invalid input is distinct from actual native-startup failure with valid64-character fingerprints. Do not introduce a production fake-identity bypass.
7. Add meaningful tests for wrong hash, config/profile tamper, duplicate/missing source evidence if exposed, exact/one-less budget, checked overflow, correlation, stdout framing, deterministic repeat and descriptor-without-native. Commit test before each production fix.

ACCEPTANCE: The two inherited metadata/forged-Hello tests and new metadata adversaries pass; real descriptor matches independently computed hashes/config and respects artifact budget; tiny native lifecycle controls remain passing; scoped fmt/check/all-target compile pass. Run full end-to-end product acceptance only to document its remaining S4B/C state; do not claim it passes now. Produce actual frame bytes/source hashes/raw exit observations for root POST. Finish production code, not another test-only return.


Implementation shape for checked file-byte accounting:

```rust
fn checked_artifact_bytes(lengths: impl IntoIterator<Item = u64>, limit: u64) -> Result<u64, &'static str> {
    let total = lengths.into_iter().try_fold(0u64, |n, bytes| {
        n.checked_add(bytes).ok_or("artifact size overflow")
    })?;
    if total > limit { Err("artifact budget exceeded") } else { Ok(total) }
}
```

Write/observe the new helper's failing test before implementing it; inherited CLI REDs already cover the externally visible missing behavior. The descriptor CLI writes the existing `ReplyEnvelope` and uses caller generation/request identity; normal Hello compares a recomputed `ModelIdentity` before `RuntimeOwner::start`.

---

### Task 2: embedding-nomic-inference

**Task ID:** `embedding-nomic-inference`

**Depends on:** `embedding-model-artifacts`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `Cargo.lock`
- `crates/spur-embedding-worker/Cargo.toml`
- `crates/spur-embedding-worker/src/main.rs`
- `crates/spur-embedding-worker/src/models.rs`
- `crates/spur-embedding-worker/src/runtime.rs`
- `crates/spur-embedding-worker/tests/embedding_models.rs`
- `crates/spur-embedding-worker/tests/native_lifecycle.rs`
- `crates/spur-embedding-worker/tests/support/model_reference.rs`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S4B/`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL S4B: Serve real Nomic embeddings through the existing EmbeddingClient using one finite worker-owned reusable model, preserving exact preformatted input and clean native shutdown. S4A's reviewed metadata contract is an input, not a placeholder.

FILES: worker src/models.rs (create), src/main.rs, src/runtime.rs as required; worker Cargo.toml/Cargo.lock; tests/embedding_models.rs, tests/native_lifecycle.rs and tests/support/model_reference.rs if needed; evidence S4B/. No host/client production migration.

PRE: Root will attach fresh source-bound S4B Jev PRE and receipts before approving S4A/releasing this node. Missing worker-local Jev is already bridged. Inspect the actual approved S4A API and use its artifact/profile identity.

IMPLEMENT:
- Reproduce the inherited product_nomic_matches_registered_reference_and_closes RED. Its independent FastEmbed reference already produced two finite768-dimensional rows and checked environment release; worker vector/cache/repeated-close assertions have not passed.
- Add normal macOS FastEmbed5.8.0 hf-hub-native-tls/ort-load-dynamic dependencies consistent with the existing managed ORT1.22.0 wrapper. Keep worker normal/build tree free of graph/analyst/DuckDB/PGQ, client free of native inference, and test probes behind required test-support features.
- Create a worker-owned ModelCache containing Option<fastembed::TextEmbedding>; load once for the pinned identity, reuse for subsequent batches, never an immortal static. Preserve current registered Nomic behavior, max_length8192, pooling/normalization and tokenizer configuration. Bind the exact model/tokenizer/config bytes actually consumed, not a path hashed before the library silently fetches/reopens different files. Verified bytes or immutable verified inputs must stay tied to the descriptor.
- Pass input strings unchanged into model.embed: callers already supply search_query:/search_document: formatting. Validate exact row count, dimensions768 and finite floats using the shared session/protocol before sending a response. No tiny fixture labeled as product output.
- On Shutdown stop admission, join owned work, drop cached sessions and retained outputs, then checked RuntimeOwner close; publish Closed only after success. Error/EOF paths must also drop cache before environment release. Ordinary main returns normally. No restart of managed environment inside one process after checked close.
- Record real load/reuse observations with narrowly scoped test-support instrumentation; same PID/generation alone does not prove the model wasn't reloaded. Traces must not contain user input content or credentials.
- Other product model IDs stay explicit Unavailable until S4C, with all enum variants and compatibility requirements preserved.

ACCEPTANCE: Real Nomic vectors through real client match the independent registered reference on the inherited already-formatted inputs, exact rows/dimensions/finiteness and declared reference tolerance; second batch uses one loaded model/same generation; two independent load/infer/close worker cycles exit0. Forged/stale identity, wrong dimensions/nonfinite and budget controls reject. Run affected client/native/model tests, all-target compile and scoped fmt/check. Preserve raw waits and actual model/runtime/source/build hashes; root POST must verify measured rows/cache/lifecycle rather than expected flags. Do not stop after only reproducing RED.


Cache lifetime shape:

```rust
struct ModelCache { model: Option<fastembed::TextEmbedding> }
impl ModelCache {
    fn clear(&mut self) { drop(self.model.take()); }
}
```

The loaded cache is local to the worker. Every close/error path clears it before the existing `RuntimeOwner::close`; no `Closed` reply precedes successful checked release. The existing real reference test provides the Nomic RED and expected vectors; record actual cache load/reuse rather than infer it from PID.

---

### Task 3: embedding-models-acceptance

**Task ID:** `embedding-models-acceptance`

**Depends on:** `embedding-nomic-inference`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `Cargo.lock`
- `crates/spur-embedding-worker/Cargo.toml`
- `crates/spur-embedding-worker/src/main.rs`
- `crates/spur-embedding-worker/src/model_artifacts.rs`
- `crates/spur-embedding-worker/src/models.rs`
- `crates/spur-embedding-worker/src/runtime.rs`
- `crates/spur-embedding-worker/tests/embedding_models.rs`
- `crates/spur-embedding-worker/tests/embedding_metadata.rs`
- `crates/spur-embedding-worker/tests/support/model_reference.rs`
- `crates/spur-embedding-worker/tests/fixtures/`
- `crates/spur-embedding-worker/README.md`
- `crates/spur-graph/README.md`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S4C/`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL S4C: Complete the original S4 three-model contract and end-to-end acceptance, extending the working Nomic pipeline to CodeRankEmbed and JinaEmbeddingsV2BaseCode with exact identities and repeated native exits.

FILES: worker src/model_artifacts.rs, src/models.rs, src/main.rs/runtime.rs only for actual exercised defects; worker Cargo.toml/Cargo.lock; tests/embedding_models.rs, tests/embedding_metadata.rs, tests/support/model_reference.rs and test fixtures; worker README or graph README documentation only; evidence S4C/. No host production migration.

PRE: Root will attach fresh S4C Jev PRE from approved S4A/B exports before approving S4B/releasing this node; root performs POST after GREEN.

IMPLEMENT:
- Preserve registered Jina FastEmbed5.8.0 behavior: model JinaEmbeddingsV2BaseCode, dimensions768, max_length8192, actual tokenizer/config, pooling/normalization and raw legacy inputs. Extend metadata resolution and verified-byte loading consistently.
- Port the existing custom CodeRank source behavior from graph/src/store/lance_sections.rs without importing graph: repo jamie8johnson/CodeRankEmbed-onnx, revision151669b173750250e611e1be00c812f112ea6020, onnx/model.onnx, tokenizer.json/config.json/special_tokens_map.json/tokenizer_config.json, hf-hub from_env/progress/retries3, Mean pooling, max_length8192, registered/custom output metadata and normalization preserved. Hash all actual artifacts/external weights/config used.
- Existing Nomic/CodeRank query prefixes and Nomic document formatting occur in callers; helper preserves already-formatted bytes. Jina and CodeRank documents remain raw. Identity profiles describe this behavior without prepending a second prefix.
- Acquire missing CodeRank/Jina assets into explicit isolated /private/tmp caches; previous HEAD controls returned200 but were not inference. Do not edit global cache or infer unavailability from a non-following symlink search. Missing actual files/network failures remain explicit acceptance gaps, not skips labeled pass.
- Run each independent existing-loader reference and worker cycle in separate processes as needed: managed environments cannot reinitialize after close. Compare actual client vectors for all3 models to their matching reference, record row/dimensions/finite values, normalization/prefix behavior and exact hashes. Repeated same-worker batches reuse the loaded model, repeated independent worker lifecycles exit0.
- Add identity/config tamper, artifact swap between descriptor/load, undersized budget, wrong-model/generation and active/retained-output close adversaries. New real regressions require test-first fixes and root PRE/POST. Preserve S3 real native fixtures and all earlier tests.

ACCEPTANCE: Complete worker/client/native product suite, all targets and formatting pass; real all3 supported product models match references and shut down cleanly. Tiny controls alone, metadata alone, reference-only runs or skipped models do not satisfy S4C. Export exact actual outputs/waits/cache/source/runtime/model identities and independent expected policy for root POST. Original S4 contract can be accepted only after this gate; broader wrapper/O1 and later Rust declaration gaps remain outside this native inference acceptance.


---

### Task 4: embedding-graph-routing

**Task ID:** `embedding-graph-routing`

**Depends on:** `embedding-models-acceptance`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `Cargo.lock`
- `crates/spur-graph/Cargo.toml`
- `crates/spur-graph/build.rs`
- `crates/spur-graph/build_support/ort_runtime.rs`
- `crates/spur-graph/src/store/lance_sections.rs`
- `crates/spur-graph/src/store/mod.rs`
- `crates/spur-graph/src/store/ort_runtime.rs`
- `crates/spur-graph/tests/`
- `crates/spur-embedding-worker/Cargo.toml`
- `crates/spur-embedding-worker/tests/`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S5/`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL: Graph section/code-symbol embedding production paths use spur-embedding client; spur-graph normal/build dependencies cease linking/loading native inference.
CONTRACT: Keep existing stored model selection, dimensions, vector identity, batching semantics and skip flags. Replace raw TextEmbedding caches/loader returns with client/model handles and migrate callers; there is no silent native in-process fallback. Remove fastembed/ort/hf-hub inference imports and macOS native bundle build hook from graph production; move their materialization ownership to the worker. The native tests currently in graph (ort_runtime_cache, ort_lifecycle, ort_shutdown, ort_owner_constructors, ort_custom_allocator, o1_node_attribute_view_review, o1_initializer_names and required ONNX fixtures) must be relocated to worker-owned tests or explicit standalone wrapper regression targets while retaining assertions, real behavior and historical evidence. Do not delete, ignore or weaken them to remove a dependency; keep broad wrapper test features out of normal CLI/helper trees. Adjust graph source references only for the relocation and neutral client migration.
TEST FIRST: graph-level embedding invocation, disabled flags never start helper, correct model/batch/vector artifact persistence, worker error/death rejects pending publication and no source identity drift; use actual client/protocol. Add dependency boundary regression. Run affected embedding/store/artifact/incremental tests, native relocated tests and graph all-target checks. Existing native-broad tests may expose a real regression: signal with evidence rather than restart the full 48-row research audit. S6 still removes analyst native deps, so do not claim the whole CLI is ORT-free yet.

PLANNED FILE SCOPE: crates/spur-graph/Cargo.toml, crates/spur-graph/build.rs, crates/spur-graph/build_support/ort_runtime.rs, crates/spur-graph/src/store/lance_sections.rs, crates/spur-graph/src/store/mod.rs, crates/spur-graph/src/store/ort_runtime.rs, crates/spur-graph/tests, crates/spur-embedding-worker/tests
EVIDENCE: docs/research/artifacts/2026-11-03-embedding-isolation/S5/.


PRE DELIVERY: Root prepares fresh source-bound Jev PRE before upstream approval/releasing this node, then actual-output POST before approval. Retain the original acceptance scope in the source issue as historical context; your dispatch issue is authoritative. Actual supported helper functions come from approved S4A/B/C, not the old Unavailable scaffold.

---

### Task 5: embedding-analyst-routing

**Task ID:** `embedding-analyst-routing`

**Depends on:** `embedding-graph-routing`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `Cargo.lock`
- `crates/spur-analyst/Cargo.toml`
- `crates/spur-analyst/src/embedding/mod.rs`
- `crates/spur-analyst/src/embedding/in_process.rs`
- `crates/spur-analyst/src/embedding/model_cache.rs`
- `crates/spur-analyst/src/embedding/runtime.rs`
- `crates/spur-analyst/src/embedding/mode.rs`
- `crates/spur-analyst/src/embedding/sidecar_service.rs`
- `crates/spur-analyst/tests/`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S6/`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL: Analyst query embeddings, warm-up/cache/jobs and the public socket service retain behavior through the separate helper, with no FastEmbed/ORT normal/build dependency in analyst or host.
CONTRACT: Remove native TextEmbedding caches/loaders and analyst fastembed dependency; reuse neutral model/client handle and bounded supervisor. Preserve SPUR_EMBED_SOCKET and external socket v1 request compatibility via translation to internal S1 protocol. spur embed serve must proxy to the separate helper, not initialize ORT. Preserve Auto/Sidecar/Off behavior and test override isolation. The old SPUR_ANALYST_EMBED_MODE=inprocess selection cannot restore native execution: keep a documented compatibility alias to the local supervised helper or an explicit deprecation error, never silent fallback. Warm-up jobs and cancellations are owned/joined and have bounded queue/deadline semantics; avoid orphaned detached loaders and clone/handle cycles. Optional retrieval degradation must be explicit; do not count missing embeddings as successful embedding computation.
TEST FIRST: existing analyst embedding/socket tests plus local-helper selection, external-v1 translation, no-embed mode no process, missing helper actionable error, worker exit/error/cancel during warm and query, shared client reuse and teardown without leaks. Use actual request/response traces for POST, including wrong model/generation negatives. Check normal/build dependency closure of spur-cli, spur-core, spur-graph and spur-analyst excludes ort/ort-sys/fastembed even with default product embed features. No release packaging changes except tests needed to demonstrate this contract; S7 owns deployment.

PLANNED FILE SCOPE: crates/spur-analyst/Cargo.toml, crates/spur-analyst/src/embedding/mod.rs, crates/spur-analyst/src/embedding/in_process.rs, crates/spur-analyst/src/embedding/model_cache.rs, crates/spur-analyst/src/embedding/runtime.rs, crates/spur-analyst/src/embedding/mode.rs, crates/spur-analyst/src/embedding/sidecar_service.rs, crates/spur-analyst/tests
EVIDENCE: docs/research/artifacts/2026-11-03-embedding-isolation/S6/.


PRE DELIVERY: Root prepares fresh source-bound Jev PRE before upstream approval/releasing this node, then actual-output POST before approval. Retain the original acceptance scope in the source issue as historical context; your dispatch issue is authoritative. Actual supported helper functions come from approved S4A/B/C, not the old Unavailable scaffold.

---

### Task 6: embedding-packaging

**Task ID:** `embedding-packaging`

**Depends on:** `embedding-analyst-routing`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `crates/spur-embedding/src/client.rs`
- `crates/spur-cli/src/commands/embed.rs`
- `crates/spur-cli/tests/embedding_worker_cli.rs`
- `scripts/spur-cargo`
- `scripts/cloud-build`
- `xtask/src/main.rs`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S7/`
- `docs/superpowers/plans/2026-11-03-supervised-embedding-helper.md`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL: Default product CLI builds/runs/tests/distribution can find the separately built helper and preserve embedding capability without ORT in the host artifact.
CONTRACT: Use existing build/release layout conventions, discover a platform/version/protocol-compatible sibling spur-embedding-worker executable with an explicit executable-path override for controlled deployments/tests; execute it directly without shell parsing or cwd-relative surprise. Missing/mismatched helper returns actionable typed error. Build/install both required artifacts through repository wrappers/release packaging, no recursive cargo in build.rs or runtime compilation. Inspect actual release/install entry points before editing; planned scripts/xtask paths are scope roots, not permission to rewrite unrelated release systems. Preserve CLI command syntax and embed flags; SPUR_EMBED_SOCKET can remain an external service choice. Set product capacity/deadline/restart defaults only from existing caller contracts and measured cold/warm startup/inference behavior, with fresh catalog/resource bounds and user-configurable validated policy. Do not invent performance optimality from a SAT model.
TEST FIRST: helper lookup with spaces, override and version mismatch, default CLI with actual helper, negative missing-helper, helper normal exit/crash, no on-demand model compiler, and package manifests/install layout. Run the unchanged analyst_temporal_views::analyst_build_emits_external_dependency_surface_views against the actual default embedding-enabled host artifact, expecting the original five rows including duplicate serde::Serialize. Inspect normal/build deps and Mach-O linked/exported symbols/runtime loaded libraries to confirm host has no ORT and helper has no DuckDB/PGQ. Handle dev-feature unification honestly by inspecting the actual host artifact, not only cargo tree text. Preserve default capability, not just --no-default-features success. Record actual packaging limitations per platform rather than claim untested cross-platform execution.

PLANNED FILE SCOPE: crates/spur-embedding/src/client.rs, crates/spur-cli/src/commands/embed.rs, crates/spur-cli/tests/embedding_worker_cli.rs, scripts/spur-cargo, scripts/cloud-build, xtask/src/main.rs, docs
EVIDENCE: docs/research/artifacts/2026-11-03-embedding-isolation/S7/.


PRE DELIVERY: Root prepares fresh source-bound Jev PRE before upstream approval/releasing this node, then actual-output POST before approval. Retain the original acceptance scope in the source issue as historical context; your dispatch issue is authoritative. Actual supported helper functions come from approved S4A/B/C, not the old Unavailable scaffold.

---

### Task 7: embedding-acceptance

**Task ID:** `embedding-acceptance`

**Depends on:** `embedding-packaging`

**Worker:** codex / profile codex / gpt-6.1-terra / xhigh

**Files:**

- `crates/spur-cli/tests/embedding_worker_cli.rs`
- `crates/spur-cli/tests/analyst_temporal_views.rs`
- `crates/spur-embedding/tests/supervisor.rs`
- `docs/research/artifacts/2026-11-03-embedding-isolation/acceptance/`
- `docs/research/artifacts/2026-11-03-embedding-isolation/S8/`
- `docs/superpowers/plans/2026-11-03-supervised-embedding-helper.md`

CONTEXT: Continue the user-approved supervised embedding helper spec docs/superpowers/specs/2026-11-03-supervised-embedding-helper.md (approved design bd-16wv, source epic bd-2tan/master bd-hdi). S1 protocol, S2 supervisor and S3 real native lifecycle are approved and in this base. Former S4 stopped twice with tests/reference only; do not treat RED or a reference-only inference as product completion. Base c6dbb22a1de4308e1070e8742786762d54ea7a39 preserves all tests and proof. Authored histories are protected at spur/review-evidence/embedding-s4-attempt1-authored and spur/review-evidence/embedding-s4-attempt2-authored.

BINDING: Use ONLY the canonical issue ID supplied by your current dispatch envelope for audits/signals and verify it is in_progress and assigned to this delegation. bd-2dhg and other old issue IDs are historical source context, never your audit target. Never infer a delegation ID from branch UUIDs. User explicitly prefers agent codex, profile codex, model gpt-6.1-terra, effort xhigh; the plan passes them unchanged.

PROCESS: Complete this focused production goal through test commit -> observed RED -> minimal implementation/fix commit -> real scoped GREEN. Do not return merely because a failing test exists. Root supplies and executes fresh Jev PRE inline before dispatch, then fresh Jev POST from actual exported outputs before approval. Worker-local Jev is unavailable and is NOT a blocker. Read/reload the exact supplied catalog receipts via get_solve_result, preserving all historical archives. A Jev gate is interpretation only. New generic-only theories need typed discovery/preflight; actual measured values and compiler checked arithmetic are not guessed product defaults. Report genuinely new policy interpretations to root while completing unaffected code. At GREEN deliver actual outputs and precise POST input candidates/independent policy/negatives; state GREEN/root Jev POST review pending. No self-approval, self-close or downstream dispatch.

ISOLATION: SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo only; fmt local; one cargo writer /private/tmp/rust-gap-g1-acceptance-target. CARGO_NET_OFFLINE=true may reuse Rust packages; model downloads are separately authorized into isolated /private/tmp caches. Pinned SPUR_ORT_ARCHIVE is /private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz (archive cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07; dylib2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a). Read cached model assets but do not edit global runtimes/cache, dirty main, old/live notebooks, ../spur-notebook source or graph/CURRENT. Never swallow native aborts, force successful exits, forget native owners or mock C API release. Native tests stay real; normal packages exclude test-support/owner-test-hooks.

EXPECTED OUTPUT: Unified diff, one-sentence rationale, actual test/fix/evidence commit IDs, exact RED/GREEN commands/results, source/build/runtime/model hashes, actual exported observations, independent expected policy and exact root POST candidates with adversaries. Durable completion audit on your assigned issue; leave in_progress for root review. Do not label partial/skipped gates as passed.

GOAL: Complete independent runtime, dependency, regression and notebook acceptance for the shipped narrow inference architecture.
REQUIRED: Preserve the original PGQ assertion/query; run the complete relevant graph suite (all targets/doc tests), analyst and CLI embedding/temporal regressions with actual default capability. Repeated real product-model embeddings and native clean exits must pass. Inject actual child abort before reply, mid-frame, during work and at shutdown; reject wrong generation/model/row/dimension/nonfinite/truncated responses; exercise timeouts, bounded queue/restart, cancellation and pending publication. Assert host graph/notebook query continues and no pending results are silently certified. Test old caches/model identities and public socket compatibility. Inspect exact host/helper artifacts, pin runtime/model/tokenizer/build hashes, confirm no test-support/owner-test-hooks in normal packages.
Measure cold/warm latency, batch throughput, peak RSS and shutdown time on the actual supported host; keep workload/methodology and distinguish per-process from combined memory. Compare to a preserved supported in-process control when runnable; if unavailable, report missing comparison, not fabricated improvements. Defaults must satisfy declared constraints; performance is measured, not proved by family SAT.
Use notebook MCP (navigate/read skill before any notebook operation) to create a NEW isolated acceptance notebook, run Rust cells that exercise the built client/helper and query continuity around injected child failure, and display measured outcomes. Do not edit prior notebooks, live ../spur-notebook project or its graph/CURRENT. Notebook tools are available in current session; if actual runtime cannot execute Rust, record that distinct blocking gate and continue the unaffected acceptance instead of silently substituting a different runtime. Native Rust experiments must run the actual separately built helper and parent, not catch_unwind pretending to intercept SIGABRT.
Each new discovered regression receives a fresh Jev PRE→test RED→fix GREEN→Jev POST cycle. Final POST checks independent expected policy against actual exported process/status/publication/dependency rows and source/build digests, with stale/forged/missing/duplicate negatives. Old broad O1 verification remains historical and unapproved; this acceptance covers only the actual exposed inference surface and real resources it uses. Provide compact durable evidence and leave review pending. Do not claim the G2 declaration gap or G3–G5 complete; root resumes those approved objectives after this runtime gate.

PLANNED FILE SCOPE: crates/spur-cli/tests/embedding_worker_cli.rs, crates/spur-cli/tests/analyst_temporal_views.rs, crates/spur-embedding/tests/supervisor.rs, docs/research/artifacts/2026-11-03-embedding-isolation/acceptance, docs/superpowers/plans/2026-11-03-supervised-embedding-helper.md
EVIDENCE: docs/research/artifacts/2026-11-03-embedding-isolation/S8/.


PRE DELIVERY: Root prepares fresh source-bound Jev PRE before upstream approval/releasing this node, then actual-output POST before approval. Retain the original acceptance scope in the source issue as historical context; your dispatch issue is authoritative. Actual supported helper functions come from approved S4A/B/C, not the old Unavailable scaffold.

---

### Review of the revised plan

The chain is acyclic and intentionally serial: the same worker files and one local cargo target are shared. Every original compatibility, no-native-host, lifecycle, external socket, packaging, measurement and fresh notebook acceptance requirement remains in S4A–S4C or S5–S8. G2 declaration and later G3–G5 work are still deferred until this runtime gate. Each task has scoped files, a real production/acceptance goal, test-first work and root PRE/POST gates. S4A does not claim product inference; S4B does not claim all three models; S4C requires all three. Known future end-to-end REDs remain preserved and explicit during earlier focused stages, then must pass before the complete contract is accepted.
