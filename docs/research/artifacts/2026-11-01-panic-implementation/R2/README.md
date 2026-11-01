R2 resolves ordinary Rust methods only with compatible lexical receiver evidence. A unique inherent method on an exact local nominal type is selected; missing or ambiguous evidence leaves Calls unresolved.

Assigned issue `bd-382` (R2, plan `670123f8-257e-43ea-bf28-a95c82978883`) was verified in_progress before edits; R1 was approved. Historical source issue bd-1pv was not used as the assigned target. Incoming worker base: `6520e079b73438b0e0ca8acc909741af2c661b7a`; approved research baseline: `41b058e4c8021f0808d73962545647fe020d9aa3`. Production fix: `817c6ae85`. [Unified diff](unified.diff) contains only the three approved code/test paths relative to the incoming base.

The private R1 helper now follows nearest completed lets, explicit parameter/local annotations, direct unit/tuple/record constructors, borrowed local origins, and exact block/shadow/initializer intervals. Nominal types are matched by their AST declarations, including homonyms in inline modules. Unsupported function/closure/loop/match/conditional binding inference blocks lookup. Tuple constructor lookup reuses R1's value-namespace logic so a shadowing local function cannot supply a nominal type. Existing edge receiver_text/scope_text retain source evidence; public GraphFacts and graph schema v11 remain compatible.

**Runtime evidence.** Uncomplicated string/Option negatives already remained unresolved on this incoming baseline. Their historical research-graph wrong pairs are not claimed as newly reproduced. The new typed-shadow regression did reproduce both incompatible targets before production edits: an earlier &str receiver selected Pretender::len and an earlier Option receiver selected Lookalike::unwrap. Both now remain unresolved.

The original Option fixture retains twelve unwrap syntax sites: eleven Option sites collapse to ten caller/label graph pairs, all unresolved, while the actual custom Lookalike target resolves. The original receiver fixture retains exact Alpha and borrowed Beta targets, constructor/rebinding targets, the custom len target, and unresolved standard string lengths. The separate-caller matrix prevents graph pair deduplication from concealing a wrong binding.

Tests were committed in `888f3baba`, `44db0d0d8`, `3b0405611`, `27c6faa5b`, `f22fa16d3`, `5216ff09d` and `a477ce32a` before the corresponding production changes. The initial fixtures needed distinct method declaration text, src/lib.rs layout, and a selector including pub. Retained intermediate logs expose those corrections. green-attempt.log is an implementation compile error from iterator temporary lifetimes, not runtime RED; green-attempt-2.log exposes a test selector error. No generated graph or snapshot was made a test input.

An existing unit test expected Panel to bind Widget's method based on crate membership. Its expectation now requires unresolved, as approved. The nested receiver query test checks emitted facts because Rust resolution now emits those edges while the AST is available; its receiver/caller assertions remain intact.

| Validation | Result | Local evidence |
|---|---|---|
| Both incompatible bindings RED | exit 101, assertion names both wrong targets | [red-both-incompatible.log](red-both-incompatible.log) |
| Constructor-shadow RED | exit 101, wrong Tuple target | [red-constructor.log](red-constructor.log) |
| Receiver GREEN | exit 0, 6 passed | [green-shipped.log](green-shipped.log) |
| Directly affected integrations | exit 0, 50 passed including those 6 | [green-regression-final.log](green-regression-final.log) |
| Resolver unit tests | exit 0, 66 passed | [resolver-unit-shipped.log](resolver-unit-shipped.log) |
| Crate check / scoped format / diff | exit 0 | [commands.json](commands.json) |
| Independent evidence check | 33 observations, 24 receipt reloads | [retained-verification.json](retained-verification.json) |

All Cargo operations used SPUR_REMOTE=0 scripts/spur-cargo. [commands.json](commands.json) retains exact commands, logs and exits. Pre-existing FileOidMatch dead-code warnings remain. After verification, disk space blocked the dossier write; the required wrapper cleaned only generated spur-graph package artifacts (1.0 GiB), allowing the write to proceed.

**SOLVE.** Fresh catalog discovery selected implemented `data_integrity.mutually_consistent` for complete exact typed tuples, `data_integrity.foreign_key` for owner existence, `data_integrity.unique` for one matrix observation per owner, and `data_integrity.cardinality` for coverage. solve_rules owns family validation; generic solve_constraint_check is inapplicable, and no generic solver was used.

