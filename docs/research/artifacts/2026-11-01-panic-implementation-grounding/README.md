The four proposed areas are supported by the new experiments. Implementation should start with lexical and receiver correctness, then preserve invocation/body identity and semantic facts before exposing bounded panic results. The current source graph can select candidates; its resolved targets and topology cannot by themselves establish panic feasibility.

The [new notebook](../../../../2026-11-01-panic-implementation-grounding.ipynb) was created and executed through Notebook MCP. This dossier contains research controls and their evidence. Production crates, the published graph and the CGA producer are unchanged. The independent [grounding report](grounding/REPORT.md) describes the 32 held-out source sites and raw compiler observations.

**Measured results.**

| Check | Result |
|---|---|
| Held-out source controls | Four missing lexical self-call sites; two homonym negatives bind correctly; closure target remains missing |
| Independent receiver corpus | Three expected target pairs, four unresolved sites, two wrong pairs |
| Option/custom-method adversary | Eleven Option invocation sites incorrectly bind to `Lookalike::unwrap`; ordinary rustc MIR confirms the Option targets in both configurations |
| Invocation/body evidence | Five targeted method-site IDs survive repeat snapshots; free-call and closure/async execution identities remain incomplete |
| Exhaustive ordinary Rust controls in final notebook runs | 7,680 executions across 15 functions/two overflow configurations, plus 768 compiler-fixture inputs; all panic and successful return outcomes match independent oracles |
| SOLVE | 24 preflighted typed requests plus nine catalog requests; all 33 persisted and reloaded |
| Fresh returned-formula Z3 replay | 18 SAT, 15 UNSAT; no solver UNKNOWN/timeout |
| Independent generic-model enumeration | 19,712 complete assignments across the 24 typed requests |
| Compiler witness replay | Four solver witnesses reproduce the expected panic payloads, covering bounds, overflow and explicit assertion |
| DuckDB algorithms | Petgraph and Duckrouting agree with BFS on four directed checks; identical control topology has different guard feasibility |

Counts describe these controls, not workspace precision or recall. Source candidate rows lack ordinary callsite identities, so a pair match is not an exact invocation-to-edge mapping. The Option corpus has 12 distinct `unwrap` syntax sites: eleven Option sites and one actual custom-method site. Its two shadowed Option invocations collapse to one graph pair. The retained method sidecar keeps both source spans and honestly reports unknown compiler targets. All 16 ordinary function ranges match the author-bound ranges.

**Grounding the four implementation areas.**

| Area | Evidence and implementation gate |
|---|---|
| 1. Lexical self-call and receiver resolution | Require recursion before and after a local shadowing block, qualified recursion, and nested/module homonym negatives. A closure with the same spelling must not become a self-edge. Use lexical binding intervals and receiver type evidence; retain uncertainty for inference, autoderef and trait lookup that the source lane cannot establish. The standard string-length and Option/custom-unwrap controls must stop producing incompatible local targets. |
| 2. Precise identities and semantic facts | Keep definition identity, snapshot body identity and invocation identity separate. Existing method sites are a useful partial starting point. Add free calls and executable closure/async bodies with explicit creation-versus-invocation links. Facts need typed domains, reaching value versions/phi relations, mutation effects and evaluation guards. Source bytes, body/span ownership, build/cfg and derivation authority must bind each fact. |
| 3. Bounded predicates and replay | Encode the root, source/build identity, typed values, evaluation order, guards and panic preconditions under explicit call/loop bounds. Preserve raw status separately from missing facts and open frontiers. Replay a SAT model before reporting a reproduced counterexample. Exclude counterexamples only within the complete encoded scope after UNSAT. |
| 4. Compiler validation | Retain concrete trait-instance, expansion and implicit-check observations alongside the source evidence. Keep raw CGA references, exact locations and provenance. Do not promote address casts to execution, virtual enumeration to feasible dispatch, or macro-definition locations to invocation identity. The known generic virtual substitution defect still requires its own repair and regressions. |

Rust's [lexical scope rules](https://doc.rust-lang.org/reference/names/scopes.html) support binding-sensitive recursion, and its [method lookup rules](https://doc.rust-lang.org/reference/expressions/method-call-expr.html) require receiver candidates and visible methods. These rules support a narrow source repair with explicit unsupported cases; they do not justify selecting a method by name alone. [Overflow behavior](https://doc.rust-lang.org/reference/expressions/operator-expr.html#overflow) depends on build settings. The tests therefore retain compiler flags and both configurations.

**Why a SAT/UNSAT field alone is insufficient.** The mutation control initializes `value=Some(x)`, writes `None` under `x==3`, then unwraps under the same guard. Its correct model is SAT at `x=3`, and Rust panics. Deliberately retaining the stale initial origin yields UNSAT. That is a rejected translation, not a proof of source safety. Exact hashes/spans establish which bytes were used; semantic translation needs its own validation.

The shadowing control has two different bindings both spelled `value`. Its inner unwrap returns at `x=7`; the outer unwrap panics at `x=8`. The short-circuit control is UNSAT because the left operand restricts evaluation of the unwrap. A preceding non-dominating `if` does not supply that guard. MIR independently exposes the separate locals, reassignment and short-circuit edge. The 45 exact source fragments and 51 named-constraint bindings in [normalization-ledger.json](semantics/normalization-ledger.json) are explicitly authored for this corpus, checked against MIR/runtime, and are not automatic Rust semantic extraction.

