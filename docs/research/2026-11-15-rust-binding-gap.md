# Rust binding and macro expansion evaluation — 2026-11-15

The DuckDB extension remains useful for source extraction and SQL analysis. This experiment confirms two missing semantic layers: workspace call binding and compiler expansion. The 12.48% local-target rate is a property of recorded syntax facts, not measured compiler-call recall or accuracy. A higher rate produced by name matching could make retrieval worse.

Experiments ran from Rust cells in `duckdb-temporal-performance.ipynb` (`nb-0532cd73b116e20ad9508402`), through pinned native DuckDB 2.0.0 and the approved `spur_graph` extension. Source is frozen at `61ba3d200439a541dce4c1617ca9277be7d438af`; the original database was attached read-only. All evidence below lives in [the artifact directory](artifacts/2026-11-15-rust-binding-gap/).

The baseline is unchanged: **259,828 recorded Rust calls; 32,414 local targets; 227,414 unresolved**. Of unresolved calls, 139,808 have receiver text, 54,768 have scope text, and 288 have a qualified target label. These fields overlap. A separate exclusive partition retains every unresolved row: 46,940 bare; 125,418 receiver without scope/qualified label; 54,768 scope without qualified label; 288 qualified label.

The extraction seam helps explain the limit: `BytesExtractor::extract_file_facts` creates a fresh `FactBuilder`, extracts one file and calls `resolve_pending_edges` on that builder. Its [previously verified native symbol receipt](artifacts/2026-11-15-spurcode-project-quality/native-extract-file-facts.json) preserves the exact body and file identity. SQL batching and worker-local parser reuse do not make this file-scoped builder a workspace resolver.

SQL compared each unresolved call's final `::` component with extracted function/method labels. This deliberately measures candidate multiplicity, not valid resolution. The inventory includes test/example/vendor files and alternate configurations; same-package membership does not establish visibility or the active compilation unit. Constructors and enum variants are outside this function/method candidate set.

| Candidate scope | No candidate | Exactly one | Multiple |
|---|---:|---:|---:|
| Same file | 210,038 | 12,222 | 5,154 |
| Nearest Cargo package | 151,578 | 30,799 | 45,037 |
| All ingested Rust files | 90,529 | 37,939 | 98,946 |

The nearest-package map comes from frozen Cargo manifests, including nested packages. Package context is unknown for 240 of all recorded call rows. Those rows cannot acquire a same-package candidate. The analysis preserves `(logical_path, edge_id)` and checks all 259,828 unique call keys; no inner join silently removes unresolved calls.

A naive global inner join would drop **90,529** unresolved calls and produce **9,778,507 candidate pairs**, with up to **566** targets per call. Even the 37,939 singleton names are unsafe: 25,654 have receiver text. In real source, `std::env::args().skip(1)` has a singleton `skip` candidate in `spur-core/src/plan/outcomes.rs`; `std::env::args()` has a singleton `args` candidate in `spur-embedding/src/discovery.rs`. These examples illustrate name collisions, without claiming a compiler audit of the whole repository.

An exact lexical join of `(package, scope_text, target_label)` against a callable's immediate `impl`/`trait` owner label gives **6,373 singleton candidates**, **134 ambiguous sites**, and **220,907 with no match**. Real examples include `AgentKind::from_name`, `SpurAgentCaps::new`, and `NativeAcpConnection::new_with_kind` in `compat_spike.rs`. These are useful review candidates. The experiment does not verify module visibility, aliases, generics, traits, `cfg`, or the correctness of every inferred scope string, so none were promoted into the graph.

Explicit spelling also cannot classify the whole unresolved population as library calls. A precedence-ordered diagnostic found 4,126 `std/core/alloc` root spellings, 1,598 `crate` spellings, 2,372 workspace-crate spellings, 5,830 declared dependency-alias spellings, 41,130 other scope/qualified spellings, and 172,358 without either hint. Optional/dev/build/target dependencies are declarations, not proof they participate in a particular build. Common unresolved labels such as `expect`, `Some`, `Ok`, and `to_string` further show why the denominator is broader than missing project function definitions.

The isolated receiver control was specified before observation in [compiler-expectations.json](artifacts/2026-11-15-rust-binding-gap/compiler-expectations.json). It defines one project `Domain::len` returning 999, calls `v.len()` for a `Vec<u8>` and `d.len()` for `Domain`, and prints `3:999`. Actual rustc 1.94.1 MIR identifies **`Vec::<u8>::len`** for `observed` and **`Domain::len`** for `expected`. The extension leaves both targets null while retaining `Vec<u8>` and `Domain` scope hints. Thus a same-name singleton would introduce an incorrect Vec edge; an owner-aware enrichment has useful evidence for the Domain edge. This is a controlled counterexample, not an estimate of corpus precision.

