# Fresh playground after local-main integration

User request: merge locally and verify the new extension in another playground.
Local `main` already contains implementation merge `108e395ba`; the ancestor
check passed. No duplicate merge or extension source change was required.

Fresh notebook:
`/Users/kevintruong/project/spur-notebook/2026-11-17-spur-graph-local-main-playground.ipynb`.
Notebook MCP created and ran all cells, saving durable outputs. Its separate
DuckDB 2.0.0 kernel (PID 4929, generation 1) had zero Spur Graph functions
before loading the SHA256-verified `c5f36a0f...` candidate.

All **8 executable cells** passed. Real snapshots again return **46/24 symbols**,
**17/11 functions**, **50 symbol changes** and **10 cell events**. Independent
checks against original JSON bytes resolve all **70 UTF-8 refs**, match all
**28 function anchors**, and validate both same-source no-op comparisons.

All **18 independent scenarios** pass: three languages/four functions,
duplicate helper names in separate cells, Rust dependency masking with original
coordinates, dep/load type syntax, one-function modification, dependency-only
changes, output-only edits, reordering, added/deleted notebooks, same-document
no-op, unsupported command/language coverage, malformed/missing inputs,
ordinary Rust compatibility, actionable legacy guards and connection recovery.

[receipt.json](receipt.json) retains exact MCP-read source, outputs, versions
and execution provenance. One Python preflight authoring correction replaced
DuckDB 2 relation `fetchone()` with `fetchall()[0][0]`; it passed before loading.
The production extension required no correction.

Restart this notebook's kernel before a complete top-to-bottom rerun; its
preflight deliberately checks that no extension functions are already loaded.
The candidate remains local and unsigned. Full app UI and cross-cell execution
semantics remain separate work. Existing user edits and earlier notebooks remain
untouched.
