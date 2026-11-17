# Notebook Cell Symbol Uplift Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-11-17-notebook-cell-symbol-uplift.md`
**Formal @spec cells (if notebook):** none; accepted evidence is the executed Rust PoC bd-26sp.
**Design epic:** `bd-21na` (closed)
**Implementation epic:** `bd-fquv`
**Goal:** Native cell-aware notebook symbol extraction/review with original-source coordinates and truthful coverage.
**Architecture:** Shared core normalization/preparation feeds graph and symbol extraction. A cell-aware comparator retains typed before/after refs and structural/config events; additive DuckDB functions feed coherent consumer exports. Dependencies serialize overlapping core/adapter edits.
**Tech Stack:** Rust, existing tree-sitter/core matchers, DuckDB 2.0 C_STRUCT_UNSTABLE bindings, Python native acceptance, typed Parquet.
**Authorization:** User approved implementation with TDD. Proceed to orchestrator without an additional approval prompt.
**Routing:** codex for all bounded tasks; existing claude-code dispatch failed authentication in this thread. Override codex's multi-file avoid_for because contracts and small file boundaries are fixed, dependencies are serial and root reviews every gate.

---

## U1: Share normalized notebook cells and Rust preparation

**Issue:** `bd-3ukt`
**Task ID:** `U1`
**Depends on:** none
**Suggested Worker:** codex

**Files:**
- `crates/spur-graph/src/extract/notebook_model.rs`
- `crates/spur-graph/src/extract/notebook.rs`
- `crates/spur-graph/src/extract/mod.rs`

**Acceptance and scope:**

CONTEXT: Existing notebook.rs owns IDs/language/source decoding and extract_cell; graph Rust :dep source currently hides valid functions. GOAL: Add public extract::notebook_model normalized cell/prepared-source model and refactor graph extraction to consume it without changing existing valid graph stable keys. Export normalize_notebook and typed models sufficient for U2. Preserve existing language registry and ID rules; use original decoded source as authority, analysis bytes only for parsing. Rust preparation leading single-line :dep mask must preserve UTF8 byte/CRLF coordinates and record original+analysis hashes/directives/policy. Diagnose unsupported forms (including non-leading directives outside strings/comments) conservatively and preserve inventory. Do not read files named by :load or execute commands. Existing GraphFacts schema unchanged; new normalization returns typed diagnostics for U2. Invalid/duplicate effective IDs/source shapes explicit. Tests embedded in new module or notebook.rs only. ACCEPTANCE: meaningful RED/GREEN for IDs/language precedence/source encodings, empty/unknown/invalid/duplicate/anonymous cells, root/cell metadata, graph stable-ID compatibility, :dep function recovered graph path, multibyte+CRLF offsets, directive-looking raw/ordinary strings/comments untouched, unsupported command/multiline/trailing forms diagnosed. Run focused no-default-features spur-graph lib tests using wrapper; committed source doesn't depend on notebook runtime.

**TDD sequence:**

1. Add the task-specific regression using the cases below and existing test harness conventions; commit the failing test separately.
2. Run the focused test with `scripts/spur-cargo` (Rust remote, no local fallback) or native DuckDB2 Python as appropriate. Capture the expected assertion failure; a build/setup failure is not meaningful RED.
3. Implement the smallest scoped change; run the identical test to GREEN, then relevant existing regressions once.
4. Record exact commands/exits and source/native identities, commit the implementation, and return the final interface. Root reviews before the next dependency dispatches.

```rust
let source = br#"{"metadata":{},"cells":[{"id":"rust","cell_type":"code","metadata":{"spur":{"code_type":"rust"}},"source":":dep sha2 = \\"0.10\\"\\nfn f() {}\\n"}]}"#;
// Shared normalization is the public seam; assert original bytes and mapped analysis offsets.
let result = normalize_notebook(std::path::Path::new("n.ipynb"), source).unwrap();
assert_eq!(result.cells.len(), 1);
```

## U2: Add typed cell symbols and conservative notebook comparison

