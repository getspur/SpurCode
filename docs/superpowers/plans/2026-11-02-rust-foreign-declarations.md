# Rust Foreign Declaration Coverage Implementation Plan

> **For SPUR orchestrator:** Execute through `submit_plan` with per-task review gates.
> Source Beads issues provide lineage; the Plan Engine's assigned issues authorize workers.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md` and its executed notebook.
**Formal @spec cells:** none.
**Design epic:** `bd-3cs` (closed).
**Implementation:** `bd-hdi`; G2 phase `bd-3nv`, after completed G1 `bd-1go`.
**Accepted source base:** `c4f32bb11106758b764620697d344be4fd44c323`.

**Goal:** Represent the nine measured foreign functions and three extern blocks with correct names, ownership and canonical source provenance.

**Architecture:** Add `NodeKind::ExternBlock` as an explicit non-callable container and coordinate graph schema v12 with existing readers. Capture bodyless foreign declarations as `Function`; structural containment retains the block while Rust lexical names skip it. Extend the separate declaration evidence contract to v2 with typed foreign and block vectors, keeping current tuple rows intact. No new persistent evidence tables or artifact carrier fields are introduced until G3.

**Tech Stack:** Rust 2021, locked tree-sitter 0.26.13/tree-sitter-rust 0.24.2, existing graph identity/store/query code, serde, SOLVE catalog, frozen independent syn inventory.

## Authorization, routing and verification

The user approved G1–G5 with TDD and PRE then POST solve per task. Use `agent=codex`, `profile=codex`, `model=gpt-6.1-terra`, `effort=xhigh`. Brain owns the contracts; the strict G2a → G2b → G2c chain mitigates the worker's advisory preference against architectural coordination. No further permission question is needed within this scope.

G1 acceptance is complete: 1,306 passed, zero failed, 21 ignored over 101 result lines, including two doctests. Nineteen ignores predate this work; two are opt-in frozen export harnesses. `acceptance/g1d-final-results.json` and its compressed log retain the exact command. All 446 tuple rows matched exact independent source inventory; one recovered source remains explicitly unknown.

Every task must:

1. Verify its actual assigned issue is `in_progress` and write a binding audit before edits. If no assigned ID is in the envelope, use `list_issues` with the task's unique `spur:plan-task-id:<slug>` label and current status, then verify assignee/delegation. Source IDs below are lineage. Branch layout is `spur/worker/v2/codex/<brain-session>/<worker-session>`; neither component is a delegation ID.
2. Discover the SOLVE catalog, execute fresh PRE before planned test/implementation edits, and persist/reload receipts through `get_solve_result`. Record finite scope and SAT/pass versus UNSAT/rejection meanings. Use schema-checked generic constraints only when no catalog family fits; preserve validation errors and disjoint projection limits honestly.
3. Commit a real failing test, capture the actual RED, implement minimally, run GREEN and actual-output POST plus adversarial rejection. Distinguish compile RED from executed assertion RED. An existing diagnostic RED can be retained without inventing a new failing stub. Keep test-before-fix history reachable when the engine normalizes result commits.
4. Save exact requests/responses/reloads, source hashes, current-code exports, commands/exits and compact logs under `docs/research/artifacts/2026-11-02-rust-gap-implementation/G2a/`, `G2b/`, or `G2c/`. Do not copy historical payloads recursively or rely only on live receipt IDs. Never read private solver storage.
5. Use `SPUR_REMOTE=0 scripts/spur-cargo` for already-authorized local SpurCode builds; never bare Cargo. Use `TMPDIR=/private/tmp`; reuse the idle `/private/tmp/rust-gap-g1-acceptance-target` via `CARGO_TARGET_DIR` if useful. `fmt` is local. Long builds should yield and report progress; no competing writer to the shared target.
6. Run tests with `INSTA_UPDATE` and `SPUR_GRAPH_BLESS` unset. Loopback mock tests require authorized elevated execution, not skips or production changes. Preserve failures; use exact appropriate reruns. Final G2c acceptance requires a complete zero-failure run.
7. Preserve dirty active main, frozen research, notebooks, global registries and live `../spur-notebook` sources/graph/CURRENT. Build frozen copies in temporary roots outside an enclosing Git repository. Do not use stale binaries as POST evidence or rebuild the external project.
8. Signal additional production scope, unexpected semantic changes or genuine blockers before extending work. Schema/extractor changes may update only proven metadata paths in six corpus goldens and the canonical layout snapshot. Any other expectation change needs source-grounded explanation; never broadly bless tests or erase checks.
9. Return unified diff plus one-sentence rationale, commits, RED/GREEN and receipt evidence, source bindings and limits. Leave the assigned task in progress for root review. Do not merge, approve, close or dispatch successors.

## Source map and fixed contracts

The accepted G1 source provides `rust_declarations.rs` and canonical validation. Its existing `rows` are tuple fields, not a generic declaration union. Keep that public element type in G2; add typed vectors as specified below and explicitly advance the separate evidence version.

The measured sources are pinned in `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/fixtures/structural-inputs.json.gz`; `captures/structural-candidates.json` and `captures/structural-syn-rows.json` independently describe 446 tuple fields, nine foreign functions and three extern blocks. Experimental `candidate_id` values are not production identity hashes.

Relevant seams:

