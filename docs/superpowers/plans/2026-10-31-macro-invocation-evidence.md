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

**Architecture:** Add a separate typed evidence collection to GraphFacts and a versioned optional evidence set to GraphIndexArtifact. Store sites in macro_invocations.parquet, bound to snapshot/source provenance, and query by file or caller without fabricating runtime Calls edges. Preserve legacy-unavailable versus captured-empty, unknown resolution and build conditions.

**Tech Stack:** Rust 2021, tree-sitter Rust queries, serde, Arrow/Parquet, existing GraphQueryClient and SPUR solver/catalog MCP.

**User approval and routing:** User authorized implementation with TDD and SOLVE PRE then POST task by task using SPUR worker `codex`, model `gpt-6.1-sol`, effort `xhigh`. All tasks form a sequential chain M1 -> M2 -> M3 -> M4 -> M5 -> M6 because each consumes the preceding contract/artifact and several share files. Brain reviews each task before the next dispatch. No extra approval is needed to start this approved work.

**Build-mode update:** The user requested local builds. All six execution task bodies were updated atomically by plan mutation `ed862b95-9d08-4220-b1ed-b9c199cd6a28`; dependencies and codex / gpt-6.1-sol / xhigh remain unchanged. This supersedes the original remote-only constraint.

**Phase boundary:** Macro namespace alias/re-export resolution, actual cfg evaluation, expansion and CGA/MIR are later phases. Direct sites remain syntax evidence with execution uncertainty.

## Shared task requirements

CONTEXT: User explicitly approved phase-one direct Rust macro evidence from notebook 2026-10-31-panic-graph-experiment.ipynb, cell 5cc809ce-a082-47f9-9f82-17146b946f25, baseline 0a03bf8e8. Agent routing is user-selected codex; model gpt-6.1-sol, effort xhigh, unchanged. Brain owns architecture/review, workers implement one bounded task at a time. Research: existing query misses 29 direct fixture invocations; six real candidate sites in the 270-symbol build call slice are missing. Existing macro_call captures mean call-shaped tokens INSIDE macro arguments and must be preserved.

MANDATORY WORKFLOW: Read applicable foundation skills and discover test-driven-development through skill_navigate/skill_read. Before implementation navigate solve_rule_spec; data_integrity may cover finite identity/reference/cardinality checks. Run meaningful SOLVE PRE (persist:true; verify positive and rejecting fixtures / counterexample as appropriate), write tests, observe genuine RED, commit test(scope), implement minimum GREEN, run tests, run SOLVE POST against the exact landed contract/observed fixture facts, persist receipts, commit feat/fix(scope). Use generic typed solver only after catalog mismatch plus solve_constraint_spec/check; no hand-duplicated catalog rules. Solver checks establish the encoded finite contract, not Rust execution or global panic freedom. If any MCP is absent, emit blocked signal; do not invent results or bypass the requested gate.

BUILD: User explicitly requested local builds after the initial remote-only plan. Use SPUR_REMOTE=0 scripts/spur-cargo test/check/build; never bare cargo. scripts/spur-cargo fmt is local. The existing macro-query baseline passed locally (4 tests, exit 0, build 43.18s). Compiler/setup failures are not behavioral RED/GREEN. Keep the same solver, TDD, commit and review gates. Preserve unrelated dirty changes, stay in worktree, commit only task files, never merge or close issues. Record commands, exact failing/passing assertions, commit OIDs, source bindings, selected rules and solve IDs in issue audit. Emit scope_drift BEFORE needing unlisted files; do not silently broaden.

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
TEST: SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_artifacts --test integration_git_incremental; relevant canonical_hash and store::build tests; check dependent crates after mechanical updates.

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

## Review and completion

M3 attempt 1 made no changes because it selected source template `bd-shq` rather than its assigned execution issue `bd-1k5`. The latter was correctly in progress; worker audit/signal tools correctly rejected the template. Mutation `b87390e6-7aa7-4463-80d3-80bb5f49acac` prefixes the actual execution identity for M3–M6 and removes stale automatic context paths. The retry must target the execution issue; no rebind or source-template activation is required.

M1 attempt 2 produced the typed model and contract tests but failed compilation at the additional memory-evaluation GraphFacts initializer. Review accepted that same-crate mechanical scope addition in mutation `181d774c-216a-4aab-a303-d2e8ba13ed94` and requested changes with prior work preserved. Review also independently found incorrect fixture line annotations: the two debug assertions are on lines 3 and 4, the register invocation on line 6, the function covers lines 2–5 and the file covers lines 1–6. Earlier PRE receipts (`sol_464b1e08c2c44f3f`, `sol_c6fbad0d1cc0486c`; negative controls `sol_b3b6d03c0d374e62`, `sol_03b385b54c424b49`) verify only their supplied finite facts, which included those inaccurate line values. The retry must correct source-bound facts, refresh PRE, exercise a runtime regression rejecting FunctionBodySyntax on a file-scope row without a caller, complete local GREEN and persist POST receipts before approval. No M1 success is claimed from the earlier compile-time API absence or E0063 setup failure.

Review beads status/audit/signals before each diff. Independently inspect actual test logs and solver requests/results; do not infer runtime safety from a solver model. Use review_task(approve) only after both gates pass. Missing infrastructure or MCP capabilities require a recorded blocker and recovery, never silently skipped checks. Integrate approved committed work, preserve unrelated working-tree changes, append verified outcomes to the research notebook via notebook MCP, and close the implementation epic only after all acceptance criteria are met.
