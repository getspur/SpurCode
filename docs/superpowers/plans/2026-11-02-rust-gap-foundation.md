# Rust Declaration Foundation Implementation Plan

> **For SPUR orchestrator:** Execute this plan through `submit_plan` with persisted Beads tasks.
> Each task becomes a Beads issue with plan and task labels; approval gates the next task.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md` and the executed `2026-11-02-rust-gap-closure-grounding.ipynb`.
**Formal @spec cells:** none; the approved research report is the design contract.
**Design epic:** `bd-3cs` (closed).
**Implementation epic:** `bd-hdi`; this phase is `bd-1go`.

**Goal:** Repair Rust declaration ownership, represent physical tuple fields individually, and expose canonical source-bound tuple evidence.

**Architecture:** Assemble physical declaration candidates before choosing owners and generating qualified names or IDs. Reuse the existing `Field` node and identity algorithm. Add a separately versioned declaration evidence API without changing `GraphFacts`, `GraphIndexArtifact`, or graph Parquet schema v11 in this phase.

**Tech Stack:** Rust 2021, current tree-sitter Rust grammar, existing graph builders, serde, existing SOLVE catalog and Z3 service, frozen syn inventory.

## Authorized scope and subsequent phases

The user approved implementation with TDD and PRE then POST solve per task, using `agent=codex`, `profile=codex`, `model=gpt-6.1-terra`, `effort=xhigh`. This plan starts execution without another approval request. Brain owns architecture; the chosen worker receives bounded tasks and sequential review despite the worker's advisory preference against architectural coordination.

The complete approved program remains tracked under `bd-hdi`:

| Phase | Issue | Deliverable |
| --- | --- | --- |
| G1 | bd-1go | Ownership, tuple nodes, canonical declaration evidence (this plan). |
| G2 | bd-3nv | Foreign functions and non-callable extern structure with transparent lexical naming and coordinated schema consumers. |
| G3 | bd-3l2 | Durable source/declaration/body/callsite evidence through full/delta/overlay/query/hash/cache paths. |
| G4 | bd-1o0 | Effective compiler invocation and configured observations with explicit unresolved/generated lineage. |
| G5 | bd-3lq | Validated bounded analysis, witness replay and final notebook coverage acceptance. |

Each later phase is planned against the approved interfaces of its predecessor and proceeds under the existing authorization. Physical source coverage never implies compiled eligibility, call-target correctness, or complete panic freedom.

## Shared execution and evidence contract

1. Use the **actual assigned Beads ID** supplied by the Plan Engine, verify it is `in_progress`, and use it for every audit/signal. Source IDs below are lineage, not delegation authorization. Report a binding failure to brain; do not substitute the task slug.
2. Discover the SOLVE catalog and run a fresh PRE before writing tests or production changes. Persist and reload receipts through `get_solve_result`; never read solver storage files. Catalog verify SAT means pass, UNSAT means rejection. Decompose exact finite projections if limits require it; do not claim sampled constraints cover the entire corpus.
3. Commit a failing regression with `test(spur-graph): G1x ...`, then record its actual RED command/output. Distinguish new-API compile RED from runtime assertion RED. Do not manufacture assertion failures with a deliberately broken production stub.
4. Implement the smallest source change, commit separately as `fix` or `feat`, run GREEN and affected checks, then POST against **actual exports from the changed implementation**, with a valid case and source-bound adversarial controls. Persist/reload every reported receipt.
5. Put task evidence under `docs/research/artifacts/2026-11-02-rust-gap-implementation/G1a/`, `G1b/`, or `G1c/`. Retain exact source bytes/hash, commands, exit codes, expected interpretations, exported rows, solver requests/responses/reloads and a concise README. Avoid repetitive copies of historical artifacts or embedding full diffs recursively.
6. All Cargo calls use `SPUR_REMOTE=0 scripts/spur-cargo` from the worker checkout; local builds were explicitly authorized earlier. `fmt` is local. Use a task-specific target directory outside the checkout when isolation is needed. Do not repurpose HOME or expose environment secrets. A RED build failure is not a passing test.
7. Preserve existing frozen research, dirty main work, target `../spur-notebook` source/graph/CURRENT, global graph registries and user notebooks. Fixture roots live in temporary directories **outside an enclosing Git checkout**. Do not rebuild the external project or use old precompiled artifacts for POST.
8. The context graph failed to prepare its analyst query during orientation; narrow raw source access is justified. Use normal code retrieval first when available; do not repair global registry/analyst infrastructure in this plan.
9. Emit scope/risk signals before extending production scope. Affected Rust corpus golden changes are allowed only when the new source-grounded ownership/field output explains them. For G1b, the required extractor-version bump also permits updating only the `extractor_version` field in otherwise unchanged non-Rust corpus goldens; prove the changed JSON paths and run the unblessed suite. Never bless a broad unrelated change. Leave the task `in_progress` for brain review; no merge or self-approval.
10. Return **unified diff + one-sentence rationale**, commits, RED/GREEN commands and exits, PRE/POST IDs and meanings, limitations and the completion audit. No downstream dispatch outside the Plan Engine.

## Grounded inputs

Baseline commit: `b098e5db8bf9a8477e68addeba8e4890e56093f8`.

The research artifact directory is `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/`:

- `audits/structure.md`: definition candidate/owner/identity seams and migration risks.
- `enum-ownership-results.json` and `fixtures/enum-ownership.rs`: actual incorrect `E::x` fields.
- `captures/structural-candidates.json`: 446 tuple fields, nine foreign declarations, three extern blocks.
- `fixtures/structural-inputs.json.gz`: exact bytes of the 136 affected source files.
- `captures/structural-syn-rows.json`: independent source spans and held-out controls.

`planning/g1a-pre.json` in the new implementation artifact directory preserves root's catalog PRE receipts. Correct `(source location, variant owner)` pairs pass (`sol_5d158551545c416e`); current enum-owner pairs fail (`sol_838a29da0a4a48ca`). This is useful handoff evidence, not a replacement for each task's fresh PRE.

Stable IDs keep the existing path/FQN/kind/start algorithm. Repairing an incorrect owner changes affected IDs intentionally; no stable-ID hash replacement or promise of edit-stable ordinals. Code-only extraction changes must advance `EXTRACTOR_VERSION` so unchanged-file reuse cannot preserve old omissions.

## Task G1a: Repair enum payload ownership before identities

**Task ID:** `G1a`; source issue `bd-2jq`.
**Depends on:** none.
**Worker:** codex / codex / gpt-6.1-terra / xhigh.

**Files:**

- Modify `crates/spur-graph/src/extract/languages.rs`: candidate assembly and direct-parent selection.
- Modify `crates/spur-graph/src/extract/tree_sitter.rs`: combine definition owners before naming/emission.
- Modify `crates/spur-graph/src/store/build.rs`: extractor compatibility constant only.
- Create `crates/spur-graph/tests/rust_declaration_ownership.rs`.
- Add scoped evidence under the G1a directory; affected Rust goldens only with recorded explanation.

The graph path currently emits tags before enum variants from the edge query. The byte extractor merges candidates first. Establish the complete owner set before both paths produce FQNs, stable keys and containment; do not rename already-emitted nodes while leaving stale keys or edges.

**Acceptance:** `E::A::x` and `E::B::x` have distinct correct direct `Contains`/`Defines` owners and stable-ID inputs; graph and byte extraction agree. Nested modules, repeated field names, impl/method containment and an unrelated declaration retain correct identities. No tuple-field capture, extern/schema or broad resolver change.

**Test seed** (extend with direct edge and identity assertions in the committed test):

```rust
#[test]
fn repeated_variant_fields_use_their_immediate_variant() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"),
        "enum E { A { x:u8 }, B { x:u8 } } fn unrelated() {}").unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let mut fields: Vec<_> = graph.symbols.iter()
        .filter(|s| s.symbol_kind == "field")
        .map(|s| s.qualified_name.as_str()).collect();
    fields.sort();
    assert_eq!(fields, ["E::A::x", "E::B::x"]);
}
```

**Sequence:** fresh PRE exact-owner relations → test commit → run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_ownership` and retain expected wrong-owner assertion RED → complete-candidate owner fix and extractor bump → same command GREEN → `--lib resolver::`, `--test rust_lexical_calls`, `--test rust_receiver_resolution`, affected extractor corpus checks, `check -p spur-graph --all-targets`, scoped fmt/diff checks → POST from real graph and byte exports.