**Issue:** `bd-3o5l`
**Task ID:** `U2`
**Depends on:** `U1`
**Suggested Worker:** codex

**Files:**
- `crates/spur-graph/src/extract/notebook_symbols.rs`
- `crates/spur-graph/src/extract/mod.rs`
- `crates/spur-graph/src/git_walk.rs`

**Acceptance and scope:**

CONTEXT: Approved U1 exports normalized notebook model; inspect actual approved signatures first. Legacy BytesExtractor.extract(.ipynb) is intentionally not a flattened-symbol API. GOAL: Public extract::notebook_symbols::extract_notebook_symbols and compare_notebook_sources with typed document/cells/symbols/SourceRefs/statuses/diagnostics/changes; reuse shared normalization/preparation. Add narrow explicit-language inner ordinary matcher in git_walk; leave ordinary wrapper behavior compatible. Canonical IDs use notebook#cell identity paths, no synthetic .py/.rs paths. Parent/cell metadata/source/original+analysis hashes/prep/directive facts retained for U3. Correct original-source before/after refs on changes; snapshot labels added externally by consumer. Preserve cell/document/dependency events independently of symbol count; unknown/ambiguous identities give partial diagnostics, never silent HashMap loss. Handle M/A/D and safe rename; explicitly reject unsupported cases. ACCEPTANCE: failing-first real frozen app notebooks (real-inputs.json contains exact raw source) produce 46/24 symbols and17/11 functions with names/IDs/hashes/spans, 28 anchor checks; duplicate helpers across cells unchanged ->0; same-cell collisions partial; add/delete/import-only/output/reorder/type/language/profile/rootmetadata/dependency changes; anonymous edits conservative; malformed/missing sides errors. Bound matching logic and attach exact refs, no semantic equivalence claims. Reuse pre/post catalog uniqueness/SourceRef checks with finite-model limits stated. Return exact public Rust signatures/schema to downstream U3. Do not modify SQL crate.

**TDD sequence:**

1. Add the task-specific regression using the cases below and existing test harness conventions; commit the failing test separately.
2. Run the focused test with `scripts/spur-cargo` (Rust remote, no local fallback) or native DuckDB2 Python as appropriate. Capture the expected assertion failure; a build/setup failure is not meaningful RED.
3. Implement the smallest scoped change; run the identical test to GREEN, then relevant existing regressions once.
4. Record exact commands/exits and source/native identities, commit the implementation, and return the final interface. Root reviews before the next dependency dispatches.

```rust
let left = br#"{"cells":[{"id":"a","cell_type":"code","metadata":{"spur":{"code_type":"python"}},"source":"def helper():\\n    return 1\\n"},{"id":"b","cell_type":"code","metadata":{"spur":{"code_type":"python"}},"source":"def helper():\\n    return 2\\n"}]}"#;
let result = compare_notebook_sources(std::path::Path::new("n.ipynb"), None, "M", Some(left), Some(left)).unwrap();
assert!(result.changes.is_empty());
```

## U3: Expose typed notebook SQL functions and guard legacy notebook calls

**Issue:** `bd-1jtd`
**Task ID:** `U3`
**Depends on:** `U2`
**Suggested Worker:** codex

**Files:**
- `crates/spur-graph-duckdb/src/notebook.rs`
- `crates/spur-graph-duckdb/src/lib.rs`
- `crates/spur-graph-duckdb/tests/test_notebook.py`
- `crates/spur-graph-duckdb/tests/test_contract.py`
- `crates/spur-graph-duckdb/build.rs`

**Acceptance and scope:**

