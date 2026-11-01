# R7 attempt 2 — explicit compiler check option agreement

Rationale: honor compiler-supported underscore aliases, bare check booleans and last-value precedence across synonymous keys while retaining the original ordered inventories and identity hashes.

Assigned issue **bd-399** was verified `in_progress` before edits and is left there for root review. Plan: `04f4a253-7e5d-46f3-99ba-afcd3f016f85`, task R7. R8 remains pending. No delegation was performed.

The pre-applied attempt-1 source and evidence were preserved verbatim in `e0171042e` (comparison against authored history `fa66258061c26aaf7a251236c63b033544871c36` produced no diff for the four scoped files or historical R7 evidence). The existing ref `spur/evidence/r7-attempt-1-authored` remains intact. Historical R1/R2/R2C and attempt-1 evidence are unchanged. New evidence is isolated in this directory. Root's supplied review remains at `/private/tmp/r7-root-review/`.

Commits:

- `e0171042e` — preserve imported attempt-1 implementation/evidence.
- `4df40b36f` — commit failing regression tests and fresh catalog PRE before production edits.
- `65cc5674b` — minimal check-parser fix and scoped formatting.

`unified.diff` contains the four source/test files against the assigned baseline `21e2131d6628c76b15b5992cb436d994770aaaa2`. `rework.diff` contains only this attempt's code/test changes against the imported attempt-1 baseline. Both are zero-context unified diffs. The fix itself changes 24 lines in `compiler_validation.rs`; the legacy importer API, schema v11, sidecar/report version, pinned CGA format and independent configuration binding remain unchanged.

## Runtime and native evidence

All Cargo commands used the local wrapper and requested warm target:

```bash
export SPUR_REMOTE=0
export CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5

# Runtime RED at committed tests, prior to the production fix: exit 101.
scripts/spur-cargo test -p spur-graph --test panic_compiler_validation review_flag_ -- --nocapture

# Final target GREEN: exit 0, 14 passed.
scripts/spur-cargo test -p spur-graph --test panic_compiler_validation -- --nocapture

# Directly affected existing suites: exit 0, 93 passed.
scripts/spur-cargo test -p spur-graph --test panic_compiler_evidence --test panic_call_sites --test panic_method_sites --test panic_semantic_contract --test panic_semantics --test panic_evidence_end_to_end

scripts/spur-cargo check -p spur-graph
scripts/spur-cargo fmt --manifest-path /private/tmp/r7a2-scoped-fmt/Cargo.toml --check -- --config skip_children=true
git diff --check

python3 docs/research/artifacts/2026-11-01-panic-implementation/R7/rework2/native_controls.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R7/rework2/verify_rework.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R7/rework2/verify_rework.py --receipts
```

The five named `review_flag_*` tests all reached runtime assertions and failed with actual acceptance `true`, expected `false`. `red.log` is genuine assertion RED, not a build failure. There was no stale solver-cache build-script failure in this attempt. `green-initial.log` retains a preliminary passing run; `green-final.log` verifies the final formatted source after retaining the original panic-option handling. `commands.json` records exact final argument arrays, environments, exit codes and logs. The only Cargo warning is the pre-existing unused `FileOidMatch::as_bool` method outside scope.

`check-cases.json` is hand-authored test input, committed before RED. It is not generated from output. Its 66 controls include the exact five reported bugs and matching positive controls, attached/separate short/long codegen forms, underscore/hyphen spellings, bare/on/off values and mixed-order adversaries. Eight invalid-syntax controls cover numeric `0`/`1`, empty values and invalid values followed by a valid alias. Recognized occurrences are validated before last-value selection, matching the pinned compiler's rejection of invalid earlier options.

`native_controls.py` invokes only the already installed `rustup run nightly-2026-10-18 rustc`; it does not install or change toolchains, global configuration, CGA sources/drivers/launchers, or frozen capture files. `native-version.json` exactly matches the retained capture's verbose version: commit `8006091eb71ae7d9e9bccb66c6122c9d68af4466`, host `aarch64-apple-darwin`. `native-controls.json` retains every exact command, compile/runtime stdout/stderr/exit, source hash and observed effective value. Temporary native binaries were removed with their own temporary directory. Numeric values are syntax rejections, never semantic counterexamples.

