# R8: bounded JSON pipeline and fixture replay

Actual assigned issue: **bd-gpd**, plan **04f4a253-7e5d-46f3-99ba-afcd3f016f85**, task **R8**. The source issue bd-1h3 is lineage only. Assigned baseline: `1427759de0a3caa00104e21b2dbc17230d75e82d`. All six plan dependencies were approved before work. Historical R1/R2/R2C and later-task evidence is unchanged.

Rationale: expose source-authenticated bounded panic queries and require source/build/request-bound compiler execution before recording fixture reproduction.

The scoped implementation is committed, but **R8 is not fully accepted**. Source-collected `assert!` witness replay requires collector scope expansion (`scope-gap.md`), and the complete regression run exposes three inherited expectations outside R8 scope (`regression-gap.md`). The final audit reports **1,284 passed, 3 failed, 19 pre-existing ignores** across 98 targets including doctests. Both scope categories have durable scope-drift signals. No ignores, gate weakening, snapshot updates, library-source changes, original CGA changes, global configuration edits, user notebook access, or workspace graph publication were used. The issue remains in_progress for root adjudication/review.

## Code and contract

`unified.diff` is a zero-context unified diff of all scoped source, fixture, test, and crate-README changes against the assigned baseline; evidence artifacts are retained separately in this directory. New commands are `analyze`, `solve`, `compiler_report`, and `replay`. Existing synchronous `execute` behavior and snapshot/collect/identity/import/validate commands remain available. The executable uses the separate async entrypoint for solve/replay. The crate README defines required fields, domains, limits, outcomes, unknown/error handling, persistence and fixture-only replay constraints.

The source collector is authenticated by canonical recollection before narrowing caller-supplied domains. Replay reconstructs the analysis, reloads its original persisted receipt, checks source/build/request/model/receipt bindings, compiles the exact standalone source in a fresh directory, and executes a generated native Rust catch-unwind harness. A solver candidate always retains `runtime_reproduced: false`. A separate replay record becomes reproduced only after observing the exact panic payload. Compiler-report correlation retains unknown obligations and never establishes semantic completeness.

Commits:

- `8866d0215`: initial new-API compile RED tests and seven hand-authored source fixtures.
- `909105052`: compiler-report JSON test and corrections to test assumptions (an unused/opaque parameter may have no modeled value; removing an already empty site set is not a corruption).
- `f5413b49e`: pin the synthetic compiler format control to the required nightly lineage; obtain the intended assertion RED.
- `7706faf5b`: scoped implementation, formatted tests and public command contract.
- Evidence/contract clarification is committed separately after final verification.

## Fresh SOLVE PRE and POST

Catalog discovery selected implemented `workflow.transition_allowed` (workflow v1, bounded_trace) and `data_integrity.mutually_consistent` (data_integrity v1, finite_relational_snapshot). Finite workflow and provenance tuples use `solve_rules`, never duplicated generic formulas. No catalog family models Rust panic branch compilation, so exact source-generated typed requests use generic B-prime, each with a successful exact `solve_constraint_check` preflight.

Eight fresh PRE receipts were persisted and reloaded **before** RED/production edits: four catalog workflow/identity positive and adversarial controls, and four exact source-generated requests from a fresh run of the shipped baseline analysis API. `pre-existing-export-cached.log` retains their complete source/facts/report exports. This is fresh task evidence, not a historical receipt substitute. The first cold build was interrupted (exit 130) and its task-created cache was later cleaned; the retained warm cache is only a build cache, never a fixture oracle.

Twenty-one POST receipts were persisted and reloaded: all 14 actual new JSON-boundary query requests with their source/build provenance, plus seven catalog workflow/identity controls derived from an actual new replay export. `post-inputs.json` is extracted from shipped command outputs, not copied PRE fixtures. Source/build/request/replay identity alterations fail the catalog relation; a solve-only transition fails the workflow relation. Opaque/assertion/call/loop/state queries can return raw UNSAT while their actual exported report remains **incomplete_semantics**. No unknown, timeout, unsupported semantics or raw UNSAT with obligations is interpreted as safety.

