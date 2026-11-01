# R7 — compiler correlation without uncertainty promotion

Assigned issue **bd-399**, plan `04f4a253-7e5d-46f3-99ba-afcd3f016f85`, task R7. The source issue `bd-hb3` is lineage only. Startup verified `in_progress`, no supersession, and approved upstream tasks (including R2C). The issue remains `in_progress` for root review. Baseline: `21e2131d6628c76b15b5992cb436d994770aaaa2`.

**Rationale:** bind compiler observations to canonical source/build/body evidence while preserving raw categories and every unresolved semantic obligation.

`unified.diff` is the complete source/test unified diff (zero context) against the dispatched baseline. Only the four allowed source/test files and this R7 evidence directory changed. Historical R1/R2/R2C evidence, graph schema v11, legacy importer signatures/wire format, notebooks, protected user files, graph publication pointers, original CGA sources/binaries/launchers, and global configuration were preserved. Nothing was published or sent upstream.

## Implementation and trust boundary

`PanicCompilerValidation`, `PanicCompilerValidationRequest`, `PanicCompilerConfigBinding`, and `correlate_panic_compiler` are exported through `panic_evidence`. The v1 report reuses the legacy importer and its pinned producer/toolchain/build/source/input/checksum/reference/coordinate validation. A crate-private helper shares its existing definition-header matching rules; it adds body-range provenance without altering old importer output.

The report keeps every raw node and observation, including exact opaque compiler instance names and `(from_node, edge_index)` references. Separate lists preserve Called, VTableCalled, FnPtrCast, BuiltinAssert and AsyncPanic. Direct calls and canonical implicit operations receive exact-span links only after matching the compiler caller's definition and the source's innermost body. Address casts receive no call links. Virtual enumeration receives no dispatch/feasibility claim. A macro-definition observation remains at its definition span and never acquires an invocation identity.

Configuration agreement requires an **independently retained** `PanicCompilerConfigBinding` joining the expected capture build ID and the expected complete semantic build hash. It is a trusted producer declaration, not compiler attestation. The legacy raw format does not export resolved edition, complete cfg or effective defaults; this bridge cannot recover them from source locations. Shared target, exact verbose compiler identity, ordered compiler flags and selected features must agree. Explicit check/panic flags are also checked, including short/separated and long codegen forms. Callers must obtain the binding from a trusted capture procedure, rather than construct it from untrusted facts. Replacing both independently trusted expectations is outside this authentication boundary.

All five unknown obligations in the concrete held-out fixture remain byte-for-byte equal to canonical source collection. The report's only completeness state is `Unestablished`, and `cleared_obligations` is always empty. Receiver typing, assertion reasons/payloads, expansion stacks/invocation identity, generic virtual producer correctness, capture procedure integrity and complete configuration declarations remain explicit limitations. No source correspondence proves feasible execution, type inference, semantic completeness or safety. Unknown/timeout solver states would be inconclusive; none occurred in successful PRE/POST requests.

## Frozen inputs and replay assumptions

The concrete input is the untouched historical `grounding/fixtures/compiler/lib.rs` and raw `grounding/capture/compiler.callgraph.json` under `2026-11-01-panic-implementation-grounding`. Source SHA-256 is `d5b043f5f88a8a7373b0b9a87fa0e901dee74142532bd677131db429bcff24a0`. Six nodes and seven observations survive: three Called, one VTableCalled, two BuiltinAssert and one FnPtrCast. The unit implementation is node 5; the expanded call remains on line 6 in the macro definition. The synthetic additional control supplies the AsyncPanic category and a source-normalized implicit-index operation.

The test constructs a **replay envelope** around these frozen bytes and a newly built source graph. It retains the historical capture root correspondence, exact retained `rustc -vV`, compiler flags, producer binary digests and linker-patch digest. Its manifest/lock inventory and `frozen-raw-replay` command are test scaffolding, not claims about a fresh Cargo/compiler run or a newly attested capture. The config binding is an explicitly trusted test declaration (including a bounded cfg inventory), not evidence that cfg was independently re-captured. Raw compiler provenance and current replay-envelope provenance must not be confused. No new producer execution, launcher invocation, generic virtual repair or ambient build validation is claimed. The existing `cga-format.json` remains explicitly synthetic; its report carries `SyntheticCapture`.

These bounded tests establish correlation and rejection behavior for the retained fixtures. They do not establish the correctness of arbitrary producer output or arbitrary Rust. R8 owns complete regression acceptance; root owns architecture, notebook verification, trusted capture/config authorization and final approval.

## TDD and verification

Commits:

- `e32aac6021f9b88926fc90e1c936fb7f2ce4c201` — initial failing API tests and nine fresh PRE receipts.
- `0972950c8158f6e68decd295b3b8947a231c499e` — long-form check-flag regression and corrections to synthetic fixture spans/forged-owner selection.
- `045e4eea1fc1e1334e559d0661985a2a63569a28` — implementation and minimal flag-parser fix.

Exact commands/outcomes are recorded in `verification.json`. All Cargo invocations used `SPUR_REMOTE=0 scripts/spur-cargo`.

