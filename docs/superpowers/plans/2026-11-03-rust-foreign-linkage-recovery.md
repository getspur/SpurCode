# Rust Foreign Declaration Linkage Recovery Implementation Plan

> **For SPUR orchestrator:** Execute with `submit_plan`; each task has a Beads issue, explicit dependencies and root review.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md`; approved `2026-11-02-rust-foreign-declarations.md`; G2a attempt-3 RTTI evidence.
**Formal @spec cells:** none.
**Design epic:** `bd-3cs` (closed). **Implementation:** `bd-hdi`, phase `bd-3nv`.
**Recovery source:** `6dfc1d06635e1d92bec944b450cbe2687a3caa52`, preserving the unapproved kind/schema implementation and all three investigations.
**Goal:** Repair the linked C++ exception boundary, then satisfy every original G2 acceptance gate and close the measured nine-function/three-block gaps.
**Architecture:** Separate the newly proven build/link boundary from protocol acceptance. Keep the original G2b/G2c contracts and execute all four tasks serially; no source-capture or durability requirement is dropped.
**Tech Stack:** Rust workspace, DuckDB/duckdb-rs 1.4.4, DuckPGQ 42eea11, fastembed 5.8.0, ort-sys 2.0.0-rc.10, locked tree-sitter grammar, SOLVE.

## Recovery decision and evidence limits

Original plan `0d8e2661-c185-44c3-82d2-d1794451709c` exhausted three G2a attempts. It is superseded through explicit cancellation, not approved or counted as complete implementation. G2a `bd-vg6`, capture `bd-1e8` and evidence `bd-2p7` become historical execution issues; source issues `bd-2re`, `bd-37y`, `bd-vnq` remain lineage for the replacement. New linkage source issue is `bd-8nl`.

Root reviewed scope signal `e23bd806-829e-46dc-979c-155b55835aae` and authorizes a narrowly causal repository build/link repair. The same ordinary PGQ query fails before G2a and after it. DuckPGQ deliberately throws Binder `use duckpgq_bind instead`; the host misses Planner::CreatePlan's std::exception handler and catches it at the C API boundary. A diagnostic throw-time host-RTTI substitution restores the unchanged query and five rows. The linked ort-sys archive defines private weak standard exception RTTI. Its origin and the final repair still require clean linkage evidence; the interposer is not an implementation or acceptance GREEN.

Correct the historical crash interpretation: the earlier C API probe crashed when reading an invalid result, not proven within duckdb_query. The separate native SQL PREPARE crash is a distinct control. Never use either as evidence that the original direct native query fails.

Native CLI and bundled engine report DuckDB v1.4.4/source_id6ddac802ff; both executables are arm64. DuckPGQ artifact SHA256 is `7b57ab52199ac5a023e6193552f3b2f09f3affb20a66597b214c33ddcadd77c5`; ort archive SHA256 is `e5a8cea427fae8a5abee88e148b75bee0415bdbb6148f3c6d29a8ddf094c1595`. Recalculate after rebuilding; recorded interposer addresses are historical and unsafe as a fix.

Preserve refs `spur/evidence/g2a-attempt1-authored`, `spur/evidence/g2a-attempt2-investigation`, `spur/evidence/g2a-attempt3-authored`. Fresh source exports and GREEN must come from the actual recovery branch. Base inheritance preserves prior implementation; it does not approve it.

## Routing and task-wide discipline

User authorization persists: `agent=codex`, `profile=codex`, `model=gpt-6.1-terra`, `effort=xhigh`, TDD and fresh persisted/reloaded PRE then POST SOLVE for each task. Root owns architecture and reviews sequentially. No new confirmation is required. The replacement plan explicitly carries model/effort because generic mutation-created children do not preserve these overrides.

Verify the actual assigned issue, current worker assignee and successful binding audit before edits. Source issues are lineage; branch components are brain/worker session identifiers, not delegation IDs. Leave assigned issues in_progress for root review; no self-approval, merge, close or successor dispatch.

Use graph-first orientation and exact current bytes for this isolated branch. For dependencies resolve exact lockfile/version/source, use external_* at that revision, and retain the local-to-external seam ledger. DuckPGQ source is `git:github.com/cwida/duckpgq-extension`, package `duckpgq`, revision `42eea11`; its index job `3c34fa55-8765-4a8f-866e-092ce1b807c9` is complete. ort-sys `registry:crates-io`, version `2.0.0-rc.10`, index job `d47ed1d0-c69c-433e-8bb1-df70613ad0d7` is complete. Do not silently query main/latest.

Every implementation task: catalog discovery and fresh PRE before planned edits; test-first commit with genuine RED; minimal fix and GREEN; POST from actual current outputs with adversarial controls; complete request/response/reload envelopes saved compactly and audited. Compile RED is not executed assertion RED. Receipt projections remain finite and do not prove runtime or Rust semantics.

Only `SPUR_REMOTE=0 scripts/spur-cargo` for workspace build/test/check; fmt is local. Use `TMPDIR=/private/tmp`, unset `INSTA_UPDATE`, `SPUR_GRAPH_BLESS` and diagnostic injection variables. One writer to `/private/tmp/rust-gap-g1-acceptance-target`. Prefer a separate temporary target for build-flag A/B. Cached build.rs can embed a removed worker's env!(CARGO_MANIFEST_DIR); refresh only the affected package cache, record setup failures, never treat them as assertion RED. Report progress during long work to retain leases. Required loopback access is authorized, not a reason to skip tests.

Preserve dirty main, user notebooks, live ../spur-notebook source/graph/CURRENT, global extension caches and historical evidence. No runtime RTTI rewriting or DYLD interposer in production or acceptance. No native CLI subprocess replacing the in-process query, relational fallback replacing PGQ, ignored tests, disabled default embeddings, relaxed schema checks, or speculative dependency upgrade. Additional scope needs a concrete signal before editing; continue unaffected work while root reviews it.

## R0 — Repair the exception linkage

**Task ID:** `rust-gap-g2-linkage`; **source:** `bd-8nl`; **depends on:** none.
**Suggested worker:** codex / gpt-6.1-terra / xhigh.

**Candidate production files:** `.cargo/config.toml`, root `Cargo.toml`/`Cargo.lock`, `crates/spur-graph/Cargo.toml`, `crates/spur-cli/Cargo.toml` and `crates/spur-cli/build.rs`. These are alternative causal seams, not a request to edit all of them. Start with the smallest macOS linkage/configuration change that the pinned source and link map justify. Scope excludes new query engines, schema changes, rewrites of exception objects, upstream publication and embedding process redesign. A repository-local patch of a pinned dependency requires a concrete additional scope report before adding vendored code.
**Tests:** retain `crates/spur-cli/tests/analyst_temporal_views.rs`'s existing graph assertion; add `crates/spur-cli/tests/analyst_duckpgq_exception_boundary.rs` only if a focused independent regression is necessary.
**Evidence:** `docs/research/artifacts/2026-11-02-rust-gap-implementation/G2-linkage/`.

1. Fresh catalog PRE describes required coexisting capabilities and forbidden workaround/configuration pairs. Reload it. Establish the unchanged original runtime RED with the exact ordinary command below before the fix; add and commit a focused failing regression if necessary. Preserve existing test-first history rather than manufacturing a failing stub.

```bash
env -u INSTA_UPDATE -u SPUR_GRAPH_BLESS -u DYLD_INSERT_LIBRARIES SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target TMPDIR=/private/tmp scripts/spur-cargo test -p spur-cli --test analyst_temporal_views analyst_build_emits_external_dependency_surface_views -- --exact --nocapture
```

2. Obtain a linker map or equivalent pinned-archive attribution for private standard exception RTTI in the host. Compare a clean controlled build/link variant. The absence of the original intentional Binder exception handler is the defect; do not change SQL or its expected results to conceal it. Use runtime failure/success plus source and symbol evidence to choose a minimal platform-scoped repair. Keep inference about ort origin explicit until the A/B proves it.

3. Implement only the proven repository configuration/build change. Keep existing pinned package semantics, default embedding capability and packaging behavior. A dynamic-library strategy, if proven necessary, must include reproducible runtime discovery/packaging; it cannot require an undocumented user-installed ORT library. Avoid broad linker mode changes unless narrower options demonstrably fail and impacts are verified.

4. Without diagnostic injection, run the original query on a freshly linked default test executable and the ordinary Rust API. The exact preserved assertion is:

```rust
let pgq_imports = query_csv(
    &db_path,
    "FROM GRAPH_TABLE (code
       MATCH (s:duckpgq_nodes)-[i:imports]->(e:External)
       WHERE s.file_path = 'crates/spur-core/src/lib.rs'
       COLUMNS (e.qualified_name AS external_symbol)
     ) ORDER BY external_symbol;",
);
assert_eq!(
    pgq_imports.trim(),
    "alloc::vec::Vec\ncore::fmt::Formatter\nserde::Serialize\nserde::Serialize\nstd::fmt::Debug"
);
```

5. GREEN requires both CLI targets (`analyst_build_cli` 7/7 and `analyst_temporal_views` 14/14), all targets check for spur-cli and spur-graph, scoped fmt/diff, and relevant default embedding/ORT initialization checks using existing local assets if available. Record exactly what ran and any unavailable runtime/model control; do not claim embeddings verified merely because SQL passes. A packaging/embedding behavioral change is material and must be resolved before acceptance. Retain a failing ordinary variant as the RED control; a diagnostic injected pass is never counted.

6. POST actual serialized rows, runtime/build configuration and source hashes with row-omission/duplicate-loss and forbidden-workaround controls; persist/reload complete receipts. Report finite interpretations separately from runtime facts. Return test/fix/evidence commits, unified diff, causal rationale, exact commands/counts/hashes, linker provenance and limits. Protect TDD commits in a reachable evidence ref. Leave issue in_progress.

## R1 — Accept the preserved G2a kind/schema protocol

**Task ID:** `rust-gap-g2-kind-acceptance`; **source:** `bd-2re`; **depends on:** `rust-gap-g2-linkage`.
**Files:** existing G2a six source/test paths and evidence `G2a/acceptance/`; no production edits are expected. A demonstrated regression may receive a separately recorded test-first minimal correction within the original G2a scope. Build/link repair is R0's approved input.

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


## Recovery review and phase completion

The DAG is R0 linkage → R1 protocol acceptance → G2b capture → G2c canonical evidence. All tasks use the explicit user-selected agent/profile/model/effort and independent root review. R0 may require a coupled Cargo/config update because foreign C++ exception matching crosses the final binary boundary; it is not a general dependency refactor. R1 revalidates the existing implementation; G2b and G2c retain their full original contracts.

Only after four approvals may root merge to a dedicated isolated branch, verify tested/integrated source equivalence, reconcile source issues and close G2. G3 durability, G4 configured compiler evidence and G5 bounded panic replay/notebook acceptance remain authorized and gated as before. No graph schema requirement, complete canonical-source control, fixture correspondence, CLI compatibility check, full regression requirement or uncertainty boundary was removed.

Self-review: approved scope and scope signal are covered by R0; all preserved G2a protocol properties are R1; nine foreign functions/three blocks are G2b; versioned ABI/body provenance and 446+9+3 frozen-source acceptance are G2c. Serial dependencies prevent overlapping build/config mutations and downstream API assumptions. Every task has explicit source lineage, file boundary, meaningful verification and complete persisted evidence.