The final suite emits **29 accepted correlation reports and 37 rejections**. Native-normalization checks all 66 outputs against the pinned compiler, including unchanged raw ordered flags and independently retained binding hashes. Every successful report keeps the exact six raw nodes/seven observations, the concrete unit instance, separate execution-observation/address/virtual sets, raw macro-definition observations, five retained unknown obligations and unestablished semantic completeness. Existing forged identity/source/owner/edge, panic-strategy and legacy-importer controls all pass. No golden or snapshot was updated or used as newly generated input.

## Fresh catalog PRE and POST

Catalog discovery and full rule guidance are retained in `catalog-*.json`. Selected implemented rules: `data_integrity.mutually_consistent` and `workflow.transition_allowed`. `data_integrity.foreign_key` was inspected but not needed. Family requests use `solve_rules`, whose compiler validates the schema; no generic encoding or generic preflight was used.

**26 fresh PRE calls** completed and were persisted/reloaded before RED/test/production edits. They cover source/build/owner/raw-reference tuples, separate category transition policies, unknown preservation and explicit-check agreement. The first aggregate flag request exceeded the catalog's 64-variable limit; `pre-size-error.json` retains the exact request and error. It produced no verdict/receipt. Splitting into bounded batches (at most 56 lowered variables) preserved every case. Historical receipts were not substituted for these calls; any solver response `cached` field is retained as returned by the API.

**32 POST calls** consume newly emitted final reports/rejections and fresh native observations. The first 17 regenerate source/owner/reference and category/obligation requests from actual reports using the preserved normalizer. Ten more check all 66 actual acceptance results against native syntax/effective values, with at most seven four-field rows (63 variables) per request. Five adversarial requests flip the actual rejection bit for each review counterexample and must fail. The verifier also independently checks raw capture equality, ordered inventories, source/body identity, report bindings, unchanged observation categories and unresolved obligations.

Representative PRE IDs:

- `sol_b7f3b17729664715` — matching explicit checks: SAT/pass.
- `sol_f902213535f049ea` — contradictory explicit checks: UNSAT/fail.

Representative POST IDs:

- `sol_0c4f5cbc3f47409a` — actual report/native agreement: SAT/pass.
- `sol_6a90f0374aba47fa` — forged acceptance of the mixed-alias counterexample: UNSAT/fail.

`solve-manifest.json` lists **all 58 IDs**, original request hashes, compiled reload hashes, statuses and paths. `pre/` and `post/` retain exact family requests, responses and `get_solve_result` reloads. `verify_rework.py --receipts` verifies the exact request inventories, every finite policy, every SAT model variable bound and compiled constraint, every negative diagnostic, and matching reload IDs/statuses. No unknown/timeout outcome occurred; UNSAT/fail applies only to the supplied finite snapshots/transition policies. No `.spur/solver` files were inspected.

## Bounds and remaining obligations

The check parser does not infer absent target/profile defaults. The independent complete configuration binding remains a trusted capture-procedure declaration. Raw flags and their original ordering remain in both compiler identity and semantic build hashes; canonicalization exists only in the validator's local effective-option map.

Exact source correspondence does not prove path feasibility or receiver typing. Expansion stacks/invocation identity, generic virtual producer trust, assertion payload/reasons and complete configuration declarations retain their existing limits; no compiler observation clears an unknown obligation. This rework preserves panic-strategy handling and observation semantics. R8 owns full regression acceptance and root owns architecture/plan/notebook review.

`retrieval.json` documents failed orientation and exact graph calls and the bounded raw fallback. No notebook, graph CURRENT/publication, upstream message, original CGA component, global configuration or unrelated user file was changed. Tracker audits contain only compact local path/commit references. `verification.json` records hashes and command outcomes; log normalization is losslessly reconstructable from `log-normalization.json`.
