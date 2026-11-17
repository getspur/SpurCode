# Notebook Rust-cell PoC and required extension updates

Date: 2026-11-17. Task bd-26sp; parent gap bd-3ct2; Rust-directive gap bd-3ofb.
Status: notebook PoC completed and replayed; production extension is unchanged.

## What ran

[Executed notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-spur-graph-notebook-rust-poc.ipynb): 13 Rust cells, replayed in order after restarting the dedicated Evcxr slot to generation 2. The final POC_ACCEPTANCE_PASS marker confirms 30 checks: 15 boundary, 8 context and 7 Rust-directive checks. All replay outputs were inspected for error/panic payloads. Adding hash dependencies emits a benign warning that the unused bootstrap spur variable was redefined.

The Rust cells implement candidate normalization, source references, comparison policy and kernel preprocessing. [native_bridge.py](native_bridge.py) supplies Python DuckDB bindings to the retained, hash-checked native DuckDB 2.0.0 extension. Each extraction and symbol comparison runs the real native functions. The production Rust crate is not linked directly into Evcxr; this PoC establishes policy and integration behavior through the released SQL boundary.

The dedicated notebook's SQL default remains unrelated to this Rust-only experiment. Every bridge connection independently enforces DuckDB 2.0.0 and extension SHA256 28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122.

## Measured behavior

The real fixture is the original frozen app_gallery/code-graph-workbench/app.ipynb, base cd24d5c4779d11a5b2dd1bbe55c93da12856ae41 and head 07db59f08dbced2c8bedd17290c78f57b428a2bc. [real-inputs.json](real-inputs.json) retains exact source text and verified UTF-8 SHA256 identities.

| Measurement | Existing native notebook route | Rust candidate with native per-cell extraction |
| --- | --- | --- |
| Base/head symbols | 0 / 0 | 46 / 24 |
| Base/head function coverage | Graph finds 17 / 11; symbols find none | 17 / 11, matching independently frozen function anchors |
| Identical two-cell helper comparison | Flattening produces false modified + added | 0 symbol deltas |
| Real notebook comparison | Raw notebook produces no symbol deltas | 55 deltas across two code cells |
| Rust cell with leading :dep and function | 0 symbols; graph result says ok | Function recovered at its original byte range and line 2 |

The 55 reported deltas are 12 added, 9 modified and 34 deleted; 50 belong to wb-graph and 5 to the added Python cell. These are native matcher results, not counts of independent semantic changes or defects. Ten separate structural events retain four added cells, three deleted cells, one move, one source change and one metadata change.

All 70 symbol references resolve against their frozen decoded cell sources. All 28 function byte/line anchors match the independent earlier probe. All 55 delta references resolve against the correct before/after document. Eleven independently measured code-cell records match ID, normalized language, source hash and symbol count.

## New Rust-specific finding

A valid Evcxr dependency header can hide the function following it from ordinary Rust syntax extraction. The notebook separately executes a real cell containing :dep sha2 = "0.10", a function and a successful assertion. Native syntax extraction of the equivalent directive form returns zero while reporting graph status ok.

The candidate adapter masks leading single-line :dep lines with ASCII spaces while preserving every UTF-8 byte offset and CR/LF position. It retains original source, original source hash, analysis source hash, directive text, preprocessing policy version and an explicit identity coordinate map. The original source remains the authority for source references. Unicode in a directive and CRLF line endings are tested.

A dependency version edit with an unchanged function produces no symbol delta and an explicit dependency_directives_changed cell event. Unknown leading commands and multiline dependency declarations produce partial coverage in the PoC. Directive-looking text inside a Rust string is not rewritten.

This is a bounded adapter, not complete Evcxr preprocessing. Other commands, directives after code, comments before headers, load semantics, multiline forms and arbitrary kernel syntax need parity tests against the kernel parser before production support. The graph route and symbol route must share the same preparation policy.

## Cell behavior established

- Compare inside cell identity and normalized language. Two cells may declare the same helper without colliding.
- A same-cell duplicate identity is explicitly ambiguous and yields partial comparison; it is not silently overwritten in a map.
- Keep imports-only, empty, unsupported and non-code cells in inventory. Cell add/delete and reorder remain visible independently of symbol deltas.
- Preserve explicit IDs and the current BLAKE3 source-derived anonymous ID scheme. Anonymous edits produce conservative add/delete with identity-limited diagnostics; repeated anonymous sources remain explicitly uncertain.
- Treat language changes as conservative symbol removal/addition. Do not infer cross-language renames.
- Preserve both notebook-level and cell-level metadata, including Rust kernel profile selection. Metadata changes are observable; this does not establish effective runtime compatibility.
- Keep snapshot, notebook source hash, cell ID/language/source hash and decoded-cell UTF-8 coordinate frame on references. Reject wrong snapshot, cell, source hash or frame.
- Output-only changes produce no code or cell-structure deltas in the tested case. This does not prove execution equivalence.

