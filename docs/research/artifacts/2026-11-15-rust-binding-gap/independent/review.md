Independent read-only review completed for Beads **bd-j85a**, plan `e33b08d8-77e3-4b78-9442-5999c30717eb`, task `rust-binding-gap-evidence-review`, parent **bd-1uf4**. **Critical findings: none. Important findings: none. Minor findings: none.** These verdicts apply to the reviewed report and supplied evidence; the access and experiment limits below remain explicit.

Reviewed [the current report](/Users/kevintruong/project/SpurCode/docs/research/2026-11-15-rust-binding-gap.md), SHA-256 `d2f78250c7b44fd472d36185b937077efcdf528d464f73b30254ab7f7ca74e2d`. Native evidence was read directly. No parent script was executed, no production or notebook file was changed, and no workspace build, installation, upgrade, merge or issue closure was performed. Independent scripts, compiler outputs and receipts are confined to this directory.

Schema discovery preceded analytical queries. Both independent DuckDB connections were in-memory with the original database attached `READ_ONLY`; temporary relations and spills were local to the audit. [recomputed.json](recomputed.json) records each independent query, parameters and result. [schema.json](schema.json) records the initial schema discovery.

Independently obtained **259,828 Rust call rows and 259,828 distinct `(logical_path, edge_id)` keys**, **32,414 local targets**, and **227,414 unresolved**. The local-target fraction is **12.475175885585848%**, correctly rounded to **12.48%**. This measures the recorded local-target fraction; the report does not present it as compiler-call recall or accuracy.

| Lexical candidate scope | Zero | One | Multiple | Candidate pairs |
|---|---:|---:|---:|---:|
| Same file | 210,038 | 12,222 | 5,154 | 30,771 |
| Nearest Cargo package | 151,578 | 30,799 | 45,037 | 1,142,442 |
| All ingested Rust files | 90,529 | 37,939 | 98,946 | 9,778,507 |
| Exact package/scope/immediate owner | 220,907 | 6,373 | 134 | 6,662 |

Every row partition sums to **227,414**, and every candidate table contains **227,414 distinct original call keys**. The global inner join drops **90,529** input rows, generates **9,778,507** pairs, and has maximum fanout **566**. **25,654** global singleton sites carry receiver text. None of these candidates was inserted into the frozen database or treated as a verified binding.

Reconstructed the package catalog independently from `git ls-tree` and `git show` at `61ba3d200439a541dce4c1617ca9277be7d438af`: all **40 manifest hashes**, **39 package records**, and dependency aliases match the saved catalog. All **1,726** Rust inventory paths are covered by the path table; **7** paths have unknown package ownership and account for **240** of all call rows. There are no call rows missing from the inventory join. Package names, dependency declarations and lexical owners do not establish visibility, active configuration, aliases, generic instantiation or trait dispatch.

The overlapping unresolved hint counts reproduce: receiver **139,808**, scope **54,768**, qualified target **288**. The exclusive evidence partition reproduces: bare **46,940**, receiver without scope/qualified label **125,418**, scope without qualified label **54,768**, qualified label **288**. Null target labels and empty receiver/scope hints are zero. The spelling partition also conserves all unresolved rows: std/core/alloc **4,126**, crate **1,598**, workspace crate **2,372**, declared dependency alias **5,830**, other scope/qualified **41,130**, no scope/qualified **172,358**.

The real `compat_spike.rs` singleton examples reproduce: edge **33**, `std::env::args().skip(1)`, has only `spur-core/src/plan/outcomes.rs::skip` as a global name candidate; edge **146**, `std::env::args()`, has only `spur-embedding/src/discovery.rs::args`. Owner-hint examples reproduce for `AgentKind::from_name` (**151**), `SpurAgentCaps::new` (**159**, **194**), and `NativeAcpConnection::new_with_kind` (**173**). [final-checks.json](final-checks.json) verifies all **36** saved diagnostic caller snippets and line ranges against frozen source spans. These source examples support review, not a corpus precision estimate.

Compiler reproduction used the already-installed **rustc 1.94.1 (e408947bf)**, explicitly selected with `RUSTUP_TOOLCHAIN=1.94.1` from the temporary directory. [compiler-audit.json](compiler-audit.json) contains exact argv, working directory, environment overrides, return codes, stderr/stdout, provenance, and **all three full extension extractions**.

