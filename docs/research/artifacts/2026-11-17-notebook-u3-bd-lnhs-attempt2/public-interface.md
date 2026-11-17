# U3 typed native notebook SQL v1

Both functions directly reuse the approved U2 functions in
`spur_graph::extract::notebook_symbols`, without a second normalizer or matcher:

```rust
pub fn extract_notebook_symbols(path: &str, bytes: &[u8])
    -> anyhow::Result<NotebookSymbolDocument>;
pub fn compare_notebook_sources(path: &str, old_path: Option<&str>, change_kind: &str,
    left_source: Option<&[u8]>, right_source: Option<&[u8]>)
    -> anyhow::Result<NotebookComparison>;
```

```sql
spur_graph_notebook_symbols(path VARCHAR, source VARCHAR) -> Document
spur_graph_notebook_changes(path VARCHAR, old_path VARCHAR, kind VARCHAR,
                            left_source VARCHAR, right_source VARCHAR) -> Comparison
```

The exact expanded `DESCRIBE` strings, registration arguments, descriptions and
examples are retained in `native-interface.stdout` in this attempt-2 directory under `describe` and
`functions`; `test_notebook.py` independently asserts the exact schemas. U5 owns
copying the release schemas into `docs/release-contract.md`. No field was added
to `spur_graph_info()` or the ordinary v1 symbol/change schemas.

The following aliases describe the nested types; `[]` is a typed SQL LIST.
Numeric ranges/ordinals use UBIGINT, contract_version uses UINTEGER; all other
scalar fields use VARCHAR. Optional Rust values become SQL NULL.

```text
Diagnostic(code, side?, cell_ordinal?, byte_start?, byte_end?, message)
SourceRef(notebook_path, notebook_source_sha256, cell_id, cell_ordinal,
          cell_identity_quality, cell_source_sha256, cell_language,
          coordinate_space, byte_start, byte_end, line_start, line_end)
SymbolFact(entity_name, symbol_kind, enclosing_scope?, byte_start, byte_end,
           line_start, line_end, anchor_hash, tokens VARCHAR[])
NotebookSymbol(id, symbol SymbolFact, source_ref SourceRef)
Directive(text, byte_start, byte_end)
PreparedSource(original, original_sha256, analysis, analysis_sha256, policy,
               coordinate_map, directives Directive[])
Cell(id, identity_quality, ordinal, cell_type?, language?, metadata_json,
     source PreparedSource?, status, diagnostics Diagnostic[], symbols NotebookSymbol[])
Document(contract_version, path, source_sha256, metadata_json,
         cells Cell[], status, diagnostics Diagnostic[])
SymbolChange(kind, before NotebookSymbol?, after NotebookSymbol?)
CellRef(notebook_path, notebook_source_sha256, cell_id, cell_ordinal,
        cell_source_sha256?, language?, cell_type?)
CellChange(kind, before CellRef?, after CellRef?)
DocumentChange(kind, before_path?, after_path?, before_metadata_json?, after_metadata_json?)
Comparison(contract_version, status, diagnostics Diagnostic[], before Document?,
           after Document?, symbol_changes SymbolChange[], cell_changes CellChange[],
           document_changes DocumentChange[])
```

DuckDB quotes reserved field names in DESCRIBE, including `"language"`,
`"source"`, `"policy"`, `"text"`, `"before"`, `"after"`. Field lookup uses their
ordinary names. Status is `complete`, `partial`, or `invalid`; language strings
are native registry variant names (e.g. `Rust`, `Python`, `Javascript`).
Diagnostic/change/identity codes use explicit stable snake_case mappings in
the adapter. Coordinate space is `decoded_code_cell_utf8`, and byte bounds are
half-open. Source refs identify original sources, never synthetic .rs/.py paths.

Metadata JSON preserves arbitrary opaque facets: a JSON `null` remains text
`null`; SQL NULL means an absent optional value. Saved profile names remain
caller metadata. Document/cell diagnostics and statuses describe coverage;
complete never claims execution equivalence or dependency resolution.

Symbols returns SQL NULL for NULL path/source. Changes returns SQL NULL only
for NULL path/kind. A requires absent left and present right; D requires present
left and absent right; M requires both. Missing/unexpected sides produce typed
invalid coverage. NULL old_path defaults to path except R/R100, which requires
a distinct valid old_path and both sources. Other kinds return partial with an
unsupported_change_kind diagnostic. Malformed JSON/source/type/ID shapes retain
typed invalid diagnostics; unsupported languages/directives and ambiguous
identities retain inventory and explicit partial coverage. Core infrastructure
errors use SQL errors; the adapter never converts an Err into a coverage row.

Legacy `spur_graph_symbols` and `spur_graph_symbol_changes` reject notebook paths
with the new function name and instructions to pass original notebook sources.
Ordinary-file nested schemas and matcher/extractor results remain v1. U4 must
route notebooks before the integrated release is accepted.

Runnable checks from the repository root, using a fresh package (no historical
package overwrite):

```sh
export GRAPH_PYTHON=/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python
export SPUR_GRAPH_EXTENSION=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-final/spur_graph.duckdb_extension
export SPUR_GRAPH_BUILD_SCRIPT=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target/debug/build/spur-graph-duckdb-bc3a2be281cee99d/build-script-build
"$GRAPH_PYTHON" crates/spur-graph-duckdb/scripts/verify_package.py "$SPUR_GRAPH_EXTENSION"
"$GRAPH_PYTHON" -m unittest discover -s crates/spur-graph-duckdb/tests -p test_notebook.py -v
"$GRAPH_PYTHON" -m unittest discover -s crates/spur-graph-duckdb/tests -p test_contract.py -v
"$GRAPH_PYTHON" docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs/inspect_candidate.py
```

Rebuild through `scripts/spur-cargo --workdir crates/spur-graph-duckdb build
--locked --lib --examples --target-dir <task-target> --message-format=json`,
then use `scripts/package_extension.py --library <task-target>/debug/libspur_graph_duckdb.dylib
--out <fresh-package>/spur_graph.duckdb_extension`. Remote remains default; this
task deliberately used SPUR_REMOTE=0 under the verified exit-200 infrastructure
recovery authorization. U5 runs the existing release driver with a fresh --out
and --host-native; if recovery is still needed use the already authorized
--rust-tests-mode host and label those results HOST.

Finite PRE/POST solver snapshots check declared identity/reference tuples only.
Runtime SQL tests establish concrete parser, matcher and vector behavior; neither
proves full Evcxr support, arbitrary parsing, runtime dependencies, notebook UI
integration or cross-cell execution analysis. Consumer migration/publication,
the complete release suite and notebook-runtime readback belong to U4/U5/root.

Attempt 2 adds only legacy `spur_graph_file_facts_result` compatibility: core
notebook JSON/inventory errors remain `extraction_error` / `malformed_input`,
with the exact original error text and NULL facts; internal/setup errors still
propagate. It does not alter either new notebook schema or the old v1 schemas.

For the complete unchanged 76-test legacy/consumer selection on this candidate:

```sh
export SPUR_GRAPH_LIBRARY=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target/debug/libspur_graph_duckdb.dylib
export SPUR_FILE_FACTS_ORACLE=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target/debug/examples/reference_file_facts
"$GRAPH_PYTHON" docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs/regressions.py
```

Exact capture argv/environment, source and native hashes are in the attempt-2
phase JSON receipts. The fresh candidate is an unsigned local macOS arm64
DuckDB 2.0.0 artifact. HOST execution is authorized recovery; no remote success,
full release-driver run, downstream consumer notebook routing, or UI acceptance
is claimed by U3.