```bash
# Initial RED: exit 101, 15 missing-new-API compilation errors, honestly compile RED.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_compiler_validation

# Refinement assertion RED: exit 101, accepted explicit overflow=off with semantic checks=true.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_compiler_validation conflicting_long_form_check_flags

# Target GREEN: 8 passed; emits actual reports, calls, facts and 21 runtime rejections.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_compiler_validation -- --nocapture

# Final combined GREEN: 101 passed, 0 failed.
SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo test -p spur-graph --test panic_compiler_validation --test panic_compiler_evidence --test panic_call_sites --test panic_method_sites --test panic_semantic_contract --test panic_semantics --test panic_evidence_end_to_end

SPUR_REMOTE=0 CARGO_TARGET_DIR=/private/tmp/r1-rework2-target-9ac34ff5 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt --manifest-path /private/tmp/r7-scoped-fmt/Cargo.toml --check -- --config skip_children=true
git diff --check
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R7/verify_evidence.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R7/verify_receipts.py
```

Crate check passed with the existing unused `FileOidMatch::as_bool` warning. Scoped formatting passed; `format-manifest.toml.txt` preserves its exact file list. No snapshots/goldens were changed or fed back as inputs.

Operational history is retained: the first cold default-target build was interrupted (130); only its regenerable cache was cleaned. The first implementation run encountered a full-disk error (101); `spur-cargo clean -p spur-graph` in the warm target freed package cache before retry. The next runtime run passed five tests and exposed two test-fixture mistakes: selecting the unchanged owner and a header span extending into the body. `green-second.log` retains those failures; correcting those inputs produced seven passing tests before the separately recorded long-flag RED/fix. None of those operational/fixture failures was treated as semantic proof. Terminal empty log lines are normalized with reconstructable original hashes/counts in `log-normalization.json`.

## SOLVE PRE and POST

Catalog discovery selected implemented `data_integrity.mutually_consistent` and `workflow.transition_allowed`; their complete rule guidance is retained in `catalog/`. `foreign_key` was inspected but not selected. No generic request was necessary, so generic `solve_constraint_check` preflight is inapplicable: `solve_rules` performs family validation itself.

Nine fresh initial PRE requests ran and were persisted/reloaded before tests/production edits. They verify the proposed finite source/build/owner/edge identity policy and observation-only transitions: valid controls pass, forged identity and address/virtual/unknown promotion controls fail. These initial tuples are a policy abstraction with `valid`/`forged` labels, not a source-origin attestation. Representative IDs: `sol_4e7802cf248c41b5` (valid identity), `sol_f11946f3917f4662` (invalid source), `sol_c516a04e071345aa` (observation-only transition), `sol_71931ef9debc495e` (address promotion rejected).

A supplemental exact raw-tuple request exceeded the family's 64-variable cap before implementation. `pre-raw-request.json` and `raw-pre-size-error.json` retain the rejected request/error; it produced no receipt or proof. Before the long-flag test/refinement, two fresh supplemental PRE receipts use the exact emitted build/config/source/owner/edge tuple: `sol_a080df3311254b79` (pass) and `sol_c0f650ece31348e9` (forged source rejected). The chronology is preserved rather than presenting this refinement as the initial PRE.

Seventeen POST requests consume **actual exported correlation reports** from the final GREEN run, not unchanged manual PRE fixtures. Independent normalization cross-checks frozen raw bytes, source blob/SHA identities, raw reference/callee/category/span tuples, exact body anchors, call/operation joins, preserved unknowns and producer provenance. Each observation is checked separately to respect the catalog variable cap. Negative controls alter actual capture/source/body/edge/category values, including a different valid same-file body. Workflow requests consume the shipped disjoint category lists and reject execution/feasibility/discharge promotion. Representative POST IDs:

- `sol_c76b34f161a74a8d` — observation 0 exact source/build/body/raw-reference tuple passes.
- `sol_147c8999393a43bd` — different real body rejected.
- `sol_61d5f2520798440f` — separate observed categories pass.
- `sol_790133e31b9b4cdb` — address execution rejected.
- `sol_99539bba3fa3451f` — virtual feasibility rejected.
- `sol_5914ff0e5c504dd1` — unknown discharge rejected.

`solve-manifest.json` lists all **28** persisted/reloaded IDs, exact request hashes, phases, rule IDs and statuses. `solve-pre.json`, `solve-pre-flag-refinement.json` and `solve-post.json` retain exact executed family requests, full responses and `get_solve_result` reloads. Reloads contain the catalog-compiled typed requests; the original family envelopes are retained alongside them. Independent receipt verification evaluates each finite policy and every returned SAT model against all compiled hard constraints. Family UNSAT means rejection of that supplied finite snapshot/trace, not unbounded safety. No `.spur/solver` files were inspected.

## Retrieval and tracking

`retrieval.json` retains the actual assigned issue and plan check plus the two required failed graph-first attempts: orientation failed to prepare the analyst query, and exact `CaptureBuildIdentity` symbol search reported no graph artifact. Bounded raw fallback covered the named panic-evidence modules/tests, frozen inputs, capture provenance and wrapper/previous task verification instructions. No graph build or publication was attempted.

Tracker audits contain compact local path/commit references only. Intent audit `c65c02c7-ccba-433e-a6b0-d48f3b8216ab` was accepted on **bd-399**. Completion evidence is submitted to that same actual assigned issue; detailed sources and solver receipts remain local.