The combined PRE request exceeded the family 64-variable cap and was rejected before solving; [solve-pre-oversize.json](solve-pre-oversize.json) retains the exact rejection. Five-row groups preserve the same predicates within that limit. No validation error, unknown or timeout was treated as UNSAT.

PRE ran before RED/production changes, including fresh PRE before additional adversary tests. Every successful PRE/POST solve used persist=true and was reloaded through get_solve_result. Complete requests, returned formulas/responses and reloads are local in solve-*.json; [preparation.json](preparation.json) retains catalog, assignment/DAG and graph-retrieval responses.

POST supplies final graph edges and the resolver's exported nominal scope metadata from [exported/](exported/), projected by [project_post.py](project_post.py). Only the allowed relation comes from the independent PRE oracle. Observed receiver/target fields come from exports; target paths derive from actual target-node spans and enclosing impl/module nodes. Actual owner edges, target nodes, stable keys and source SHA-256 are in [post-projection.json](post-projection.json). Missing, extra or duplicate relevant owner/method edges fail projection. Missing local nominal identity is explicitly unknown, including standard types outside this local type lane. TraitOnly/Ambiguous retain known receiver identities while method dispatch remains unresolved.

| Stage | Case | Receipt | Raw status / outcome |
|---|---|---|---|
| PRE | group 0 | `sol_e2050e20fb5d42ec` | sat / pass |
| PRE | group 1 | `sol_3d2af5d7f06e4ba3` | sat / pass |
| PRE | group 2 | `sol_b9974b2b367e41e4` | sat / pass |
| PRE | group 3 | `sol_5349a8e10f034d5d` | sat / pass |
| PRE | group 4 | `sol_533920fba3194d2e` | sat / pass |
| PRE | group 5 | `sol_66ca131e1ccf448b` | sat / pass |
| PRE | wrong_standard_target | `sol_12d5401c9b3e4854` | unsat / fail |
| PRE | missing_receiver | `sol_46aff85c23524272` | unsat / fail |
| PRE | wrong_owner | `sol_384271984a804d78` | unsat / fail |
| PRE | ambiguous_target | `sol_2f6c04d06c72472c` | unsat / fail |
| PRE | pre_extra | `sol_4c24176377384f7a` | sat / pass |
| PRE | pre_constructor | `sol_b02fb7b1c69b4b01` | sat / pass |
| POST | group 0 | `sol_51c83e38978a4269` | sat / pass |
| POST | group 1 | `sol_b14ab2d7ad854ca7` | sat / pass |
| POST | group 2 | `sol_54f7ab6d326a4d01` | sat / pass |
| POST | group 3 | `sol_d791fb94d4d246d6` | sat / pass |
| POST | group 4 | `sol_c9217da5d3594a41` | sat / pass |
| POST | group 5 | `sol_12e53c1faf55406d` | sat / pass |
| POST | group 6 | `sol_685772de304a4c1f` | sat / pass |
| POST | wrong_string_target | `sol_325f19c97ac04df2` | unsat / fail |
| POST | wrong_option_target | `sol_23246cd503d14616` | unsat / fail |
| POST | missing_receiver | `sol_f074ed07bbdf4874` | unsat / fail |
| POST | wrong_owner | `sol_b3d7670fedc04762` | unsat / fail |
| POST | ambiguous_target | `sol_fe1ac2b8285940e3` | unsat / fail |

[verify_retained.py](verify_retained.py) independently evaluates exact tuple membership, MATCH SIMPLE owner checks, uniqueness and cardinality using Python; compares results with solver status; verifies reload identities/constraints; and reprojects actual exports. It reads no notebook, live graph, or .spur/solver files.

**Bounds and obligations.** This is physical-source evidence for directly parsed method calls, not compiler target authority. Exact local nominal types and built-in references are supported. Aliases/imported or external types, scoped type paths, general function-return inference, inferred patterns, generic instantiation, trait dispatch and user-defined autoderef remain unresolved. A known receiver can have unresolved dispatch. No build/cfg selection, monomorphization, borrow checking, macro expansion or general Rust type checking is claimed. Negative extraction fixtures include intentionally unresolved/ill-typed source shapes. Existing explicit dyn-trait and macro-token evidence retain their separate paths.

No notebook, graph publication/CURRENT pointer, global install/configuration, original CGA checkout/driver/launcher, upstream message, or unrelated user file was modified. Root retains architecture/plan review and notebook verification; R8 owns complete regression acceptance. Leave bd-382 in_progress for root review.

