# Direct Rust Macro Invocation Evidence Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.
> The live submit_plan schema persists issues automatically and has no persist_as_epic argument.

**Source spec:** `2026-10-31-panic-graph-experiment.ipynb`, design cell `5cc809ce-a082-47f9-9f82-17146b946f25`
**Formal @spec cells (if notebook):** none; approved narrative contract and measured fixture/corpus cells.
**Design epic:** `bd-11o` (closed approved-design record)
**Source planning epic:** `bd-krq`
**Execution epic:** `bd-156`
**Execution plan:** `6888d1b0-fee6-47eb-8cc7-c8f18d8ca570`

**Goal:** Capture direct Rust macro sites with exact source evidence and preserve them through full/incremental artifacts, Parquet and scoped queries.

**Current status:** All six tasks are approved. The brain independently passed the complete default-parallel gate (1,128 passed, 0 failed, 19 existing ignores) on the exact reviewed code and integrates only those feature deltas into main. Earlier worker failures and serial results below are historical evidence; the final brain integration record supersedes their pending-review status.

**Architecture:** Add a separate typed evidence collection to GraphFacts and a versioned optional evidence set to GraphIndexArtifact. Store sites in macro_invocations.parquet, bound to snapshot/source provenance, and query by file or caller without fabricating runtime Calls edges. Preserve legacy-unavailable versus captured-empty, unknown resolution and build conditions.

**Tech Stack:** Rust 2021, tree-sitter Rust queries, serde, Arrow/Parquet, existing GraphQueryClient and SPUR solver/catalog MCP.

**User approval and routing:** User authorized implementation with TDD and SOLVE PRE then POST task by task using SPUR worker `codex`, model `gpt-6.1-sol`, effort `xhigh`. All tasks form a sequential chain M1 -> M2 -> M3 -> M4 -> M5 -> M6 because each consumes the preceding contract/artifact and several share files. Brain reviews each task before the next dispatch. No extra approval is needed to start this approved work.

**Build-mode update:** The user requested local builds. All six execution task bodies were updated atomically by plan mutation `ed862b95-9d08-4220-b1ed-b9c199cd6a28`; dependencies and codex / gpt-6.1-sol / xhigh remain unchanged. This supersedes the original remote-only constraint.

**Phase boundary:** Macro namespace alias/re-export resolution, actual cfg evaluation, expansion and CGA/MIR are later phases. Direct sites remain syntax evidence with execution uncertainty.

## Shared task requirements

CONTEXT: User explicitly approved phase-one direct Rust macro evidence from notebook 2026-10-31-panic-graph-experiment.ipynb, cell 5cc809ce-a082-47f9-9f82-17146b946f25, baseline 0a03bf8e8. Agent routing is user-selected codex; model gpt-6.1-sol, effort xhigh, unchanged. Brain owns architecture/review, workers implement one bounded task at a time. Research: existing query misses 29 direct fixture invocations; six real candidate sites in the 270-symbol build call slice are missing. Existing macro_call captures mean call-shaped tokens INSIDE macro arguments and must be preserved.

MANDATORY WORKFLOW: Read applicable foundation skills and discover test-driven-development through skill_navigate/skill_read. Before implementation navigate solve_rule_spec; data_integrity may cover finite identity/reference/cardinality checks. Run meaningful SOLVE PRE (persist:true; verify positive and rejecting fixtures / counterexample as appropriate), write tests, observe genuine RED, commit test(scope), implement minimum GREEN, run tests, run SOLVE POST against the exact landed contract/observed fixture facts, persist receipts, commit feat/fix(scope). Use generic typed solver only after catalog mismatch plus solve_constraint_spec/check; no hand-duplicated catalog rules. Solver checks establish the encoded finite contract, not Rust execution or global panic freedom. If any MCP is absent, emit blocked signal; do not invent results or bypass the requested gate.

BUILD: User explicitly requested local builds after the initial remote-only plan. Use SPUR_REMOTE=0 scripts/spur-cargo test/check/build; never bare cargo. scripts/spur-cargo fmt is local. Keep existing caches and cloud configuration. Run final checks with SPUR_GRAPH_BLESS unset; TMPDIR=/private/tmp avoids the known macOS canonical-path harness mismatch. The existing macro-query baseline passed locally (4 tests, exit 0, build 43.18s). Compiler/setup failures are not behavioral RED/GREEN. Keep the same solver, TDD, commit and review gates. Preserve unrelated dirty changes, stay in worktree, commit only task files, never merge or close issues. Record commands, exact failing/passing assertions, commit OIDs, source bindings, selected rules and solve IDs in issue audit. Emit scope_drift BEFORE needing unlisted files; do not silently broaden.

SEMANTICS: Direct macro_invocation evidence is separate from runtime Calls edges and CallOrigin::MacroBody. Keep raw and normalized paths and exact source ranges. Bare or std-looking names are candidates, not established standard macro identities. Retain aliases unresolved; retain cfg/cfg_attr text without evaluating a build; absence of a test marker is unspecified, not proof of production execution. Nested token-tree text and macro-definition bodies do not become direct runtime macro invocations. Outside-function invocations get no invented caller. No macro expansion, Rust namespace resolver, MIR/CGA, cfg evaluator, panic feasibility claims, or unrelated refactors.

EXPECTED OUTPUT: Unified diff + one-sentence rationale, followed by concise verification evidence (RED/GREEN outputs, PRE/POST solve IDs and interpretations, commit OIDs, changed files, unresolved limitations). Worker must not claim completion unless both solver and runtime gates actually ran.

## Durable scope and source-domain clarification

Mutation `c1b2c0da-8dc3-4081-9dd7-b0c051e87721` updates M3–M6 before M3 dispatch. Physical Rust files are the supported macro evidence domain; notebook-cell Rust keeps ordinary definitions/calls but requires a separate future source contract. Evidence/query metadata exposes `source_scope = physical_rust_files` and distinguishes unsupported source coverage from captured-empty and legacy-unavailable.

M3 adds `GraphFacts.macro_scopes: Vec<MacroScopeEvidence>` and extends `MacroInvocationEvidenceSet` with `scopes: Vec<MacroScopeEvidence>` and typed `source_scope`. Retain M2's already-collected locators; do not reparse snapshots or pretend evidence-only scope IDs are graph symbols. Keep the existing validation entry point, compare persisted scope records with authoritative supplied locations, and require every enclosing/attribute-owner reference to resolve to matching scope provenance. Rows and scopes follow the same per-file full/incremental/delta replacement, move, deletion and canonical ordering/hash rules.

Additional M3 write scope is limited to `src/macro_invocations.rs`, `src/extract/{mod,tree_sitter,rust_macros}.rs`, and `tests/{macro_invocation_contract,rust_macro_invocations}.rs` inside `crates/spur-graph`, for scope transport, validation and fixture updates. The existing broad constructor migration remains mechanical. M4 may additionally update the typed macro model only for persisted scope/source validation.

M4 persists core `macro_scopes.parquet` beside `macro_invocations.parquet`, including explicit evidence/source-domain metadata, manifest counts, completeness, delta handling and legacy distinctions. Complete new empty evidence requires both zero-row tables; missing or conflicting scope data is invalid. M5 exposes the relevant scope locators through backend/overlay parity and avoids complete-coverage claims for unsupported sources. M6 adds end-to-end scope retention/removal assertions and documents the two-table SQL join and coverage boundary.

The new TDD cases cover non-indexed const/type/file scopes and raw-attribute owners, missing/conflicting scope IDs, snapshot/source mismatches, canonical/full-vs-incremental equality, and stale scope removal/move. Runtime RED/GREEN and fresh catalog PRE/POST remain required for each task; no new panic, expansion, cfg-evaluation or production-performance claim is implied.

## Interface contract

M1 defines and tests `MacroInvocationEvidence` and `MacroInvocationEvidenceSet`; subsequent tasks consume that exact API. `GraphFacts.macro_invocations` is a vector of evidence rows. M3 adds `GraphIndexArtifact.macro_invocations: Option<MacroInvocationEvidenceSet>`. Missing legacy evidence is `None`; a new captured graph with no macro sites is `Some` with zero rows. Evidence-set metadata binds graph content hash, schema and extractor version; each row binds source blob OID and exact location. The stable site key is snapshot scoped by the collection hash; no self-referential content hashing is permitted.

### Behavioral regression example

```rust
let source = "fn demo() { debug_assert!(true); debug_assert!(false); }";
// The production extraction helper is the existing build_facts_for_paths.
// The test creates lib.rs in a temp directory and extracts its facts.
assert_eq!(facts.macro_invocations.len(), 2);
assert_ne!(facts.macro_invocations[0].site_id, facts.macro_invocations[1].site_id);
assert!(facts.macro_invocations.iter().all(|site| site.caller_stable_symbol_id.is_some()));
assert!(facts.macro_invocations.iter().all(|site| site.debug_assertions_candidate));
```

`facts` above is the GraphFacts returned by existing extraction, not a new helper API. Artifact, Parquet and query tests assert the same two-row identity set after each boundary, then delete one invocation and assert exactly one row. Attribute and resolution assertions inspect the M1 typed enums. These tests run first to establish the expected failure; infrastructure errors never count as RED.


### Task 1: Define versioned macro evidence and identity contract

**Task ID:** `M1`

**Assigned execution issue:** `bd-3dj`

**Source planning template:** `bd-gn4` (do not target worker audit/signal calls here)