CONTEXT: Read approved U2 public types and functions, reuse them directly. GOAL: Add registered typed STRUCT/LIST spur_graph_notebook_symbols(path,source) and spur_graph_notebook_changes(path,old_path,kind,left_source,right_source), each contract_version1 and explicit document/cell coverage diagnostics. Preserve metadata as metadata_json only; other facts/statuses/refs/deltas typed. Match spec NULL and absence semantics; infrastructure failures propagate. Legacy spur_graph_symbols / spur_graph_symbol_changes for .ipynb must fail with actionable new-API message; keep exact v1 ordinary schemas and ordinary results. Do not change exact spur_graph_info STRUCT; native discovery+function docs expose new capabilities. Ensure extractor provenance includes every new core/adapter file and preparation policy (build.rs only if current recursive source inputs fail to cover new files). ACCEPTANCE: SQL native missing-function/behavior RED before implementation then GREEN; DESCRIBE exact schemas, real source17/11 functions, duplicate-helper no-op and canonical refs; NULL/M/A/D/rename/unsupported/error cases; multi-row varied-size nested lists with null/empty/large cells; registry discoverability; old contract/regressions. Use existing package build/test helpers, record true native binary identity; no old binary as GREEN. Only scoped files; signal if registration/profile tests need additional paths. Report final SQL schema and runnable commands for U4/U5.

**TDD sequence:**

1. Add the task-specific regression using the cases below and existing test harness conventions; commit the failing test separately.
2. Run the focused test with `scripts/spur-cargo` (Rust remote, no local fallback) or native DuckDB2 Python as appropriate. Capture the expected assertion failure; a build/setup failure is not meaningful RED.
3. Implement the smallest scoped change; run the identical test to GREEN, then relevant existing regressions once.
4. Record exact commands/exits and source/native identities, commit the implementation, and return the final interface. Root reviews before the next dependency dispatches.

```python
result = con.execute("""SELECT spur_graph_notebook_symbols('n.ipynb', ?)""", [notebook_source]).fetchone()[0]
assert result["contract_version"] == 1
assert sum(len(cell["symbols"]) for cell in result["cells"]) > 0
assert "spur_graph_notebook_changes" in {r[0] for r in con.execute("SELECT function_name FROM duckdb_functions()").fetchall()}
```

## U4: Route notebook consumer and preserve typed cell evidence

**Issue:** `bd-1l8z`
**Task ID:** `U4`
**Depends on:** `U3`
**Suggested Worker:** codex

**Files:**
- `crates/spur-graph-duckdb/examples/app_consumer.py`
- `crates/spur-graph-duckdb/tests/test_app_consumer.py`
- `crates/spur-graph-duckdb/tests/fixtures/app_consumer.json`

**Acceptance and scope:**

CONTEXT: Approved U3 guards old notebook APIs and adds typed notebook functions. Existing app_consumer only calls ordinary symbol APIs, exports typed Parquet and atomically publishes validated native-child runs. Base file_facts.sql pipeline is graph-only and need not be rewritten. GOAL: Route .ipynb sources/pairs to new notebook functions; retain ordinary file behavior. Export typed cell inventory/symbols/refs/diagnostics/structural+dependency+metadata events coherently with parent source/snapshot lineage. Keep source identity scheme exactly sha256:utf8-v1 and attach snapshot labels after validating frozen source bytes. Explicitly version additive/new consumer contract and preserve historical v1 read compatibility where safe; no silent schema reinterpretation. Keep atomic publication, child exit/timeout cleanup, immutable readback hashes, old pointer on failure. ACCEPTANCE: failing tests before code for notebook route, original source refs and hashes, partial/anonymous/unknown coverage, structured changes and empty/import-only cell retention; new export/readback schema assertions; existing ordinary consumer and fault tests pass. Test frozen real-input fixture through native child, current source and historical version context checks. Do not modify publisher or generic graph cache unless evidence requires scope_drift.

**TDD sequence:**

1. Add the task-specific regression using the cases below and existing test harness conventions; commit the failing test separately.
2. Run the focused test with `scripts/spur-cargo` (Rust remote, no local fallback) or native DuckDB2 Python as appropriate. Capture the expected assertion failure; a build/setup failure is not meaningful RED.
3. Implement the smallest scoped change; run the identical test to GREEN, then relevant existing regressions once.
4. Record exact commands/exits and source/native identities, commit the implementation, and return the final interface. Root reviews before the next dependency dispatches.

