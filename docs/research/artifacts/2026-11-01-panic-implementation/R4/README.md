# R4 typed panic semantic contract

Assigned issue **bd-1f8**, plan `04f4a253-7e5d-46f3-99ba-afcd3f016f85`; leave in_progress for root review. R2C/R3 were approved before work. Historical R1/R2/R2C/R3 evidence is unchanged.

Rationale: a separately versioned, source-bound typed contract lets later collection and analysis preserve unknown semantics while rejecting structurally invalid evidence without changing graph schema v11.

Test commit: `e7e5c92c9` (`test(spur-graph): R4 specify typed panic semantic contract`). Implementation: `ba98f6a90` (`feat(spur-graph): R4 validate source-bound typed panic facts`). Base: `4417055a4e48278a3d9a58b717cad4f32d789a91`. The [unified diff](unified.diff) contains only the three scoped Rust files, with zero context lines so raw patch context whitespace does not create evidence-file lint noise.

## Public contract

`PanicBuildConfig` requires target, edition, cfg, overflow_checks, debug_assertions, panic strategy, rustc identity, flags and features. Canonical serialization is SHA-256 bound. `PanicSemanticFacts::validate_against(graph,sources,call_sites)` revalidates the R3 canonical call/body evidence and checks exact source/body inventories, invocation tuples, UTF-8 half-open spans and innermost bodies, typed domains and reference sorts, mandatory origins, unique value versions, assignment links, phi predecessor coverage, ordered DAG flow and dominating guard availability, and primitive authority/build binding. ID-indexed vectors have deterministic order. No graph v11 type or dependency changed.

`validate_for_build(expected)` independently pins the caller's intended build; a self-contained document cannot authenticate a fully rehashed replacement build. `validate_complete_against` additionally rejects every retained unknown or open frontier. Its success describes structural completeness of the supplied finite model, not source translation truth or source safety. No `complete` boolean may be injected into JSON.

Opaque, mutation, cfg, parse, callee, call-depth, loop-bound and state-budget obligations retain locations and explanations through JSON, with exact per-body inventories. Frontiers require an obligation of the matching kind. No missing domain/origin or missing relation inventory is defaulted into false information.

## Verification

[verification.json](verification.json) retains exact commands and outcomes; raw output is in the named logs.

- RED: `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract` exited 101: **compile RED for a brand-new API**, E0432 missing exports and consequent E0282. It is not an assertion RED. The initial cold build was interrupted (130) to reuse the R3 scratch cache because only 3.6 GiB remained; it is not counted as RED. Initial full formatting check found only unformatted new tests; exact raw output is losslessly archived in `red-format-initial.json`.
- Initial implementation run: four passed, one harness panic from a nonexistent phi JSON pointer. The pointer was corrected from value index 4 to 2, and the wrong-sort origin adversary was made type-specific. No production gate was relaxed.
- GREEN: `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end -- --nocapture` exited 0: **57 passed** (5 contract, 10 call/body, 23 compiler evidence, 9 end-to-end, 10 method-site).
- `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph` exited 0. Only the existing `FileOidMatch::as_bool` dead-code warning remains.
- Scoped formatting uses `SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r4-scoped-fmt-68aa37a1/Cargo.toml --check -- --config skip_children=true`, exit 0. The exact temporary manifest is retained in `format-manifest.toml.txt`; it names only the three scoped files.
- Full scoped diff checks include source and retained evidence. A preliminary diff check flagged literal rustfmt context spaces in the raw formatter log. The log was archived losslessly as a JSON string; no source/test/solver evidence was discarded. Terminal blank lines in two Cargo logs are normalized with reversible byte counts and original/normalized hashes in `log-normalization.json`; exact output is reconstructible. The final diff check is retained separately.

