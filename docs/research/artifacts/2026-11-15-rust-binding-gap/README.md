The accompanying report is `../../2026-11-15-rust-binding-gap.md`.

`binding_pilot.py`, `evaluate_gap.py` and `owner_diagnostics.py` are the reproducible experiment sources. Their native DuckDB 2.0 environment, approved extension and frozen snapshot are local prerequisites pinned by SHA-256 in the scripts. `evaluate_gap.py` also needs the recorded local rustc toolchain. It writes isolated fixture outputs here; it does not modify production source or the original database.

`binding-pilot.json`, `binding-analysis.json` (28 receipts) and `owner-analysis.json` retain SQL/parameters/results. `package-catalog.json` retains the frozen Cargo manifest hashes and nearest-package metadata. Rows are candidates, not compiler-proven bindings.

`compiler-expectations.json`, the two control `.rs` files, `receiver-control.mir`, `macro-expanded.rs` and `compiler-controls.json` retain compiler expectations, source adaptations, commands, actual compiler outputs and extension extractions. Native executable binaries are local artifacts and are intentionally not committed.

`mcp-*.json` retain bounded extension discovery and function-registration observations, for the notebook's managed DuckDB 1.5.5 connection. Those build-availability records do not establish DuckDB 2 compatibility. The experiment itself used native DuckDB 2.0.0.

`notebook-cells.json` contains final cell sources and MCP outputs. The initial macro fixture compilation failure (missing the invocation's source semicolon) and the wide-source sample query's 1GB OOM are retained separately. Successful runs corrected those experimental construction/query-shape errors. Notebook status fields alone can report success even when Rust panics; the saved outputs and durable result assertions determine success here.

`retention-final.json` verifies the frozen database and five existing dirty user files. `manifest.json` hashes the retained evidence; `independent-review.json` records the final review outcome when available.