**Depends on:** none

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/src/macro_invocations.rs`
- `crates/spur-graph/src/lib.rs`
- `crates/spur-graph/src/extract/mod.rs`
- `crates/spur-graph/src/store/build.rs`
- `crates/spur-graph/src/memory_eval/memory_graph.rs` (mechanical GraphFacts initializer update; scope accepted in review)
- `crates/spur-graph/tests/macro_invocation_contract.rs`

**Scope Boundary and Acceptance Criteria:**

GOAL: Add the typed evidence model and empty GraphFacts integration used by downstream tasks; establish a tested identity/provenance/span contract.
IN SCOPE: Create macro_invocations.rs, re-export via lib.rs, add GraphFacts.macro_invocations: Vec<MacroInvocationEvidence> initialized empty, and mechanically initialize the new field in store/build.rs and memory_eval/memory_graph.rs.
CONTRACT: MacroInvocationEvidence includes site_id, file_path, source_oid (Git blob hash of exact bytes), nullable caller_stable_symbol_id, enclosing stable ID/kind (file scope permitted), raw_macro_path, normalized_macro_path, byte_range (half-open), line_range (one-based inclusive), raw attributes with owning scope/span, conservative test_context, debug_assertions_candidate, execution_phase, resolution evidence and optional sink_family_candidate. Typed enums distinguish unresolved/explicit_path_candidate/local_definition_candidate; local evidence records its definition location without claiming full name resolution. Test context must represent unspecified and conditional. Function body syntax does not prove runtime execution. Define MacroInvocationEvidenceSet carrying schema/extractor version, graph_content_hash and rows; downstream artifact field will be Option<MacroInvocationEvidenceSet>, where None means legacy/unavailable and Some(empty) means captured with zero sites.
IDENTITY: A site digest is deterministic and collision-resistant over unambiguous length-delimited version/path/source_oid/span identity; global identity is (set.graph_content_hash, site_id), explicitly snapshot scoped. No truncation-based arbitrary constant and no circular hash. Provide focused validation for nonempty/matching provenance, ordered valid spans, duplicate identity and enclosing/caller consistency expressible without compiler semantics. Pick one public helper API, document it in tests and completion so M2-M5 use it consistently.
ACCEPTANCE: Serde roundtrip preserves every field and unknown states; negative range/provenance/duplicate cases reject; two identical macro names at different spans remain distinct; deterministic identity insensitive to enumeration order, changed source/span changes identity; GraphFacts::empty has no sites and existing facts tests compile. SOLVE PRE/POST verifies the finite identity/range/reference contract with faithfully supplied fixture rows and deliberate invalid cases. Do not add GraphIndexArtifact member or parser capture in M1.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_contract; run the directly affected existing unit tests. RED must be the expected contract failure, not a remote/tool error.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

### Task 2: Capture direct Rust macro sites and conservative context

**Task ID:** `M2`

**Assigned execution issue:** `bd-1rr`

**Source planning template:** `bd-3ns` (do not target worker audit/signal calls here)

**Depends on:** `M1`

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/queries/rust/spur-edges.scm`
- `crates/spur-graph/src/extract/rust_macros.rs`
- `crates/spur-graph/src/extract/mod.rs`
- `crates/spur-graph/src/extract/languages.rs`
- `crates/spur-graph/src/extract/tree_sitter.rs`
- `crates/spur-graph/tests/rust_macro_invocations.rs`
- `crates/spur-graph/tests/rust_macro_token_tree_query.rs`

**Scope Boundary and Acceptance Criteria:**

GOAL: Populate the M1 GraphFacts evidence from direct Rust macro_invocation AST captures, preserving all existing edge capture behavior.
IN SCOPE: Dedicated @macro_invocation and path capture in Rust spur-edges.scm; focused Rust collector module called at language extraction with actual file bytes/definitions; narrowly expose FactBuilder recording if required. Do not send these sites to pending Calls resolution. Use M1 API verbatim and report necessary contract corrections before changing it.
ACCEPTANCE FIXTURES: Direct panic/unreachable/todo/unimplemented/assert/assert_eq/assert_ne/debug_assert/debug_assert_eq/debug_assert_ne; std/core qualified and leading :: paths; raw identifiers; whitespace/comments in qualified paths; (),{},[] delimiters; repeated sites; multiline and UTF-8 before invocation; alias explode! retained unresolved; local macro_rules shadowing retained as local definition candidate when lexically visible, never unrelated function; test and async test attributes, #[cfg(test)] modules, nested cfg/cfg_attr treated conservatively, nested functions attributed to exact innermost definition; module/file/const/type sites have appropriate enclosing kind and nullable function caller; macro definition bodies, wrapper!(panic!(...)) and stringify!(panic!(...)) never fabricate nested executable sites; comments/strings/normal calls are negatives; recoverable parse errors do not fabricate impossible spans. Read research fixture source cell bd771410-a4aa-4821-bbc5-66662b45d8e5 through notebook MCP if useful, but do not edit notebook. At least 29 annotated direct sites and exact expected site set/span/caller/context assertions. Preserve ordinary calls, HOF references and macro-body helper calls; run existing rust_macro_token_tree_query and hof_references_edges.
CONTEXT: Attributes carry ancestor evidence, not a declaration that unknown cfg is true. Debug condition is a syntactic candidate and never inferred merely from release/debug profile. Preserve all direct macros even when not in sink name list.
SOLVE PRE/POST: Verify emitted finite site uniqueness, exact caller references/ranges and context policy using observed fixture data; include an invalid duplicate/unrelated-caller negative contract. Runtime tests own parser correctness.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_macro_invocations --test rust_macro_token_tree_query --test hof_references_edges.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

### Task 3: Carry macro evidence through full and incremental artifacts

**Task ID:** `M3`

**Assigned execution issue:** `bd-1k5`

**Source planning template:** `bd-shq` (do not target worker audit/signal calls here)

**Depends on:** `M2`

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/src/store/lance_sections.rs`
- `crates/spur-graph/src/store/canonical_hash.rs`
- `crates/spur-graph/src/store/build.rs`
- `crates/spur-graph/src/store/parquet.rs`
- `crates/spur-graph/src/store/cache.rs`
- `crates/spur-graph/src/search.rs`
- `crates/spur-utilities/mentions/src/code/source.rs`
- `crates/spur-graph/src/query_client.rs`
- `crates/spur-graph/src/schema.rs`
- `crates/spur-graph/src/traversal.rs`
- `crates/spur-graph/src/memory_eval/memory_graph.rs`
- `crates/spur-graph/src/git_walk.rs`
- `crates/spur-graph/src/temporal.rs`
- `crates/spur-graph/src/mcp/overlay_runtime.rs`
- `crates/spur-graph/src/mcp/request_cache.rs`
- `crates/spur-graph/src/mcp/mod.rs`
- `crates/spur-graph/src/mcp/rebuild_singleflight.rs`
- `crates/spur-analyst/src/overlay/session_tests.rs`
- `crates/spur-graph/src/selector.rs`
- `crates/spur-graph/benches/incremental.rs`
- `crates/spur-graph/benches/parquet.rs`
- `crates/spur-core/src/server/handlers/rebuild_singleflight.rs`
- `crates/spur-core/tests/worker_server_dispatch.rs`
- `crates/spur-graph/tests/constructs_edges.rs`
- `crates/spur-core/tests/code_graph_e2e.rs`
- `crates/spur-graph/tests/lance_sections.rs`
- `crates/spur-graph/tests/calls_range_resolution_edges.rs`
- `crates/spur-graph/tests/overlay_client.rs`
- `crates/spur-graph/tests/parquet_roundtrip.rs`
- `crates/spur-graph/tests/resolver.rs`
- `crates/spur-graph/tests/defines_edges.rs`
- `crates/spur-graph/tests/hof_references_edges.rs`
- `crates/spur-graph/tests/diagnostics_persist.rs`
- `crates/spur-graph/tests/mcp_overlay_notfound_retry.rs`
- `crates/spur-graph/tests/parquet_schema_invariants.rs`
- `crates/spur-graph/tests/load_artifact_dispatch.rs`
- `crates/spur-graph/tests/query_client_parity.rs`
- `crates/spur-graph/tests/hcl_address_resolution.rs`
- `crates/spur-graph/tests/extractor.rs`
- `crates/spur-tui/src/mentions/registry.rs`
- `crates/spur-graph/tests/range_resolution_edges.rs`
- `crates/spur-graph/tests/hcl_reference_edges.rs`
- `crates/spur-cli/tests/analyst_temporal_views.rs`
- `crates/spur-graph/tests/hcl_definition_query.rs`
- `crates/spur-core/tests/knowledge_context_pack_2_eval.rs`
- `crates/spur-graph/tests/go_definition_query.rs`
- `crates/spur-graph/tests/temporal_resolution.rs`
- `crates/spur-graph/tests/sql_definition_query.rs`
- `crates/spur-graph/tests/integration_git_incremental.rs`
- `crates/spur-graph/tests/python_implements_edge.rs`
- `crates/spur-graph/src/store/snapshots/spur_graph__store__canonical_hash__tests__canonical_bytes_layout.snap`
- `crates/spur-graph/tests/macro_invocation_artifacts.rs`

**Scope Boundary and Acceptance Criteria:**

GOAL: Add GraphIndexArtifact.macro_invocations: Option<MacroInvocationEvidenceSet> and preserve evidence exactly through full builds, bucket reuse, dirty overlays, canonical hashing and JSON compatibility.
IN SCOPE: schema.rs, store/build.rs, store/canonical_hash.rs and canonical hash snapshot; new tests/macro_invocation_artifacts.rs. Mechanical artifact constructor initialization only in explicitly listed files; no unrelated behavior changes. M1 owns type API and M2 capture behavior. Make new full builds Some(empty/nonempty), legacy JSON defaults None. Bump graph schema/extractor compatibility deliberately (currently spur-graph-schema-v10 / 2026-08-30-html-css-extraction-v1), update version assertions, and include evidence in artifact content hash while preserving legitimate legacy validation semantics. Source graph hash is based on file contents; bind evidence set metadata to it without hashing a value into itself. Snapshot identity cannot silently reuse stale evidence.
ACCEPTANCE: Full build includes correct source_oid from bytes and stable caller IDs; JSON roundtrip all fields, legacy missing field remains unavailable; editing only invocation evidence changes canonical artifact hash; incremental add/change/remove/rename/move replaces all sites for changed paths, removes deleted rows, preserves unchanged rows, rebinds set snapshot metadata/site identity consistently and matches full build evidence for the same bytes. Test dirty worktree content vs Git HEAD, retained outside-function sites, repeated sites and source OID mismatch rejection. Do not copy old evidence into changed files or lose it in FileBucket/rebuild_from_buckets/build_worktree_delta_artifact. When updating dozens of constructors, only initialize new optional member (None for legacy fixture) and format touched blocks.
SOLVE PRE/POST: data_integrity verifies fixture file/source/caller references, unique snapshot-site keys and exact replacement/removal row sets; include old-row leakage and dangling caller invalid snapshots.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_artifacts --test macro_invocation_contract --test rust_macro_invocations --test rust_macro_token_tree_query --test hof_references_edges; store:: and full extractor tests, canonical/hash/delta tests and dependent/all-target checks. Full disk/cache integration acceptance is owned by M4 as clarified above.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

### Task 4: Persist macro evidence as a complete versioned Parquet table

**Task ID:** `M4`

**Assigned execution issue:** `bd-iea`

**Source planning template:** `bd-3tg` (do not target worker audit/signal calls here)

**Depends on:** `M3`

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/src/store/parquet.rs`
- `crates/spur-graph/src/store/macro_invocations.rs`
- `crates/spur-graph/src/store/mod.rs`
- `crates/spur-graph/src/store/cache.rs`
- `crates/spur-graph/tests/macro_invocation_parquet.rs`
- `crates/spur-graph/tests/parquet_schema_invariants.rs`
- `crates/spur-graph/tests/parquet_roundtrip.rs`
- `crates/spur-graph/tests/load_artifact_dispatch.rs`