Tests reject missing domains/sorts/context/origin, duplicate sites/value/expression/flow IDs, a different existing body, stale bytes/source/build/graph, mismatched sorts and ranges, absent/detached guards, wrong spans, missing phi arms, stale/missing primitive authority, omitted inventories, reordering, derivation/flow cycles and incomplete build config. Eight uncertainty classes round-trip, block completeness and fail validation if their referenced obligations disappear.

## SOLVE PRE and POST

Selected implemented family: `data_integrity` v1 / finite relational snapshot. Rules: `unique`, `foreign_key`, `conditional_required`, `mutually_consistent` (required exact tuples), `value_range`. No manual generic rule duplication or generic preflight was used; the family tool validates its requests. Rule specs and returned encodings are in `pre-receipts.json`.

PRE: 10 persisted/reloaded receipts, one valid SAT/pass and nine UNSAT/fail adversaries, eight catalog predicates per model. Missing-value presence is checked separately because unique uses NULLS DISTINCT and foreign_key uses MATCH SIMPLE.

POST: [project_post.py](project_post.py) reads **actual shipped test exports** from `green-affected.log`, including invalid serialized variants, and writes `exported.json` and exact `post-requests.json`. Canonical invocation tuples come independently from the R3 call/body set. Build identity is independently rehashed from the exported build bytes. One oversized initial request was rejected by the 64-variable compilation cap **before Z3**, retained in `post-initial-validation.json`; it is not UNSAT. Exact relation/field projections produce 21 positive and 14 negative single-rule requests, all persisted and reloaded.

