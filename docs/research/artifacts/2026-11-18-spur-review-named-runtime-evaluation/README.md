# Named runtime review evaluation evidence

[Evaluation and acceptance sequence](../../2026-11-18-spur-review-named-runtime-evaluation.md) · [Exact observations and Solve requests](results.json) · [Captured diagnostic notebook](experimental-notebook.ipynb)

Live notebook: `/Users/kevintruong/project/spur-notebook/2026-11-18-spur-review-named-runtime-evaluation.ipynb`. Task: `bd-3k3i`; named Deno startup reproduction added to `bd-2dsf`.

This is an evaluation, not a released/native application. Seven code cells have successful saved outputs, one DuckPGQ cell failed, and one named Deno cell remained idle after MCP rejected startup. The port probe intentionally catches unavailable publication. All source edits, profile mutations and executions used notebook MCP. The captured notebook was read back through MCP before copying.

`results.json` retains current configuration contracts, save/preparation receipts, interpreter and artifact identities, observations, the initial/corrected Python 2.0 probe, DuckPGQ diagnostics, recursive SQL control, final profiles/DAG/doctor, and exact solver requests/responses. `verification.json` checks the capture and evidence without treating expected negative probes as passing app behavior. `capture-manifest.json` inventories task artifacts.

`fixture/` contains the exact copied captured PR design data plus dependency-free local Deno/Python configuration modules. The live profiles point to an isolated temporary directory recorded in the receipts. Replaying on another machine requires restoring the fixture to a chosen project directory, editing relative profile paths and explicitly qualifying an existing Python interpreter with DuckDB 2.0 and ipykernel. Do not install anything merely to reproduce a preview.

The Python 2.0 analysis cell precedes extension loading. The failing DuckPGQ traversal is retained for diagnosis; the separate recursive SQL cell reconstructs its own input tables and can run after native loading without relying on a failed cell's partial work. The standalone Deno control redisplays the existing captured HTML and does not run named Deno. The notebook defaults remain factory; every named code cell explicitly selects its profile.

Extension binaries remain at their existing local task-owned paths; only identities/receipts are captured. The C++ smoke uses previously captured binaries, not a new build. The review fixture and its historical stakeholders are not refreshed GitHub state. No named Python Arrow bridge, named Deno frontend, native app doctor success or complete repository linking is claimed.
