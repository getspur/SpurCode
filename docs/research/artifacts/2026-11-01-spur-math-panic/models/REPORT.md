The frozen call graph supports structural candidate checks. Panic feasibility in these controls additionally requires branch, value, type, and ownership facts. The equal-topology Option pair demonstrates the gap: its safe guard has no modeled panic input, while changing only `==` to `!=` makes 255 of 256 inputs modeled panic inputs. These are bounded mathematical results over explicitly normalized source semantics, with no new Rust execution.

This worker owns only `models/`. Notebook work, DuckDB extensions, and the comparison over the full retained graph belong to the root. No Notebook tool, notebook file, compiler command, production edit, graph rebuild, publication, installation, or upstream message was used. The research base supplied by the task is `39c32bb5e6a0e2eb574b6e9663571bd5f6501989`; retained CGA is pinned to `2fc63108`, nightly `2026-10-18`. No full CLI graph or trustworthy virtual expansion is assumed.

| Measurement | Result |
|---|---:|
| Catalog cases | 11 |
| Generic cases, each checked with its exact execution request | 9 |
| SAT / UNSAT / inconclusive solver results | 10 / 10 / 0 |
| Solve calls / distinct persisted IDs / successful reloads | 20 / 20 / 20 |
| Responses marked cached | 1 |
| Complete generic assignments enumerated | 5,376 |
| Source-bound input/oracle rows | 768 |
| Returned SMT scripts | 19 |
| Sum of reported solver duration, excluding MCP/check/reload time | 117 ms |
| Z3 version | 4.16.0, 64 bit |

All returned SAT models satisfy the reloaded typed constraints and domains. All nine generic requests passed exact preflight before execution. Generic UNSAT cores were checked by enumeration; deleting each named member makes the remaining core satisfiable. The reported time is a receipt total, not a performance benchmark. The model/input agreement is 20/20 controls, not a production precision or recall estimate.

The paired fixtures are exact UTF-8 with trailing LF, both 98 bytes. Both contain `entry` and one `value.unwrap()` at byte offset 73. Their normalized callable/call topology is identical; no compiler-derived graph of these new fixtures is claimed.

| Source | SHA256 | Normalized panic predicate, `x` in 0..255 | Result |
|---|---|---|---|
| `fixtures/option_safe.rs` | `3bf436183c570834d63050f3ea80d6c1cbae699813172089db9476ee168d0b79` | `(x == 0) AND NOT (x == 0)` | UNSAT; 0 inputs |
| `fixtures/option_panic.rs` | `ca423b00c3fcbaac2a496c98c9dbcd34a7b80b81f1a97fc8b6d13b9b7aae6e24` | `(x != 0) AND NOT (x == 0)` | SAT; 255 inputs, returned `x=1` |
| `fixtures/checked_add.rs` | `4d836d093a6d557b9000fc08ce596de4d5ed64b214c20d7a48cd159ffa67538c` | `x + 1 > 255` | SAT; only `x=255` |

The Option initializer text supplies the candidate `is_some = (x == 0)` relation. The second `if` text supplies the candidate reach condition. Their meaning, `unwrap(None)` panic behavior, and the absence of additional mutations are human-normalized assumptions recorded in `normalization.json`; there is no automatic Rust semantic extractor here. Exact-byte checks and source hashes bind the assumptions to the fixtures. `finite_oracle.jsonl` contains the direct mathematical oracle for every input of all three sources, independently compared with the request-AST evaluator.

Deleting the guard or deleting the initializer relation deliberately leaves a Boolean unconstrained. Each weakened query is SAT, but its source panic feasibility is UNKNOWN. The result does not supply the missing source fact. No actual solve returned `unknown` or `timeout`; either would remain inconclusive. An absent guard must never be filled with `false`, or treated as proof that a path is safe.

`checked_add.rs` uses `u8` (unsigned, width 8), avoiding build-dependent implicit-overflow semantics. The model uses unbounded mathematical addition followed by the explicit representable interval check, so it cannot accidentally wrap before testing overflow. A separate typed bit-vector control checks `(x +_8 1) <_unsigned x` and finds exactly `#xff`. Excluding 255 makes that query UNSAT. The guarded integer query adds `x <= 254` as an EXTRA model assumption; that guard is absent from the fixture. These are arithmetic/Option models, not executed MIR assertions.