**Scope Boundary and Acceptance Criteria:**

GOAL: Persist macro_invocations.parquet with queryable site columns and snapshot metadata, for full and delta writes, and make completeness/backcompat explicit.
IN SCOPE: Focused store/macro_invocations.rs Arrow/Parquet helpers; wire full/delta readers/writers and manifest row counts/version/completeness in store/parquet.rs; canonical cache publication validation in cache.rs where required; store/mod.rs exports; targeted tests. Retain all M1 fields losslessly (raw attributes may be structured JSON column if justified), with top-level query columns for site_id/file_path/source_oid/caller/path/span/context/resolution. Bind graph_content_hash/schema/extractor versions in manifest/file metadata. An evidence-bearing new artifact must write a valid zero-row table even when empty.
ACCEPTANCE: Full and delta roundtrips preserve Some(empty), Some(rows), None legacy distinctions; legacy schema artifacts missing the table remain readable as evidence unavailable; current schema declaring complete evidence must reject missing/truncated table, bad row counts, mismatched graph/source/version metadata and invalid row provenance before publication/loading. Do not hide corruption as empty. Include table in artifact completeness and canonical cache reuse/publication; do not conflate this core evidence with optional Lance embedding sidecars. Existing normal/HOF/macro-body edges unaffected; write/read/source OID and nested context roundtrip tests cover all fields.
SOLVE PRE/POST: Verify finite declared-vs-written counts, row identity/foreign keys and snapshot metadata consistency, including missing/extra/stale row negative fixtures. Solver cannot establish IO atomicity; fault-injection runtime tests do.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_parquet --test parquet_schema_invariants --test parquet_roundtrip --test load_artifact_dispatch; targeted cache publication tests.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

### Task 5: Expose macro evidence queries with overlay and backend parity

**Task ID:** `M5`

**Assigned execution issue:** `bd-3u8`

**Source planning template:** `bd-1kx` (do not target worker audit/signal calls here)

