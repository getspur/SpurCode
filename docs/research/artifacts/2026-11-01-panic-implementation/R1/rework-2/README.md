R1 rework resolves generic free calls from their lexical function bindings and checks nearer namespace bindings before selecting an inline module, preserving explicit unresolved alias/type evidence instead of emitting an incompatible outer target.

Assigned issue: `bd-2po`, verified `in_progress` before edits. Prior implementation/evidence overlay was preserved in `a023e0cb0`; all 92 original evidence hashes remain unchanged. Original runtime RED provenance remains in the parent directory. The copied root review inputs (`review.json`, `probes.rs`, `red.log`, `namespace-runtime.rs`) are verbatim; `red.log` here is the root's review run, whereas `red-final.log` and `red-type-parameter.log` are this attempt's runtime assertions. The parent README describes the earlier attempt; its qualified-call coverage did not include the reviewed alias/type failures. This rework supersedes that limited claim.

The [unified diff](unified.diff) is against the preserved prior implementation `a023e0cb0` and contains only approved code/test files. Production changes are limited to `extract/rust_bindings.rs` and its existing integration in `extract/tree_sitter.rs`. No graph schema, public API, query file, notebook, publication pointer, original CGA driver/launcher, global configuration or unrelated user file changed. `build_facts` remains the tested public path. Root owns notebook verification and plan/architecture acceptance; R8 owns complete regression acceptance.

The helper unwraps the `generic_function` callee and supplements the compiled Rust query with identifier/scoped-identifier generic free calls. The shared query file is unchanged. Unsupported associated/type paths and field receivers retain their previous extraction behavior: the two generic method preservation controls currently emit no Calls edges. This is an existing limitation, not newly claimed generic-method support.

Relative module paths now inspect enclosing blocks through the containing module, respecting hoisted local modules, import aliases, local type declarations and type parameters. Expired sibling-block aliases and value-namespace closures do not shadow a module path. Explicit `self`, `crate` and `super` paths retain module anchoring. If a nearer unsupported namespace binding hides a candidate inline module, the resolver emits an unresolved Calls edge. It does not claim the hidden outer function as `syntax_exact`/`fqn`. When no inline module candidate exists, the established receiver/import path remains in charge. Source-only alias/type resolution remains conservative.

Catalog-first PRE preceded new regression tests and production edits. Selected implemented rules: `configuration.attribute_allowed_pair` (finite exact owner/callee compatibility) and `data_integrity.temporal_consistency` (nonempty lexical intervals and the expired-block finish/start ordering). Family execution supplies compiler validation; no generic requests or raw SMT were used, so generic `solve_constraint_check` preflight was inapplicable. Discovery and bounded raw fallback are recorded in [discovery.json](discovery.json). Both graph orientation and exact-symbol reads failed; native reads were restricted to the assigned files, directly relevant query/grammar contracts and explicitly named evidence.

All receipts retain exact family requests, raw responses and `get_solve_result` reloads. No `.spur/solver` files were inspected. Initial temporal requests omitted required empty relation-definition maps and were rejected before solving; the exact rejected requests/errors and repaired requests are retained. Unknown/timeout is never interpreted as UNSAT or safety.

| Fresh PRE | Status / outcome | ID |
|---|---|---|
| Generic, alias, type and lexical module tuples | SAT / pass | `sol_cf432ddf3e1f4579` |
| Missing generic self-call and wrong outer alias target | UNSAT / fail | `sol_fb10cf2ea6d64063` |
| Lexical binding/call intervals and expired block order | SAT / pass | `sol_5f93a05cb5c44a75` |
| Empty binding interval control | UNSAT / fail | `sol_a4d5c16f74024c67` |
| Supplemental type-parameter namespace tuple | SAT / pass | `sol_cb25c0ded026497f` |
| Type parameter forged as outer module | UNSAT / fail | `sol_a8897b8615d84652` |

The initial runtime run failed six assertions, including an overreaching test that expected new generic receiver support. That preservation control was corrected to the observed existing no-Calls behavior and committed before production edits; the final pre-fix run (`red-final.log`) then failed five genuine assertions: ordinary generic recursion, qualified generic recursion, local module alias, local type, and hoisted namespace alias. Nine other tests passed. The additional type-parameter test was separately PRE-grounded, committed, and observed failing at runtime before its fix (`red-type-parameter.log`). Test commits: `159687250`, `8364d0741`, `f44696678`. The first production iteration fixed namespace cases but still missed generic captures (`green.log`, 12 pass / 2 fail); the query supplement then passed 14 tests (`green-complete.log`). Those intermediate receipts/logs are retained and explicitly superseded by the final verification below.

All Cargo operations used `SPUR_REMOTE=0 scripts/spur-cargo`. Rust Analyzer started a separate workspace check and held the target lock; the waiting task test was terminated with exit 143 before running any tests (`green-final.log`), and the already built target was cloned with macOS copy-on-write into `/private/tmp/r1-rework2-target-9ac34ff5`. Final commands use that isolated `CARGO_TARGET_DIR`; Cargo still validates fingerprints and rebuilds changed inputs. Rust Analyzer was left running. No remote failure was rerun locally. Exact validation commands/outcomes are in [commands.json](commands.json).

