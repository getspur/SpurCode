# R8: current bounded panic evidence

Rationale: bind the existing panic pipeline to exact current source, persisted solver evidence and observed standalone native replay.

The canonical ledger is [`final-evidence/verification.json`](final-evidence/verification.json). This R8L reconciliation belongs to **bd-1sr**, delegation **17621941889c4fe6**; it leaves the issue **in_progress** for root review. Root approved **R8G**, reviewed the cumulative R8 source diff, and retains overall acceptance and the final Notebook MCP experiment. Inclusion in the base does not confer approval. This task changes documentation/evidence only.

The approved R7 baseline is `1427759de0a3caa00104e21b2dbc17230d75e82d`; the original R8 source tip is `6f350edbb1d0c8bd7f4ce841036d672295052301`. The cumulative [source diff](final-regression/r8-cumulative-source.diff) covers 18 files, 1,791 additions and 39 deletions. All **2,434** source/build inputs still match R8G manifest `9cf6d4cebe4754404ff5b5240a49cb3ac2baa4274934f5d8a35ccfb195d52b46`. Exact per-file hashes, all 17 original commits and the complete acceptance checklist are in the ledger.

## Current verification

R8G's source-matched complete local regression exited **0**: **1,292 passed, zero failed, 19 existing ignores**, across **98 targets**, including both doctests. All-target compilation, crate formatting and cumulative crate diff checks exited **0**. R8L independently checks those raw gzip logs, hashes and totals; it does not relabel them as a newly executed suite.

