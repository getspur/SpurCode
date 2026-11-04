# R5: automatic typed origins and evaluation guards

Actual assigned issue **bd-280**, plan `04f4a253-7e5d-46f3-99ba-afcd3f016f85`; left **in_progress** for root review. R2C, R3 and R4 were approved before work. Base: `d98855e1a8e1472ed6d8cc46469fe77e284f8fe3`. No historical R1/R2/R2C/R3/R4 evidence was changed.

Rationale: source-derived versions and evaluation guards prevent stale or non-dominating values from certifying Option unwraps.

The public collector is `collect_panic_semantics(graph, sources, build)`. It recollects R3 identities, translates actual Rust AST nodes, validates the resulting R4 structure and exposes `validate_canonical` for expected-build-pinned recollection. A self-consistent forged conclusion passes the structural validator in a negative test but fails canonical validation. There is no fixture/function-name lookup in production.

Option<u8> lowers to separate presence/payload versions. None's zero payload is padding, meaningful only when presence is true. Reassignment creates guarded versions; lexical shadows have separate binding identities; branches join reaching versions; early returns remove continuation paths; `&&` and `||` guard only the evaluated operand. The versioned R5 extension retains scalar helper arguments/results, guarded returns, build-dependent primitive addition, bounded constant indexing and symbolic literal loop templates. Graph schema v11 and the existing R4 structural API remain intact.

Commits:

- `d0d15335b`: original failing API/semantic contract and fresh PRE evidence.
- `7db498417`, `f9a3eaa73`, `7eb9d242f`: additional adversarial, corpus and typing regressions before their fixes.
- `ad9a1f171`: verified collector and canonical validation.

[implementation.patch](implementation.patch) is the unified source/test diff against the dispatched base, with zero context to preserve literal patch whitespace under repository whitespace checks (`git apply --unidiff-zero` if applying it). It includes only the four authorized Rust files.

RED and GREEN evidence:

| Phase | Exact command / environment | Outcome |
|---|---|---|
| Initial cold command | `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_semantics` | Interrupted, exit 130, to reuse R4's existing scratch target; not counted as RED. |
| New API RED | `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics` | Exit 101: E0432 missing collector and E0599 missing canonical validator. **Compile RED**, not assertion RED; `red-cached.log`. |
| Initial implementation assertions | Same test command | Exit 101: two expected completeness assertions exposed discarded tail if-expression results; `green-second.log`. |
| Numeric/helper adversaries | Same command plus `unsupported_types_and_lexical_helper_shadows_cannot_supply_facts` | Exit 101: unsupported u16 was accepted; `red-heldout.log`. |
| Primitive aliases | Same command plus `primitive_type_shadows_and_unconstrained_option_payloads_are_unknown` | Exit 101: shadowed primitive alias accepted; `red-types.log`. |
| Type-origin regression | Same command plus `typed_local_aliases_and_reversed_option_phi_remain_supported` | Exit 101: valid reversed phi retained an unsupported-type obligation; `red-typing-origins.log`. |
| Final affected suite | `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_semantics --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end -- --nocapture` | Exit 0: **73 passed** (10 R5, 11 R4 contract, 10 call/body, 10 method, 23 compiler evidence, 9 end-to-end); `green-final-typing.log`. |
| Crate check | `SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph` | Exit 0; `check-final.log`. Only the pre-existing `FileOidMatch::as_bool` warning. |
| Scoped format | `SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r5-scoped-format-66f67a3f/Cargo.toml -- --check --config skip_children=true` | Exit 0; manifest retained as `format-manifest.toml.txt`. |

The initial six generic and two ownership PRE checks preceded test edits. Four additional PRE controls ran while the compile-RED build was pending, before production edits. Catalog discovery is in `catalog.json`. `data_integrity.mutually_consistent` owns exact finite source/body/site tuples and was executed with `solve_rules`. Rust guard/value equations have no matching catalog family: exact typed requests were preflighted with `solve_constraint_check` before `solve_constraints`. Every successful solve was persisted and reloaded via `get_solve_result`; no `.spur/solver` files were inspected.

