The graph contains useful structural call evidence, but three adjudicated receiver mistakes and missing callsite/body identities prevent treating it as a reliable execution graph. CGA supplies compiler observations under a pinned build; its reproduced virtual-substitution defect prevents treating it as an unrestricted oracle. The [parent report](../README.md) contains the broad ledger, thirteen targeted witnesses and ranked proposals. This root review adds independently recomputed mapping, root support, and four source-snapshot controls, all operated through the experiment notebook.

Rationale: source identity, call-target correctness, root support and runtime feasibility are separate questions. Every measurement below keeps those questions distinct.

| Successful target capture | Strict mapped definitions | Source pairs | CGA Called pairs | Shared | Source-only | CGA-only |
|---|---:|---:|---:|---:|---:|---:|
| Library | 410 | 408 | 346 | 243 | 165 | 103 |
| panic_evidence example | 449 | 535 | 569 | 476 | 59 | 93 |

The cohort requires exact graph Git blob, declared capture-input SHA256, and unique definition-start byte identity. Monomorphized instances collapse to source definitions. Source edges require both `relation=calls` and `edge_kind=calls`; compiler observations require `Called` and two strict endpoints. These are scoped set overlaps, not global precision or recall. CLI dependency exports are secondary: their overall build failed and no `spur::main` graph exists. The source graph has no callsite coordinates, and the indexed commit is null.

Even an individual `Called` edge may only be reachable through an uncertain context. Taking the best support over instances of each strictly matched pair gives:

| Capture | Called-only root path | Needs virtual candidate | Needs address-flow edge | Total strict pairs |
|---|---:|---:|---:|---:|
| Library | 206 | 137 | 3 | 346 |
| Example | 566 | 0 | 3 | 569 |
| CLI graph dependency | 206 | 137 | 3 | 346 |
| CLI TUI dependency | 3,053 | 260 | 2 | 3,315 |

The 137 library pairs are not declared false; their context needs indirect-dispatch qualification. `FnPtrCast` is address flow, and cannot itself establish an executable path. A library cohort restricted to definitions with Called-only root support has **276 source pairs, 206 compiler pairs and 151 shared**; the example has **534, 566 and 475**. Source membership means both endpoint definitions are Called-reachable in the compiler export. It does not prove that each source edge executes. Root paths remain static candidates without guards, evaluation order, CFG or async scheduling evidence.

Three receiver mistakes are supported by exact source/capture bytes, receiver declarations and raw calls: `Vec::iter` incorrectly targets `PersistentMap::iter`; `Instant::elapsed` incorrectly targets `SessionDetailView::elapsed`; and `OsStr::to_string_lossy` incorrectly targets `GitPath::to_string_lossy`. The elapsed edge has `method_crate_singleton` provenance and `syntax_exact` confidence 1.0. Removing precisely that false pair changes its seed's source reachable set from two nodes to one. Extraction confidence cannot substitute for target-binding evidence. Separately, 138/165 library and 57/59 example source-only pairs have lexical closure/coroutine candidates; these aggregate candidates are not all proven ownership errors. Two fixed witnesses establish actual outer-owner loss.

| Bounded retained control | Method sites | Source call rows | Resolved pairs | calls_dyn rows | Retained CGA result |
|---|---:|---:|---:|---:|---|
| Generic owned dyn argument | 1 | 1 | 0 | 0 | ICE, exit 101; graph unavailable |
| Static UFCS | 0 | 1 | 0 | 0 | Concrete direct pair, source target unresolved |
| Nongeneric dyn trait | 1 | 2 | 1 | 1 | Agrees on interface pair; implementation is a separate virtual candidate |
| Generic by-reference dyn | 1 | 1 | 0 | 0 | Exit 0, but producer exports unresolved `Trait<S>` |

The existing compiled source-snapshot sidecar ran on four exact copies of previously captured dependency-free controls. No new compiler command or installation ran. Each snapshot's blob/SHA, method-site span, compiler definition position, pair and dispatch kind is retained. UFCS is outside the method-site collector's syntax scope; a general callsite ledger must include it. Generic dyn syntax preserves receiver text but lacks the nongeneric dynamic interface edge. The nongeneric case has one method site but two call rows, so row counts cannot stand in for callsite counts. These four controls are bounded diagnostics, not a workspace accuracy sample.

Called-only root traversal reaches **21,430** library nodes, **61,132** after virtual candidates, and **78,869** if address flow is incorrectly admitted. The example gives 29,467 / 35,809 / 77,283. Shortest pinned `panic_fmt` witnesses have three and four hops. Root-reachable library marker observations include 52 `BuiltinAssert` and 54 `AsyncPanic` with workspace-source locations; the example has 194 `BuiltinAssert`. They are not unique source sites or observed failures; assert reasons are absent. A panic_fmt-only sink policy would omit these markers. Source macro evidence binds physical tokens; compiler correlation still does not establish expansion provenance or feasible guards.

The effective next changes are:

1. Require typed receiver/lexical evidence before selecting a target; preserve ambiguity instead of a crate-wide method singleton. Regress all three wrong receivers, local `git_stdout`, recursion and both constructors. Keep syntax extraction and target confidence separate.
2. Add stable callsite spans and explicit closure/coroutine ownership, with lexical containment separate from calls. Include UFCS, repeated same-target calls, nested bodies, Unicode coordinates and generic dyn interface edges. Then measure site-level coverage on held-out, source-bound fixtures.
3. Repair CGA concrete trait substitutions under all seven retained compiler controls. Require both successful compilation and exact concrete targets; then capture the real CLI target. Keep virtual candidates quarantined until this condition holds.
4. Attach build/cfg/root and path-policy identity to queries. Add macro expansion and implicit-check provenance where available. Any claim of a runtime panic or safety additionally requires guard/CFG reasoning or a reproducer.

`independent.py` independently recomputes all four strict mappings, pair intersections, three root policies, twenty raw witness references and the exact false source path. It uses a frozen projection of 13,835 callable definitions from 454 Rust files and all 40,128 primary source rows, bound to the original table hashes. `root-experiments.tar.gz` freezes the four snapshots and their source/control comparisons. The retained panic source is hash-bound to the producer's pinned Rust source. Replay reads the prior workspace raw captures by reference; it does not read CURRENT, an analyst database, Notebook bytes or a compiler.

From the repository root:

```sh
python3 docs/research/artifacts/2026-11-01-execution-cga-crosscheck/review/replay.py --main "$PWD"
```

The original worker replay also reproduced all 26 generated files byte-for-byte under Python 3.14/DuckDB 1.5.0. Independent SQL/raw verification passed under DuckDB 1.5.4. An initial cross-version byte replay stopped at `inputs.json`, which fingerprints the runtime version; cross-version byte equality is not claimed. The root frozen replay uses Python standard library only and separately verifies the original 45 payload hashes, its own payload hashes, raw references and microcontrols. Notebook SQL uses DuckDB 1.5.5 to render the final tables.

Review evidence includes `independent-sql-verification.json`, `exact-runtime-replay.json`, `runtime-note.json`, root `replay-result.json`, and `notebook-receipts.json`. `audit-repair.json` records the exact worker completion request reloaded from Beads comment 1024 after its standalone audit RPC rejected the assignment; the rejected RPC is not reported as successful. Worker commit `e3122c309ae9e2fe13d52739095953f59b32c94b` is preserved. No application or producer fix, publication or graph rebuild is part of this investigation.
