# Spur Graph real-repository probe

User-approved notebook probe `bd-361m`, 2026-11-17. See [report.json](report.json), [analysis.json](analysis.json), [lifecycle.json](lifecycle.json), and the executed notebook in `getspur/spur-notebook/docs/research/2026-11-17-spur-graph-real-repo-probe.ipynb`.

The frozen component is `app_gallery/code-graph-workbench/` in `spur-notebook`: head `07db59f08dbced2c8bedd17290c78f57b428a2bc` against first parent `cd24d5c4779d11a5b2dd1bbe55c93da12856ae41`. All 25 base and 26 head UTF-8 file versions were admitted. Seven of the complete commit's 12 changed paths belong to this component. The current dirty working tree is excluded.

The accepted unsigned/local macOS ARM64 extension (`28ddce83…`, source `ea849cf9…`) ran on native DuckDB 2.0.0. Git blob OIDs and `sha256:utf8-v1` source identities are verified separately. Published source labels remain honestly `caller_supplied` in the generic consumer; the outer probe independently verifies their Git origin.

Observed integrity: 51 source occurrences, 32 cache keys, zero cache additions on replay, ten typed/checksummed exports. An unsupported `requirements.txt` remains visible once per snapshot, so extraction coverage stays partial. A fresh Python AST check matches 265 named-definition occurrences. The 373 exported symbol ranges are in bounds and UTF-8 safe.

**Readiness gap:** raw notebook graph facts contain 17 base / 11 head functions, while raw notebook symbol extraction returns zero on both sides and file-pair change matching returns zero changes/diagnostics. `bd-3ct2` tracks this issue. Standalone cell extraction/comparison is a diagnostic witness, not an implemented notebook matcher. Notebook function coordinates refer to decoded containing-cell source; 28 function anchors were checked this way.

The 31 native change records classify as 22 identical declaration spans moved, four changed source spans and five added records. Identical declaration bytes do not prove unchanged behavior; parent/child ranges overlap. Keep Git/cell diffs primary in the app.

A candidate note is bound to exact source, symbol, query and publication identities; it reopens in a fresh process. Probe context checks mark other supplied Git snapshots historical. Deliberate child exit 23 after candidate creation preserves the prior pointer and all eleven published files; the child is reaped and staging removed. These are headless checks, not a production note API, automatic UI stale-state protection or successful-refresh/history acceptance.

Reproduce by running the notebook in its saved native DuckDB 2.0 runtime. It freezes the Git sources, invokes the existing isolated consumer, and runs `native-analysis.py` and `native-lifecycle.py`. The scripts do not execute the analyzed application's source or project tests. The derived notebook-cell source records came from notebook MCP reads of byte-identical Git notebook copies and were independently corroborated.

Small reports, scripts and typed Parquet results are retained in Git. The generated raw fixture, frozen notebook copies and writable DuckDB database remain local; the notebook can regenerate inputs from the pinned Git objects. Package/native runtime paths are specific to this workspace. No native source/app implementation, public push or deployment occurred.
