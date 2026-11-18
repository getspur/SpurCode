# Spur Review: six requirement experiments and repair contracts

Two gates are now qualified: **DuckPGQ execution on named Python DuckDB 2.0** and **a reference app package with a rendered frontend and full Level 2 doctor**. The other four have concrete causes and regression witnesses. A live named review application remains unqualified.

[Experimental notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-spur-review-six-gates-lab.ipynb) · [Reference app](artifacts/2026-11-18-spur-review-six-gates/app/app.ipynb) · [Exact evidence and solver inputs](artifacts/2026-11-18-spur-review-six-gates/results.json)

This work follows the [previous evaluation](2026-11-18-spur-review-named-runtime-evaluation.md) and [review UX specification](../superpowers/specs/2026-11-18-spur-review-app-ux.md). The notebook is the experiment workbench. Production runtime source was inspected but not modified; no Rust build, extension rebuild, dependency install or app pack occurred.

## Results by requirement

| Gate | Result | What was established |
|---|---|---|
| R1 named Deno launch | Unqualified; cause located | Current MCP cold-start and lifecycle routing omit named Deno. The saved-profile cell still fails before source execution. |
| R2 named Python publication | Unqualified; implementation required | Importable `spur.put/get` deliberately raise `ports_unavailable`; authoring an Arrow producer is rejected before a cell exists. The origin experiment supplies a concrete concurrency regression witness. |
| R3 named Deno consumption | Blocked by R1/R2 | The project API is asynchronous. Its attempted consumer never executes. Stateful frontend composition also needs qualification because the project API lacks standalone `spur.anywidget()`. |
| R4 Python DuckPGQ queries | Qualified | Activating extension grammar on the same named 2.0 connection fixes graph queries; clearing it reproduces the failure. Both captured graph variants match recursive SQL. |
| R5 native app manifest/doctor | Qualified for reference package | Embedded app manifest and declared frontend/capability, successful display/cascade, full doctor Level 2. This is the captured UI, with no server or live port pipeline. |
| R6 live collectors | Components verified; orchestration unqualified | Existing `github-pat` query, fresh connector base/head, all tracked head entries, five-file diff and DuckTails history work. These stages are not yet an automatic coherent named-runtime pipeline. |

The original 13-binding qualification model now has **9 passing bindings and 4 failing bindings**. Its overall result remains `unsat / fail`. This is a model of supplied acceptance evidence, not a solver inspection of the running app.

## R4: the apparent driver failure was missing connection configuration