POST must reject moving a field to a **different existing** variant/enum owner; a foreign-key-only rule is insufficient. Commit the fix and evidence separately. If a private extractor needs a parity unit test, place it in the listed extraction file rather than exposing a new public API solely for testing.

## Task G1b: Add physical tuple field nodes

**Task ID:** `G1b`; source issue `bd-f5y`.
**Depends on:** `G1a`.
**Worker:** codex / codex / gpt-6.1-terra / xhigh.

**Files:**

- Modify `crates/spur-graph/queries/rust/spur-edges.scm`: direct tuple field captures.
- Modify `crates/spur-graph/src/extract/languages.rs` and `src/extract/tree_sitter.rs`: ordinal labels, spans and owner-aware emission.
- Modify `crates/spur-graph/src/store/build.rs`: extractor compatibility constant only.
- Create `crates/spur-graph/tests/rust_tuple_fields.rs`; task evidence and narrowly affected Rust goldens.

**Acceptance:** exactly one `Field` per direct tuple-struct/tuple-variant entry, using labels `0`, `1`, etc. `S::0`, `S::1`, `E::A::0` and `E::B::0` are distinct under their actual owners. Nested tuple types, generics and function-pointer types never create extra fields. The physical ordinal counts entries irrespective of cfg; no compiled `.N` inference. Unicode, attributes, comments, visibility and trailing commas are source-grounded. Graph/byte extractor output agrees.

