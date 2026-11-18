# Spur Review on named Deno and Python configurations

The analysis engine works under a named Python configuration with DuckDB 2.0.0. The complete named-configuration review app is **not qualified yet**. Named Deno startup fails through MCP, named Python publication is explicitly disabled, and the current review artifact has no native app manifest or reactive pipeline.

[Evaluation notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-spur-review-named-runtime-evaluation.ipynb) · [Captured evidence](artifacts/2026-11-18-spur-review-named-runtime-evaluation/README.md) · [Existing UX specification](../superpowers/specs/2026-11-18-spur-review-app-ux.md)

## What was tested

This is a fresh runtime evaluation of captured DuckDB PR #28731 evidence. It pins base `629e395a854d72dcb1f414e1a0451c68ac0e9f1f` and head `d29b26bb92ac940a0807be0f43cc385b31623839`. It does not refresh the PR, recollect history, rebuild extensions or re-index the repository. The fixture SHA256 is `b8e1a021c9460ffdbe0882c8a3363cdf99df9569d2f91590c2554707d965c0d2`.

| Experiment | Fresh observation | Practical meaning |
|---|---|---|
| Save named Deno `review-ui` | Stable UUID saved; notebook default remains factory | Portable intent is persisted |
| Execute named Deno | MCP rejects the saved slot as unknown/foreign/superseded before source execution; cell stays idle | Neither project imports nor a named frontend are qualified; existing issue `bd-2dsf` updated |
| Prepare/run named Python `review-analysis` | Existing interpreter qualified without installation; project module, cwd and SQL work | Python named execution works |
| Inspect its DuckDB | 1.5.5 | A named profile does not automatically select the notebook SQL runtime or compatible extensions |
| Prepare/run `review-duckdb2` | Existing `venv-duckdb-2.0` interpreter runs DuckDB 2.0.0 in a separate process | The required engine can be selected explicitly without installation |
| Load local extensions | DuckTails v1.9.0, DuckPGQ `9c11bc192a`, Spur Graph 0.1.0 load after checking all three captured binary SHA256 values | Loading is verified on the named 2.0 connection; the extension's DuckDB API version is 2.0.0 |
| Spur Graph C++ function | Two symbols extracted from a small helper/caller source; metadata reports `single_file` resolution | Native extraction works; repository semantic linking remains separate |
| Publish from named Python | Live contract has `ports_arrow:false`; `spur` is absent and the explicit publication probe catches NameError | No qualified Python-to-Deno data path |
| Query DuckPGQ | Both `CREATE PROPERTY GRAPH` and `CREATE OR REPLACE PROPERTY GRAPH` fail at `PROPERTY` despite loaded inventory | Extension loading alone does not qualify graph-query execution through this Python adapter; root cause remains undiagnosed |
| Recursive SQL control | Three incoming paths with candidates, one syntax-only path; all path witnesses resolve | Captured context is queryable with explicit evidence classes; this is not a passing DuckPGQ test |
| Standalone Deno display | Existing unchanged HTML emits `text/html` successfully | Captured design remains displayable; this is a separately labeled control |
| Native app doctor / DAG | Doctor fails Level 0 because `spur-app.json` is absent; zero nodes, edges or ports | The current delivery is a design artifact plus experiments, not a running native review app |

The initial 2.0 analysis probe used the 1.5 relation `fetchone()` API and failed with an explicit replacement message. The notebook probe was corrected to `fetchall()[0][0]` and passed. The original failure remains in the evidence. Python connection selection and 2.0 API adaptation both belong in the app preflight.

The initial relative interpreter path was calculated against `/var/...`, while the host canonicalized the project to `/private/var/...`. The probe's path calculation was corrected before qualification. This was an experiment setup correction, not evidence of a product defect.

## Mapping to the expected review journey