| Case | Raw result and interpretation | Persisted ID |
|---|---|---|
| endpoint_valid | SAT / verify pass | `sol_58d5e1d1b2504480` |
| endpoint_dangling | UNSAT / verify fail | `sol_4823179c3dc14aea` |
| receiver_compatible | SAT / verify pass, hypothetical compatible tuple | `sol_97bc4b6a39374126` |
| receiver_wrong | UNSAT / verify fail, candidate rejected under supplied relation | `sol_2dd8259261b74cbe` |
| ownership_closure | SAT / verify pass | `sol_f955e1f69db148a9` |
| ownership_lexical | UNSAT / verify fail as exact body identity | `sol_96a413388e2448c7` |
| workflow_called | SAT / bounded solution, 2 steps | `sol_819362c7441a498f` |
| workflow_bound1 | UNSAT / bounded infeasible at bound 1 | `sol_d6c2f09a9c414479` |
| workflow_address_excluded | UNSAT / bounded infeasible | `sol_31eb88fd7f8c44ec` |
| workflow_virtual_excluded | UNSAT / bounded infeasible, cached result | `sol_7de84a63f0ec4e6b` |
| workflow_virtual_candidate | SAT / bounded candidate solution | `sol_63e17732e6c44e71` |
| option_safe | UNSAT / no encoded counterexample | `sol_45634c90ed234e1e` |
| option_panic | SAT / encoded counterexample | `sol_4447478e7b8b41be` |
| option_missing_guard | SAT / source feasibility UNKNOWN | `sol_58531b6ab1064487` |
| option_missing_constructor | SAT / source feasibility UNKNOWN | `sol_14aecbb4ef18457e` |
| checked_add_panic | SAT / encoded counterexample | `sol_721af860e18a48a9` |
| checked_add_guarded | UNSAT / under extra caller guard | `sol_ccb9aa1392e14c02` |
| u8_wrap_witness | SAT / unsigned modular boundary | `sol_038e2ae3cc114a04` |
| u8_wrap_excluded | UNSAT / under explicit exclusion | `sol_d2af9b39c74f45c5` |
| named_guard_conflict | UNSAT / conflicting named requirements | `sol_144d647964d744b0` |

The catalog was navigated before authoring constraints. `data_integrity.foreign_key` checks a closed, complete child target against two finite callable rows. `data_integrity.mutually_consistent` checks explicit receiver/owner tuples. These families do not inspect Rust. The foreign-key profile uses MATCH SIMPLE; this control supplies complete keys, and does not rely on missing values passing the rule. Catalog verify uses SAT/pass and UNSAT/fail; generic counterexample UNSAT has a different interpretation.

The receiver control reuses `wrong-elapsed-receiver` from the frozen crosscheck. The exact field excerpt is `start_time: Instant`; the caller is `DashboardView::elapsed`, stable ID `09d8f4c6b4593112`. Its source edge targets `SessionDetailView::elapsed`, stable ID `cb75667a69fdbe95`. `source_evidence.json` retains the caller, field, target method and enclosing impl excerpts with hashes and source bindings. The relation accepts only equal labels among `Instant` and `SessionDetailView`; the observed mixed tuple fails. This is mathematical rejection under a caller-supplied compatibility relation, not a general proof about Rust method lookup, trait impls or autoderef. The independent retained CGA `Called` observation targets `std::time::Instant::elapsed` at the same source callsite. Its evidence remains separately attributed to the prior compiler capture, with no new compiler run. Source `syntax_exact` / confidence 1.0 is retained as original metadata and contributes no semantic proof.

The ownership control reuses the retained `compare_exact_rows` closure observation. The compiler owner is `spur_graph::query_hot_index::compare_exact_rows::{closure#0}`; the source row projects the lexical outer function. Those identities are mapped manually to `Closure` and `LexicalOuter`. A lexical projection is useful for navigation but fails this explicit exact-body policy. The test does not claim that every lexical projection is a malformed graph edge.

The workflow family uses `initial_state_allowed`, `transition_allowed`, and `bounded_reachability` in synthesis mode, with horizon 2 and an explicit initial state. A target needs two direct-call steps: bound 2 is SAT, bound 1 is UNSAT. Termination and stuttering through `Done` are explicit, so exact-horizon deadlocks do not silently remove short terminating paths. Strict policy excludes a `FnPtrCast` edge from execution and excludes virtual candidates. Enabling `VTableCalled` yields a structural candidate witness only. The enabled relation is manually normalized before the solver sees it. In particular, address-excluded and virtual-excluded have IDENTICAL exact family and lowered typed requests; the solver cannot distinguish the removed raw kinds. Nothing here executes `FnPtrCast`, establishes virtual-call feasibility, or proves unbounded safety/liveness.

The named conflict receipt has core `require_zero, forbid_zero`. Both rules are individually satisfiable and jointly impossible over the finite domain. The safe Option core is `source_option_constructor, source_unwrap_guard, panic_requires_none_and_reached`: it explains why that counterexample query cannot hold. A core diagnoses the supplied formula; it cannot establish that a source fact was extracted correctly.

