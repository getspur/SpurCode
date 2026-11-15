# Direct notebook SQL analysis of Rust local targets

The notebook now runs **DuckDB 2.0.0**. The approved `spur_graph` extension **0.1.0** loaded directly in its retained SQL connection, and notebook MCP independently observed `spur_graph_file_facts` registered there. Six fresh SQL extractions reproduced all five stored fact collections exactly. This phase uses the native notebook SQL connection; the earlier external-runner workaround is unnecessary for it.

The [notebook](/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb) contains a new section with extension setup, read-only attachment, package chart, binding evidence, target diagnostics, top callers/callees, source samples and compiler controls. [Saved cell sources and outputs](artifacts/2026-11-15-rust-local-targets/notebook-cells.json) and [SQL replay](artifacts/2026-11-15-rust-local-targets/replay.sql) preserve the exact work. Source revision is `61ba3d200439a541dce4c1617ca9277be7d438af`; the existing five dirty user files are preserved.

All **259,828 recorded Rust call edges** and their distinct `(logical_path, edge_id)` keys survive the joins. **32,414 have local target IDs (12.4752%)**, and **227,414 remain unresolved**. IDs must be joined with their logical path because they are local to a file. These are stored-edge counts, not raw textual call-site counts or a compiler accuracy score.

| Package | Recorded calls | Local targets | Local-target fraction |
|---|---:|---:|---:|
| spur-graph | 56,292 | 7,723 | 13.7195% |
| spur-core | 65,492 | 7,442 | 11.3632% |
| spur-tui | 42,979 | 5,650 | 13.1460% |
| spur-context-service | 17,904 | 3,054 | 17.0576% |

The full [package result](artifacts/2026-11-15-rust-local-targets/packages.json) contains 39 groups, using the previously validated nearest-Cargo-package catalog rather than directory names alone. The saved Perspective Y Bar view charts the fraction for groups with at least 100 recorded calls; the table retains all groups. SQL views `spur_rust_package_rates` and `spur_rust_caller_rates` support further exploration.

| Recorded binding method | Local-target edges | Share of local targets |
|---|---:|---:|
| singleton | 23,593 | 72.7865% |
| macro_body_singleton | 4,977 | 15.3545% |
| scope_match | 3,421 | 10.5541% |
| fqn | 276 | 0.8515% |
| same_file_duplicate | 64 | 0.1974% |
| null method / calls_dyn | 56 | 0.1728% |
| method_crate_singleton | 27 | 0.0833% |

Every local target exists, is a function or method, and has a name matching the call's final component. There are **1,149** records with multiple same-file function/method definitions bearing that name, and **89** self-edges. Those are review diagnostics, not error counts: the sampled self-edge in `merge_at` is legitimate recursion. A stored `syntax_exact` classification is not compiler proof of target correctness.

The constructor control demonstrates why that distinction matters. [The fixture](artifacts/2026-11-15-rust-local-targets/name-collision-control.rs) defines `LocalFactory::new` and calls `Vec::new()` inside `assert_eq!`. Direct SQL extraction stores a `macro_body_singleton` edge to **`LocalFactory::new`**. Actual **rustc 1.94.1 MIR** identifies **`Vec::<u8>::new`** in that function. The program compiles and prints `binding-control-ok`; the ordinary helper control resolves correctly. The separate `drop(guard)` fixture remains unresolved, while MIR identifies `std::mem::drop<MutexGuard>`. The constructor mismatch is confirmed; this fixture does not reproduce a wrong drop target.

[The comparison](artifacts/2026-11-15-rust-local-targets/name-collision-result.json), [compiler commands and assertions](artifacts/2026-11-15-rust-local-targets/name-collision-compiler.json), and [MIR](artifacts/2026-11-15-rust-local-targets/name-collision-control.mir) retain that evidence. Expectations were saved before extraction. Follow-up defect **bd-38yz** records the constructor collision; production code was not changed in this analysis.

The nine corpus samples include a macro-body record in `capability_evidence.rs` pointing to `EvidenceEpoch::new` where the caller's `new` expressions are `Vec::new`, and a bare `drop(active_guard)` record pointing to `Drop for NativeAcpConnection`. Complete caller/target bytes and node-owner identities are retained. All **18 source spans and starting lines** were independently matched against pinned Git bytes. There are **120** macro-body records targeting `new` and **27** local-target records named `drop`; these are candidate review populations, not counts of proven errors. No corpus precision or recall estimate follows from the deliberately selected samples and control.

Fresh `spur_graph_file_facts` calls on `lib.rs`, `file_facts.rs`, `tree_sitter.rs`, `agent_model_catalog.rs`, `schema.rs` and graph `mcp/mod.rs` match stored nodes, edges, spans, macro invocations and macro scopes in full. This validates the direct integration and reproducibility; it does not establish independent semantic correctness because both runs use the same extractor.

The original database remains read-only with SHA-256 `c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e`; extension SHA-256 is `e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`. No binding candidates were promoted, no whole-workspace compiler expansion was run, and no throughput improvement is claimed.

All 19 new SQL cells and the Rust compiler-control cell executed successfully. `notebook-checks.json` records the 2.0 runtime, loaded extension, conserved denominator, six-file parity and the one reproduced wrong-target control. The macro constructor defect remains open. On a SQL-kernel restart, rerun the load/attach/setup cells in order. The SQL replay uses the committed compiler receipt; rerun the saved Rust cell to regenerate compiler evidence. Native executable output is excluded from version control.
