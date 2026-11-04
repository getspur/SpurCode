# ORT Lifecycle and Rust Foreign Coverage Recovery Implementation Plan

> **For SPUR orchestrator:** Execute through `submit_plan` with explicit Beads tasks, dependencies and per-task root review.

**Source spec:** approved Rust gap design `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md`; prior foreign-declaration/linkage plans and executed R0 evidence.
**Formal @spec cells:** none. **Design epic:** `bd-3cs` (closed). **Master/phase:** `bd-hdi` / `bd-3nv`.
**Source base:** `b1bbfd57e1a34bb6d7cbf7fda46f669b72e2f2a8`, an UNAPPROVED runtime experiment with preserved original G2a implementation and evidence.
**Goal:** Release native ORT resources safely, independently accept the complete bundle, then finish every original Rust foreign-declaration coverage gate.
**Architecture:** Isolate lifecycle ownership from bundle acceptance. A pinned wrapper-local repair and necessary application cache/owner seams must make normal shutdown safe; no raw private-handle release is allowed. Protocol acceptance and original capture/provenance contracts follow only after complete runtime acceptance.
**Tech stack:** Rust workspace, pinned ort/ort-sys2.0.0-rc.10 and ORT1.22.0, FastEmbed5.8.0, DuckDB1.4.4/DuckPGQ42eea11, SOLVE, locked Rust grammar.

## Review decision and preserved evidence

Plan46be4aba-57d7-4917-8538-c70a6dd34d9d is superseded after R0's three ordinary attempts. None of its tasks is approved. Cancellation means replacement, not successful acceptance. All code and evidence are inherited unchanged; the unsafe candidate is never merged to main or released.

R0 attempt3 passes temporal14/14 and all-target compilation, but CLI build1/7 and runtime cache/relocation2/3 fail. ORT-only and combined children return correct inference output then abort134. A native dlopen control retaining OrtEnv reproduces; explicitly releasing it exits0. This isolates a lifetime incompatibility but does not prove every native destructor involved.

Root reviewed scope signal918d5628-6f2d-4b0d-a40c-841e589e3b5f. Pinned ort has custom G_ENV OnceLock<Environment>; get_environment returns &'static Environment, and Environment::drop owns ReleaseEnv. Static native models also exist in graph lance_sections and analyst model_cache. Releasing a guessed native pointer behind those live references is forbidden.

Protected root ref `spur/review-evidence/g2-linkage-attempt3-authored` at42b0d6c2e preserves tests e39163a24/c164d96dd, experiment291188d5d and evidence. Retain all earlier G2a/linkage evidence refs. The dynamic control and tiny model are real; they are not a safe application or complete embedding proof.

The current relocation test compiles the private helper again with #[path]. Its cache tests provide useful pure-function checks, but final lifecycle/coexistence tests must use the actual production module/state or a production-facing application path. A duplicated test-only INIT cannot certify application ownership.

## Routing and task discipline

User authorization remains codex/profilecodex/gpt-6.1-terra/xhigh, TDD and fresh persisted/reloaded PRE then POST per task. Root owns the contracts and serial reviews. No additional confirmation is needed within this explicit recovery scope. New plan specs carry all model/effort overrides.

Verify the actual dispatch issue/assignee and binding audit. Source issues are lineage; brain/worker branch sessions are not delegation IDs. Leave assigned tasks in_progress for root review; do not self-approve, merge, close or dispatch successors.

Use graph-first retrieval and exact current branch bytes when indexes are stale. Use exact pinned external ort/ort-sys2.0.0-rc.10 and DuckPGQ42eea11 sources. Preserve upstream source identity and small actual patch deltas. Never silently substitute latest.

Before implementation edits: catalog discovery, fresh completed/reloaded PRE, committed meaningful failing test and genuine RED, minimal correction/GREEN, then actual-output POST with adversarial controls and complete persisted/reloaded envelopes. Existing real exit134 assertions are RED; missing APIs are compile RED and reported separately. Finite projections do not prove native memory/concurrency safety.

Use SPUR_REMOTE=0 scripts/spur-cargo; fmt local; TMPDIR=/private/tmp; one writer to shared target/private cache. Unset update/bless and diagnostic loader/ORT overrides for acceptance. Refresh only stale temporary package caches if build.rs embeds a removed worktree. Loopback/network escalation is already authorized for these tests and pinned artifacts. Report progress through long builds; do not return an interim checkpoint as completion while authorized work remains.