Final POST is generated by [project_post.py](project_post.py) from fresh [exported/](exported/) `build_facts` output. Source text, nodes, edges and spans are actual test exports, never golden inputs. Expected targets are independently selected by exact source declaration; observed target identities include source SHA-256, byte range and stable symbol key. Whole owner/label outcome sets reject missing/extra targets, and unexpected owners are checked rather than filtered away. The method controls additionally compare the absence of all Calls edges against RED exports. Function-span interval requests are partitioned into independent batches of at most twelve rows (five variables each, under the family compiler's existing 64-variable limit); no cross-row ordering obligations are dropped.

Finite source cases establish these exported outcomes, not compiler-grade Rust resolution or unbounded completeness. External module/import resolution, cfg evaluation, macro expansion, receiver/type inference and per-invocation identity remain outside this repair. Existing macro captures and unresolved evidence are exercised by affected tests. Historical source/log whitespace remains verbatim; diff checks are explicitly scoped to the four approved shipped code/test files, excluding source-only/log artifacts.

| Final verification | Outcome | Evidence |
|---|---|---|
| Lexical regressions | 15 passed | [green-isolated.log](green-isolated.log) |
| Affected integrations | 31 passed | [green-isolated.log](green-isolated.log) |
| Resolver unit tests | 66 passed | [resolver-final.log](resolver-final.log) |
| Crate check | exit 0 | [check-final.log](check-final.log) |
| Scoped format/diff checks | exit 0 | [fmt-check.log](fmt-check.log), [diff-check.log](diff-check.log) |

Final POST checks 47 fresh exports, 53 owner/outcome rows and 72 exported function intervals. These receipts supersede the earlier `post-*` receipts after the supplemental type-parameter fix.

| Final POST | Status / outcome | ID |
|---|---|---|
| [final-post-adversarial-empty-interval](final-post-adversarial-empty-interval.json) | UNSAT / fail | `sol_e757853bd03240f6` |
| [final-post-adversarial-generic_recur](final-post-adversarial-generic_recur.json) | UNSAT / fail | `sol_c38265b2a36046be` |
| [final-post-adversarial-local_block](final-post-adversarial-local_block.json) | UNSAT / fail | `sol_022e9e6e60734af9` |
| [final-post-adversarial-local_module](final-post-adversarial-local_module.json) | UNSAT / fail | `sol_3a3d8649989c4419` |
| [final-post-adversarial-local_type](final-post-adversarial-local_type.json) | UNSAT / fail | `sol_b39f2fe5b488435a` |
| [final-post-adversarial-module_alias](final-post-adversarial-module_alias.json) | UNSAT / fail | `sol_096c7de3c6fa4716` |
| [final-post-adversarial-shadow_recur](final-post-adversarial-shadow_recur.json) | UNSAT / fail | `sol_4f968bd6b36f4343` |
| [final-post-adversarial-type_parameter](final-post-adversarial-type_parameter.json) | UNSAT / fail | `sol_2f2948f2a20d4f5d` |
| [final-post-compatibility](final-post-compatibility.json) | SAT / pass | `sol_0f3f90549d5a4b46` |
| [final-post-intervals-1](final-post-intervals-1.json) | SAT / pass | `sol_470b91f42eaa4291` |
| [final-post-intervals-2](final-post-intervals-2.json) | SAT / pass | `sol_966621688e634a56` |
| [final-post-intervals-3](final-post-intervals-3.json) | SAT / pass | `sol_b860e0044d77488f` |
| [final-post-intervals-4](final-post-intervals-4.json) | SAT / pass | `sol_1520fe6410184010` |
| [final-post-intervals-5](final-post-intervals-5.json) | SAT / pass | `sol_faed5e875ef04e8d` |
| [final-post-intervals-6](final-post-intervals-6.json) | SAT / pass | `sol_e55717240d13466e` |

Reproduce the retained source/receipt/command checks without a compiler, live graph, notebook or Z3:

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R1/rework-2/verify_retained.py
```

The versioned [verification.json](verification.json) manifest binds the shipped code/test hashes to the local verification inventory. Prior evidence hashes are verified without editing the parent artifacts. The existing `FileOidMatch::as_bool` warning is outside scope. The task issue remains `in_progress` for root review.

Implementation commit: `4ea29585d` (test commits precede it). The [rework diff](unified.diff) is against `a023e0cb0`; [full-task.diff](full-task.diff) includes the preserved original repair against task baseline `ed4d8d58d`. Compact audit `f3e39ff2-6d80-41bf-a117-3e54540ab72a`, comment `1080`, was persisted and idempotently reloaded on assigned issue `bd-2po`; detailed source and solver payloads stayed local. See [audit-record.json](audit-record.json), [audit-reload.json](audit-reload.json), and [assignment-final.json](assignment-final.json).