`solver-manifest.json` lists all **29** solve IDs, statuses, selected rules and local receipts. Every `pre-*.json`/`post-*.json` receipt (except the input inventory) includes the exact request, complete response, and complete `get_solve_result` reload; generic receipts also include their exact successful preflight. Catalog receipt reloads contain the family compiler's lowered typed request; the original family facts/rules and response envelope are retained alongside it. No `.spur/solver` files were inspected.

`verify-evidence.py` independently checks all retained reloads, exact generic request equality, six complete u8 enumerations and five successful replay digests/payloads. Its result is `evidence-verification.json`/`.log`. PRE IDs include `sol_0e20ccc8ff6a4193` (executed workflow pass), `sol_0957e116d990437e` (solve-only fail), `sol_aa27da3c068b4ffa` (bound identity pass), `sol_492f9679ce5b4e77` (changed source fail), `sol_e6f50d7951214e27` (checked-add SAT), `sol_c1e820dfcb134b7e` (overflow-on SAT), `sol_ee7929d655e54e2e` (overflow-off UNSAT), and `sol_fd58cbd18f914be2` (guarded bounds UNSAT). POST identities and exact requests are indexed in the manifest, including `sol_802cb92004704be6`/`sol_7a1617e700044d01` (workflow positive/negative), `sol_e9af68ce9ccc4b9d` (identity positive), and four identity negatives.

## RED and GREEN commands

Every Cargo invocation used `SPUR_REMOTE=0 scripts/spur-cargo`. Unless stated otherwise the warm cache was `CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5`.

```bash
# Initial API RED: exit 101. Missing execute_async, not a runtime assertion RED.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end

# Compiler-report assertion RED: exit 101, command variant is missing.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end compiler_report_json -- --nocapture

# Complete new target GREEN: exit 0, 7 passed, 0 ignored.
R8_EVIDENCE_DIR=docs/research/artifacts/2026-11-01-panic-implementation/R8/exported SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end -- --nocapture

# Regenerate domain/frontier receipts using an absolute evidence destination: exit 0.
R8_EVIDENCE_DIR="$PWD/docs/research/artifacts/2026-11-01-panic-implementation/R8/exported" SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end domains_and_loop_call_state_bounds_never_become_general_safety -- --nocapture

# Directly affected GREEN: exit 0, 123 passed across nine targets, 0 ignored.
TMPDIR=/private/tmp RUST_TEST_THREADS=1 SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_analysis_end_to_end --test panic_analysis --test panic_semantics --test panic_semantic_contract --test panic_call_sites --test panic_method_sites --test panic_compiler_evidence --test panic_compiler_validation --test panic_evidence_end_to_end

# All-target compilation: exit 0 (existing dead-code warnings retained).
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph --all-targets

# Scoped formatting and diff: exit 0.
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r8-scoped-fmt/Cargo.toml --check -- --config skip_children=true
git diff --check

# Actual stdin/stdout CLI, known native assertion controls and evidence checks: exit 0.
python3 docs/research/artifacts/2026-11-01-panic-implementation/R8/cli-smoke.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R8/native-assertion-control.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R8/verify-evidence.py

# Complete regression audit, no skips/added ignores, localhost permission granted.
TMPDIR=/private/tmp RUST_TEST_THREADS=1 SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --no-fail-fast
```

The initial RED is retained in `red.log`. The first compiler control failed its nightly-identity precondition (`red-compiler-report.log`); correcting that fixture produced the intended runtime assertion RED in `red-compiler-report-corrected.log`. `green-first.log` retains development failures from an ineffective empty-site mutation, an opaque parameter assumption, and a missing forwarded toolchain environment; `green-second.log` and the complete `green-e2e.log` retain subsequent GREEN. Cargo integration-test cwd is the crate directory; the first exports were relocated from the task-created crate-relative directory into this authorized root evidence directory. Later exports use an absolute evidence path. All transient task-created crate-relative evidence directories were removed.

`green-e2e.log` reports 7 passed. `green-bounds-persisted.log` confirms the four domain/frontier controls with persisted requests for exact POST reuse. `green-affected.log` reports 123 passed. `check-all-targets.log`, `fmt-check.log` and `diff-check.log` retain successful static checks. No library API changed, so there is no newly exposed dependent-crate API to check separately.

Six captured logs contained terminal blank lines or a whitespace-only pretty-diff line. Their exact bytes are preserved in corresponding `.raw.gz` archives and SHA-256/length checked against `raw-log-manifest.json`; readable `.log` views only normalize trailing whitespace. No result or test output content was removed. `verify-evidence.py` verifies both the raw archives and readable views. This preserves exact evidence while allowing the complete staged diff whitespace check to pass.