[verify_evidence.py](verify_evidence.py) independently re-evaluates all finite catalog predicates, compares response/reload IDs/status/models, checks retained compiled constraint identities, and evaluates each SAT model against every lowered typed constraint. Result: **45 receipts, 115 finite predicates, all reloaded, zero unknown/timeout**. It does not rerun Z3 or prove arbitrary source semantics. Family requests and lowered persisted requests are both retained; the solver service persists its lowered typed form.

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R4/project_post.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R4/verify_evidence.py
```

## Source/build/bound assumptions and remaining obligations

The 157-byte Rust corpus is embedded in the contract test. Its author-normalized IR exercises a bool parameter, u8 parameters/versions, guarded mutation and phi merge, two distinct calls and three bodies. Fixture limits are call depth 1, loop iterations 1, states 32; these are explicit sample inputs, not production defaults or a bound-selection policy. The fixture build metadata and primitive authority are synthetic structural test declarations, not a captured compiler build. Actual local tooling is recorded separately in `build-environment.json` (rustc/cargo 1.94.1, aarch64-apple-darwin).

The source SHA-256 is `d6fc88283b07c214a934520c58d256b18291d69aa27c45cdda321b7076db7dcb`; build-declaration hash is `sha256:7011a67078f7428f17d810b1faf3e194f1bb8b75aca4c71eb33858fb140e5ad3`. File hashes are in `sha256.json`.

R5 still owns automatic source-faithful collection and canonical recollection, including Option/helper/loop forms; R6 owns solver translation and bound exploration; R8 owns complete regression acceptance. This contract authenticates structure and retained identity, not compiler execution, model authority payload contents, a complete cfg inventory, or arbitrary semantic claims. The R3 validator still rejects malformed or unevaluated cfg-bearing sources; a parse/cfg obligation does not bypass that gate. This version's control flow is a finite unrolled DAG. No unbounded safety claim follows from empty uncertainty/frontier lists.

Both graph-first retrieval calls failed; exact errors and the bounded raw fallback are in `retrieval.json`. No notebook read/write, graph publication, original CGA checkout/driver/launcher edit, global installation/config change, protected user-file edit or upstream message occurred. Cargo always used the required wrapper with SPUR_REMOTE=0. Full evidence remains local; issue audits contain compact paths and commit references only.

## Receipt IDs

| Phase | Control | Raw status | Persisted/reloaded ID |
|---|---|---|---|
| PRE | valid | sat | `sol_fd382e0882b143e6` |
| PRE | wrong_existing_body | unsat | `sol_bac54cf4fbb245db` |
| PRE | stale_source | unsat | `sol_802550fd996b43e0` |
| PRE | stale_build | unsat | `sol_d5158c9a70184117` |
| PRE | sort_mismatch | unsat | `sol_7c64b8a5c2c64ade` |
| PRE | missing_domain | unsat | `sol_904ccc5e0dab4156` |
| PRE | missing_origin | unsat | `sol_f39f0c29ad504b75` |
| PRE | detached_guard | unsat | `sol_748203eb9f714c13` |
| PRE | range | unsat | `sol_8158831e79524d47` |
| PRE | reused_site | unsat | `sol_9b47335ce41648d6` |
| POST | valid__sites_identity | sat | `sol_b26d206bf34a42ae` |
| POST | valid__flow_identity | sat | `sol_cf7eaaf11b754a10` |
| POST | valid__values_identity | sat | `sol_081636b216434a56` |
| POST | valid__expressions_identity | sat | `sol_f84dcbd52bbc48e7` |
| POST | valid__sites_body | sat | `sol_773a4841643a48b1` |
| POST | valid__values_body | sat | `sol_621c672d40cb400b` |
| POST | valid__expressions_body | sat | `sol_884f1a9b0dcf4ce0` |
| POST | valid__site_source | sat | `sol_5d42681392744156` |
| POST | valid__site_flow_guard | sat | `sol_e144e2daa9b34ec4` |
| POST | valid__exact_site_binding | sat | `sol_7ad916d848f24f80` |
| POST | valid__exact_build_binding | sat | `sol_b764aa88c2514b3f` |
| POST | valid__values_typed_domain | sat | `sol_4f1d7fd02db34f9f` |
| POST | valid__values_required_domain | sat | `sol_fb3ba3bf62b64290` |
| POST | valid__values_required_origin | sat | `sol_0534a64bd0104d8b` |
| POST | valid__values_u8_minimum | sat | `sol_952b196d71224509` |
| POST | valid__values_u8_maximum | sat | `sol_8d076c7bdce445d9` |
| POST | valid__expressions_typed_domain | sat | `sol_ed8b0687466d41ec` |
| POST | valid__expressions_required_domain | sat | `sol_f8b19cf257124e9c` |
| POST | valid__expressions_required_origin | sat | `sol_233372c98feb4591` |
| POST | valid__expressions_u8_minimum | sat | `sol_b47dc803fac046d4` |
| POST | valid__expressions_u8_maximum | sat | `sol_5dd7fbe7ab454eeb` |
| POST | duplicate_sites__sites_identity | unsat | `sol_05b01591aeb646f4` |
| POST | duplicate_values__values_identity | unsat | `sol_73da1de9754245cc` |
| POST | duplicate_expressions__expressions_identity | unsat | `sol_395783305c02405a` |
| POST | duplicate_control_flow__flow_identity | unsat | `sol_d51befcb390f4bf3` |
| POST | wrong_existing_body__exact_site_binding | unsat | `sol_54e9b0d90c124ef0` |
| POST | stale_source__exact_site_binding | unsat | `sol_1a29a44173a04391` |
| POST | stale_build__exact_build_binding | unsat | `sol_d87f2fe8b0e3495c` |
| POST | stale_graph__exact_build_binding | unsat | `sol_5813e70a6c61428e` |
| POST | sort_mismatch__expressions_typed_domain | unsat | `sol_ad59ff00d4b14762` |
| POST | out_of_range__values_u8_maximum | unsat | `sol_782c30e218224c5e` |
| POST | detached_guard__site_flow_guard | unsat | `sol_74d22af0889a4309` |
| POST | missing_expressions_domain__expressions_required_domain | unsat | `sol_295c0899e75a4b2a` |
| POST | missing_values_domain__values_required_domain | unsat | `sol_f4204fa80fe24e15` |
| POST | missing_origin__values_required_origin | unsat | `sol_5996e4ab7c854eb8` |
