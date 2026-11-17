# Notebook symbol extraction diagnosis

Date: 2026-11-17. Investigation: bd-nb4c. Repair issue: bd-3ct2 (open).
Status: root cause and repair contract established; native fix is not implemented.
The existing DuckDB 2.0 native extension remains usable for its tested ordinary-file workflows.

## Finding and app impact

Notebook symbol/change extraction needs targeted rework before notebook-aware Review can meet the app expectation. The existing graph-facts notebook extractor can be reused. A whole-parser rewrite is unnecessary.

The frozen real-repository probe used app_gallery/code-graph-workbench/app.ipynb, base cd24d5c4779d11a5b2dd1bbe55c93da12856ae41 and head 07db59f08dbced2c8bedd17290c78f57b428a2bc.

| Extraction route | Base | Head | Interpretation |
| --- | ---: | ---: | --- |
| Notebook graph function nodes | 17 | 11 | Existing cell-aware graph route finds code |
| Notebook symbol function result, all kinds | 0 | 0 | Explicit unsupported early return masquerades as successful empty output |
| Symbols from separately decoded code cells, all kinds | 46 | 24 | Existing language extractors work on the cell source |
| Functions projected from decoded cell symbols | 17 | 11 | Necessary coverage target for this fixture |

Raw notebook symbol changes are also empty without diagnostics. The app must not present that result as evidence of no code changes. Explore already has useful cell containment, but source navigation must use decoded cell coordinates. Existing measurements checked all 28 graph function ranges in their containing cells; those offsets do not refer to raw notebook JSON.

Prior frozen evidence: [real probe evaluation](../2026-11-17-spur-notebook-real-probe-bd-361m/). This diagnosis does not change its snapshots, publication pointer or retained release binary.

## Causal source trace

1. spur_graph_symbols calls BytesExtractor::extract in crates/spur-graph-duckdb/src/lib.rs:322–334.
2. crates/spur-graph/src/extract/tree_sitter.rs:326 returns Ok(Vec::new()) immediately for Language::JupyterNotebook, before UTF-8 or JSON validation.
3. The separate graph route calls extract_file_facts → extract_graph_facts → extract_notebook_file, which decodes cells, resolves their languages, and parses their source.
4. The ordinary-file matcher in crates/spur-graph/src/git_walk.rs:2386 keys symbols by entity_name, symbol_kind and enclosing_scope. It has no cell discriminator.
5. The v1 SymbolFact SQL struct has no notebook cell, language, source identity or coordinate frame. A correct implementation cannot simply concatenate cell lists or silently reinterpret its offsets.

Exact graph reads, raw adapter source fallback and current file Git blob OIDs are retained in [source-trace.json](source-trace.json). The graph index was stale; each relevant returned source blob was checked against current file bytes. The missing adapter symbol body was read from the working tree.

## Native counterexamples

Both execute against retained Mac ARM64 extension SHA256 28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122 in DuckDB 2.0.0.

- Malformed source "not json" at broken.ipynb: symbols returns []; graph result reports extraction_error / malformed_input. Empty symbols are therefore not proof of valid empty notebook extraction.
- Two separately parsed Python cells each declare top-level helper(), with different bodies. Concatenate their two symbol lists and compare that list with itself as two-cells.ipynb: the matcher emits two false changes, [modified, added], with no diagnostics. Grouping by cell is required.

SQL sources and exact synchronous notebook receipts are in [runtime.json](runtime.json). The initial counterexample used legacy lambda syntax, which DuckDB 2 rejected; correcting the query to lambda d: d.change_kind produced the witness. That harness correction is retained separately. The notebook also retained stale earlier failure provenance on this edited cell; the successful synchronous receipt and separate native replay establish the result.

## Solver checks and limits

Catalog-first verification used implemented version-1 data_integrity and configuration rules. [solve-cases.json](solve-cases.json) retains every exact executed request, raw response and rejected attempt; [catalog.json](catalog.json) retains the loaded rule guidance.

