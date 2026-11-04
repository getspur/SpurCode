# Final panic-evidence notebook validation

The approved implementation is integrated on
`spur/plan-merge-2002cf0d-1166-42aa-b575-1af20f512c38-1267bf4ff06544acaea60e882178a307`.
Implementation integration commit `62f203acc5d4e42908abb871faec66169f78f551`
has exactly the reviewed `11d72a0ea294766d676ce8e78dcf0717696cf482` tree.
Main was not switched or reset.

Open [the executed notebook](../../../../2026-11-02-panic-evidence-validation.ipynb).
All notebook creation, editing, running, output inspection and extension operations used Notebook MCP.
The new notebook preserves the earlier grounding and panic-graph notebooks.
The motivating [CGA article](https://tweedegolf.nl/en/blog/242/finding-panics-with-call-graph-analyzer/)
describes compiler call-graph analysis; this experiment adds explicitly bounded feasibility and native observations.

## Measurements

| Evidence | Result |
|---|---|
| Fresh source → typed analysis → actual Z3 → native replay | 13 cases: 7 reproduced panics, 4 complete bounded no-counterexamples, 2 incomplete semantic results |
| Valid-shape adversarial replay metadata | 10 rejected forgeries; 2 wrong-payload nonreproductions |
| Separate MCP typed-query cross-checks | 5 preflighted, solved, persisted and reloaded; exact source/build/fact/request/status/model bindings agree |
| Rebuilt pinned CGA | 5 successful configurations; 677 observations, 45 native controls, 26 corruption rejections |
| Managed DuckDB capabilities | Signed Shellfs and Duckrouting loaded; JSON and Parquet exercised |
| Source-matched implementation regression retained from R8G | 1,292 passed, 0 failed, 19 existing ignores; 98 targets; all-target compile/format/diff checks passed |

The saved observations demonstrate why precise identities, typed value origins and guards matter:
the same assertion graph finds input 173 over 0–255, but has no witness over 0–172.
The same addition graph changes its panic result with overflow-check configuration.
Bounds/overflow fixtures have no ordinary call edges but still yield reproduced implicit-check panics.
The unsupported formatted assertion remains incomplete even when its modeled query is UNSAT.

Duckrouting found the three-edge candidate
`panic_pilot::main → panic_pilot::generic::<u16> → Option::<u16>::unwrap → std::option::unwrap_failed`.
Only direct `Called` observations enter that route. This is not a branch-feasibility proof.
Physical source Calls, compiler observations and semantic operations are different grains; no recall percentage is inferred.

## Provenance and files

- `integrity.json`: reviewed/integrated tree equality, retained binary SHA, source manifest and read-only R8L/R8G verification.
- `cases.json`, `cases.parquet`, `cases-summary.json`: fresh case outcomes.
- `bounded-20261102T042551429206/`: exact snapshot/analyze/solve/replay requests, outputs, hashes and forgery controls. Sources are embedded in requests; temporary source paths are provenance.
- `mcp-solver-crosscheck.json`: exact five typed requests, source/build/fact bindings, catalog discovery, preflights, responses and complete reloaded receipts. These new MCP IDs supplement the CLI-local receipts; they are not ID substitutions.
- `cga-bootstrap.py`, `cga-*.command.json`, logs: fresh pinned-producer build and invocation.
- `cga-capture/`: 412 exact archived capture files, 7,907,202 bytes, excluding compiler targets, cargo-home and private solver stores. Per-file hashes are in `cga-archive-manifest.json`.
- `cga-summary.json`, `cga-observations.json`, `cga-nodes.json`: typed compiler joins with build IDs and observation categories.
- `candidate-panic-route.json`, `domain-build-comparison.json`: measured SQL outputs.
- `extensions-observation.json`, `duckdb-runtime.json`: as-of signing/loading and runtime versions, not a promise about future kernel connections.
- `notebook-corrections.json`: observed notebook/verification corrections; no production behavior was changed to pass an experiment.
- `notebook-mcp-validation.json`: final tool readbacks and DAG health.
- `verify.py`, `verification.json`, `SHA256SUMS.json`: read-only binding checks and the retained payload seal.

The bounded/native lane used Rust **1.94.1**, commit
`e408947bfd200af42db322daf0fadfe7e26d3bd1`.
The CGA lane used revision **2fc63108f0b674fe7a178191ffa15647a18879f1**
and **nightly-2026-10-18**, rustc
`8006091eb71ae7d9e9bccb66c6122c9d68af4466`.
Only the already-reviewed macOS rpath link patch was reapplied, SHA
`72f3d4cd64439d988deba0ee1e0f449df8290e6f1d1e0ae83ba82d97b1592aca`.
Fresh driver SHA:
`0251f7cdda9a4c21dee689394a999704e0c182ce51530f88e4c336afe42360b2`;
launcher SHA:
`4d49e584f723fc7b17eafc4534352a319a96aeb045fd5ce849532cdedbf423a7`.
The two corpora/toolchains are not silently joined as the same source/configuration.

## Recheck and rerun

From this artifact directory, `python3 verify.py` checks saved hashes, source Git blob OIDs,
exact request/model/replay bindings, all five separate MCP receipts, the archived raw CGA hashes,
direct route edges, controls, and extension observations. It is read-only and needs only Python's standard library.
It does not rerun Rust, recover evicted receipts, inspect a private solver store, or parse notebook JSON.
`SHA256SUMS.json` excludes itself and the regenerable `verification.json`.

To rerun the experiment, use Notebook MCP on the delivered notebook after providing the
source-matched example binary and pinned toolchains identified in `integrity.json`.
The executed setup cells document exact local paths; update paths through Notebook MCP if they move.
The helper `run-bounded-cases.py` creates new timestamped evidence and uses the actual example API;
`cga-bootstrap.py` builds through `scripts/spur-cargo` with `SPUR_REMOTE=0`.
The capture cell invokes the integrated `scripts/experiments/panic-evidence-capture.py`
with explicit source revision, producer hashes, patch hash and isolated output paths.
Targets and binaries are intentionally not committed.

## Limits

Complete results apply only to the stated domains, supported primitive semantics, roots,
build flags and exploration limits. A candidate is not marked reproduced until a separate
native fixture run matches its expected payload. Unsupported dispatch/expansions/effects and
exhausted bounds stay obligations. A successful bounded fixture capture is not a full `spur::main`
capture: the previously isolated upstream virtual generic drop-glue ICE remains a blocker there.

This fresh CGA matrix contains no `FnPtrCast` or `VTableCalled` observations, so it establishes
no new coverage for those categories. Address flow is not execution. The retained compiler
corpus has features outside the bounded semantic subset; its successful capture is not a
claim of complete source-SMT semantics.

Earlier historical receipt availability remains explicit: 173 available, 258 unavailable.
Separate newly persisted MCP cross-checks do not restore those historical IDs.