Preserve dirty main, user notebooks, live external source/graph/CURRENT, global registries/extensions and frozen evidence. No forced success exit/_exit, abort interception, leaked-handle workaround, ignored test, RTTI substitution, disabled default embedding, schema relaxation, or speculative runtime/version upgrade. No direct unsafe freeing/dropping of a OnceLock/global/borrowed Environment. Extra unrelated scope requires a concrete signal.

## N0 — Native environment and cached-model lifetime

**Task ID:** `rust-gap-g2-native-lifetime`; **source:** `bd-10bo`; **depends on:** none.
**Worker:** codex / gpt-6.1-terra / xhigh.

**Allowed production seam:** pinned local ort2.0.0-rc.10 patch under vendor/ort with root Cargo.toml/Cargo.lock; necessary wrapper environment/session/allocator/resource ownership internals; graph ort_runtime/lance_sections/model-cache ownership; analyst embedding/model_cache/in_process only where cache lifetime requires it. If explicit application quiescence/owner cleanup is required, minimal CLI/lib entry/shutdown wiring is authorized. Keep the effective non-macOS default path unchanged through an opt-in feature or equivalently bounded configuration. Import the exact crate with license/provenance and hash proof of untouched files; generated FFI and native C ABI/version are unchanged.

**Ownership contract:** a native environment remains valid while any native dependent can use it. Sessions, values/allocators and asynchronous/model owners retain whatever ownership the pinned API requires. Cached models/workers are quiesced and released before the environment; library lifetime exceeds every dependent. Native release occurs once, and no safe borrowed reference can access a freed native environment. New/repeated initialization, failed session construction, concurrent/last-owner drop and outstanding-owner shutdown must have defined behavior. Do not turn the current immortal &'static reference into a dangling reference. Prefer explicit owned/refcounted or checked quiescent ownership inside the dependency; an application must not infer private pointers or cast/drop arbitrary globals.

**Boundary:** This authorizes the pinned wrapper/application lifecycle correction requested by the scope signal. It does not authorize an unreviewed generic atexit free. Any exit integration must have a demonstrated quiescence/resource-order contract, including cached models; process shutdown success alone does not prove ownership. Preserve process-global initialization ordering and the verified bundled runtime selection.

**Tests and sequence:**

1. Complete fresh PRE for owner/resource-order and forbidden-workaround invariants. Model finite actual trace events separately from native proof.
2. Commit subprocess regressions for ORT-only, PGQ-only and combined normal execution using the actual production state. The current relocated assertion is an existing genuine runtime RED:

```rust
assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
```

```bash
env -u INSTA_UPDATE -u SPUR_GRAPH_BLESS -u DYLD_INSERT_LIBRARIES -u DYLD_LIBRARY_PATH -u ORT_DYLIB_PATH -u ORT_LIB_LOCATION -u ORT_PREFER_DYNAMIC_LINK SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph --test ort_runtime_cache -- --nocapture
```

3. Add meaningful multiple-owner/failed-init/session-drop/busy-shutdown tests before changing those contracts. Trace native release ordering/counts through real ownership or focused instrumented dependency tests, not guessed logs. Include cached model owner cleanup if the correction relies on it. Retain exact runtime exit134 before fixing.
4. Implement the smallest safe lifecycle correction and necessary integration. Check every changed native ownership edge against pinned source. Do not merely release the environment early to get exit0 or suppress the mutex error. Preserve model selection, inference values and the original graph query.
5. GREEN requires actual Rust ORT-only, PGQ-only, combined and relocated processes to exit0 without test-only cleanup/injection; active-owner and failed-construction cases; original temporal graph assertion; relevant wrapper/graph/analyst checks, scoped fmt/diff. Full bundle/application acceptance is independently re-run in N1. Repeated initialization after a successful initialization must not create duplicate owners or free a live environment.
6. POST actual resource/config/source traces with invalid release-order, outstanding-owner and duplicate-release controls; persisted/reloaded complete receipts. Explain finite limits. Commit test/fix/evidence and protect authored history. Evidence `G2-lifecycle/`. Leave task in_progress; do not claim bundle or G2 phase acceptance.

## N1 — Complete bundle and lifecycle acceptance

**Task ID:** `rust-gap-g2-runtime-acceptance`; **source:** `bd-8nl`; **depends on:** `rust-gap-g2-native-lifetime`.
**Files:** existing graph build/build_support/runtime/helper/store integration and their tests, graph/analyst Cargo files, necessary pinned patch integration and focused docs/evidence. No additional native architecture is expected; signal a newly proven defect before extending scope.