| Model | Verdict | Meaning |
| --- | --- | --- |
| Observed notebook function coverage 0/17 and 0/11 | unsat / fail | Violates the declared necessary coverage equality |
| Proposed cell-aware coverage 17/17 and 11/11 | sat / pass | Counts meet equality; counts alone do not prove accuracy |
| Two helper symbols under ordinary-file identity | unsat / fail | Distinct cells collide |
| Same witness with cell-qualified identity | sat / pass | The cell discriminator separates this witness |
| Frozen base/head wb-graph SourceRefs | sat / pass | Two measured function references match their two cell parents |
| Replace one frame with notebook_json | unsat / fail | Source reference no longer matches |
| Put base source hash on head reference | unsat / fail | Snapshot/source pair is invalid |
| Omit a cell ID | unsat / fail | Required-field rule blocks NULL escaping the foreign key |
| Notebook request produces silent empty outcome | unsat / fail | Neither success nor unsupported provider selected |
| Cell-aware success outcome | sat / pass | Meets declared outcome contract |
| Explicit unsupported outcome | sat / pass | Truthful outcome, still lacks notebook review capability |

These are verification-mode meanings, not generic assert-negation proofs. The solver does not inspect the implementation or establish correctness for arbitrary notebooks. The full 11-cell/28-reference relational model exceeded the catalog's 64-variable limit and was rejected before solving; the accepted SourceRef models use two parents and two references. The complete runtime coordinate measurements remain separate evidence.

The solver artifact store is full (512 items). The persist:true attempt was rejected, then all accepted models used persist:false. No persisted solve IDs exist for this investigation; local exact requests and responses are retained.

## Smallest compatible repair contract

Reuse a single normalized notebook-cell representation in spur-graph, then expose it through an additive, typed notebook/document SQL result in spur-graph-duckdb. Public function names are proposed design choices, not APIs implemented by this investigation. Keep ordinary-file v1 nested structs and behavior compatible.

Each normalized cell needs notebook identity, explicit or fallback cell identity and identity quality, ordinal, cell type, resolved language, decoded source and source hash. Each symbol/source reference needs snapshot/notebook identity, cell ID, language, cell source identity, byte/line range and coordinate_space=decoded_code_cell_utf8. No cell ID encoded into enclosing_scope.

Match paired cells first; run the existing language matcher within each cell with the normalized language supplied explicitly. The virtual graph identity path <notebook>#cell:<id> is not a recognized ordinary-file extension, so it cannot be fed blindly into the existing path-derived-language wrapper. Preserve the existing ordinary-file wrapper and introduce a suitable inner helper or notebook wrapper.

Use explicit cell IDs when unique. Existing anonymous fallback IDs depend on source content; edits change those IDs. Treat uncertain cross-version matching conservatively and report identity limitations. Duplicate explicit IDs must be rejected or diagnosed before matching, never merged in a map.

Retain cell inventory and per-cell diagnostics even when symbol count is zero. Unsupported language, malformed JSON, empty supported code, absent input, and extraction failure must remain distinguishable. Report partial coverage. A language/type change needs an explicit policy and diagnostic, not a cross-language rename guess.

Output-only changes should not manufacture symbol edits. Cell reorder should preserve symbol identity but remain visible as a notebook structural change because execution order can matter. Added/deleted imports-only or declaration-free cells must remain visible even if they produce no symbols.

The app should navigate source/diff by notebook + snapshot + cell + source hash and declared coordinates. Human and agent queries need identical coverage and identity metadata. Existing graph success must not imply symbol-review support. A temporary explicit unsupported signal must be coordinated with consumers: the current headless consumer calls symbols after graph extraction succeeds.

## Test-first implementation sequence

1. Establish a shared normalization seam without changing current graph IDs or ordinary-file extraction. Add failing tests for mixed JavaScript/Python, source string/list encodings, Unicode ranges, language precedence, invalid JSON, missing IDs and duplicate IDs.
2. Add the typed cell/symbol result and explicit coverage states. Real notebook assertions must check names, source slices, identities and diagnostics, not just totals. Keep v1 SQL schemas stable.
3. Add cell-aware comparison with failing regressions for two same-named helpers, identical snapshot no-op, cell add/delete, cell type/language change, output-only changes and reorder. Preserve the ordinary-file matcher tests and avoid silently overwriting ambiguous symbols within a cell.
4. Update the consumer/cache/publication contract and app evidence mapping together. Verify source identities and per-cell coverage survive typed export and readback.
5. Run focused Rust checks through scripts/spur-cargo and native SQL tests, including NULL and variable-sized list batches. Build a new candidate artifact, retain the old binary, then replay the frozen real-repository probe and the two native counterexamples.
6. Re-run these contract models if the policy changes. Accept notebook review only after symbol names/ranges and cell-aware no-op/change results are validated against frozen sources. General graph/SQL regression success alone is insufficient.

No native source code was modified by this investigation. bd-3ct2 stays open until that TDD implementation and fresh native probe pass.