There are **12 contract PRE receipts**, **100 POST receipts** (97 generic, 3 catalog) and one additional SAT catalog-example discovery control. All 112 contract verdicts match expectations (40 SAT, 72 UNSAT); none is unknown/timeout. `receipt-verification.json` lists every PRE/POST ID and retained request/response/reload path. The catalog compiler persists a lowered typed request rather than the original family envelope; both are retained and the named rule binding, status, outcome and model are checked explicitly. Generic reload requests match exactly after documented serialization defaults.

Representative IDs:

| Query | PRE | POST |
|---|---|---|
| Mutation panic | `sol_f8dec018e800402c` SAT | `sol_d2a661edc0384a83` SAT |
| Short-circuit exclusion | `sol_13c6ed4ceb3e4263` UNSAT | `sol_6522d00c677d4a5c` UNSAT |
| Non-dominating if | `sol_70e683da8d40499e` SAT | `sol_887fad4f1a0a40f8` SAT |
| Exact source tuple | `sol_629360a1a6204bc2` SAT | `sol_91b342271a1c4ae0` SAT |
| Wrong existing body | `sol_d0960a8110034ed6` UNSAT | `sol_2546ff96afeb4ec9` UNSAT |

POST formulas are expanded from the collector's newly exported expressions, assignments, phi origins and guards, with exact origin/guard locations retained in `post-requests.json`. They are compared to an independently authored finite oracle. Dropping the actual mutation origin disagrees with that oracle (`sol_00914f7eed87445f`, SAT), while canonical validation rejects forged conclusions. Source and span catalog adversaries reject wrong same-file owners and absent spans, not merely duplicate IDs.

`verify_collector.py` independently interprets SSA/guards and compiles/runs the same source using ordinary rustc. All **12,544 inputs across 19 source/build cases** agree with both independent finite and ordinary Rust oracles, including return values and **3,581 panics**. Runtime commands, source hashes and compiler output are in `runtime-commands.json`; finite results are in `finite-results.json`; the summary is in `oracle-summary.json`. `export-stability.json` and `export-stability-typing.json` show that formatting and the final type-origin traversal did not alter the original 17 exports or their 93 POST requests. The final two cases add four fresh POST checks.

Source/build/bound assumptions and retained obligations:

- The source lane covers supplied physical Rust files, not macro expansion or a whole compiled workspace. Graph-first orientation and exact symbol lookup both failed; bounded fallback is recorded in `startup.json`. No graph publication, notebook access, global config, original CGA checkout/driver/launcher or upstream messages occurred.
- Fixtures declare Rust 2021, aarch64-apple-darwin, bool/u8 domains, unwind and explicit overflow settings. This is a build declaration, not compiler attestation. The ordinary oracle used rustc 1.94.1 (`e408947bfd200af42db322daf0fadfe7e26d3bd1`) on aarch64-apple-darwin with explicit overflow flags. Input domains are bool's two values and all 256 u8 values.
- Helpers are unambiguous same-file free functions with scalar bool/u8 signatures. Unknown imports/type aliases, custom/shadowed constructors/unwrap, opaque effects, aliasing, unsupported casts/patterns, attributes, deferred/constant bodies and macro syntax retain obligations. Unconstrained default integer types are not silently treated as u8. Only lossless u8-to-usize indexing casts are supported.
- Literal loops retain one symbolic iteration, exact range and an unresolved LoopBound obligation. The independent verification oracle expands the declared tiny literal range; collection performs no bounded search and does not discharge R6's caller limits/frontiers. Collection's structural budget fields declare no exploration (call/loop depth zero and one initial state).
- cfg/parse uncertainty prevents R3 body identities, so collection returns a typed `PanicCollectionObligation` error rather than inventing a body owner. Unknowns are never safety results. R8 retains complete regression acceptance.

Reproduce the local evidence checks without a notebook, live graph or solver:

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/verify_receipts.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R5/verify_collector.py
```

The second command freshly compiles/runs temporary ordinary Rust harnesses. It reads fresh collector exports as evidence, never generated goldens as collector inputs. Log whitespace normalization, if needed for `git diff --check`, preserves the exact original log bytes in `raw-log-archives.json`.