- Receiver control prints **`3:999\n`**. The independently generated MIR matches the saved MIR after substituting only the temporary path, and function-scoped inspection confirms `Vec::<u8>::len` in `observed` and `Domain::len` in `expected`. The extension retains `Vec<u8>`/`Domain` hints but leaves both targets null. The entire fresh extraction equals the saved receipt.
- Reconstructed the macro fixture from frozen bytes, then verified both relevant source files equal their Git revision bytes. `NativeLabel` spans bytes **4201–4258**, `labels!` definition **4340–4604**, invocation **4606–4967**, and `RelationKind` enum **15653–15864** in their respective source files. The invocation has `local_definition_candidate` provenance; it is not compiler expansion. Restored the actual following semicolon at zero-based byte **4967**. The isolated enum omits external derive attributes and the fixture appends a new `main`, as the report states.
- Independently reproduced the missing-semicolon compilation failure and retained its nonzero exit. Corrected original fixture compiles normally and prints all **14** expected labels: imports, calls, constructs, contains, implements, defines, references, extends, links, touches, produces, consumes, binds, emits.
- `RUSTC_BOOTSTRAP=1 rustc -Zunpretty=expanded` reproduces the saved expanded text exactly. Both full macro extraction structures equal the parent receipts: original **20 nodes/36 edges/15 macro invocations**, expanded **22 nodes/42 edges/14 macro invocations**. Generated `label` method bodies containing `match self` change from **0 to 1**. Expanded positions belong to the expanded artifact. This proves the isolated declarative control only; no procedural/derive or workspace-wide expansion coverage follows.

An initial version check in the temporary directory selected default rustc **1.96.0** and failed before compiling anything. That failed attempt is retained in `initial-default-toolchain-*`; selecting the existing 1.94.1 toolchain resolved it without installing or upgrading anything.

The fresh native connection reports DuckDB **2.0.0**, no `spur_graph` functions before `LOAD`, then exactly `spur_graph_file_facts`, `spur_graph_symbol_changes`, and `spur_graph_symbols`. Saved managed-runtime receipts identify DuckDB **1.5.5**, zero membership entries, and no runtime function match at observation. Saved graph cards support `duckgql`/`onager` availability and `duckpgq` unavailability only for **1.5.5/osx_arm64**. Graph results are paginated; the compiler result contains only `ducktinycc` and `luajit`. The report correctly limits these observations to returned cards, without asserting native2 compatibility, exhaustive Rust compiler absence, or semantic binding/expansion capability.

Live worker notebook MCP returned **“worker is not authorized to read this notebook or cell”**. No raw notebook was read. Saved `notebook-cells.json` shows successful Rust wrappers invoking the three pinned Python scripts; the refreshed evaluation stdout still reports the same **28** main receipts and control results. These are saved-receipt checks, not independent live notebook execution. A live `code_read_symbol` attempt exceeded its **15,000 ms** deadline; the newly cited native extraction receipt instead matches frozen source bytes verbatim with identical file OID `09476ea2c0d9b6dc6d16e4c9e81165edc4c97b1e`. Its fresh per-file `FactBuilder`, one extraction and `resolve_pending_edges` support the added file-local seam paragraph. The retained 1 GB OOM traceback and successful narrow-key receipts support the report's query-shape observation; no throughput benchmark or speedup was independently measured or claimed.

All **4,404** inventory Git blob OIDs match source revision `61ba3d200439a541dce4c1617ca9277be7d438af`. The original database and approved extension retain these SHA-256 hashes before and after:

```text
snapshot.duckdb: c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e
spur_graph.duckdb_extension: e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028
```

Of **35** initially hashed parent inputs, **31** remained byte-identical. During this audit the parent concurrently updated the report, `evaluate_gap.py`, `notebook-cells.json` and `notebook-evaluation.json`, and added an execution-provenance receipt. Those changes were not audit writes. The latest changes were re-reviewed: added seam explanation is verified above; runner records its own SHA; refreshed cell metadata preserves result values. [observed-parent-concurrent-changes.json](observed-parent-concurrent-changes.json), [input-hashes-before.json](input-hashes-before.json), [input-hashes-after.json](input-hashes-after.json), and the saved latest-parent copies identify exact reviewed versions. Latest execution provenance matches the runner, revision, DuckDB version, snapshot and extension. This observation was recorded as risk signal `65b3657e-4bb3-4fba-b1c4-fac451d850c5` on **bd-j85a**; parent review owns its disposition.

The worker tree has **empty `git status --short --untracked-files=all` and empty `git diff --stat HEAD`**, as expected for this task. No issue was closed and no approval or merge was performed.

Exact independent commands, each with exit **0** on the completed checked run:

```sh
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python /private/tmp/spur-rust-binding-audit-6mpf6brm/recompute.py
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python /private/tmp/spur-rust-binding-audit-6mpf6brm/compiler_audit.py
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python /private/tmp/spur-rust-binding-audit-6mpf6brm/final_checks.py
```

The compiler subprocess commands and full outputs are embedded in `compiler-audit.json`; complete query and conservation evidence is in `recomputed.json`. Scripts and their stdout/stderr are retained beside these receipts. The final check deliberately records the four observed parent metadata changes rather than claiming every input stayed unchanged. No additional work is required within this read-only review; parent approval and durable retention of these temporary audit artifacts remain parent responsibilities.