Use a **full declaration-entry span** including owned outer attributes/visibility and a separate exact type span in G1c. Entry ownership excludes commas and adjacent comments/attributes belonging to the next field. If the grammar has no field wrapper, reconstruct entry boundaries from direct list children, not recursive type matches. Existing identity uses the declared entry start. This policy must be exercised against independent syn full/core spans rather than accepted by fuzzy overlap.

**Test seed:**

```rust
#[test]
fn equal_types_and_nested_types_have_one_node_per_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"),
        "struct S(u8,u8,(u8,u8),fn(u8)->u8); enum E { A(u8), B(u8) }").unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let mut fields: Vec<_> = graph.symbols.iter()
        .filter(|s| s.symbol_kind == "field")
        .map(|s| s.qualified_name.as_str()).collect();
    fields.sort();
    assert_eq!(fields, ["E::A::0", "E::B::0", "S::0", "S::1", "S::2", "S::3"]);
}
```

**Sequence:** fresh PRE distinct parent/ordinal and exact cardinality contracts → test commit → `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_tuple_fields` RED (missing fields) → minimal direct-entry capture/name/span code and extractor bump → same test GREEN → G1a tests, `--test rust_macro_token_tree_query`, `--test rust_macro_invocations`, affected Rust corpus checks, all-target check and scoped fmt/diff → POST actual exported tuples.

Rebuild graphs of the 136 frozen source files in an isolated temporary fixture root using **current implementation**, join every one of the 446 tuple rows by exact file/source/kind/declared span policy, and validate owner/ordinal uniqueness. Record any inventory normalization explicitly. Do not read or rebuild the target project's live graph. Retain a cfg-removal control showing physical ordinal need not equal a compiled projection; insert/reorder tests must report the actual offset-sensitive ID behavior. Commit tests before implementation, then fix and evidence.

## Task G1c: Expose canonical declaration evidence

**Task ID:** `G1c`; source issue `bd-1lu`.
**Depends on:** `G1b`.
**Worker:** codex / codex / gpt-6.1-terra / xhigh.

**Files:**

- Create `crates/spur-graph/src/rust_declarations.rs`: typed evidence and source recollection.
- Modify `crates/spur-graph/src/lib.rs`: public exports only.
- Create `crates/spur-graph/tests/rust_declaration_evidence.rs`.
- Add scoped task evidence under G1c.

**Public interface:**

```rust
pub struct RustDeclarationSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}
pub fn collect_rust_declarations(
    graph: &GraphIndexArtifact,
    sources: &[RustDeclarationSource<'_>],
) -> anyhow::Result<RustDeclarationEvidenceSet>;
impl RustDeclarationEvidenceSet {
    pub fn validate(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[RustDeclarationSource<'_>],
    ) -> anyhow::Result<()>;
}
```