The initial full suite (`green-full-suite.log`) failed 12 unit tests on macOS /var versus /private/var temp-path identity. Canonical `TMPDIR=/private/tmp` cleared all 12; `green-full-suite-canonical-tmp.log` then hit the existing localhost listener sandbox denial. The approved unrestricted run (`green-full-suite-canonical-tmp-unsandboxed.log`) passed 571 library units and both listener/accounting controls, then reached the inherited artifact-range regression. `full-suite-no-fail-fast.log` and `full-suite-summary.json` retain the final complete target audit, exit 101: 1,284 passed, 3 failed, 19 existing ignores. Failures are `artifact_preserves_recovered_method_crate_singleton`, `rust_extractor_matches_sample_corpus_golden_artifact`, and `typed_gitpath_receiver_as_ref_binds_to_matching_impl_scope`. These are failures, not passing acceptance runs.

## Runtime evidence and bounds

The new mandatory target enumerates **1,536** u8/configuration cases: all 256 inputs for bounds, plain addition and checked-add under both overflow settings. Every expected native predicate is independently computed using ordinary Rust array access/overflowing_add/checked_add. Each case is checked with actual Z3, and each fixture/configuration is compiled by ordinary rustc and executed over all 256 values. `exported/enumerated-*.json` records all expected/native/solver panic inputs. Five exact candidate replays cover bounds under both settings, checked-add unwrap under both settings, and checked overflow. Plain addition with checks off is bounded UNSAT and cannot create a reproduced record. Replay records retain the actual panic strings, source OIDs, build/request/fact identities, original receipt, compiler arguments/environment, harness/binary digests, exit/stdout/stderr and outcome. The native harness catches the real panic to preserve its exact payload, so successful harness execution exits 0.

`cli-smoke.py` runs the actual runner through the required Cargo wrapper, retaining every exact JSON request and stdout/stderr. Analyze/solve/replay/compiler_report succeed; changed replay identity exits 1 with no successful stdout. A separately renamed/shadowed entry also solves and replays its exact Option panic, and cfg collection returns unknown. `cli-summary.json` lists all nine observed command exits.

Held-out controls retain lexical shadows and renaming, trait dispatch uncertainty, cfg collection uncertainty, opaque calls/macros, explicit safe subdomains, and open state/call/loop frontiers. Exact facts omissions, altered source/build/request/model/receipt identities and forged solver reproduction claims are rejected. A wrong expected payload records the actual panic with reproduction false. The original synthetic compiler-format control remains explicitly synthetic and clears no semantic obligations.

Only the declared bool/u8 model and explicit entry domains/call/loop/state budgets are analyzed. Replay supports a single standalone source file and a top-level synchronous function with fully modeled primitive arguments; its compiler must match the declared native host/version and complete cfg inventory, with explicit arithmetic/debug settings, unwind and no additional flags/features. The retained environment is per subprocess, not a global edit. Source/build declarations and hashes do not attest a producer. No arbitrary workspace binary or general Rust safety is claimed.

The assertion fixture runs natively under both arithmetic settings and produces the exact `held-out assertion` payload (`native-assertion-control.json`), but the source collector intentionally leaves its macro opaque. These are native controls only, **not** source-collected solver witnesses. Closing that acceptance gap requires root-approved changes outside the assigned files. Root also owns resolving the inherited regression expectation before full approval.

## Retrieval and collaboration records

Initial `knowledge_context_pack_2` failed preparing the analyst candidate query, and exact `code_symbol_search` for execute in the example failed because the graph artifact is absent. Bounded raw fallback was used for the panic evidence APIs/tests and necessary solver receipt declarations; no graph rebuild/publication was attempted. The full-suite failure triage read only the named failing Rust test and retained logs; it did not inspect a user notebook.

`scope-audit.json`, `scope-signal.json` and `regression-signal.json` retain compact tracker requests/responses. Full source/solver evidence stayed in this local directory. Tracker messages contain only local paths and commit references. The first report_signal call used the advertised minimal SDK shape and was rejected for missing signal_id; the corrected full signal was recorded. Assignment validation for the compact audit succeeded.