Review the whole inherited bundle and N0 patch. Check checksum-pinned archive/payload manifests for arm64/x86_64, no committed native binaries or absolute worker paths, deterministic error/corruption behavior, useful offline/cached build behavior, default embedding retained and effective non-macOS behavior preserved. Native payload is embedded and extracted into a content-addressed app cache. Only actual embedding use initializes ORT. No startup dylib/diagnostic loader dependency is acceptable.

Use the actual production module/state for lifecycle tests; remove shadow-global helper inclusion as evidence of production coexistence. Test cache reuse/corruption/write failure and moved/installed binary startup plus ORT inference and PGQ in one process. Test initialization failure and normal process return. Keep all five original graph rows including duplicate serde::Serialize. Verify normal application cached-model cleanup, not just a standalone manually released C environment.

Fresh PRE then current runtime/source checks; inherited test-before-fix history is retained, and any new correction needs genuine TDD. Run analyst_build_cli7/7 and analyst_temporal_views14/14, runtime/cache/relocation/coexistence checks with exit0, relevant embedding/cache tests, spur-cli+spur-graph all-target compilation, scoped fmt/diff. Inspect symbols/dependencies: no private static ORT standard RTTI and no startup ONNX Runtime load command on macOS. Artifact metadata and tested host are explicit; x86 runtime and full model claims require actual execution.

Final commands must work with update/bless and all experimental ORT/DYLD/RUSTFLAGS/archive overrides unset. Verify existing release baseline14.0 against official dylib minos13.3; do not silently raise deployment requirements. Save exact selected dependency/import hashes and a baseline/delta proof for a vendored patch. POST actual rows, configurations, cache/relocation/resource output with omission/duplicate-loss/forbidden-workaround rejection, persist/reload. Evidence `G2-runtime-acceptance/`. No N1 approval while any required gate is red.

## Preserved protocol and declaration work

R1 protocol acceptance now depends on `rust-gap-g2-runtime-acceptance`. Original capture depends on protocol; canonical evidence depends on capture. The following original requirements are retained without reduced assertions or missing coverage lanes.

## R1 — Accept the preserved G2a kind/schema protocol

**Task ID:** `rust-gap-g2-kind-acceptance`; **source:** `bd-2re`; **depends on:** `rust-gap-g2-runtime-acceptance`.
**Files:** existing G2a six source/test paths and evidence `G2a/acceptance/`; no production edits are expected. A demonstrated regression may receive a separately recorded test-first minimal correction within the original G2a scope. Complete bundle/lifecycle acceptance is N1's approved input.

Fresh PRE, then inspect current code and original test/fix history. Revalidate ExternBlock discriminator/serde/Parquet representation, distinction from Module/External, explicit non-callability/non-construction/non-import-fallback, persisted symbol retention, writer schema v12 and the CLI's shared writer-constant alias with strict incompatible-manifest rejection. Synthetic artifacts prove representation only. No foreign source capture or declaration-v2 claim in this task.

Run rust_extern_kind, private role test, node-kind/store/schema/canonical tests and affected extractor/Parquet checks, both CLI targets, graph all-target check, scoped formatting/diff. Preserve all original expectation strengths. Fresh POST must use actual current serialized kind/roles/schema and reject callable-role and incompatible-version controls; save/reload receipts. Verification-only work may reuse the protected historical genuine test-before-fix RED; do not introduce artificial code changes to create a new RED. Record exact tested revision and evidence, then await root approval.

## Original capture and provenance tasks

The following sections reproduce the original G2b/G2c contracts unchanged. Replacement task IDs are `rust-gap-g2-capture` and `rust-gap-g2-evidence`. Capture depends on `rust-gap-g2-kind-acceptance`; evidence depends on capture. Source issues remain `bd-37y` and `bd-vnq`. References below to G2a now mean accepted R1; all original zero-failure phase gates remain mandatory.

## G2b — Physical nodes and transparent namespaces

**Task slug:** `rust-gap-g2-capture`; **source issue:** `bd-37y`; **depends on:** `rust-gap-g2-kind-acceptance`.

**Files:** modify `queries/rust/spur-edges.scm`, `src/extract/languages.rs`, `src/store/build.rs`; create `tests/rust_foreign_declarations.rs`. Minimal transparent-ancestor adjustments in `src/extract/tree_sitter.rs` require a scope signal/explanation. Allowed metadata assets are the six `tests/fixtures/{sample,python,typescript,javascript,cpp,markdown}_corpus/expected_graph_index.json` files and `src/store/snapshots/spur_graph__store__canonical_hash__tests__canonical_bytes_layout.snap`.