| Negative control | Raw result | Required interpretation |
|---|---|---|
| Stale pre-mutation origin | UNSAT | Reject the source translation; runtime has a panic |
| Missing constructor/value fact | SAT at `x=0`, replay does not panic | Inconclusive due to missing semantics; reject the spurious witness |
| Call depth zero | UNSAT | Open helper frontier; the full control panics |
| Only first loop iteration explored | UNSAT | Open loop frontier; iteration two contains the counterexample |
| Caller/callee used as unique invocation key | Catalog verify UNSAT/fail | Distinct invocations collide |
| Missing span under uniqueness alone | Catalog verify SAT/pass | NULLS DISTINCT permits absence; completeness needs a separate rule |

Catalog exact-tuple validation rejects a missing span, a different existing same-file body, and a forged source binding. This validates the supplied finite identity relation, not the truth of arbitrary semantic facts. Workflow bound 1 excludes a target reached at bound 2 and leaves the helper frontier open. None of these experiments returns an actual solver `unknown`; application-level inconclusiveness is recorded separately.

**Connecting compiler evidence to mathematical predicates.** The fresh concrete fixture calls a unit implementation, expands an identity macro, indexes a two-element array, adds two `u8` values and checks an assertion. For one entry call, `object=()`, `x=0..255`, `index=0..2`, and overflow checks on, its hand-normalized predicate is:

```text
index >= 2 OR x + x > 255 OR index != 0
```

The 768-input Rust run gives 640 panics and 128 successful returns, all matching the independent oracle. Successful values equal `7 XOR (x+x)`. The ordered models preserve earlier checks, so an overflow witness first passes the bounds check and an assertion witness first passes both earlier checks.

| Case | Persisted solve | Witness or scope | Fresh panic payload |
|---|---|---|---|
| Bounds | `sol_136a67312f0a45f8` | `x=0, index=2` | index out of bounds |
| Overflow | `sol_d702f6040a6747e6` | `x=128, index=0` | attempt to add with overflow |
| Assertion | `sol_5fa01f88cdb34ffc` | `x=0, index=1` | assertion failed: index == 0 |
| Safe caller assumptions | `sol_08d32698438049bd` | UNSAT with `index=0 AND x<=127` | No encoded counterexample in this declared scope |

These are source-normalized equations with explicit concrete receiver assumptions. They do not establish safety for arbitrary trait objects or the full `usize` input domain. The successful CGA export contains six nodes and seven observations: three `Called`, one `VTableCalled`, two `BuiltinAssert`, one `FnPtrCast`. Local instances and edge locations are checked against exact source bytes. The expanded call points at `expanded_target($value)` in the macro definition; invocation/expansion identity remains unknown. The external panic target retains its location. The full application was not captured. This is consistent with [CGA's compiler-call-graph approach](https://tweedegolf.nl/en/blog/242/finding-panics-with-call-graph-analyzer/), supplemented here by explicit predicates and runtime controls.

**Scope and reproducibility.** Notebook DuckDB is 1.5.5; existing Petgraph v0.5.0 and Duckrouting 509ece1 were loaded, with built-in JSON/Parquet. No extension installation or upgrade was needed. The small CFG projections are author-normalized, not extractor-produced CFGs. Both contain a structural four-hop panic path; the safe Option predicate is UNSAT while the changed-guard predicate is SAT. Agreement among graph algorithms does not validate guards or receiver bindings.

Ordinary rustc is nightly-2026-10-18, commit `8006091eb71ae7d9e9bccb66c6122c9d68af4466`, aarch64-apple-darwin, panic=unwind. Root emitted MIR with overflow checks on/off and ran the fixture with each setting. The pinned CGA revision remains `2fc63108f0b674fe7a178191ffa15647a18879f1`; original driver and launcher hashes are preserved. The snapshot executable is hash-pinned and all 100 dep-info-listed `spur-graph` source/query inputs match its prior build inventory and current source. Two unrelated ACP adapter inputs differ from that inventory; freshness is limited to the extraction subsystem. No whole-application freshness claim follows.

Graph-first orientation and exact symbol tools failed in the current MCP session. `grounding/retrieval.json` and the notebook document the raw fallback. The seven original graph tables and six protected files, including the prior experiment notebook, are hash-checked for preservation. No graph rebuild or publication occurred. The worker's compact audit append failed delegation-assignment validation; root persisted compact completion/approval references through the issue API. Detailed source and solver evidence stays local.

Standard-library retained-evidence verification:

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation-grounding/frozen_replay.py
```

This hashes the payload, reproduces finite models and runtime comparisons in a scratch copy, checks both compiler grounding captures and returned SMT-prefix provenance, and validates stored extension paths and witness payloads. It reads no notebook or live graph and runs no compiler or solver. Fresh execution commands and durable Notebook MCP receipts are retained separately. The source fixtures intentionally retain their byte layout; formatting checks target the runtime harness without recursively formatting included fixtures.

The implementation gate is a narrow first slice with these held-out regressions. General semantic extraction, cfg/generic coverage, full macro expansion stacks, opaque memory effects, unbounded safety and the CGA producer repair remain separate requirements. This investigation does not implement them.
