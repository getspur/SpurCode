# New spur-notebook build recheck

[Report](report.html) · [Chart](before-after.png) · [Summary](summary.json)

Graph: **spur-graph-schema-v11**, hash fcf40b9cd5785e09a0dbf3e45167f6e5f599348a44a1e184c7ff78d3e8e5cfec.

| Coverage | Previous published graph | New published graph |
|---|---:|---:|
| Source-matching Rust files | 651 / 689 | **689 / 689** |
| Stale / unindexed Rust files | 17 / 21 | **0 / 0** |
| Callable declarations represented | 11,517 / 12,682 | **12,682 / 12,682** |
| All declaration kinds represented | 23,197 / 26,507 | **26,049 / 26,507** |
| Persisted macro invocation correspondence | unavailable | **28,362 / 28,362** |

Callables here mean ordinary function, method and trait-method declarations. Foreign-function declarations are tracked separately.

All 689 Rust source hashes match the previous measurement exactly. This comparison reuses the prior independent syn 2.0.118 physical AST after verifying its archived hash and all current source hashes. It does not change the source denominator to favor the new build. Baseline artifact commit: c4195cb8c221e0fa75e0ed8391004db76f88667d, sibling directory 2026-11-02-rust-graph-coverage. Full AST parsing covers 688 files; the directive-bearing ports_bootstrap.rs fragment is excluded. Eleven extra persisted macro rows belong to that fragment and are excluded from numerator and denominator.

Native Rust reserialized all 28,373 macro invocations and 8,516 scopes using the production serde types. BLAKE3 checksum **0407e9c2a3e07f963d5aeca0a9ae9b49cf9c058871f78a4c269150d8f445e3db** matches the manifest. The experiment also verified all 13 manifest row counts against Parquet footers, both sidecar counts, macro schema metadata, exact source OIDs, scope containment, caller references and unique correspondences.

The 458 remaining symbol gaps are **446 tuple fields, nine foreign-function declarations and three extern blocks**. Their parent bytes may still be present; direct inspection found no individual matching nodes at these locations. The count is larger than the prior 376 eligible gaps because the previously stale/unindexed files are now checked.

The current artifact persists macro sites but contains no method/free-call invocation identity table. Complete compiler semantics, expanded code, Cargo features, call-target correctness and panic completeness remain unmeasured. Macro family tags include 622 panic candidates, 16,632 assertion candidates and 14 debug-assertion candidates across all physical cfg branches/test contexts; those are source candidates, not proven bugs.

The graph reports 9,229 diagnostics; category counts are retained in summary.json. Sample warnings concern historical ambiguous renames. This coverage verification does not adjudicate temporal rename accuracy.

The connected project registry still points to unavailable /Volumes/Projects/spur-notebook. Graph-first lookup was attempted and failed; schema-discovered read-only Parquet access used the actual ../spur-notebook path. Registry configuration was not changed.

The target repository HEAD, Git status, all source bytes, graph pointer and manifest remained unchanged. No project build or production change was performed. The separate executed notebook is 2026-11-02-spur-notebook-new-build-check.ipynb at the SpurCode root.

Data: declaration-comparison.parquet and remaining-gaps.csv provide per-source-line differences; macro-coverage.parquet contains the exact AST-site correspondence. The original macro checksum input is compressed with recorded hashes. The prior baseline evidence archive is required for fully reproducing the AST comparison. Run verify.py from this directory with Python/pandas/pyarrow to recheck the saved counts, bindings and correspondence.

Scope-table accounting: 630 files contain macro invocations and have file-scope rows. The other 59 files have zero macro sites in the independent AST. The collector stores invocation ancestors, so a zero-site file has no scope row; its source identity remains in the file manifest.