The supported PoC language subset is Python/Python3, JavaScript and Rust/Evcxr. It was aligned to the current core token registry after a RED witness caught overly broad initial aliases. Production must reuse the full registry rather than copy this subset. Source string/list validation is stricter in the candidate than the current graph helper's silent filter_map behavior; production needs an explicit compatibility and diagnostics policy.

## Required production update map

| Area | Concrete update | Acceptance evidence to carry forward |
| --- | --- | --- |
| spur-graph extract/notebook.rs | Expose a shared normalized notebook/cell model: identity quality, ordinal/type, resolved language, original source/hash and metadata. Reuse current IDs/language rules; diagnose duplicate IDs and invalid source shapes. | Frozen 11-cell parity, source string/list equivalence, anonymous and duplicate-ID cases |
| Notebook-context Rust extraction | Introduce shared kernel preparation with original-to-analysis source mapping, directive/config facts and explicit unsupported coverage. Apply it to graph and symbol paths. | Real Evcxr execution, raw zero/recovered function, UTF-8/CRLF ranges, dependency-change event |
| extract/tree_sitter.rs and public extraction seam | Add a cell-aware symbol/document route that dispatches to language extractors instead of the notebook early return. Do not parse raw notebook JSON as Rust/Python/JavaScript. | Real 46/24 symbols, 17/11 functions and 70 original-source ranges |
| git_walk.rs matching seam | Supply language explicitly to an inner matcher and group by cell. Preserve structural changes and report ambiguous symbol keys. Return sufficient before/after provenance. | Duplicate helper no-op, real no-ops, type/language changes and same-cell ambiguity |
| spur-graph-duckdb typed SQL contract | Add a compatible notebook/document result with parent/cell statuses, diagnostics, metadata, source refs and preprocessing identity. Keep existing v1 nested symbol structs stable. Publish per-API capabilities truthfully. | Schema contract, NULL behavior, invalid input, variable-sized vector/list batches, native app fixture |
| Cache, consumer and snapshot publication | Store/export cell inventory, symbols, structural/config changes and diagnostics coherently. Include extractor/preparation/schema identity in cache migration assessment. Preserve source identity through readback. | Typed export/readback, coherent snapshot and failure-path publication tests |
| Notebook app and agent interface | Explore and Review navigate notebook/snapshot/cell source refs. Display partial coverage and identity limits; distinguish symbol, structure, dependency and metadata changes. Apply the same context guard for humans and agents. | Real notebook source/diff navigation and current-vs-historical note/source guards |

The proposed source coordinate contract is decoded_code_cell_utf8. Native matcher IDs and file paths under poc-cells/... are transport scaffolding to supply a recognized extension; they are explicitly labelled native_transport_delta. They are not production notebook stable IDs. Production should preserve canonical notebook/cell identity directly and expose complete typed before/after references instead of inheriting those synthetic paths.

## Verification and limitations

[replay.json](replay.json) retains restart and all 13 synchronous run receipts; [final-receipt.json](final-receipt.json) is emitted by the final Rust acceptance cell. [rust-cells.json](rust-cells.json) records exact executed cell sources. RED cells deliberately catch failed assertions so the experiment can continue; six expected failures establish the earlier/native gaps before their candidate fixes.

[solver-verification.json](solver-verification.json) retains pre/post catalog verification of the two-cell uniqueness contract: flat identity fails, cell-qualified identity passes. These finite checks do not prove parsing, runtime behavior or arbitrary notebook correctness. Executions use persist:false because the existing solver artifact store is full; no persisted solve IDs are claimed.

Initial notebook-harness borrow errors were corrected using owned strings and scoped references. One uncaught exploratory assertion was labelled succeeded by Evcxr transport; it was changed to an explicit expected-RED check. Final verification inspects outputs and the acceptance marker rather than trusting transport status alone.

The bridge runs rows individually. DuckDB vectorized batch behavior, complete kernel syntax, cross-cell runtime dependency resolution, repository/notebook renames, production cache migration and app UI integration have not been validated by this PoC. Shared normalization and kernel preparation, typed SQL design, native TDD, regression checks and a rebuilt artifact are still required. No workspace Rust implementation files or release binary were changed.