DuckDB 2.0 requires a connection to opt into extension SQL grammar after loading the extension. This is documented by both the [DuckDB 2.0 graph guide](https://www.duckdb.org/docs/2.0/guides/sql_features/graph_queries) and [DuckPGQ](https://duckpgq.org/).

```python
engine.execute("SET active_grammar_extensions = ['duckpgq']")
```

The named `review-engine` profile uses the existing DuckDB 2.0 interpreter. All three local binaries match the previous captured SHA256 identities. With grammar `[]`, `CREATE PROPERTY GRAPH` fails at `PROPERTY`. After the setting, both property graphs and path queries succeed. Clearing the setting makes `MATCH` fail again; the notebook then restores the working setting.

The captured incoming-context graph returns the unchanged wrapper at one hop and two unchanged clients at two hops. Its syntax-only graph returns just the wrapper. Both result sets exactly match recursive SQL. Candidate edges remain candidates; graph traversal does not supply missing C++ bindings. The final cell also verifies two native C++ symbols and all twelve context-row references.

This corrects the earlier undiagnosed Python-adapter hypothesis. The fix belongs in each app connection's bootstrap/preflight. A loaded-extension inventory is insufficient readiness evidence.

## R1: a narrow routing defect with a precise repair boundary

Current external source HEAD is `c686dcf6ce81d1cc517cd11f0db0edc7062ef9b7`; the lab records hashes and source excerpts. The graph index was stale, so current bytes were inspected after graph-first retrieval.

The cold path is `notebook_run_cell` → `RunCellCommandRunner::ensure_kernel` → `start_kernel` → lifecycle resolver. `src/dag/engine.rs:351` defers guarded startup for named Python and other native kernels but omits named Deno. `src/mcp/tools/native_profile_lifecycle.rs:136` enumerates C++/Rust/Python profiles, so a valid discovered Deno slot cannot match and reaches the exact unknown/foreign/superseded rejection at line 185.

The lower target resolver already understands JavaScript → Deno and desktop setup registers the production project bridge. Therefore the first repair is routing, followed by actual packaged execution qualification.

The focused TDD contract is:

- Construct a valid cold named Deno slot and call the actual `RunCellCommandRunner::ensure_kernel(KernelEnsureRequest {slot_id, spec_name})`; expect successful deferral with no process allocated by ensure. Current code takes the rejecting branch.
- Test explicit and inherited/default named Deno target resolution, factory Deno returning `None`, wrong notebook/UUID/spec and superseded-target rejection.
- Mirror Python's optional no-slot resolution for Deno. Enumerate named Deno with an **empty factory-spec list**. Defer only opaque named Deno in engine ensure.
- Do not blindly add factory Deno to generic native enumeration: its target is intentionally `None`, while the existing enumeration closure expects a native target.
- Run focused and relevant regression suites through the external repository's `scripts/spur-cargo`. Then rebuild the host and rerun the identical notebook alias/JSON/TSX probe, with provenance inspection. Pure resolver tests cannot prove the desktop bridge works.

This source fix is not implemented by the present investigation. `bd-2dsf` remains the live failure record.

## R2: qualify publication authority before enabling an API

Current `kernel_config/target.rs:223` rejects named Python `produces`, `consumes` and external `source` declarations. `runtime.rs:505` provides an importable stub. `backend/local.rs:500` strips factory port variables. Canonical publication fencing at `runtime_provenance.rs:180` and bridge registration at `src/dag/deno_publication_bridge.rs:256` are Deno-specific.

The lab confirms two independent boundaries. Explicitly importing `spur` changes the failure from missing global to deliberate `RuntimeError: ports_unavailable`. Inserting a named Python producer is atomically rejected, with no cell or port created. Neither a global assignment nor copied factory bootstrap is a repair.

The two-cell origin experiment is the key new regression witness. Cell A creates an async task and a plain thread, both held by events. Cell B releases them. The async task still reports A's message ID; the plain thread reports B's latest message ID. Thus a bridge trusting `get_parent()` alone can misattribute delayed work. No publication bypass was attempted and no actual mispublication occurred.

Catalog `data_integrity.mutually_consistent` checks the observed `(spawning cell, reported parent)` pairs against explicit same-origin tuples. The async observation returns `sat / pass`; including the plain thread returns `unsat / fail`. This proves the supplied tuples disagree, not that an unimplemented carrier is safe.

The implementation needs a qualified execution-origin carrier, process registration and Running receipt, then host admission/atomic commit/recovery using existing publication infrastructure. Keep notebook, profile UUID, process generation, fingerprint, run ID, source/version, selection, lifecycle and cancellation checks. Reject unbound work without independently admitted origin; also reject late results when the original run is no longer authorized, even when the async origin ID is correct. Enable the capability and lift authoring guards only after these paths pass.

Reuse `t10_host_*` in `src/dag/port_publication_tests.rs` and `t12_bridge_*` in `src/dag/deno_publication_bridge_tests.rs` for changed origins, lost registration, cancelled runs, malformed IPC and lost-reply recovery. Add the observed A/B thread case as a real interpreter regression, then test schema/null/order/empty batches on the actual named profile.

## R3: consumer and frontend contracts

The named Deno v2 contract uses `await spur.get(...)` / `await spur.getIPC(...)`. A synchronous factory read is not a drop-in replacement. The attempted consumer remains unexecuted because R1 fails first; R2 also supplies no qualified producer.

The project API currently exposes `api_version`, `put`, `putIPC`, `get`, `getIPC` and `publication`. It lacks `spur.anywidget()`. A stateful native frontend therefore needs an explicit composition decision and acceptance test; startup success alone would not qualify it. The reference HTML frontend avoids that helper but does not establish widget/comms behavior.

Acceptance must use named Python publication → versioned host port → named Deno asynchronous read → selected review view. Change the source, confirm a new port version and preserved human selection, and reject an older run's delayed result. Test actual project Arrow module/IPC compatibility without changing the declared data types or silently selecting factory kernels.

## R5: a real reference package with an honest validation level

The app was scaffolded through `notebook_app_init` as `vanilla-widget` / `frontend-only`. Its canonical manifest is embedded in notebook `metadata.spur_app`; it declares a frontend cell and active output scripts, with external workspace access disabled. The cell embeds the existing self-contained HTML design and emits one `text/html` output.

Opening the notebook returns an app identity in `notebook_context_pack`. The frontend cascade succeeds and its saved MIME output was inspected. Full doctor returns `ok:true`, `level_reached:2`. Warnings remain for absent Agent Plugin packaging and an unused port manifest; no MCP server is declared. There are no data DAG edges or live ports.

This qualifies the manifest/doctor gate for the captured reference. It does not qualify a connected named-runtime app, browser permission grants, new host interactions, or packaging. No `.spurapp` was created.

## R6: proven sources and the remaining orchestration

The lab reuses the saved `github-pat` definition, then queries PR #28731 through the managed SQL gateway. A fresh GitHub connector receipt supplies base/head and update time; they match the existing pinned comparison. The named 2.0 engine enumerates all **18,342 tracked head entries**, computes the real five-file Git diff and runs DuckTails history for both changed C++ files at the base revision. Complete inventory is not complete semantic indexing.

The inbox is a managed factory SQL connection; the graph/history engine is named Python 2.0. The agent explicitly materializes connector metadata into a cell. There is no invisible cross-process bridge and this is not an automatic collector service. The production collector must expose one run request, pin repository/PR/base/head identities, gather provider results with their coverage and failures, and finalize one immutable review packet before publication. A moved head must request a new run rather than mixing old and new results.

Reviewer fields returned `null`, so current assignments remain unknown. Historical author counts are expertise evidence, not formal ownership or reviewer assignment. Architecture policy and AI review remain outside the current gate qualification.

The connection attachment call unexpectedly dropped the saved provider/reference metadata while retaining the manifest apart from terminal newline normalization. The original fields were restored through `save_api_connection_template` then `attach_saved_connection` with the existing reference and empty supplied credentials. Supported listing verifies provider/reference restored, manifest semantically unchanged and status ready. No credential values were read into the notebook or rewritten. This tool behavior needs a regression: attaching an existing connection must preserve provider/reference metadata.

App authoring also exposed a tool-schema mismatch: `notebook_save` advertises string cells, while the live handler accepts structured Jupyter cell objects and rejects the strings. The valid structured request succeeded. The advertised schema and handler need a shared round-trip regression so agents can author app metadata from the published contract.

## Implementation order and human/agent acceptance

Treat runtime publication and the review application as separate implementation scopes. The narrow R1 routing fix can be tested independently. In parallel, qualify the Python origin carrier before extending host publication. R3's cross-kernel acceptance depends on both. Collector code can be developed against immutable packets while publication remains gated. The existing captured app is the UI reference for final integration.

For humans, preflight must distinguish Saved, Prepared, Running and Ports ready; show coverage and actionable errors; preserve selected file/symbol/evidence across refresh. For agents, return typed stage results and exact profile/run/source identities, make tools idempotent, and report unavailable stages without implying success. Both must consume the same immutable review packet and citation IDs.

The next source work should be TDD from the concrete notebook witnesses, with isolated file scopes and a separate runtime design for Python publication. This investigation supplies repair contracts and evidence; it is not an executed multi-subsystem implementation plan.