- `schema.rs`: `NodeKind`, discriminator/serde conversion.
- `extract/languages.rs`: definition candidates, ranges, naming and both definition binding types.
- `extract/tree_sitter.rs`: Rust query candidate assembly and callable/construct target predicates.
- `store/build.rs`: schema/extractor/manifest compatibility, symbol persistence, `qualified_name` and `enclosing_scope` derived from structural edges.
- `queries/rust/tags.scm`: signatures are captured only inside traits; ordinary `function_item` captures do not cover foreign signatures.
- `queries/rust/spur-edges.scm`: add direct foreign block/signature patterns here, preserving existing captures.
- `spur-cli/src/commands/analyst.rs`: `SUPPORTED_GRAPH_SCHEMA_VERSION` already aliases `spur_graph::store::SCHEMA_VERSION`; preserve the alias and strict checks.

Code orientation now returns main-worktree graph evidence, but it does not index this isolated G1 integration. Use exact accepted bytes for changed/private code; do not silently substitute indexed older source. G1 also demonstrated that global extractor metadata reaches both corpus goldens and the canonical-byte guard.

Preserve path/FQN/kind/start stable-ID semantics. New block labels are `<extern>`; their own qualified names include this non-addressable segment and IDs distinguish source positions. Descendant Rust names do not include it: `m::imported`, never `m::<extern>::imported`. For example, a nested foreign function structurally belongs to a block but lexically belongs to `m`. At file scope its lexical owner is the physical file. More deeply nested lexical owners use the nearest represented non-extern scope; this does not claim complete Rust name resolution.

## G2a — Non-callable kind and coordinated schema

**Task slug:** `rust-gap-g2-kind`; **source issue:** `bd-2re`; **depends on:** none (accepted G1 is the explicit base).

**Files:** modify `src/schema.rs`, `src/store/build.rs`, `src/extract/languages.rs`, `src/extract/tree_sitter.rs` within `crates/spur-graph`; create `tests/rust_extern_kind.rs`. Only necessary schema test/metadata fixture updates are allowed with path proof. CLI source should retain its existing writer-constant alias; signal if an actual dependent source change is required.

**Acceptance:**

- `NodeKind::ExternBlock` round-trips as `extern_block` through discriminator, serde and current artifact/store paths. It differs from `External` and `Module`.
- It cannot be a callable, constructor or import-rebind fallback target. Source ranges and ordinary stable IDs are preserved by serialization.
- Set the writer graph schema to `spur-graph-schema-v12`; current readers and CLI consume the shared constant and reject incompatible manifests as their existing contracts require. Do not add a copied reader literal or weaken the rejection that previously caught v11/v10 drift. Preserve deliberately supported legacy paths only where explicitly versioned; never certify missing evidence as empty coverage.
- No foreign syntax capture or declaration evidence shape change in this task. Existing extraction output keeps its source semantics.

**Test seed:**

```rust
#[test]
fn extern_container_kind_has_a_distinct_lossless_discriminator() {
    let kind = spur_graph::NodeKind::from_discriminator("extern_block")
        .expect("non-callable extern container kind");
    assert_eq!(kind.discriminator(), "extern_block");
    assert_ne!(kind, spur_graph::NodeKind::External);
    assert_ne!(kind, spur_graph::NodeKind::Module);
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"extern_block\"");
    assert_eq!(serde_json::from_str::<spur_graph::NodeKind>(&json).unwrap(), kind);
}
```

Add private predicate assertions in the existing tree-sitter test module rather than exposing private functions just for testing. A synthetic current artifact round-trip tests the new kind representation; it is not evidence that source capture exists yet.

**Sequence:** catalog PRE for kind/body-role/schema compatibility → test commit → `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_extern_kind` assertion RED for missing discriminator → minimal kind/mapping/schema changes → same target GREEN, schema/store/canonical compatibility tests, graph all-target check, CLI analyst build/temporal tests using current reader constant, scoped fmt/diff → POST from current serialized round-trip plus forbidden callable/body-role pair. Run the existing requested graph-build fixture through CLI tests, not a live target rebuild.

**Boundary:** This is one coupled enum/protocol change across four source seams; splitting them would leave an uncompilable kind or writer/reader mismatch. No general resolver, source collector or persistence redesign.

## G2b — Physical nodes and transparent namespaces

**Task slug:** `rust-gap-g2-capture`; **source issue:** `bd-37y`; **depends on:** `rust-gap-g2-kind`.

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

## Review and phase completion

Brain checks Beads status, completion audits/signals, source diff, real RED/GREEN logs, source-bound joins and reloaded receipts before approval. Integration must preserve approved ancestors. On three approvals, merge into a dedicated isolated branch, compare integrated source to tested output and close G2 only with the complete acceptance evidence. G3 `bd-3l2` then becomes eligible for its concrete durability plan under existing authorization. Main checkout/live target remain preserved.

**Self-review:** G2a handles protocol/kind support; G2b closes the measured nine/three node gaps without a synthetic Rust namespace; G2c handles ABI/body/link syntax and canonical validation. All prior G1 tuple semantics remain covered. The chain is acyclic and deliberately serial because each task consumes new predecessor types. Persistent tables/carriers, cfg/compiler interpretation and panic solving remain assigned to G3–G5; no requirement is silently dropped or claimed implemented here.