```python
result = run_consumer(fixture, output, package, expected_extractor_id=extractor_id)
reopened = read_result(output, requested_snapshot)
assert reopened["manifest"]["source_identity_scheme"] == "sha256:utf8-v1"
assert reopened["snapshot_id"] == requested_snapshot
# Validate exact new typed notebook export rows/source refs and unchanged publication pointer on injected child failure.
```

## U5: Verify new native release and document notebook contract

**Issue:** `bd-338y`
**Task ID:** `U5`
**Depends on:** `U4`
**Suggested Worker:** codex

**Files:**
- `crates/spur-graph-duckdb/README.md`
- `crates/spur-graph-duckdb/docs/release-contract.md`
- `crates/spur-graph-duckdb/scripts/check_release.py`
- `crates/spur-graph-duckdb/tests/test_release_driver.py`
- `docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release`

**Acceptance and scope:**

CONTEXT: Approved U1-U4 implement notebook symbols/changes and consumer routing. GOAL: Final reproducible release-quality check and accurate public docs using fresh compiled artifact; preserve prior package28ddce83 and prior immutable evidence. Read existing release driver before modifying; only change driver with meaningful failing tests when required to include new focused core/native acceptance, don't refactor working release automation. Run scripts/check_release.py --out <fresh task-owned path> --host-native with existing DuckDB2 Python; Rust remote tests via wrapper with local fallback disabled, separate intentional host-native build for LOAD (not retry of remote red). Full native Python suite must run with no hidden skips. Add standalone source/hash checked real-notebook native consumer acceptance using tracked real-inputs.json from PoC; require names/cell IDs/ranges/17+11 functions and no-op correctness; explain source-grounded delta-count differences instead of forcing55. Include Rust :dep original-coordinate and explicit unsupported cases. Retain exact argv/exits/stdout/stderr, source fingerprint, package/sidecar hashes, schemas/counts, no-op and child publication-failure evidence. Document exact SQL v1 schemas, examples, coordinate/preprocessing/coverage/identity limits and consumer migration. No frontend edits/deployment. New artifact, not old binary. Root brain will perform final independent package/native notebook review and merge after evidence; don't close issues yourself.

**TDD sequence:**

1. Add the task-specific regression using the cases below and existing test harness conventions; commit the failing test separately.
2. Run the focused test with `scripts/spur-cargo` (Rust remote, no local fallback) or native DuckDB2 Python as appropriate. Capture the expected assertion failure; a build/setup failure is not meaningful RED.
3. Implement the smallest scoped change; run the identical test to GREEN, then relevant existing regressions once.
4. Record exact commands/exits and source/native identities, commit the implementation, and return the final interface. Root reviews before the next dependency dispatches.

```python
python = "/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python"
# Execute via the existing driver with a new --out; retain raw exits for every phase.
# The host-native build is an intentional separate target needed for local LOAD.
command = [python, "crates/spur-graph-duckdb/scripts/check_release.py",
           "--out", "docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release/driver",
           "--host-native"]
```

## Self-review and execution

- DAG verified with graph_plan: U1 → U2 → U3 → U4 → U5; only U1 dispatches initially.
- U1 owns normalized identity/preparation/graph parity; U2 owns symbols/matching; U3 owns typed SQL and legacy guards; U4 owns coherent consumer routing/readback; U5 owns native release/docs. All spec requirements map to these gates.
- No parallel writer overlap. U1/U2 touch extract/mod.rs sequentially. Any expansion requires an explicit scope signal and root decision.
- Core API names are fixed in the source spec. U1/U2 report exact Rust types/signatures after review; downstream tasks use the approved implementation directly rather than cloning PoC types.
- Preserve old release artifact/evidence and all tracked user edits. Finished scoped work is committed and root merges only approved integrated results.
- Final root acceptance additionally opens/verifies the fresh native candidate in a notebook DuckDB2 runtime, then records exact results and limitations. No frontend production changes or deployment.