| User or agent task | Available evidence | Remaining requirement |
|---|---|---|
| Configure a local repository and runtimes | Named profile identities, relative project roots, explicit interpreter qualification | One preflight that shows selected profile UUID, Python executable, DuckDB ABI, extension identities, ports readiness and launch errors separately |
| Select a PR against the whole tracked tree | Prior capture inventories all 18,342 tracked head entries; this replay keeps five changed files and pinned revisions | Live GitHub/repository collectors must publish one immutable run; a complete inventory is not complete semantic linking |
| Understand impact and blast radius | Unchanged wrapper and two unchanged clients retained; candidate/syntax separation survives SQL traversal | Reliable repository resolution, provider-specific call/callback/contract/config edges, bounded queries and coverage/uncertainty reporting |
| See graph propagation | Captured graph edges retain source evidence; removing candidates removes two external-client paths | Qualify the chosen graph-query adapter and connect a selectable native graph/path list to exact evidence |
| Review architecture alignment | Six captured CMake components and source witnesses | Versioned component mapping and an explicit architecture policy; membership alone is not alignment approval |
| Identify stakeholders | Seven historical candidates; two GitHub logins mapped in the capture | Refresh requested reviewers, teams and declared ownership; preserve identity confidence and the distinction between history and present responsibility |
| Request AI review | Existing design previews a cited evidence packet | Explicit user-triggered job, configured provider, bounded packet, citations, cancellation/retry and persisted results; AI remains `not_run` |
| Share findings between human and agent | Twenty-one source evidence records; all twelve context-row references resolve in the fixture | Shared run/blob/span identifiers, stale-result rejection, validated tool outputs and persistent human selection/notes |

The useful incoming path is unchanged client → candidate overloaded-API reference → unchanged wrapper → syntax-bound changed implementation. When candidates are hidden, the app must explain that two paths were removed. It must not claim no impact. Current linking limits cannot be repaired by graph traversal or named configuration.

## Solve model and limits

The current catalog's implemented `configuration.requires_any` rule verifies required providers for a selected consumer. Each requirement here has one provider, so multiple bindings express the conjunction without incorrectly treating alternatives as mandatory dependencies.

Seven supplied qualification facts—named Python execution, DuckDB 2.0 runtime, three extension loads, C++ symbol smoke and captured-reference integrity—produce **sat / pass** for local engine smoke acceptance.

The named review app model additionally requires six qualified providers: named Deno execution, named Python Arrow publication, named Deno port consumption, Python DuckPGQ query execution, native app manifest/doctor and live collector qualification. These produce **unsat / fail** for the complete model. The six bindings individually fail. Named Deno consumption and live collectors are untested/unqualified rather than proven impossible. A false selection in this model means the acceptance evidence is absent or failed.

This is dependency validation over caller-supplied runtime observations. Z3 did not inspect the app, validate the C++ semantics, prove security or evaluate user experience. Exact requests and raw responses are retained in `results.json`. The solver's persistent artifact store was full at 512 artifacts, so execution used `persist:false`; there is no persisted `solve_id`.

## Next implementation and acceptance sequence

1. **Qualify named Deno startup.** Use the returned UUID/opaque slot through MCP without falling back to standalone. Test valid local alias/JSON/TSX imports and retain rejection of foreign or superseded slots. Surface a failed launch on the cell/profile instead of leaving an unexplained idle cell.
2. **Qualify the data bridge.** Implement and test host-fenced named Python publication plus asynchronous named Deno consumption. A changed source should produce a new port version and rerender the selected run; a late result from a superseded run must be rejected. Files/globals are not a substitute acceptance path.
3. **Qualify the DuckDB 2.0 execution adapter.** Assert interpreter/engine/ABI identities and probe actual extension functions and graph queries. Investigate the observed DuckPGQ parser failure before selecting this adapter for the app. Keep the notebook SQL runtime distinct from a named Python environment.
4. **Implement one native app journey.** Add the manifest and frontend to a dedicated app package: typed captured run → select changed symbol → show unchanged wrapper/client path → open exact witness. Validate the actual DAG, inspected outputs and full app doctor. Preserve the existing HTML artifact as the design reference.
5. **Add domain providers.** Collect tracked repository context and GitHub data into pinned runs; improve C++ resolution and preserve typed non-call relationships. Add architecture policy, current stakeholder enrichment and explicitly triggered AI review as separate observable stages.

The native frontend can first support an honest partial review. It should display analysis coverage and missing evidence and should never present an unexecuted architecture/AI check as passed. The first four steps establish a reliable application foundation; the domain providers then expand what a reviewer can conclude.

No production source was changed, dependencies installed, GitHub messages sent, or app packed. Both notebook family defaults remain factory. Human UI controls, restart/recovery and browser interactions were not newly validated. The evaluation intentionally retains one failed DuckPGQ cell and one named Deno cell that never executed.
