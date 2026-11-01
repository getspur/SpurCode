R1 resolves ordinary Rust calls from visible lexical bindings before source positions are lost, admitting genuine recursion while retaining unresolved closure calls and exact nested/module owners.

Assigned issue: `bd-2po`, verified `in_progress` before editing and again before completion. Plan: `670123f8-257e-43ea-bf28-a95c82978883`, task `R1`, no dependencies. Historical source intent `bd-202` was not used as the assignment. Task checkout baseline: `ed4d8d58d8628affc9c9387140abfc21829cc1cb`; approved design baseline: `41b058e4c8021f0808d73962545647fe020d9aa3`.

The [unified diff](unified.diff) contains only the four approved code/test files. The private Rust helper uses enclosing source ranges and declaration visibility: lets start after their initializer/else statement and end with their block; function items are hoisted within their own scope; parameters and patterns shadow in their bodies. Conditional bindings exclude initializers and else arms. Inline module paths bypass value shadows. Imports, constructors, cfg duplicate items, macro captures, receiver calls and other languages retain the existing resolution paths. The global self-target suppression is unchanged.

`build_facts`, artifact APIs, and graph schema v11 are unchanged. No notebook, graph publication/pointer, original CGA checkout/driver/launcher, global configuration, upstream message, or unrelated user file was modified. Tests use the exact original held-out source, retained in [held-out.rs](held-out.rs); exported graphs are solver evidence, never test goldens. Graph retrieval failures and the bounded native fallback are recorded in [retrieval.json](retrieval.json).

Verification used only `SPUR_REMOTE=0 scripts/spur-cargo` for Cargo operations. Exact commands and outcomes are in [commands.json](commands.json).

| Check | Outcome | Evidence |
|---|---|---|
| Initial runtime RED | exit 101, all 5 assertions failed | [red.log](red.log) |
| Conditional scope runtime RED | exit 101, `if_let_body` bound to self | [red-patterns.log](red-patterns.log) |
| Item shadow runtime RED | exit 101, imported binding became self | [red-items-exact-owner.log](red-items-exact-owner.log) |
| R1 integration GREEN | exit 0, 7 tests / 31 source exports | [green-complete.log](green-complete.log) |
| Existing resolver unit tests | exit 0, 66 passed | [resolver-unit-complete.log](resolver-unit-complete.log) |
| Affected integrations | exit 0, 31 passed | [regression-complete.log](regression-complete.log) |
| Crate check | exit 0 | [check-complete.log](check-complete.log) |
| Scoped format / diff checks | exit 0 | [fmt-check-complete.log](fmt-check-complete.log) |
| Retained receipt/source validation | 31 exports, 36 owner outcomes, 21 reloads verified | [retained-verification.json](retained-verification.json) |

The first implementation attempt had a Rust iterator lifetime compile error (`green-attempt-1.log`); it is not RED evidence. The first item-shadow test selected a substring inside a `pub fn` declaration (`red-items.log`); the selector was corrected and committed before the genuine assertion RED. A resolver preservation check caught a changed singleton `bind_method` (`resolver-unit.log`); ordinary nonrecursive singleton calls now retain the established resolver and provenance. The pre-existing `FileOidMatch::as_bool` dead-code warning remains outside scope. The initial relative export directory was resolved by Cargo inside the crate; those generated files were moved to this approved evidence directory, and all final exports used its absolute path.

Catalog-first execution selected implemented `configuration.attribute_allowed_pair` and `data_integrity.temporal_consistency`; [catalog.json](catalog.json) retains the exact rule discovery responses. All requests used `solve_rules`, whose family compiler performs request validation. No generic constraint request or raw SMT execution was used, so a generic `solve_constraint_check` preflight was inapplicable. Every persisted result was reloaded through `get_solve_result`; each receipt stores the exact family request, response, and reloaded lowered request. No `.spur/solver` files were inspected.

| PRE receipt | Raw status / outcome | ID |
|---|---|---|
| Exact finite owner/callee tuples | SAT / pass | `sol_f43f3975b67043e7` |
| Wrong bindings including closure-to-self | UNSAT / fail | `sol_6d94314ed0404a4c` |
| Callsite interval order | SAT / pass | `sol_1420b0ba9ebf4e83` |
| Empty interval | UNSAT / fail | `sol_c93dd0e13a744f90` |
| Source binding interval validity | SAT / pass | `sol_7796ebb819c84f7c` |
| Conditional pattern controls | SAT / pass | `sol_c339f99e86324e86` |
| Local item shadow controls | SAT / pass | `sol_eedebe9cf9b24399` |

POST uses actual final `build_facts` nodes, edges and spans in [exported/](exported/), projected by [project_post.py](project_post.py). Expected bindings are independently declared by exact source declaration; observed target identities include source SHA-256, function range and stable symbol ID. This rejects another existing function with the same spelling. Missing or extra relevant owner/outcome rows are not ignored. The two additional conservative alias/constructor cases check that Calls remain absent on their existing path; the retained export also shows the constructor's Constructs edge.

| Final POST receipt | Raw status / outcome | ID |
|---|---|---|
| 36 actual owner/outcome rows | SAT / pass | `sol_f972b288dbc0401b` |
| Exported function intervals, batch 1 | SAT / pass | `sol_82788b042d51438c` |
| Exported function intervals, batch 2 | SAT / pass | `sol_a45522b4bcdd475d` |
| Exported function intervals, batch 3 | SAT / pass | `sol_545c90ddabd34f2b` |
| Exported function intervals, batch 4 | SAT / pass | `sol_add435168faf48f8` |
| Wrong same-name nested/module targets and forged closure self-call | UNSAT / fail | `sol_e545f6d2ec774ca8` |
| Empty exported function span | UNSAT / fail | `sol_9f4936b3fa2d4b76` |

The unpartitioned interval request exceeded the family compiler's 64-variable limit ([post-intervals-attempt.json](post-intervals-attempt.json)); this was validation failure, not a solver verdict. Each row contributes five variables. The 41 independent interval rows have no cross-row predecessor constraints and were partitioned into 12/12/12/5 rows, preserving every obligation. No unknown or timeout was interpreted as UNSAT or safety. Earlier POST receipts are retained and clearly superseded by the `post-complete-*` receipts after final code changes.

These checks establish behavior for the retained finite syntax cases and declared source translation, not compiler-grade Rust name resolution. Call edges still deduplicate by owner/target: the exact held-out source has distinct before/after-shadow sites but one self-edge pair. Isolated before-shadow and after-block variants independently test both intervals. No invocation identity is claimed. Opaque macro effects, cfg evaluation, external module/import resolution, receiver/type inference and unbounded completeness remain outside this source-only repair. A deliberately unsupported alias retains the previous no-target behavior. R8 owns complete regression acceptance; root owns plan/architecture review and notebook verification. The issue is left `in_progress` for root review.

Retained-only verification (no compiler, solver or notebook access):

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R1/verify_retained.py
```

Test commits: `36b341dce` (initial failing lexical controls), `70b5db2e0` (conditional intervals), `b3929f552` (item shadows), `3bd719a05` (exact owner selector). Implementation/evidence commit: `e535cc1b84bd8c5dcdffbf41c22841a6604388fe`.

The compact worker audit was successfully persisted and reloaded on the actual assigned issue `bd-2po`: audit `53ea0bd4-37b2-4e53-a092-a5d902a8b4c6`, comment `1071`. [audit-record.json](audit-record.json) retains the compact request/response. No detailed source or solver payload was sent to the tracker.
