SPUR plus exact source predicates and SOLVE/SMT/Z3 can support useful bounded panic analysis without compiling the whole application through CGA. The experiments here validate that approach on three source-bound controls and expose two boundaries: call topology does not encode guard feasibility, and the current source graph still omits compiler-observed self-calls. This is an evaluated approach, not a production panic detector.

Rationale: use the graph to select candidate sites and paths, recover a small explicit semantic model from exact source, then ask the solver for a counterexample. Keep missing semantics and compiler uncertainty visible throughout.

The [experiment notebook](../../../../2026-10-31-panic-graph-experiment.ipynb) contains the live MCP cells and SQL tables. [Worker model report](models/REPORT.md) details the catalog requests, assumptions and solver receipts. This root report adds actual extension execution, source snapshots, fresh SMT execution and independent Rust runtime controls. The worker report's statement that it did not run Rust remains historical; root subsequently ran the separate harness recorded here.

**Actual extensions and graph comparison.** The notebook uses DuckDB 1.5.5 with `petgraph_ext` v0.5.0, `duckrouting` 509ece1, and JSON/Parquet. All were already installed; root loaded Petgraph and used the existing Duckrouting installation. No install or upgrade was needed. Runtime signatures and directed positive/negative controls preceded the graph queries. Petgraph supplies SCC and shortest-path functions; Duckrouting supplies independent Boost-based graph operations ([Petgraph documentation](https://duckdb.org/community_extensions/extensions/petgraph_ext), [Duckrouting documentation](https://duckdb.org/community_extensions/extensions/duckrouting)).

Across **16** graph/policy variants, Petgraph SCC, Duckrouting SCC and an independent iterative Kosaraju implementation agree on the complete partitions. **32** directed path comparisons agree with independent BFS; **24** exercised both extensions, while eight have an isolated endpoint and are explicitly unreachable without adding artificial edges. Dense IDs are mapped back to stable IDs. Isolates are retained as singleton components, not manufactured self-loops. Full results and frozen graphs are in `notebook/metrics.json`, `paths.jsonl`, `partitions.jsonl.gz` and `graphs.tar.gz`.

| Source-identity cohort | Definitions | Source pairs / CGA Called pairs | Source / CGA nontrivial SCCs | Source / CGA self-call pairs |
|---|---:|---:|---:|---:|
| Successful library | 410 | 408 / 346 | 1 / 1, same members | 0 / 4 |
| Successful example | 449 | 535 / 569 | 1 / 1, same members | 0 / 6 |
| TUI dependency, failed CLI build | 2,472 | 2,603 / 3,315 | 2 / 3 | 0 / 14 |

These compiler graphs are strictly source-matched definition projections, not full CGA exports or global accuracy denominators. `compiler_called_root` is a separately reported policy, with 206/566 primary pairs. `Called` edges may sit behind uncertain virtual context. There is still no full CLI-target graph. The previously reproduced CGA substitution defect is unchanged.

All four library and six example self-call pairs use the **same raw compiler instance** for caller and target; they are not artifacts of collapsing different monomorphizations. Examples include `StringPruningPredicate::leaf`, `collect_response_file_paths`, `mcp_tools::walk`, and `collect_nodes_by_kind`. These are missing compiler-observed self-call pairs, not proofs of infinite recursion. The raw-reference projection and names are retained in `self-loop-evidence.json`.

The whole published direct-call projection has 39,728 pairs and 22,176 endpoint IDs. The Rust-file subset has 38,663 pairs, 21,324 endpoints, 18 nontrivial SCCs and three self-loop vertices. All **43** IDs absent from the symbol-only `nodes` table resolve to `files`, including 13 Rust file owners. A symbol-only foreign-key check would falsely reject legitimate file ownership. File ownership still does not establish a precise executable body. The manifest has no indexed commit; frozen table/blob provenance, rather than an invented graph HEAD, binds these results.

Removing the three previously adjudicated wrong receiver pairs changes relevant shortest paths but leaves the SCC counts unchanged. Topological agreement is therefore insufficient to certify target binding. The source frontier also remains a candidate filter: in the matched library domain, 12 definitions own a macro-path or unresolved unwrap/expect candidate, and 45 source definitions have a structural path to one, versus 35 in the compiler-identity projection. The example gives seven owners and 38/45 reachable definitions. These are same-domain structural counts, not runtime panic reachability. Labels may denote custom methods or shadowed macros; cfg, guards, body ownership and external targets remain incomplete.

**The decisive guard experiment.** The two 98-byte Option sources differ by one operator. Their actual source-sidecar snapshots have the same function layout, call topology and `value.unwrap()` site at bytes 73..87. Source blobs and graph hashes are distinct and preserved. We do not claim the entire graph artifacts are identical.

```rust
let value = if x == 0 { Some(x) } else { None };
// Safe control:                 if x == 0 { value.unwrap() } else { 0 }
// Changed-guard control:        if x != 0 { value.unwrap() } else { 0 }
```

For `x:u8`, the source-normalized predicates are:

```
safe:          0 <= x <= 255 AND x == 0 AND NOT (x == 0)
changed guard: 0 <= x <= 255 AND x != 0 AND NOT (x == 0)
checked_add:   0 <= x <= 255 AND x + 1 > 255
```

| Exact fixture | SOLVE result | Inputs satisfying panic predicate | Fresh Rust panics / inputs |
|---|---|---:|---:|
| `option_safe.rs` | UNSAT, no encoded counterexample | 0 | 0 / 256 |
| `option_panic.rs` | SAT, witness `x=1` | 255 | 255 / 256 |
| `checked_add.rs` | SAT, witness `x=255` | 1 | 1 / 256 |

Root compiled the dependency-free [runtime harness](notebook/runtime/main.rs) through `SPUR_REMOTE=0 scripts/spur-cargo`, including the three fixture files verbatim. Ordinary rustc is nightly-2026-10-18, commit `8006091eb71ae7d9e9bccb66c6122c9d68af4466`, aarch64-apple-darwin, dev profile, panic=unwind, debug assertions and overflow checks on. All **768** panic outcomes match the independent finite oracle; successful return values were also checked. No application or CGA build ran. The wrapper emitted a worktree-path warning, retained in stderr, while Cargo and the harness exited zero. Exact command, compiler identity, binary hash and outcomes are in `runtime-command.json`, `runtime-rustc.txt`, `runtime-verification.json` and `runtime-results.jsonl`.

This supports the chosen normalized semantics for these exact sources, inputs and build. It is not automatic Rust semantic extraction, production detection coverage, or unbounded safety. `checked_add` uses its explicit Option contract. A separate modular bit-vector wrap control is arithmetic evidence, not a claim that release-mode wrapping itself panics. `checked_add_guarded` adds an extra caller guard absent from the fixture; its UNSAT result is conditional on that assumption.

**SOLVE, SMT and conflict diagnosis.** Catalog discovery selected 11 relational-integrity/workflow cases; nine guard/arithmetic cases used the typed generic language after exact preflight. Every result was persisted and reloaded. The worker independently checked 5,376 finite assignments and all returned SAT models, with UNSAT-core deletion checks. Root separately reloaded six key results through MCP, then the notebook ran **20 fresh Z3 4.16.0 formula executions**: 10 SAT, 10 UNSAT, no solver UNKNOWN/timeout. Raw `solve_smt` was unnecessary because the typed surface expresses these formulas.

SOLVE returned 19 SMT scripts; one cached workflow result omitted its requested SMT. For that case, root verified the exact family request and lowered typed hash equal the address-excluded case before reusing its returned script, explicitly marked in the replay. Declarations, assertions and `check-sat` are unchanged. Only post-check diagnostics are selected appropriately (`get-model` for SAT, `get-unsat-core` for UNSAT). No expected status is asserted into a formula. See `notebook/replay_smt.py` and `smt-replay.jsonl`.

The named conflict core is `require_zero, forbid_zero`. The safe Option core contains its constructor relation, guard relation and panic predicate. A core explains a conflict among encoded requirements; it does not identify which source assumption is wrong. Removing the constructor or guard yields raw SAT with free Booleans, but source panic feasibility is **UNKNOWN because facts are missing**. This is different from a solver returning `unknown`; none of these 20 solver executions did so. Catalog verify SAT/pass and generic counterexample SAT have different meanings.

**An appropriate SPUR-first implementation.** The recommended pipeline is:

1. Validate source/build identity, `nodes ∪ files` endpoint integrity, precise body ownership and named edge policies. Fix the demonstrated lexical self-call and receiver-binding gaps with positive and negative regressions. An SCC candidate is neither deadlock nor nontermination evidence.
2. Use extensions for candidate SCCs, paths and reverse reachability. Preserve unresolved candidates and opaque calls. Never treat address flow, a wrong target, a stale source or an unobserved compiler node as execution evidence.
3. Add a narrow source semantic sidecar: stable callsite/body IDs, typed input domains, value origins, Option discriminants, branch guards/dominance and recognized primitive contracts. Each constraint needs its source blob/span and derivation. Start with the three runtime-validated fixture shapes; reject custom/shadowed `unwrap` lookalikes or mark them unknown. The current graph columns do not contain these predicates.
4. Ask SOLVE for `build/cfg AND path/transition facts AND guards AND panic precondition`. Retain explicit loop/call bounds and opaque obligations. SAT is a candidate counterexample under those assumptions; replay it. UNSAT excludes counterexamples only in that complete encoded domain. Missing facts, unknown or timeout remain inconclusive.
5. Use CGA or a compiler semantic sidecar for unresolved traits, monomorphization, macro expansion, generated bodies and implicit checks. Its call graph improves target evidence but still supplies no path predicates in the retained export. A full trustworthy CLI export additionally requires fixing the known producer defect. This compiler lane validates and expands the source-first lane; it cannot be replaced by graph algorithms or a guessed predicate.

An executable detection record should retain `source_oid`, callsite/body identity, build/cfg, edge provenance, typed variables, constraint/source-span bindings, unknown obligations, exploration bounds, raw solver status/core/model, and replay outcome. This is the concrete gap between a useful candidate graph and reviewable mathematical detection.

**Replay and review.** The full payload remains local. Worker commit `b2f06eca1f3b651a8b0ed942c4a5608efc1f21ea` is preserved. Automatic approval review rejected the worker's large Beads completion append containing source-derived facts and full solver receipts. Root did not resend that payload; compact completion comment 1034 records only the local artifact/commit reference. The rejection and detailed receipts remain under `models/audit` and `models/receipts`.

Offline retained-evidence verification needs only Python's standard library and does not read CURRENT or a notebook file:

```sh
python3 docs/research/artifacts/2026-11-01-spur-math-panic/notebook/frozen_replay.py
```

Fresh Z3 formula replay:

```sh
python3 docs/research/artifacts/2026-11-01-spur-math-panic/notebook/replay_smt.py \
  --models docs/research/artifacts/2026-11-01-spur-math-panic/models \
  --out /private/tmp/spur-math-z3-rerun
```

`analyze.py` reruns the actual extensions against the pinned raw Parquet and prior source-bound projections. It records schemas before queries, uses a fresh in-memory connection, then seals 64 frozen graph members into a compressed archive. `partitions.jsonl` is an ignored interactive cache; the portable artifact is `partitions.jsonl.gz`. The source-only snapshots and runtime runs are notebook cells with durable MCP receipts. Only research artifacts and the experiment notebook change; the five protected user edits, published graph and original CGA binaries are preserved.