**Depends on:** `M4`

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/src/query_client.rs`
- `crates/spur-graph/src/macro_invocations.rs`
- `crates/spur-graph/src/store/macro_invocations.rs`
- `crates/spur-graph/tests/macro_invocation_queries.rs`

**Scope Boundary and Acceptance Criteria:**

GOAL: Make direct macro evidence queryable by file and caller stable ID through GraphQueryClient, with correct in-memory/Parquet/dirty-overlay semantics and explicit coverage availability.
IN SCOPE: Add a focused query/filter/result API using M1 evidence model. Implement reference forwarding, InMemoryClient, ParquetClient and OverlayClient. Return snapshot provenance and distinguish legacy/unavailable coverage from complete empty result. An unsupported external custom backend must return explicit unavailable/error, never a default complete empty answer. Parquet queries read/prune the macro table rather than loading the whole graph; errors are surfaced. Avoid arbitrary result caps; use explicit caller supplied limit only if needed and expose truncation.
ACCEPTANCE: File/caller filtering and combined filters match identical in-memory and Parquet results; repeated sites retained and deterministically ordered; file-scope sites discoverable by file without fabricated caller; wrong/stale source identity detected; overlay replaces changed-file sites, removes deleted-file sites and retains untouched base evidence; unknown base coverage remains visibly unknown where not replaced. Query joins by caller with existing caller paths, without inserting macro names into Calls edges or pretending candidates are proven panics. Add tests for forwarding, legacy unavailable, empty complete, corrupt table and overlay add/delete/move. No new MCP tool schema or analyst SQL view is required in this phase; persisted table is already directly SQL readable.
SOLVE PRE/POST: Verify concrete filtered/overlay output identities and reference/cardinality contract, with leakage/omission negative fixtures. Tests establish actual backend behavior.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_queries --test query_client_parity --test overlay_client.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

### Task 6: Verify end-to-end macro coverage and document evidence queries

**Task ID:** `M6`

**Assigned execution issue:** `bd-2u9`

**Source planning template:** `bd-3vi` (do not target worker audit/signal calls here)

**Attempt2 authority:** scope mutation `6ceecfdf-a59f-4c29-82a8-d3d171c4fc58`
supersedes the original three-file scope below. The exclusive authorized paths
are those three plus `crates/spur-graph/src/mcp/{mod.rs,overlay_snapshot.rs}`,
`crates/spur-graph/tests/{markdown_links_edges.rs,schema.rs}`,
`crates/spur-graph/src/extract/{tree_sitter.rs,languages.rs}` and
`crates/spur-analyst/src/overlay/session_tests.rs`. Only the four bounded repairs
are authorized: portable failure fixtures, established unresolved Links test
policy, temporal capture identity projection, and optional manifest fixture
initialization. No notebook or broader resolver/snapshot behavior change.

**Depends on:** `M5`

**Suggested Worker:** codex, model gpt-6.1-sol, effort xhigh (user selected)

**Files:**
- `crates/spur-graph/tests/macro_invocation_end_to_end.rs`
- `crates/spur-graph/README.md`
- `docs/superpowers/plans/2026-10-31-macro-invocation-evidence.md`

**Scope Boundary and Acceptance Criteria:**

GOAL: Demonstrate the approved coverage contract through extract -> full/incremental artifact -> Parquet -> query, and provide exact reproducible validation/query instructions.
IN SCOPE: Add a focused end-to-end integration test, README evidence schema/SQL example/coverage limits and plan evidence receipts. Implementation fixes outside these files require scope_drift before changes and brain replanning. Do not edit the research notebook; brain will append verified results through notebook MCP after integration.
ACCEPTANCE: A held-out fixture models unreachable! in parse_raw_diff plus two debug_assert! sites in the same caller, retaining three distinct rows and correct debug/context/caller evidence; all are discoverable from a scoped caller join with no macro runtime edge. Preserve local alias/shadowing uncertainty and nested token negatives. Edit/remove one site, rebuild incrementally, roundtrip and query exact resulting set; compare with full build. Record full spur-graph regression and dependent-crate compile results, solver receipts and commits from M1-M5 in the plan. Include exact DuckDB read_parquet macro evidence SQL joined to caller stable IDs, distinguish all source candidates from panic executions and unresolved from safe.
SOLVE PRE/POST: Validate end-to-end observed expected row sets, identity/source/caller constraints and incremental deletion negative case using catalog rules; compare outputs with runtime assertions.
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph; check all affected dependent crates via scripts/spur-cargo with the same local environment. Run fmt --check using wrapper. Do not broaden tests repeatedly after successful required checks without a new failure/change.
OUTPUT: Unified diff + one-sentence rationale + complete verification matrix, PRE/POST solve IDs per task, commit OIDs and any real remaining limitations.

**Implementation sequence:** SOLVE PRE -> failing behavioral tests -> observed RED -> test commit -> minimal implementation -> observed GREEN -> SOLVE POST using landed facts -> implementation commit -> beads audit -> brain review. The worker must return exact commands/output and persisted solve IDs.

## Corrected acceptance ownership after M3 review

Mutation `a99f1c78-a651-4f7a-bddf-b69d8f263e77` resolves a staged dependency boundary. M3 owns exact in-memory/JSON/full/incremental/delta/hash transport and its scoped checks. M4 owns the complete Parquet/cache round-trip integration suite, including the five cache seed cases that currently rebuild fully when the old reader drops macro evidence. These remain required before M4 approval and before the full feature merges; no check is waived.

Independent checks at approved M2 `a6c6c6eef` reproduce the same source-hash failure and seven extractor failures. Diagnostics show `.spur/graph-index.pointer.json` becomes an extra supported JSON source after cache publication, while `src/lib.rs` is unchanged. M4 additionally owns narrow graph-owned artifact/pointer exclusion in Git and filesystem discovery, preserving legitimate JSON and unrelated hidden sources. Its scope adds `src/store/build.rs`, `src/discovery.rs`, and `tests/integration_git_incremental.rs` under `crates/spur-graph`.

M3 additionally owns stable isolated corpus preparation in `tests/extractor.rs`, an explicit golden-output exclusion regression, and intentional schema refresh of the six `expected_graph_index.json` outputs under `sample_corpus`, `python_corpus`, `typescript_corpus`, `javascript_corpus`, `cpp_corpus`, and `markdown_corpus`. The golden output remains external to extraction input; two runs with `SPUR_GRAPH_BLESS` unset must prove stability. Do not normalize away macro evidence or globally disable JSON extraction. M3's newly added composed-evidence `expect` must use the existing error channel instead of introducing an unnecessary panic candidate.

M3 attempt2 passed36 macro tests and155 store tests, dependency/all-target checks and finite PRE/POST projections; it was not approved while the boundary/golden failures remained. Retry3 preserves that work and supplies the narrow corpus repair. M6 retains the full final regression/dependent checks and the acceptance ledger. Baseline logs and a diagnostic source copy are preserved under `/private/tmp/m3-brain-baseline-*`.

## Review and completion

M3 attempt 1 made no changes because it selected source template `bd-shq` rather than its assigned execution issue `bd-1k5`. The latter was correctly in progress; worker audit/signal tools correctly rejected the template. Mutation `b87390e6-7aa7-4463-80d3-80bb5f49acac` prefixes the actual execution identity for M3–M6 and removes stale automatic context paths. The retry must target the execution issue; no rebind or source-template activation is required.

M1 attempt 2 produced the typed model and contract tests but failed compilation at the additional memory-evaluation GraphFacts initializer. Review accepted that same-crate mechanical scope addition in mutation `181d774c-216a-4aab-a303-d2e8ba13ed94` and requested changes with prior work preserved. Review also independently found incorrect fixture line annotations: the two debug assertions are on lines 3 and 4, the register invocation on line 6, the function covers lines 2–5 and the file covers lines 1–6. Earlier PRE receipts (`sol_464b1e08c2c44f3f`, `sol_c6fbad0d1cc0486c`; negative controls `sol_b3b6d03c0d374e62`, `sol_03b385b54c424b49`) verify only their supplied finite facts, which included those inaccurate line values. The retry must correct source-bound facts, refresh PRE, exercise a runtime regression rejecting FunctionBodySyntax on a file-scope row without a caller, complete local GREEN and persist POST receipts before approval. No M1 success is claimed from the earlier compile-time API absence or E0063 setup failure.

Review beads status/audit/signals before each diff. Independently inspect actual test logs and solver requests/results; do not infer runtime safety from a solver model. Use review_task(approve) only after both gates pass. Missing infrastructure or MCP capabilities require a recorded blocker and recovery, never silently skipped checks. Integrate approved committed work, preserve unrelated working-tree changes, append verified outcomes to the research notebook via notebook MCP, and close the implementation epic only after all acceptance criteria are met.


## Final execution ledger (M6)

Execution identity comes from dispatch, not the source template. All workers keep
`codex/gpt-6.1-sol/xhigh`. Notebook updates and feature integration belong to the
brain; M6 neither edits the notebook nor merges/closes issues.

| Task | Execution / source | Final authored implementation | Reviewed normalized tip |
| --- | --- | --- | --- |
| M1 | bd-3dj / bd-gn4 | `2035c09f83b239b66058ea92a9f530fb8da94a08` | `72ca468c1f6e8d97e542f7d94ef69dac96e199e0` |
| M2 | bd-1rr / bd-3ns | `59225ba135c9674e848c46ff20a80f839b2c8fe2` | `a6c6c6eefdda81f7f5f4758092a0aca5745c43c6` |
| M3 | bd-1k5 / bd-shq | `40a020e08f7f8d9b2d1aaa2b9b022463ef53d81c` | `5eefd08f70739d8c9c27698dd272d63604f299bd` |
| M4 | bd-iea / bd-3tg | `2c2bf5ade8ff68cb566770cfaffd6b0af5b62081` | `f3c56ecd8d0e656b09407ec10118d08b4d713e6e` |
| M5 | bd-3u8 / bd-1kx | `4c6e93fc25beab30e83a6860d82fb98a8ebb2ffe` | `e6205548455fd3de9047abc250146d90d4ed2e1a` |
| M6 | bd-2u9 / bd-3vi | Prior tests `2bb8c9423d966a28ee35b77825bbee25fce2a6d7`, docs `09b907c791fc5b070111f167c5243c456ebe7361`; bounded rework below | Prior normalized `dbc83f89e75c57ccd19011f6668659245d147e21`; final review pending |

### Reviewed upstream runtime and source receipts

These are historical per-task results reloaded from durable handoffs, not new M6
runs. The fresh final-revision gate is recorded separately below. Full requests,
observations and outcomes remain in the named Beads audits and persisted solves.

| Task | Genuine RED and authored test commit | Reviewed GREEN / source binding / durable record |
| --- | --- | --- |
| M1 | FunctionBodySyntax with file scope/no exact caller assertion fails (10 pass/1 fail); `735d1dcf67435b9dc0837ecaab7fb13a0ffe0f9e` | Contract11, memory11, token-query4, store initializer1 pass. SOURCE116 bytes/OID `2d20325ee56a0c1ece1588f3adca308693e93f11`; bd-3dj90–94. Initial compile-only attempt was not behavioral RED; inaccurate historical line receipts were superseded. |
| M2 | Initial collector0 vs42 and query0 vs2; `1be01b8e5981c4c6d65f940ecea9c2af450f0c46`, implementation `9ad735ac7b481ff22b4883a666604a12ab430720`. Notebook owner regression `6a1eb31fdcecfdc4746088ec1a7da8d6c75178bc` then physical-owner guard | Final collector7/query5/HOF5, contract11, notebook18 pass. Fixture2359 bytes/OID `8ddf1b0716584da082b1a45ee75257c3bdeb473f`,42 sites/19 scopes. Cell OIDs `89868b269486540d574dabe5b556020d0bed9443` and `f6f8141497811c9304326ad3c479d482f2d352aa` stay outside physical evidence; real #cell filename stays supported. Full handoff `/private/tmp/m2-attempt2-verification.md`. |
| M3 | Transport tests `0448f03f6c77af266073094f294be43e606e5ffc`, compatibility `f6b60b3b6636fd21d6e73f16da853021f88594a1`; implementation `233515eba1d65734405c2222e6628f1118b34527`. Golden-input regressions `d4736cb0b3eb649ae1cf4a23e7d7c6727af5b571` fail before exact output exclusion | Final extractor54 pass/3 existing ignored; macro36/store155/dependents pass. Exact source paths/OIDs and emitted rows/scopes match retained transport observations. Corpus PRE bd-1k5 comment195, POST198; transport165,171–177. Authorized six-golden refresh was M3 history; M6 never blesses. Six disk/cache failures remained explicitly assigned to M4 at this stage. |
| M4 | Storage0 pass/8 fail; integration19 pass/9 fail/1 ignored, including all six M3 disk cases; tests `133e73603873bfc6c373e5027cb54cebb36260df` | Final integration28/storage9/roundtrip9/schema4/dispatch2 pass; store155 pass/3 ignored. Generated graph-owned pointer paths no longer alter source hash. Five sites/four scopes, lib.rs OID `084ff75b4ee5316551cb059b42410c106096a9b1`, graph `6dc93f8fc064f623819aae85f3cb0f4f38df9a0fb2a8ea9e35d2f3654950f9a2`. bd-iea310,311,313,314. |
| M5 | API scaffold `8f51f2403ac6400090a4a3e3bc1523bf0bfb3fca`; final two regressions `53f43fbf45f2aaaf933786ae2db689485784dede` genuinely reject missing zero-site source manifest and source_oid without explicit file | Final42 pass (queries14/Parquet9/overlay12/parity7), all-target/fmt/diff pass. Initial reviewed tree `773c154a7e61b4500279f9e06246c10bf2c21dff` had40 pass yet failed independent review; preserved `3e7f67eb29da821d78817ecb0c408f5a0f51858a` is history. Final bd-3u8 audits453–455,457/completion459. Exact file blobs in `/private/tmp/m5-bd-3u8-attempt2-handoff.json`; focused count fix does not fix legacy full-reader count behavior. |

M2 baseline corpus and M3 staged disk failures were repaired by their designated
M3/M4 fixes; none were waived or relabeled as a passing earlier attempt. M4 owns
all `integration_git_incremental` cases, including its independently reproduced
generated-pointer/source-hash bug. M3 owns core in-memory/JSON transport and corpus
input/output isolation. Full extractor and incremental outcomes come from the fresh rework FINAL
revision below, independently of this historical table.

### Per-task catalog receipts

All use implemented `data_integrity` catalog rules and persisted finite facts.
`+` denotes sat/pass, `−` denotes expected unsat/fail. These establish the encoded
finite contract only, not parser correctness, cryptographic properties, IO
atomicity, macro identity/expansion, cfg truth, execution or global panic freedom.

M1 final source-corrected PRE → POST:

| Projection | PRE | POST |
| --- | --- | --- |
| identity + | sol_d8d5392abbe54013 | sol_130b6917aed3460d |
| context + | sol_91d5fb4c75354c98 | sol_77ba56afb840475c |
| scopes + | sol_003e956b4a4c4b81 | sol_020406455d2f4aaa |
| unknown function + | sol_c68d12cd4c904af9 | sol_e78e13e0c3214d5c |
| file/function-body − | sol_a1c47a86e6e14129 | sol_6d06935d4da04ba1 |
| unrelated caller − | sol_2446a38477a047d3 | sol_da071d2277d64b03 |
| bad byte range − | sol_4ae5fd984b40423c | sol_39d388c19df04610 |
| incorrect line − | sol_b19d3481038d461f | sol_c74f4a140bbf4e48 |
| stale source − | sol_b62aca75c4d2410f | sol_d41ad79714174af1 |
| duplicate identity − | sol_dbceced79b4f4100 | sol_a7afedaae1c94aee |

M2 retained42-site fixture projections and final source-owner correction:

| Projection | PRE | POST |
| --- | --- | --- |
| site batches0–3 + | sol_2f75dd3a6ae840a8, sol_52928bb31f0a4b9e, sol_368051811b16418e, sol_261a6a496fdf4547 | sol_9f5d0cadf1954a12, sol_246336c5d96842ca, sol_66e788624cfa4506, sol_19ead062f4044dad |
| duplicate − / unrelated caller − | sol_618a56151ac64c21, sol_2a3da07d919c4b5a | sol_1696f03fbe6d4817, sol_a3eb28ba231b4048 |
| exact caller/source/ranges + | planned batches above | sol_1744eb18deba448e, sol_2c066cfe0ae74f6e, sol_e0aed00a91374ea7, sol_13631e5483a14e30 |
| unrelated caller/range − | caller negative above | sol_fa116de5174f445c |
| observed half-open range batches + | planned batches above | sol_309d17880f854e7c, sol_c9f9560fed514f9f, sol_ceeccce07ac1455b, sol_de71b6fd70694b0c, sol_1cdf455cc41d4da6, sol_3f0f387b76e24f1d |
| physical source owner + | sol_0c9c76d19900490a | sol_47e6bbbd095f4c2e |
| Cell owner − | sol_f581cc4bc7ba42cd | sol_9f5572432b474c4d |
| nonzero notebook physical rows − | sol_2dc04d7c94e74453 | sol_ef95396171b54a4d |

M3 transport: PRE source `sol_a59dcd4dbd344e7b`, scope-source
`sol_5db4814e22194ffb`, scope `sol_44b5e0a55a7744f4`, caller
`sol_4c948016ab6d4a5d`, site identity `sol_0e34d6910ec74e14`, scope identity
`sol_6d946bc5929b4e57` pass; old-row `sol_6f24d053d3094f54`, dangling-caller
`sol_61d32fd155d441bb`, missing-scope `sol_944959be3693482c`, conflicting-scope
`sol_b448d4db8218457b` reject. POST full/incremental/removed domain bindings
`sol_8bfd31c3eeb544b7` / `sol_7b63c99aed0044d4` / `sol_94ddc5c78a2d4393`, full
caller `sol_64cfce77aa7845b5`, incremental caller `sol_98e44d44a4db46f7`, and
site identities `sol_d319fe59b9f843db` / `sol_d2144b1599924a78` /
`sol_812188364e904c93` pass. POST stale-row `sol_6a29bac6fd184226`, caller
`sol_b03b822689654f9d`, missing-scope `sol_2403a0a73be94a40`, scope-location
`sol_fe8a66ec5bf343f5`, duplicate-scope `sol_4361f60646624859`, stale-source
`sol_c5d8e812b2394b8f`, wrong-domain `sol_abcae45ca86c4d4d` reject. Complete
source/scope/range/replacement projections: bd-1k5 audits165,171–177 and
`/private/tmp/m3-rework-retained-transport-receipts.md`; final rework recaptured
those exact facts and checked equality before reuse.

| M3 corpus exact path/OID projection | PRE | POST |
| --- | --- | --- |
| sample + | sol_ba811c5306c14ca1 | sol_ff123071e38a43c5 |
| Python + | sol_0ae620f906c849d8 | sol_41122934a3d44cce |
| TypeScript + | sol_773446aa384941a0 | sol_69b40cc999bd4b01 |
| JavaScript + | sol_dd200b692aed42ec | sol_b10a1432552b4de1 |
| C++ + | sol_e0241209bba14fd6 | sol_d66d5194e8514850 |
| Markdown + | sol_0035772692e642bf | sol_fadd075e6e9d4224 |
| generated golden leak − | sol_1db4131a192641bd | sol_a49fd7f0d5164b89 |
| real input loss − | positive exact-input PRE above | sol_6db5f947386047f0 |

M4 PRE: positive `sol_ac56d918ee6743a7`; rejects missing-scope
`sol_79cd83cf45a64d12`, extra-site `sol_329e96da74cf4c00`, stale-source
`sol_56a65b68a9fa4459`, generated-output `sol_a482df376a364a6a`, metadata
`sol_bab991add34d44c7`. POST:

| Projection | Pass | Rejecting control |
| --- | --- | --- |
| counts | sol_cfc0221fa443415b | missing-scope sol_2b8b6a11a0834eac; extra-site sol_2497df329b704d5e |
| zero tables | sol_360ce97854924b71 | — |
| identity | sol_59619e0865234a26 | sol_34d4a0610fd24553 |
| source | sol_8a75e65861484170 | sol_17b08897b02d47bf |
| scopes/attribute refs | sol_9086026845d04b89 | missing sol_41fe19b8c5e44cf7; provenance sol_f717486ec03a409a |
| graph callers | sol_d52b578b7d144467 | sol_04a75b667c62496a |
| metadata | sol_b05ed9b6eb6d4915 | sol_964301fba4e04ea5 |
| filesystem source set | sol_1632d5a5436b4caf | generated sol_60d6020c9e4d456d; actual RED sol_07be048593fb4210 |
| Git source set | sol_6325ab453b9e48a6 | generated sol_58140eee5d604df8; actual RED sol_e3ad6b59994146da |

M5 retained filter/overlay receipts remain historical exact-fixture checks:
PRE filtered `sol_a74b1805257b481f`, overlay `sol_1d771638a18c49c1` pass;
leakage `sol_97c698ac23c84a09`, omission `sol_f55cec09240a4b93`, missing scope
`sol_83707e59bf5d47b1` reject. POST filtered `sol_e4a0c9ffc1b04782`, overlay
`sol_c513465272b642da`, scope provenance `sol_bdb4883e71bd4b21`, scope ranges
`sol_d23f6323d78d4707`, row sources `sol_bd6bc7a74f2d4015`, graph callers
`sol_edac4d10c1cf47ba`, snapshot partition `sol_280618f3a70945d7` pass;
leakage `sol_1a9b95184c0f4f4d`, omission `sol_b8015bfacab14c91`, missing scope
`sol_363a7dc7c2e44779`, extra scope `sol_93c358d7879342cd`, scope source
`sol_425f019dbc4146f1`, row source `sol_9bfde0e2214046e2`, invented caller
`sol_bbf811a729e04c3a`, snapshot `sol_b275c4db86b54d4d` reject.

Fresh final M5 regression PRE: shape `sol_d7d2a9dbd0dd4eca`, source
`sol_6fd3d80f135a4925` pass; negatives `sol_9c25bb46f4fd4233`,
`sol_02eb0935a6854392`, `sol_e6aa52c78b4944de` reject. POST source identity
`sol_23f136c991694d2f`, coverage outcomes `sol_8bc09ba3ade84705`, base/identity
shape `sol_38bc4fab5eb74ab1`, dirty/identity shape `sol_7dd7b550a0a74ede` pass;
omitted zero-site source `sol_a8a86727af054630`, actual RED coverage
`sol_e23db51503f34df2`, actual RED invalid-filter acceptance
`sol_72a8c3950b394a37` reject. All12 reloaded/pinned by brain; these regression
receipts complement earlier filter/overlay contracts.

### M6 held-out receipts and TDD interpretation

The original M6 attempt changed only its authorized test, README and this plan.
It verified existing reviewed production APIs. The brain-authorized attempt2
repairs and final acceptance appear below; the original implementation and
authored commits remain in history. The initial no-callees assertion was an M6 oracle error:
`is_empty()` and call-shaped `len()` inside macro arguments must survive. That
failure, and a corrected relation-type compile error, are not production RED.
After correction, all three tests pass on the reviewed M5 tree without production
changes. No working code was disabled to create a failure.

The independently enumerated `src/diff.rs` source has blob OID
`97f53ddf6762831a61abcdb7f8273ae7323b386d`; the caller sites are bytes181–207,
219–249,259–289 (half-open), lines6,7,8. Removing one repeated debug assertion
changes the source OID to `634457b4d2eaab0af704e4f42014e6665e5cef64`; exactly two
sites remain. Actual caller ID `539417af3087aa65` is joined through graph symbols.
Full graph hash `5e329ccc62fcb53a99598ff96cf45a610ecc0bfcd210e1db34667123a956607c`;
incremental/full-rebuild oracle hash
`387fdb17d79c1d3dc0615e3f2eda77464c78b708ccb698754fd9b0e443c794cd`.

| Finite M6 projection | PRE | POST |
| --- | --- | --- |
| full three-site source/caller/identity set + | sol_90c20957649a4209 | sol_0193d0cba04d472d |
| incremental two-site set + | sol_a2b615388c1147a8 | sol_e32ec106316440b5 |
| deleted old site leakage − | sol_6474c4b196794a98 | sol_a01b5f3ea2dc4919 |
| unrelated caller − | sol_3ab427805b3645c1 | sol_cfb36206b66749d1 |
| stale source − | sol_3badf64201044518 | sol_02e5c0d9969c4e44 |
| missing row − | full exact cardinality PRE | sol_891a5b30e1ab4050 |

PRE uses exact source spans/OIDs and logical caller selector before graph
extraction. POST uses actual runtime rows and actual graph caller IDs against those
independent expectations. Rules: unique, foreign_key, cardinality, catalog family
v1; all11 receipts reloaded/pinned. Complete PRE and POST requests/results and
source facts: bd-2u9 audits585,586. Intent584; full-gate failure587 and typed
scope_drift signal `2e42d166-cb59-4a11-8c93-cd1a9369cb7d`.

The tests cover exact raw paths/ranges/context, aliases and lexical shadowing,
non-function const/type scope locations, raw attribute owners, full Parquet
roundtrip, caller queries, captured-empty versus unavailable, actual notebook
unsupported availability, preserved notebook function/call support, incremental
removal/rename/move and unchanged scope reuse, and full-oracle parity. The
row-group test rewrites both logical tables into groups of at most two rows,
observes9 invocation groups/11 scope groups, and demonstrates the three-site caller
occupies two real groups. Filtered results equal the full artifact oracle; later
queries still work after both evidence files are removed from disk because the
client freezes immutable bytes. This is a correctness check, not a speed budget.

The README SQL was executed with local DuckDB from exported full-artifact files:
manifest domain1 row, all evidence18 rows, scoped caller3 rows (exact site-ID and
caller-ID equality with the runtime oracle), attribute-owner locations22 rows.
The graph symbol projection is `nodes.parquet`, not a separate symbols table.

### Historical attempt1 bounded local cost observation

Single debug-build observation with the default three-test harness concurrency, no performance pass threshold: six physical Rust
files plus one notebook input,18 direct physical sites/22 scopes; after one-site
removal, deletion/rename/move and bucket reuse,17 sites/22 scopes. Compiles, fixture
setup, extra per-row source reads/hash checks, full-reader roundtrip/oracle checks,
SQL, export copying and row-group rewriting are outside extraction timers.
Artifact-construction validation and incremental rebuild include their own normal
production validation work. All times are wall-clock seconds, not a production
benchmark or an isolated statistical speed comparison.

| Operation | Seconds |
| --- | ---: |
| Full `build_facts` extraction | 0.113751375 |
| Full `artifact_from_facts` construction/validation | 0.074807000 |
| Incremental `artifact_from_facts_incremental` including normal validation | 0.110345875 |
| Fresh full extraction of changed tree | 0.105411709 |
| Changed-tree full artifact construction/validation | 0.056432584 |
| Full Parquet write | 0.083549750 |
| Incremental-result Parquet write | 0.031926042 |
| First filtered Parquet query, including complete context validation | 0.003337250 |
| Subsequent same filtered query and result equality check | 0.000667583 |

Full artifact file bytes: macro invocations9745, macro scopes4548; all artifact
files32510. Incremental artifact file bytes: macro invocations9705, macro
scopes4551; all files33143. Exact per-file sizes and raw timings are in
`/private/tmp/m6-bd-2u9/observed/{full,incremental}.json`, with complete payload in
bd-2u9 audit586. No optimization was proposed. The earlier review driver's
110.865812333s for60 files/4158 sites included per-row verification IO/hash work
and is neither a production benchmark nor a regression budget.

### Historical attempt1 runtime gate (superseded by rework FINAL below)

All commands use `SPUR_REMOTE=0 scripts/spur-cargo`, shared existing
`CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target`, and
`TMPDIR=/private/tmp`; `SPUR_GRAPH_BLESS` is removed, not set to0. Scoped sandbox
escalations permit the existing Cargo/Git lockfiles; no cache deletion or cloud
configuration change. M6 test revision is
`2bb8c9423d966a28ee35b77825bbee25fce2a6d7`; documentation edits do not change code.

- Held-out command: `env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp M6_EVIDENCE_DIR=/private/tmp/m6-bd-2u9/observed CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_end_to_end -- --nocapture`: exit0,3 passed/0 failed,0.83s. Log `/private/tmp/m6-bd-2u9/held-out-green.log`.
- Required full command: `env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph`: exit101; unit567 passed/4 failed/3 ignored. Log `/private/tmp/m6-bd-2u9/full-spur-graph.log`. The command stopped before integration binaries.
- The four failures are unchanged MCP fixture setup at `src/mcp/overlay_snapshot.rs:1660` and `src/mcp/mod.rs:8755,8774`, each `Illegal byte sequence` while creating a non-UTF-8 filesystem path. Names: `overlay_snapshot_fallback_discovery_failure_retries_once_then_returns_exact_valid`, `overlay_snapshot_repeated_fallback_discovery_failure_fails_closed`, `overlay_snapshot_supported_paths_reject_lossy_normalized_collision`, `overlay_snapshot_supported_paths_reject_non_utf8_relative_path`. Their production/test source bytes are unchanged from the reviewed M5 base. No test was skipped, blessed or weakened. Repair needs brain scope/replanning beyond the three M6 files.
- Full `--no-fail-fast`, complete extractor/incremental outcomes, dependent `--tests`, formatting and historical query-scaffold control are recorded in the final verification addendum below. M6 cannot claim completion while the mandatory full gate fails.

Limits remain the documented physical-file domain and conservative semantics,
first-context compressed-table residency, and the focused query/full-reader source
count distinction. No notebook edits, issue closure or merge occurred.


### Historical attempt1 verification addendum (not current acceptance)

| Gate on M6 code revision2bb8c9423 | Result | Full log under `/private/tmp/m6-bd-2u9/` |
| --- | --- | --- |
| Required `test -p spur-graph` | **FAIL**, exit101; unit567 pass/4 fail/3 ignored; stops before integration | `full-spur-graph.log` |
| `test -p spur-graph --no-fail-fast` | **FAIL**, exit101; all87 target summaries:1121 pass/6 fail/19 existing ignored;3 targets fail | `full-spur-graph-no-fail-fast.log`, machine summary `full-summary.json` |
| Complete extractor | **PASS**,54 pass/0 fail/3 existing ignored | same full log,2.13s |
| Complete integration_git_incremental | **PASS**,28 pass/0 fail/1 existing ignored | same full log,2.86s |
| Macro suites from full run | **PASS**, artifact8,contract11,e2e3,Parquet9,query14,collector7,token-query5 | same full log |
| Affected dependent `check -p spur-core -p spur-analyst -p spur-tui -p spur-cli -p spur-mentions --tests` | **FAIL**, exit101; analyst lib-test fixture E0063 | `dependent-check.log` |
| Same dependent command with `--keep-going` | **FAIL**, exit101; same two E0063 diagnostics, remaining affected targets checked, including CLI | `dependent-check-keep-going.log` |
| `fmt -p spur-graph -- --check` | **PASS**, exit0 | `fmt.log` |
| README DuckDB and exact query oracle | **PASS**, exit0; result sets1/18/3/22 | `readme.sql`, `readme-sql.json`, `readme-sql.err` |
| `git diff --check` | **PASS**, exit0 | recorded in final audit |
| Historical scaffold held-out caller control | **Expected behavioral FAIL**, exit101; Unavailable versus Complete | `historical-scaffold-red.log` |

Additional runtime failures revealed after the four unit failures:

- `markdown_reference_style_links_resolve_to_definition_destinations`,
  `tests/markdown_links_edges.rs:50`: expected `Links` label `guide.md` with its
  file node target;0 passed/1 failed.
- `structural_symbol_ids_match_temporal_snapshot_ids_for_fixture_corpora`,
  `tests/schema.rs:177`: missing temporal snapshot for
  `inline::Mode::Fast`, enum_variant stable ID `88715eb9664f575d`;
  6 passed/1 failed.

These were actual attempt1 final-run failures. They were not waived as baseline
failures. Full details and attempt1 extractor/incremental outcomes: bd-2u9
audit591. The authorized repairs and current final outcomes appear below.

The attempt1 affected-dependent compile failed at
`crates/spur-analyst/src/overlay/session_tests.rs:308,316`: its
`GraphArtifactManifest` fixture lacks `macro_evidence`, and
`GraphArtifactRowCounts` lacks `macro_invocations` and `macro_scopes`.
The M3 historical dependent GREEN preceded M4's manifest extension and does not
satisfy this final gate. Audit592 and scope_drift signal
`5b7a6014-c0bc-4c34-8ff7-2ff7f0b422f7` request the required unlisted-file repair.
Attempt1 made no such repair; attempt2 is explicitly authorized to complete the fixture.

Historical control: archive exact commit
`8f51f2403ac6400090a4a3e3bc1523bf0bfb3fca` into
`/private/tmp/m6-bd-2u9/historical`, copy only the M6 held-out test, then run:

```sh
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo --workdir /private/tmp/m6-bd-2u9/historical test -p spur-graph --test macro_invocation_end_to_end held_out_caller_sites_scopes_and_unsupported_cells_round_trip -- --exact --nocapture
```

It compiled the historical spur-graph package and failed the actual assertion at
M6 test line186, `Unavailable != Complete`,0 passed/1 failed. This is an immutable
pre-query-implementation control, **not a failure on the reviewed M5 final tree**.
The first archive attempts reused the shared target's current-tree binary because
archive source mtimes preceded build outputs; those setup runs are preserved as
`historical-workdir-setup.log` and `historical-cache-setup.log`, not counted as
historical evidence. Refreshing only the archived query source mtime forced the
actual historical compile; no production bytes were disabled or changed.

At the attempt1 handoff, M6 was **not complete**: mandatory runtime and
dependent gates were red. Brain scope mutation
`6ceecfdf-a59f-4c29-82a8-d3d171c4fc58` subsequently authorized exactly ten
paths and the four repairs below; both scope signals were consumed. No gate
was waived. The prior tests, documentation and authored history are preserved.


Shared-target restoration note: the first current-tree rerun immediately after
historical compilation also reused the stale historical binary (two failures,
`current-after-historical.log`), so it is not a current-source regression verdict.
Refreshing the current query source mtime forces the current compile; its Git blob
OID remains `a5239a9e6e54e90f48ae39ada1823f4b2b463433` before and after, with no
source diff. This is build-cache bookkeeping only; no production bytes changed.
The authoritative full-crate/dependent logs above predate historical compilation
and compiled the current reviewed source. Final current-cache restoration is
recorded in the handoff audit.


Final cache-restored current-tree command compiled spur-graph from the active
worktree and passed3/3 again (exit0,0.90s):
`env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_end_to_end`.
Log `/private/tmp/m6-bd-2u9/current-restored-green.log`. This confirms the final
active source implementation rather than a historical shared-target binary.


### FINAL — M6 attempt2 bounded repairs and observed acceptance

All four authorized repairs are implemented. The complete complete test gate passes
with **`RUST_TEST_THREADS=1`: 1128 passed, 0 failed, 19 existing ignores, 87 target
summaries**. M6 remains `in_progress` for brain review. The default-parallel
failures below remain a real reproducibility risk; no production timeout or
snapshot policy was changed to address them. No case was excluded or waived.

Authority is scope mutation `6ceecfdf-a59f-4c29-82a8-d3d171c4fc58` on execution
issue **bd-2u9**, plan `6888d1b0-fee6-47eb-8cc7-c8f18d8ca570`, epic **bd-156**.
Source template bd-3vi was never assigned. The two earlier scope signals were
consumed; the new risk signal is `ef8b00d4-d4fc-43c5-8054-2f84ed98bc3d`.
Audits 607,609–619,622–624 record intent/action/outcome, exact requests and logs.
Nine of the ten authorized files changed; `extract/languages.rs` is unchanged.

| Sequential repair | Cause and bounded result | Verification / authored commits |
| --- | --- | --- |
| 1. Portable path/fallback tests | macOS rejected physical invalid filenames before assertions. Strict collector now receives invalid and explicitly lossy-colliding OS paths directly. Existing discovery-error injection replaces redundant invalid-file writes. Valid discovery, rejection, exact calls/retry/lifecycle, once-recovery and repeated fail-closed assertions remain. Only test code/imports changed. | Snapshot 30/0; `dc3490c610bf3c019911bebf87bca8679526fff6`. Original setup errors are not behavioral RED. |
| 2. Markdown policy | July4 test expected file binding despite Aug30 `2709b4187a7cf2f88a6d61a32731986ba4e9f9ff` deliberately keeping Links unresolved. Four independent forms verify normalized direct/full/collapsed/shortcut destinations, missing-label evidence, URL/same-name file/symbol nonbinding and no Imports. No resolver/golden change. | Parameterized test 1/0, four cases; `4d940767dfe8e8a8b5deb151bc84519811b583a8`. No fabricated production RED. |
| 3. Temporal projection | Two query-local `(pattern=1,match=2)` captures collide: symbols `App` name becomes edge-query variant label. Structural emission already groups queries separately. Temporal projection now offsets edge pattern keys by symbols-query pattern count before merging; ordering/scoping and structural IDs/ranges stay intact. | Genuine missing `inline::Mode::Fast` assertion RED, then complete schema 8/0. Regression `e3077e36f7eb67fdad99ac29854a7f92260f4c5f`; fix `d5cfb89eb370fe62e7f79ff4bdd0afaad9b0921e`. |
| 4. Analyst legacy fixture | Only `macro_evidence`, `macro_invocations`, `macro_scopes` initialized to None. | Affected dependent `--tests` exit0; `1d764ad363ab8d6b139bdb48e19118105fb82677`. E0063 was compatibility/setup evidence. |

The prior authored test/docs commits remain ancestors:
`2bb8c9423d966a28ee35b77825bbee25fce2a6d7` →
`09b907c791fc5b070111f167c5243c456ebe7361`. The prior normalized
`dbc83f89e75c57ccd19011f6668659245d147e21` is tree-identical to the latter
(tree `c8de8d79fcd935f416f5484e24d70744316429ad`).
Final tested code/timing commit is **`24fbe684ebb0b56858711d3e12636cb09f9d8c6b`**. The timing commit preserves every
prior assertion and separately measures the existing scope verification work.
The final documentation commit and exact file/log hashes are exported in
`/private/tmp/m6-bd-2u9/rework/file-bindings.json` at handoff. A normalized rework
OID has not been issued; normalization/integration belongs to the brain after
approval. The M1–M5 authored/normalized OIDs and execution IDs above remain valid.

#### Final source bindings and regression interpretation

The focused Rust fixture is blob **`37fa84bede602896293e4ea2253a2e4955300706`**,
`src/lib.rs`: module `inline` bytes[37,91], enum `Mode` [54,89], variant `Fast`
[78,82]. Structural and temporal IDs match: `c7e33b75e565d730`,
`51e4143c6a874467`, **`88715eb9664f575d`**. Temporal variant parent remains
`inline::Mode`; structural parent metadata stores its immediate name `Mode`.
The unchanged corpus test still requires a matching snapshot and identical ID
for every structural symbol, including variants. Genuine RED shows temporal
`App` at[78,82] with no matching `Fast`; the fix restores its correct name.
The initial regression assertion incorrectly expected a qualified structural
parent; that oracle setup run is preserved separately and is not behavioral RED.

| Tested file | Git blob OID |
| --- | --- |
| `crates/spur-graph/tests/macro_invocation_end_to_end.rs` | `81e859158ce92b23668debc48ba07db63fa2790f` |
| `crates/spur-graph/src/mcp/mod.rs` | `1886dc0617dc99b407a58813cca339a4791de341` |
| `crates/spur-graph/src/mcp/overlay_snapshot.rs` | `a8fab533956b953b1bddafb58a799e7594b44d10` |
| `crates/spur-graph/tests/markdown_links_edges.rs` | `32f6bac5a191b301bec6a0a5a0e836c7b4789e77` |
| `crates/spur-graph/src/extract/tree_sitter.rs` | `e785ddb9fee1e3dbdeee27a4f1bc177cb57956c1` |
| `crates/spur-graph/tests/schema.rs` | `63ec0cd235249b18a12bd331eb433dfa447bfbb0` |
| `crates/spur-analyst/src/overlay/session_tests.rs` | `f0cf6466ff39e3753e105cd026e5ee3d494ffd63` |

`extract/languages.rs` remains `dc74fdf4bb27039ccafd8811cd8317cfc364abad`.
No macro query/context/physical-owner/Calls/HOF policy, Links resolver or
structural emission was changed.

#### Fresh repair solver matrix

`+` = sat/pass; `−` = expected unsat/fail. All 19 current repair receipts and
all 11 retained original M6 receipts were reloaded/pinned. Original family
requests/results are in audits610,612,614,615,624 and
`rework/repair-solver-receipts.json`; reloads expose the lowered solver request.
These are finite snapshot/trace checks, not global parser, filesystem, IO,
atomicity, hashing, runtime execution or panic-freedom proofs.

| Finite repair projection | PRE | POST |
| --- | --- | --- |
| Once-recovery + repeated-failure traces + | sol_2ee7a3fdf4584cee | sol_54577d7a47f9493e |
| Repeated error incorrectly returns Valid − | sol_bca6093dbf444d95 | sol_6f2bbd58937345a8 |
| Four Markdown destination forms + | sol_212d9b80048242d7 | sol_b0eaa340660e4e00 |
| Missing label / same-name symbol / URL + | sol_c8c21271de924644 | sol_bb08a38401374505 |
| Phantom file binding − | sol_da7ea8245b59470a | sol_3daa5c4a17b74332 |
| Exact module/enum/variant source/span/name/parent + | sol_cf9ef08afd184e0f | sol_d117d67f78644b43 |
| Variant omission − | sol_2c109a2c0dbc4b5b | sol_74714606e64040c1 |
| Variant mislabel − | sol_673aa18158fd4d1a | sol_6bcc647626214b4a |
| Actual structural/temporal stable ID parity + | Source/span contract above; IDs observed before fix | sol_b6867843795c43d7 |
| Wrong parent − | Exact parent contract above | sol_a5f3004f812b4f81 |
| Wrong stable ID − | Exact matching requirement above | sol_eb6c42a208dd4252 |

Rules are catalog `workflow.transition_allowed` (two normalized discovery events)
and `data_integrity.unique`, `foreign_key`, `cardinality`. Rejection/lossy-collision
of in-memory OS paths is independently exercised by the real collector tests.
The observed retry calls remain `(0,BeforeDiscovery),(1,BeforeDiscovery),
(1,BeforeRediscover)` with exactly one retry and Cold→Validating→ExactFallback→Valid;
repeated failure has exactly two discovery calls, no valid response/cache entry.

PRE totals: 4 pass/4 expected reject; POST: 5 pass/6 expected reject. Three initial
oversized family requests failed validation (>64 solver variables), with no
solver verdict. The initial parser metadata receipts `sol_d40a57eb20db40b2`,
`sol_1aab5ea2e8024f4b`, `sol_146935d288c047fe` used incorrect guessed parent
ranges; they were superseded by the source-corrected PRE above before authoring
the regression. They are retained as setup history, never source-bound proof.

Original M6 PRE 2 pass/3 reject and POST 2 pass/4 reject above are retained only
after final recapture: full/incremental source strings, selected results
(including graph/snapshot/caller/scopes) and complete macro-evidence payloads
are exactly equal to attempt1. Full source `97f53ddf6762831a61abcdb7f8273ae7323b386d`,
changed source `634457b4d2eaab0af704e4f42014e6665e5cef64`, caller
`539417af3087aa65`, graph hashes and three/two exact sites remain unchanged.

#### FINAL runtime, SQL and formatting matrix

Common compile/test environment: blessing unset, TMPDIR=/private/tmp,
CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target, SPUR_REMOTE=0.
Current worktree sources were compiled; historical archives were not rerun.
The complete passing run adds only per-command RUST_TEST_THREADS=1 and uses
`--no-fail-fast`; all 87 targets and all previously failing cases execute.

| Gate | Final observed outcome | Log under `/private/tmp/m6-bd-2u9/rework/` |
| --- | --- | --- |
| Complete graph `test -p spur-graph --no-fail-fast`, RUST_TEST_THREADS=1 | **PASS**, exit0;1128/0/19 existing ignored,87 summaries | `final-serial-spur-graph.log`, `final-full-summary.json` |
| Complete extractor | **PASS**,54/0/3 existing ignored,8.57s | same full log |
| Complete integration_git_incremental | **PASS**,28/0/1 existing ignored,10.70s | same full log |
| Held-out acceptance / schema parity / Markdown policy | **PASS**,3/0 (1.68s),8/0 (0.88s),1/0 with four forms (0.26s) | same full log |
| Affected dependent `check -p spur-core -p spur-analyst -p spur-tui -p spur-cli -p spur-mentions --tests` | **PASS**, exit0,54.76s | `final-dependent-check.log` |
| `check -p spur-graph --all-targets` | **PASS**, exit0,17.31s | `final-graph-all-targets.log` |
| Wrapper `fmt -p spur-graph -p spur-analyst -- --check` and `git diff --check` | **PASS**, exit0 | `final-fmt.log`, `final-diff-check.log` |
| Executed README DuckDB plus independent exact oracle | **PASS**, result sets1/18/3/22; all sites, selected caller IDs and attribute-owner scope locations exact | `final-serial-readme-sql.json`, `final-sql-oracle.json` |

Exact commands (existing cache environment as above):

```sh
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp RUST_TEST_THREADS=1 M6_EVIDENCE_DIR=/private/tmp/m6-bd-2u9/rework/final-observed CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --no-fail-fast
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo check -p spur-core -p spur-analyst -p spur-tui -p spur-cli -p spur-mentions --tests
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph --all-targets
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -p spur-analyst -- --check
git diff --check
# cwd: /private/tmp/m6-bd-2u9/rework/final-observed/parquet
duckdb -json < /private/tmp/m6-bd-2u9/readme.sql
python3 /private/tmp/m6-bd-2u9/rework/verify_sql.py /private/tmp/m6-bd-2u9/rework/final-observed /private/tmp/m6-bd-2u9/rework/final-serial-readme-sql.json
```

Schema discovery ran against generated macro_sites/macro_scopes/graph_symbols
views before the final joins (`sql-schema.json`). One initial SQL invocation
used the repository cwd and failed to find the relative files; its `readme-sql.*`
setup outputs are retained, not counted as a data or behavioral verdict.

Earlier attempt2 default-parallel runs remain explicitly historical failures:
`final-spur-graph.log` unit569/2/3ignored; `final-spur-graph-no-fail-fast.log`
full1123/5/19ignored. They involve seven unchanged metadata/rebuild-sensitive
cases; all seven pass in the final complete run. Symptoms include
`stale_budget_exceeded`, absent Git HEAD metadata, missing fresh target and
not-found dirty symbols. Existing probes use 200ms and refreshes have a 750ms
default budget. Contention is consistent with the observations, not proven as
the sole cause. No test body, timeout, budget or production fallback was changed
for these failures; the process schedule is stated openly. This does not claim
default-parallel robustness is repaired. Full logs and risk signal remain for
brain review. The original attempt1 ledger 1121/6/19 and dependent E0063 logs
remain above and on disk, explicitly superseded by the actual repair results.

#### Final bounded cost observation and retained limits

One debug-build observation with a single harness thread, six physical Rust
files plus one notebook input: 18 sites/22 scopes →17 sites/22 scopes. Times below
are seconds. Production artifact validation remains separate from extraction;
normal incremental validation remains inside its incremental timer. Extra
`assert_scopes` source rereads, hashes and identity/context assertions are timed
separately (incremental value sums the seven filter checks). Query time is outside
that verification timer. Other oracle/roundtrip assertions, SQL, export copying,
row-group rewriting and compiles/fixture setup are outside extraction timings.
No speed threshold or optimization claim is made.

| Operation | Seconds |
| --- | ---: |
| Full build_facts extraction | 0.125642375 |
| Full artifact construction/validation | 0.192150041 |
| Incremental construction including normal validation | 0.131616291 |
| Changed-tree full extraction | 0.116278375 |
| Changed-tree full construction/validation | 0.064830000 |
| Full Parquet write | 0.042833209 |
| Incremental-result Parquet write | 0.042909542 |
| First filtered query including full context validation | 0.003480750 |
| Subsequent filtered query plus equality | 0.000695084 |
| Extra full scope/source verification IO/hash/assertions | 0.000768208 |
| Extra incremental scope/source verification IO/hash/assertions | 0.001014875 |

Final full macro table sizes 9745+4548 bytes; all artifact files 32510.
Incremental 9705+4551; all files 33143. Physical row-group oracle remains
9 invocation groups/11 scope groups, 2 rows/group, selected caller spans 2 groups.
Raw exports are `rework/final-observed/{full,incremental,row_groups}.json` and
`parquet/`. Audit624 binds these actual costs and source/scope/caller facts.

First-context validation still freezes/checks both complete compressed tables,
drops full decoded evidence and retains compressed bytes/source context. The
focused supporting-count check still does not repair legacy full-reader count
integrity. Physical-file-only macro coverage, UnsupportedSource for notebook
cells with ordinary definitions/calls preserved, aliases/local shadowing/build
context conservatism, no expansion/namespace/cfg/runtime/panic conclusions, and
the default-parallel reproducibility risk remain real limits. No notebook edit,
cache deletion, cloud/config change, merge or issue closure occurred.


## Final brain integration review

All M1–M6 execution issues were approved after their recorded source-bound solver,
TDD and runtime evidence was reviewed. The dedicated plan integration branch is
`spur/plan-merge-6888d1b0-fee6-47eb-8cc7-c8f18d8ca570-4582e1e013fb42a1a238fb67d6258d52`;
its tree equals reviewed M6 `02607f5479de23e2a05dc876bb9104167fd131f5` and authored
`d9c6321559513d287ff0638a53ca54ef0280ddcf`. Main integration applies the six reviewed
task deltas, preserving the five unrelated dirty user files and root notebook
review records rather than importing the original dirty-base snapshot.

Independent complete gate on that exact code, with default test parallelism
(`RUST_TEST_THREADS` unset) and ordinary loopback mock-server permission:

```sh
/usr/bin/time -l env -u SPUR_GRAPH_BLESS -u RUST_TEST_THREADS TMPDIR=/private/tmp SPUR_REMOTE=0 CARGO_TARGET_DIR=/Users/kevintruong/project/SpurCode/target scripts/spur-cargo test -p spur-graph --no-fail-fast
```

Result: **exit 0; 1,128 passed / 0 failed / 19 existing ignored**, all 87 target
summaries. Schema includes the new temporal capture regression. Log
`/private/tmp/m6-brain-default-full-loopback.log`, SHA256
`fba8833e6827538685ec9a987cee8929df8ba83468fa6608c89dff306db75350`.
Elapsed 235.30 s includes 49.41 s reported compilation; user CPU335.86 s and
system CPU121.73 s are parallel process totals, not a serial on/off-CPU split.
This is regression verification, not an extraction performance benchmark.

The first independent sandboxed run completed all targets with five EPERM
failures only at loopback TcpListener fixtures. It is not a source regression
verdict; the same code passes when those local mocks can bind. Earlier worker
default-parallel metadata/rebuild failures remain observations with unknown cause.
The independent default-parallel pass satisfies the gate without skips, timeout
changes or production fallback changes; it does not prove those load-sensitive
failures were repaired. Worker dependent test compilation, graph all-targets,
both-crate formatting, exact SQL1/18/3/22 and artifact/source/hash checks also pass.

The production correction in M6 offsets query-local capture pattern keys before
merging temporal projections; structural IDs remain unchanged. Test-only path
fixtures, the established unresolved Links contract and three legacy optional
manifest initializers account for the other bounded repairs. Current 19 repair
receipts plus 11 retained byte-identical macro receipts were reloaded and reviewed;
setup errors and superseded fixture metadata are excluded from proof counts.

Scope remains direct physical Rust syntax evidence with explicit availability,
source/scopes/caller provenance and lossless artifact/Parquet/overlay transport.
No macro expansion, namespace/cfg evaluation or panic-execution/freedom proof is
claimed. The final notebook records verified source recapture and these limits
through notebook MCP.

### Main commits and final source experiment

The reviewed task deltas are committed on active main as M1 `1cd740e85`,
M2 `44bfced71`, M3 `caae9e630`, M4 `6328f3c52`, M5 `eb85ed874`,
and M6 `51f7458c6`. All 62 non-document feature files match the tested tree;
the five unrelated dirty user files remain byte-identical and unstaged.
Authored test/fix histories remain reachable through local
`spur/macro-evidence/m1-authored` through `m6-authored` branches.

Notebook MCP executed a new experiment over a byte-identical mirror of all 63
current physical Rust source files under `crates/spur-graph/src`, preserving
repository-relative paths and recording origin commit
`51f7458c6d898a17544046e4273128dd8759f400`. Artifact snapshot
`37ef5600b79894ce8d016e8c6eb35dbfc5b62f051516e3754861702d1da2f014`
contains 4,256 invocation sites and 1,636 scopes; 58 sites have no function
caller, preserving their non-function contexts. The six originally missing direct candidates are all returned
by the exact caller join. Eight SQL identity/reference checks return zero
violations, and all 63 source SHA256/blob OIDs match the origin manifest.

The selected debug source run separates extraction (105.690547333s),
artifact construction/validation (78.882536291s), Parquet write (0.487259417s),
first filtered query/context (0.276618959s) and repeated query/equality (0.005686292s).
Macro tables occupy 335,674 bytes. These are single bounded observations, not a
production benchmark or a comparison with earlier mixed verification timings.

Finite post-integration verification of the six sites/four callers/two source
identities passes `sol_3334f66c731e402e` (60 variables). Expected rejecting
controls: wrong caller `sol_6cc5218dda1d4dc1`, stale source
`sol_9a35f31ed4c84345`, omission `sol_1f23cab3b6984534`, duplicate
`sol_485cf5209fb8495b`. All five receipts were reloaded/pinned; these facts do not
prove parser correctness, macro expansion, cfg inclusion or panic execution.

The notebook and ignored experiment directory retain query outputs, origin
manifest, producer source/log and exact solver requests/receipts. Artifact:
`.spur/graph/experiments/macro-invocations-final-20261031-51f7458c6.parquet`.
The serving index and earlier notebook research cells are separate snapshots;
the source changed from 60 to 63 files, so total counts are not a paired recall
metric. The next semantic comparison requires a pinned Rust build configuration
and compiler-derived evidence; CGA/MIR was not run in this phase.