The serializable set must be separately versioned and bind graph content hash plus the extractor version. Its scope is `SuppliedPhysicalRustFiles`, **not the workspace or Cargo program**. `covered_kinds` contains `TupleField` only in G1; other declaration kinds are not certified empty. Include every supplied source, including zero-row files, with actual physical stable-file ID, exact Git blob OID and byte length. Reject duplicate/conflicting paths, stale graph/source bindings and virtual notebook cells masquerading as physical files.

Use typed owner references `File { stable_file_id }` and `Symbol { stable_symbol_id }`, preserving structural and lexical ownership separately for future transparent extern containers. Tuple rows reference the actual graph `Field` stable-symbol ID; they include full entry/type/line spans, physical ordinal, visibility text/range and owned raw attribute spans/text. Tuple parents are their actual struct/variant symbols. Do not invent another standalone identity hash or fabricate compiled ordinals.

Per-source availability is `Complete { row_count }` or `Unknown { reason }`. A parser recovery/error cannot certify completeness. Cfg attributes remain raw physical syntax and do not assert configured eligibility. Valid zero-field source is complete with zero rows. Unavailable kinds remain outside `covered_kinds`.

Validation must recompute canonical declarations from supplied exact bytes and authoritative physical graph owners, comparing the complete set and source bindings. Merely checking references, self-consistent hashes, or ranges inside a file is insufficient. Reject omitted/duplicate rows, altered ordinal/type/full range/raw attribute/visibility, wrong **existing** owners, stale OIDs, missing zero-source rows and wrong graph binding. Keep the API independent: no `GraphFacts`/artifact field, persistence or schema change in this task.

**Test seed** (fixture helper returns the same graph constructed from these bytes):

```rust
#[test]
fn source_manifest_includes_zero_field_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "struct Pair(u8,u8);").unwrap();
    std::fs::write(dir.path().join("empty.rs"), "pub fn marker() {}").unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let sources = [
        spur_graph::RustDeclarationSource { file_path: "lib.rs", bytes: b"struct Pair(u8,u8);" },
        spur_graph::RustDeclarationSource { file_path: "empty.rs", bytes: b"pub fn marker() {}" },
    ];
    let set = spur_graph::collect_rust_declarations(&graph, &sources).unwrap();
    set.validate(&graph, &sources).unwrap();
    let exported = serde_json::to_value(&set).unwrap();
    assert_eq!(exported["sources"].as_array().unwrap().len(), 2);
    assert_eq!(exported["rows"].as_array().unwrap().len(), 2);
}
```

**Sequence:** fresh catalog PRE on source/owner/ordinal relations → commit API/positive/negative tests together → run `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_evidence` and retain honest missing-API compile RED → implement the minimum canonical API → same target GREEN plus G1a/G1b, `--test macro_invocation_contract`, current notebook ownership tests, all-target check, scoped fmt/diff → POST from exported serializable sets, independently validate finite identity tuples and all adversarial rejections → separate feature/evidence commits.

New-API compile RED is sufficient evidence of the missing API; do not claim runtime assertions executed. Add any independently discovered runtime regression before fixing that regression. The frozen 446-row output must preserve the per-field provenance described above; a complete zero-row source is distinguishable from unknown parse coverage.

## Brain review and phase completion

Read actual Beads status and completion audit, inspect the unified source diff and exact RED/GREEN logs, reload reported receipts, and verify actual-export provenance before `review_task(approve)`. Wrong-existing-owner controls and clean scoped commits are mandatory. Require changes for unsupported completeness claims or unexplained golden updates.

On three approvals, merge the plan into its isolated integration branch, record the commit and close G1/source lineage issues. Author concrete G2 tasks against the approved API and continue under `bd-hdi`. Do not merge into the user's dirty active checkout. The final notebook acceptance is in G5; the existing research notebook remains preserved.

**Self-review:** G1a addresses owner/ID quality; G1b closes the 446 tuple-node gap; G1c supplies canonical provenance for later persistence. G2–G5 retain all remaining approved requirements in blocked phase issues. The three-task DAG is acyclic and deliberately serial because all successors consume predecessor output. No production schema/call/compiler scope is hidden in G1.