The frozen schema in `source_evidence.json` shows ordinary edges contain source/target IDs, labels, relation/kind, receiver/scope text, binding metadata and confidence. They do not carry guard predicates, CFG dominance, SSA values, Option discriminants, numeric domains, or ordinary callsite byte ranges. Nodes carry declaration ranges and lexical scopes; they do not supply those missing dataflow facts. Macro records have their own spans but do not give ordinary calls expansion ownership. Consequently:

| Required fact | How this package obtains it | What would make it systematic |
|---|---|---|
| Input width, unsigned domain | Exact `x:u8` source signature, manually normalized | Source type extraction or compiler type identity |
| Option constructor and unwrap guard | Exact fixture bytes plus declared normalization | AST/CFG/SSA extraction, branch dominance and value-origin tracking |
| `checked_add` failure predicate | Manual mathematical contract plus exact source | Canonical callee/type contract; compiler evidence for implicit checks |
| Receiver and target owner | Frozen field/method/impl excerpts | Type propagation and canonical method resolution, including trait/autoderef cases |
| Closure body owner | Prior compiler observation, manually labeled | Source closure identity plus expansion mapping or compiler owner sidecar |
| Path execution policy | Manual `Called`/candidate/address classification | Provenance-aware typed edges and build/root qualification |
| Reachable panic under actual Rust execution | Unavailable for new fixtures | Independent matching retained runtime controls or fresh compiler/runtime validation |

The orientation tool reproduced `failed to prepare context candidate query`; its exact response is retained at `catalog/discovery3.json`. No graph freshness or rebuild is inferred. Raw/frozen source artifacts supply the bounded examples. `runtime_control_match.json` checks the three `.rs` members of the retained pilot archive and finds no exact new-fixture match. That check does not claim the entire earlier pilot had only three fixtures. Compiler/runtime verification for these new sources remains unavailable.

Catalog and request details are in `catalog/`, `requests/`, and `cases.json`. Each `receipts/<case>.json` retains the exact submitted request, the complete tool response, the exact generic preflight request/response when applicable, and the complete `get_solve_result` request/response with `pin:true`. No solver-cache file was read. Generic reload requests materialize `soft:false` and other server defaults; the original requests are preserved. Family reloads expose the complete lowered typed request, rather than original family facts and outcomes, so both envelopes must be kept. All 20 receipts are pinned; the root owns their later retention decision.

One cached response, `workflow_virtual_excluded`, omits `smt` despite `include_smt:true`. Its omission is preserved in `summary.jsonl`. The identical address-excluded request has returned SMT at `smt/workflow_address_excluded.smt2`; this is an equivalent-query reference, not text returned by the omitted field. Raw `solve_smt` was unnecessary and was not used: the typed language expresses every selected noncatalog predicate.

Reproduce the historical evidence locally with no dependencies or network:

```bash
python3 docs/research/artifacts/2026-11-01-spur-math-panic/models/build_models.py
python3 docs/research/artifacts/2026-11-01-spur-math-panic/models/verify.py
```

`build_models.py --capture-source` additionally reconstructs local evidence from the neighboring frozen research packages when they are present. Ordinary replay needs only the self-contained `models/` package. `replay_mcp.js` is a fresh-solve orchestration script for `functions.exec`: it navigates catalog metadata, preflights exact generic requests, executes and reloads each result, and writes new receipts under `models/reruns/`. Its syntax is checked; the original 20 retained solves were executed through the same MCP primitives, not by running that convenience script. Replayed model choices, IDs, durations and cache state may differ; compare predicates and raw statuses. UNKNOWN/timeout remain inconclusive.

`summary.jsonl` is the small Notebook-loadable result table; `finite_oracle.jsonl` is the larger exhaustive source-bound table. The root can load them without worker Notebook access. `changes.diff` is a unified creation diff for the scoped package, excluding itself and its checksum manifest. `SHA256SUMS.json` binds the package payloads. `source-pair.diff` highlights the one-character guard mutation.

bd-289 was verified `in_progress`; no superseding label was present. The standalone worker audit tool rejected the start evidence with `-32001 unauthorized: worker evidence: target is not assigned to this delegation`. The exact request and rejection are at `audit/start.json`. Per the task's explicit fallback, work continued and the root was notified through `report_progress`. Automatic approval review separately rejected the large completion audit append as sensitive egress to Beads. Its exact request and rejection are retained at `audit/completion.json`; no workaround or repeated append was attempted. No successful Beads audit write is claimed. The issue remains `in_progress` for root approval and Notebook integration; this worker does not close it.