All R8G Cargo commands used `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, `RUST_TEST_THREADS=1` and isolated target `/private/tmp/r8g-bd-3d9-b2ae97ed-target`:

```bash
scripts/spur-cargo test -p spur-graph --no-fail-fast
scripts/spur-cargo check -p spur-graph --all-targets
scripts/spur-cargo fmt --check -p spur-graph
git diff --check 1427759de0a3caa00104e21b2dbc17230d75e82d HEAD -- crates/spur-graph
```

The complete exact environments/arguments/outcomes are retained in `final-regression/*.command.json` and the ledger. The earlier sandbox run's five listener denials remain failures; the accepted local-listener rerun executed the entire suite without skips. Prior attempt-1's three failed expectations, attempt-2 disk-full run, attempt-3 missing-rlib doctest, expired attempts, failed GREEN attempts and rejected audit requests remain preserved. [`final-evidence/historical-root-readme.md`](final-evidence/historical-root-readme.md) preserves the previous top-level README verbatim; its earlier incomplete results are historical. [R8G's README](final-regression/README.md) explains recovery.

The hash-verified retained example SHA-256 is `984ee7bc02555b9c831addac41ce44d0bf323bf685f011c52078f815363eb5cd`. Direct execution must pin the per-process compiler selection:

```bash
RUSTUP_TOOLCHAIN=1.94.1-aarch64-apple-darwin TMPDIR=/private/tmp \
  /private/tmp/r8g-bd-3d9-b2ae97ed-target/debug/examples/panic_evidence < request.json
```

The host default in `/private/tmp` is a different compiler. A pinned executable alone does not pin rustup. Compiler identity equality remains enforced; no global configuration was changed.

## Fresh R8L source/solver/native controls

[`final-evidence/probes/`](final-evidence/probes/) retains exact fixture sources, snapshots, analyze requests/canonical facts, solve/replay requests, responses, command environments/exits, identities and actual payloads. A new physical assertion at value **73**, with payload **`R8L held-out seventy-three`**, produced a solver candidate and exact native reproduction under **both** overflow settings. Candidates retain `runtime_reproduced: false`; separate replay records establish the observations. Ten source/build/request/model/receipt forgeries were rejected, including valid-shape candidate changes. Both wrong-expected-payload controls observed the actual panic and recorded **panic_payload_mismatch**, not reproduction. Two nonpanic-domain controls returned bounded no-counterexample results; two formatted-macro controls retained **incomplete_semantics** with obligations despite raw UNSAT.

Fresh catalog PRE `sol_8012a01052bc41bb` verifies measured source/binary identity tuples. Six exact exported generic requests were individually preflighted, persisted and reloaded through worker MCP. Catalog POST `sol_1d4322548eab4b3a` verifies actual current control outcomes and summary/source bindings; adversarial altered-binding `sol_2e0d994bc9544089` returns expected UNSAT/fail. The ledger lists **all nine MCP solve IDs**, exact requests/responses and complete reloads. Rust branch semantics have no applicable catalog family, so the generic fallback follows exact `solve_constraint_check`; catalog tuple rules use the implemented data_integrity family.

The CLI's process-cwd receipt store differs from the worker MCP endpoint: its six emitted IDs return unavailable there. Native replay itself reloads and validates the original persisted CLI receipt; separately identified MCP POST solves use the exact current exported requests and matching status/model. No receipt IDs are substituted. Every lookup/error is retained; no private solver storage was read.

R8G separately retains **14 × 256 = 3,584** ordinary native/solver finite cases and **19** exact replay records. The ledger records assertion, bounds and checked/plain-overflow source/build/request/receipt identities, harness/binary digests and actual payloads. Bounds observe `index out of bounds: the len is 3 but the index is 3`; checked-add unwrap observes ``called `Option::unwrap()` on a `None` value``; checked overflow observes `attempt to add with overflow`. Overflow checks off yields bounded UNSAT for plain addition. Repeated historical runs are not summed as independent proofs.

## Historical inventory and chronology

[`final-evidence/historical-inventory.json`](final-evidence/historical-inventory.json) inventories **431** referenced IDs from attempts 1–3 and R8G, with every fresh `get_solve_result` response under `historical-reloads/`, exact historical receipt matches where available, PRE/POST request/response/reload paths, request-only exports and native observations. **173 IDs are available and 258 unavailable**; every available complete receipt exactly matches its saved historical receipt. Unavailable/evicted receipts are recorded explicitly; saved historical bytes remain immutable. The new source-derived receipts above provide current evidence without fabricating historical reloads. SAT candidate_witness and UNSAT/incomplete_semantics stay distinct.

Original authored chronology remains: `8866d0215` initial API compile RED; `909105052` corrected compiler-report assumptions; `f5413b49e` pinned the compiler precondition before intended assertion RED; `7706faf5b` JSON/replay implementation. Assertion/dispatch/namespace tests `5e9425c30`, `f25e2775c`, `88acce5d6`, `693bff11c` precede fixes `779fdc77d`, `50031882a`, `5e76699e5`. Recovery/checkpoints are `6fba3bf04`, `4334b6814`; enclosing-generic test/fix are `1e243175a`/`8b2efe2e3`, with corrected GREEN chronology retained, followed by `6f350edbb`. R8G recovery is `6b7f18f81`. No new failing test was manufactured for this documentation task. R8L's snapshot setup error, evidence-script outcome-label correction, unused-variable preflight correction and CLI receipt-store mismatch are recorded in `final-evidence/corrections.json` with original command outputs retained.

## Scope and remaining authority

The assertion lane supports a bounded physical, unqualified standard `assert!` contract over supported primitive predicates and literal/default messages. Source/build/body/order/guard authority comes from canonical recollection and conservative namespace checks. This is not general compiler macro resolution: unsupported qualified/shadowed/generated/formatted macros, primitive type/generic ambiguity, trait/cfg/effect uncertainty retain unknown obligations. Panel/Widget incompatible targets remain unresolved. GitPath implicit trait target/bind metadata stay unresolved through rebind/persistence; explicit UFCS/inherent positive controls and receiver scope evidence are retained.

Graph persistence remains **schema v11**. Separately versioned structural sidecar v1 support and producer semantic extension v2 are distinct; accepting structural v1 alone confers no primitive semantic authority. Legacy APIs remain. CGA FnPtrCast is address evidence, virtual calls remain candidates, and no complete `spur::main` capture exists.

UNSAT is limited to the exact encoded domain and assumptions. Unknown obligations remain unknown. Fixture sources and compiler are trusted local inputs; hashes bind local observations, not producer authenticity, arbitrary-workspace execution safety or general Rust semantic completeness. The broader R8G/old finite enumeration is not relabeled as a new independent semantics proof. Root still owns integration, final notebook verification and overall acceptance.

## Recheck the package

```bash
python3 docs/research/artifacts/2026-11-01-panic-implementation/R8/final-evidence/verify.py
git diff --check
```

The verifier reads tracked source and retained evidence only. It checks all source/evidence hashes, frozen chronology, exact raw-log counts/commands, generic receipt/request/source bindings, native replay digests/payloads and historical availability. It does not access private solver storage, old worktrees, live old targets or notebooks, and does not mutate historical artifacts. `verification-result.json` records its actual outcome. Git history supplies the complete unified task diff; `final-evidence/unified.diff` is the focused documentation/script review diff, with detailed evidence additions bound by `hashes.json` and the authored commit.