Capture `foreign_mod_item` as `ExternBlock` and only direct `function_signature_item` children of its declaration list as `Function`. `function_item` with an extern modifier and local body remains a regular definition; trait required/provided methods and `extern crate` are not foreign block/function records.

Both graph and byte extraction must construct the full owner set before IDs. Keep block structural `Contains`/`Defines`, but use the surrounding lexical prefix for its binding's descendants. Also skip extern ancestors in artifact `qualified_name`/`enclosing_scope` derivation: changing only builder keys would leave persisted names inconsistent. Preserve the block's own `<extern>` name and bodyless function stable-ID inputs.

New block and foreign-function graph ranges include their owned outer attributes/doc comments and visibility, ending at the declaration end. Keep a separate core syntax range in G2c. When scanning preceding siblings, skip ordinary comments as whitespace, include only owned outer metadata, and **stop at the preceding item**; do not steal its attributes. Tuple range rules and unrelated existing declaration IDs remain unchanged.

**Test seed:**

```rust
#[test]
fn foreign_block_is_structural_but_not_a_lexical_namespace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"),
        "mod m { unsafe extern \"C\" { pub fn imported(x:u8)->u8; } pub extern \"C\" fn defined(x:u8)->u8 { x } }").unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let imported = graph.symbols.iter().find(|s| s.qualified_name == "m::imported").unwrap();
    assert_eq!(imported.symbol_kind, "function");
    assert_eq!(imported.enclosing_scope.as_deref(), Some("m"));
    assert_eq!(graph.symbols.iter().filter(|s| s.symbol_kind == "extern_block").count(), 1);
    assert!(graph.symbols.iter().any(|s| s.qualified_name == "m::defined"));
}
```

Extend it to exact block/function spans, lexical ID inputs, unique direct structural parents, both extractor paths and Parquet round-trip. Test repeated blocks/ABIs, file/module/function nesting, duplicate cfg-selected names, outer docs/attributes around neighboring items, Unicode, visibility and raw identifiers under the existing naming convention. Queries must find `m::imported`; unsupported call resolution stays unresolved rather than binding to an unrelated homonym. Do not claim compiler-grade call-target correctness.

Advance `EXTRACTOR_VERSION` to `2026-11-03-rust-foreign-declarations-v1` so unchanged-file reuse cannot preserve omissions. Schema stays at G2a's v12. Prove only required metadata paths changed in unrelated language goldens/canonical snapshot; no broad blessing.

**Sequence:** fresh source-grounded PRE → test commit/real missing-node or wrong-namespace RED → minimal capture/range/namespace fix → graph/byte/store/lexical/receiver/macro/ownership/tuple/extractor/canonical GREEN, all-target checks, fmt/diff → current-code exports from isolated frozen inputs join **nine functions and three blocks** exactly by source OID/kind/full span and lexical/structural identity → POST exact owner/name/source contracts plus a different existing owner negative. Preserve experimental IDs as historical evidence, not replacement production hashes.

## G2c — Canonical ABI and body provenance, phase acceptance

**Task slug:** `rust-gap-g2-evidence`; **source issue:** `bd-vnq`; **depends on:** `rust-gap-g2-capture`.

**Files:** modify `src/rust_declarations.rs`; create `src/rust_declarations/ffi.rs` and `tests/rust_foreign_evidence.rs`; update `tests/rust_declaration_evidence.rs` for the explicit new covered-kind/version contract. Update `crates/spur-graph/README.md` and task evidence. No artifact/facts carrier, Parquet table or global registry changes.

**Public contract:** Retain `collect_rust_declarations`, `RustDeclarationSource`, tuple row fields and canonical `validate`. Advance `RUST_DECLARATION_SCHEMA_VERSION` to `spur-rust-declarations-v2`. Extend `RustDeclarationKind` with `ForeignFunction` and `ExternBlock`; `covered_kinds` explicitly lists all three. Keep `rows: Vec<RustTupleFieldEvidence>` as the documented tuple-only vector; add `foreign_functions: Vec<RustForeignFunctionEvidence>` and `extern_blocks: Vec<RustExternBlockEvidence>`. The per-source complete count is the sum of all three covered vectors. Version-1 archived evidence remains historical, not silently accepted as a current complete v2 set.

New serializable records carry physical path/stable-file ID/source OID, actual graph stable-symbol ID, full byte/line range and separate core syntax range. Each has typed structural and lexical owners. Foreign functions structurally reference their exact block; their lexical reference skips that block. Blocks reference their actual surrounding owner and stay non-callable.

Use a public source-syntax ABI type with these variants:

```rust
pub enum RustDeclarationAbiSyntax {
    Omitted,
    Explicit { literal: RustDeclarationText },
    Inherited { extern_block_id: String },
}
```

Blocks distinguish omitted ABI spelling from an exact explicit literal; foreign declarations refer to the inherited block instead of copying an assumed effective ABI. Preserve raw qualifier token ranges, visibility and every owned attribute/doc comment in source order. Raw `link`, `link_name`, `link_ordinal` and conditional `cfg_attr(...link...)` spellings remain in that attribute inventory; do not interpret them as resolved linkage or selected cfg. Unsupported syntax must stay explicitly uncertain rather than being normalized into a compiler claim.

Record body category explicitly: `AbsentForeignDeclaration` for imported functions and `NonExecutableContainer` for blocks. Do not assign local body IDs, fabricate an empty safe body, infer unwind behavior, or classify ordinary bodyful extern definitions/trait signatures as foreign declarations. A syntactic container's braces are not an executable function body.

Put FFI types/helpers in the new submodule and publicly re-export through the existing declaration module/root exports. Parse each source with the same locked grammar and canonical policy. Verify exact source/core/full/attribute/qualifier/ABI ranges and authoritative graph ownership, then compare complete canonical sets. Preserve valid zero-record sources and healthy independent rows under recovered-source `Unknown`; do not certify recovery as complete. Require current source/header/graph/extractor bindings. Prefer strict decoding for the new version so unknown fields do not acquire unstated meaning.

**Test seed:**

```rust
#[test]
fn canonical_foreign_evidence_has_distinct_body_and_abi_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let text = "mod m { #[link(name=\"fixture\")] unsafe extern \"C\" { #[link_name=\"real_name\"] pub fn imported(); } extern \"C\" fn defined() {} }";
    std::fs::write(dir.path().join("lib.rs"), text).unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let sources = [spur_graph::RustDeclarationSource { file_path: "lib.rs", bytes: text.as_bytes() }];
    let set = spur_graph::collect_rust_declarations(&graph, &sources).unwrap();
    set.validate(&graph, &sources).unwrap();
    assert_eq!(set.foreign_functions.len(), 1);
    assert_eq!(set.extern_blocks.len(), 1);
    assert!(set.rows.is_empty());
}
```

Before implementing, add tests for changed ABI text/reference, copied existing block/lexical owner, qualified name/ID/range, qualifier/body tag, raw link spelling, omitted/duplicate records and omitted zero-source binding. Include a second existing block with the same ABI so FK and ABI equality alone cannot pass the wrong-owner control. Test omitted block ABI, safe/unsafe spellings supported by the locked grammar, bodyful extern functions, trait required/provided signatures and nested physical scopes. New API compile RED is recorded honestly; runtime regressions get assertion REDs before their fixes.

**Acceptance:** Rebuild isolated frozen inputs with current code and validate all **446 tuple + 9 foreign-function + 3 block** records by exact independent source provenance. Keep the previously recovered file unknown. POST must use actual serialized records, not a manually substituted collector; verify canonical negative controls plus exact finite owner/ABI/body-role projections, persisted/reloaded with cross-batch uniqueness checked independently.

Run targeted declaration/extraction/store/notebook/macro tests, graph all-target check and current CLI analyst schema/build/temporal tests. Then run the complete authorized local graph suite with loopback access:

```bash
env -u INSTA_UPDATE -u SPUR_GRAPH_BLESS SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target RUST_TEST_THREADS=1 TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph --no-fail-fast
```

Report every target count/ignore/doctest and require zero failures. A newly exposed unrelated guard is signaled and resolved within reviewed scope, never skipped. Complete fmt/diff checks and document the public evidence/version boundary in README.



## Integration and self-review

The DAG is N0 native lifetime → N1 full runtime acceptance → R1 protocol acceptance → G2b capture → G2c canonical evidence/full phase acceptance. Every task explicitly uses the user-selected worker/model/effort and root review. All existing code and tests are retained in the isolated experimental base; none is approved by inheritance.

The lifecycle defect is owned by N0; bundle/application behavior and all inherited runtime changes are independently accepted by N1; all original G2a kind/schema properties remain R1; measured foreign9/block3 capture is G2b; canonical446+9+3 evidence and zero-failure full graph regression is G2c. No scope, uncertainty lane, schema guard or CLI gate has been dropped. Only after five approvals may root integrate into a dedicated branch, compare integrated/tested source and close G2. G3–G5 remain authorized and gated. Main/live project/notebooks remain preserved.