The compiler expansion control uses the exact frozen `RelationKind` enum span, `NativeLabel` trait, `labels!` definition, and one real invocation from the extension adapter. It omits the enum's preceding external derive attributes, restores the invocation's following source semicolon, and appends a new executable `main`. The original invocation evidence range excludes that semicolon; the first fixture failed compilation until it was restored from frozen byte 4967. That failure is retained rather than counted as a successful run.

| Observation | Original fixture | Actual rustc expanded source |
|---|---:|---:|
| Extracted `label` function/method bodies containing `match self` | 0 | 1 |

`RUSTC_BOOTSTRAP=1 rustc -Zunpretty=expanded` supplied real compiler-expanded text for this isolated declarative macro. A separate normal compilation of the original fixture ran successfully and checked all **14 enum-to-label outputs**. The expanded source is a separate artifact with separate positions. Macro definition candidates and invocation ranges alone had not supplied that generated method. This does not establish procedural/derive expansion, active workspace features, generated-source origin mapping, or compiler coverage of SpurCode.

The extension MCP inspection was deliberately separated from the native runner. The notebook's managed SQL connection is DuckDB **1.5.5**, with zero extension membership records and no registered `spur_graph_file_facts` match at observation. Native DuckDB **2.0.0** had no `spur_graph` functions before `LOAD`, then exposed `spur_graph_file_facts`, `spur_graph_symbols`, and `spur_graph_symbol_changes`. No managed runtime was upgraded or modified.

Catalog discovery returned graph packages such as `duckgql` and `onager` available for the managed 1.5.5/osx_arm64 target; `duckpgq` was unavailable for that target. Their observed cards describe graph processing, without demonstrating Rust name resolution or expansion. The bounded `compiler` search returned TinyCC for C and LuaJIT/Fennel, not a Rust compiler service. These observations are saved MCP receipts, not a compatibility test on DuckDB 2.0.0 or an exhaustive package evaluation.

For the next implementation, keep file facts as immutable columnar extraction output. Materialize package/module/import/owner tables and put SQL candidates in a separate relation carrying evidence, ambiguity, extractor version, and source revision. Obtain compiler-backed identities, trait targets, and macro expansions through a semantic sidecar keyed by build configuration; store that provenance separately from syntax facts. Retrieval should expose verified bindings and uncertain candidates distinctly. Graph-query extensions can then consume those tables, but graph traversal does not supply missing Rust semantics.

The notebook also exposed a practical query-shape issue: joining wide source blobs before limiting diagnostic samples exceeded the configured 1 GB memory cap. Materializing 12 narrow sample keys before the source join succeeded under the same cap. The original failure log is retained. This is evidence for early narrowing in diagnostic queries; it is not a parsing-throughput benchmark or a measured speedup.

Reproduce the main evaluation and owner diagnostics with the pinned interpreter (the notebook cells execute these same commands):

```sh
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python docs/research/artifacts/2026-11-15-rust-binding-gap/binding_pilot.py
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python docs/research/artifacts/2026-11-15-rust-binding-gap/evaluate_gap.py
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python docs/research/artifacts/2026-11-15-rust-binding-gap/owner_diagnostics.py
```

[binding-analysis.json](artifacts/2026-11-15-rust-binding-gap/binding-analysis.json) contains the exact 28 main SQL receipts and complete control extractions. [owner-analysis.json](artifacts/2026-11-15-rust-binding-gap/owner-analysis.json) records the additional SQL. [compiler-controls.json](artifacts/2026-11-15-rust-binding-gap/compiler-controls.json) retains commands, compiler version, outputs, fixture provenance, and interpreted results. Original snapshot SHA-256 remains `c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e`; no candidates or compiler-expanded facts were inserted into it. Production code is unchanged.

The [independent review](artifacts/2026-11-15-rust-binding-gap/independent/review.md) is approved with no critical, important or minor findings. It independently reproduced the call/candidate partitions, all three complete control extractions, exact expanded macro text and all 14 runtime outputs; checked all 36 source samples and all 4,404 inventory blob identities; and verified snapshot/extension preservation. The worker reviewed saved notebook receipts because live notebook authorization was unavailable to it; a subsequent [parent live MCP check](artifacts/2026-11-15-rust-binding-gap/parent-live-notebook-check.json) confirms the three saved cells still match their live source, execution counts and stdout with empty stderr. Intentional parent metadata updates during review were re-reviewed and retained in the hash ledger. The [approval record](artifacts/2026-11-15-rust-binding-gap/independent-review.json) preserves the exact reviewed report hash, scope limits and risk disposition. This approval adds no compiler coverage or binding claims beyond the experiments above.
