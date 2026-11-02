# R8G isolated final regression

Rationale: recover the exact late R8 evidence and establish a complete, source-stable local regression without changing production code or existing tests.

Assigned issue **bd-3d9**, delegation **b2ae97eddc1f44f8**, plan task **R8G**. The corrected intent audit was accepted as comment **1868**. The prior attempt's `intent-audit-error.json` is unchanged (SHA-256 `7d0d644123f88489c0da7c4a4c039f0094ebf8752fe41d0ddfffb8dce7791be7`). The issue stays **in_progress** for root review.

The complete successful regression exited **0**: **1,292 passed, zero failed, 19 existing ignores**, with all **98 targets**, including **both doctests**, finishing. All-target compilation, crate formatting and the cumulative crate diff check also exited **0**. Every command's source and frozen-evidence hashes match before and after. `verification.json` is the machine-readable ledger; `full-suite.result.json` contains every target summary.

Recovery commit: `6b7f18f81` (`docs(spur-graph): R8G recover exact frozen attempt-3 evidence`). The evidence commit containing this README records the remaining new outputs. `unified.diff` contains the task diff against `6f350edbb1d0c8bd7f4ce841036d672295052301`, excluding itself and the subsequently generated integrity inventory/receipt. `hashes.sha256` covers the payload, including that diff; `integrity.json` records the inventory verification. Git binds the entire artifact set.

All **388 files / 8,545,113 bytes** in the frozen manifest and the patch SHA-256 were checked before import. Exactly **48 tracked overrides and five untracked late files** required restoration. All restored bytes match their frozen originals; `attempt-3/` has remained immutable since the recovery commit. The snapshot source was `/private/tmp/r8-lease-recovery-b9b909c5/frozen/`; no old worker worktree was read. `recovery-state.json`, `frozen-manifest.json`, and `recovery-verification.json` retain the evidence. The frozen manifest's canonical digest is `52377665fee83e12b6420634212cb09d965cce94c9c1a76509b92bd08d6b0a5d`; the patch digest is `4ff53df070818d76f352bbb39a51fa7f70891fdd577935557e66a352805abd28`.

The **2,434 tracked source, test, fixture, manifest, Cargo configuration and build-script inputs** have canonical manifest digest `9cf6d4cebe4754404ff5b5240a49cb3ac2baa4274934f5d8a35ccfb195d52b46`. Gitlink inputs are recorded by their indexed OID, symlinks by their link text, and ordinary files by their exact bytes. The initial Git diff check established equality with the dispatched source. `source-before.json`, every command's `.before.json`/`.after.json`, `source-after.json`, and `source-final.json` retain complete manifests. No source/test/golden/ignore changes were made by R8G.

The dedicated target directory is **`/private/tmp/r8g-bd-3d9-b2ae97ed-target`**. It started empty, is owned by this task, and is distinct from all old worker targets. `R8G_OWNER.json` inside it records the owner. Commands ran sequentially, with recorded start/finish times checked for non-overlap. Available capacity was about **33.7 GiB** initially and about **25.8 GiB** after the suite. No cleaning or shared-cache mutation was required.

All Cargo commands used this common environment, recorded exactly in each `.command.json`:

```text
SPUR_REMOTE=0
TMPDIR=/private/tmp
RUST_TEST_THREADS=1
CARGO_TARGET_DIR=/private/tmp/r8g-bd-3d9-b2ae97ed-target
R8_EVIDENCE_DIR=<this worktree>/docs/research/artifacts/2026-11-01-panic-implementation/R8/final-regression/exports
SPUR_SCCACHE_S3=0
RUSTC_WRAPPER=
CARGO_INCREMENTAL=0
SPUR_GRAPH_BLESS unset
```

| Exact command, in execution order | Exit / outcome |
| --- | --- |
| `scripts/spur-cargo test -p spur-graph --no-fail-fast` (sandbox) | 101; 1,287 passed, five loopback listener denials, 19 ignores; all 98 targets finished |
| `scripts/spur-cargo test -p spur-graph --no-fail-fast` (accepted local listener capability) | 0; 1,292 passed, zero failed, 19 ignores; all 98 targets finished |
| `scripts/spur-cargo check -p spur-graph --all-targets` | 0 |
| `scripts/spur-cargo fmt --check -p spur-graph` | 0 |
| `git diff --check 1427759de0a3caa00104e21b2dbc17230d75e82d HEAD -- crates/spur-graph` | 0 |

`verify.py` records command/environment, exit, complete raw output, deterministic gzip, hashes, input manifests and target summaries. The sandbox's five failures were `TcpListener::bind("127.0.0.1:0")` returning `EPERM`, in `memory_benchmark` and `memory_eval_runner`. Its exact command/logs/exports are preserved under `sandbox-denied/`, with a relocation manifest retaining original execution paths and hashes. The accepted `require_escalated` retry ran the **entire** suite, with unchanged tests and no skips. Denied tests were never counted as passes. Existing dead-code warnings remain in raw logs.

The earlier attempt-2 disk-full failure and attempt-3 exit-101 run remain historical failures. Attempt-3 reported 1,290 passed, one failed doctest, 19 ignores, 98 targets; its missing-rlib doctest failure is unchanged. Comparing its raw log with the new complete run shows exactly two changed summaries: the end-to-end target now executes ten tests instead of nine, and both doctests pass instead of one passing and one failing. `historical-suite-comparison.json` records this count reconciliation without rewriting chronology.

Fresh SOLVE used implemented **data_integrity v1** and **workflow v1** families after catalog discovery. PRE finished before recovery or new evidence writes. All seven receipts were persisted and reloaded with `get_solve_result`; exact family requests, responses and complete lowered-request reloads are under `solve/`, indexed in `solve/index.json`.

| Phase / query | Solve ID | Raw status / family outcome |
| --- | --- | --- |
| PRE measured manifest tuples | `sol_4bce3a25ec9a4530` | sat / pass |
| PRE adversarial wrong hash | `sol_a6f42749add74fe0` | unsat / fail, expected control |
| PRE observed recovery workflow | `sol_6f84bdacd034480d` | sat / pass |
| POST current manifests and outcomes | `sol_ee67507b28424683` | sat / pass |
| POST adversarial wrong export hash | `sol_61d80bf01ba145aa` | unsat / fail, expected control |
| POST adversarial nonzero failures | `sol_819c4170e3e24b8c` | unsat / fail, expected control |
| POST observed full verification workflow | `sol_e2ed6d05035c48d4` | sat / pass |

Rules are `data_integrity.mutually_consistent`, `data_integrity.value_range`, `workflow.transition_allowed`, `workflow.initial_state_allowed`, and `workflow.safety_invariant`. POST binds recovery/source/prior-error/export/outcome manifest digests, command exits, zero runtime/doctest failures, 98 completed targets and exactly 19 ignores. The 98 original target rows remain in `outcomes-manifest.json`; the solver uses two caller-computed runtime/doctest aggregate rows to stay within the implemented 64-variable cap (58 variables). `solve/validation-diagnostics.json` retains the oversized-request validation error. The nonzero-failure control substitutes five failures; it is an adversarial supplied fact, not a claim about the successful run. No generic solver fallback was used. These proofs verify only supplied finite facts and normalized workflow order, not Rust semantic completeness or general execution safety.

The new suite exported **71 files / 2,755,546 bytes** under `exports/`; their bytes and hashes are in `exports-manifest.json`. R8L should use these outputs and the following retained binaries (`binaries.json`):

| Binary | SHA-256 |
| --- | --- |
| `/private/tmp/r8g-bd-3d9-b2ae97ed-target/debug/examples/panic_evidence` | `984ee7bc02555b9c831addac41ce44d0bf323bf685f011c52078f815363eb5cd` |
| `/private/tmp/r8g-bd-3d9-b2ae97ed-target/debug/deps/panic_analysis_end_to_end-cab192c7d3de793d` | `7f520da417fd6fa823eee0a27dae376b67c903142e5693a033171e5a253609c0` |

The complete **unapproved original R8** diff against approved R7 `1427759de0a3caa00104e21b2dbc17230d75e82d` is `r8-cumulative-source.diff` (SHA-256 `80a8b51a059ff8235d5bfda871851b946ff3e2f23324b9071ea1a6c4818f9410`). The source tip is `6f350edbb1d0c8bd7f4ce841036d672295052301`. `r8-lineage.json` lists all **17** original commits and all **18 files**, totaling **1,791 additions / 39 deletions**:

- The example adds strict `analyze`, `solve`, `compiler_report`, and source/build/request/receipt-bound standalone `replay` commands while retaining synchronous and legacy APIs.
- Five library files add bounded physical `assert!` collection, validation and analysis; preserve uncertain receiver scope text; and prevent unresolved receiver calls from acquiring targets/bind metadata during artifact rebind. Graph schema remains v11; the producer semantic extension becomes v2 while structural v1 support remains.
- The end-to-end test, seven source fixtures, and resolver/extractor/artifact tests cover both arithmetic modes, canonical source/body/order/guard authority, unknown cases, Panel/Widget incompatibility, GitPath trait uncertainty through persistence/rebind, and explicit UFCS/inherent positive controls. The crate README documents the bounded contract.

Historical RED/test/fix chronology is preserved, not recreated for this evidence-only task:

- `8866d0215` adds initial API compile-RED tests; `909105052` corrects compiler-report test assumptions; `f5413b49e` pins the synthetic compiler control so the intended assertion RED is reached; `7706faf5b` implements the JSON pipeline/replay. Initial compile/precondition failures remain distinct from intended runtime RED.
- `5e9425c30` and `f25e2775c` add assertion witness/authority/continuation tests; `88acce5d6` adds dispatch rebind regression tests; `693bff11c` challenges primitive namespaces. `779fdc77d` implements bounded assertions, `50031882a` fixes uncertain receiver rebind, and `5e76699e5` preserves receiver scope evidence. Retained attempt-2 RED and subsequent GREEN command files keep their actual exits; filenames beginning `green` do not imply success.
- `6fba3bf04` and `4334b6814` recover/checkpoint attempt 2; recovered formatting/export additions do not establish an independent new RED chronology.
- `1e243175a` adds the enclosing-generic regression before execution; `8b2efe2e3` withholds primitive authority under generic shadows. Its first GREEN attempt still exits 101 because of a test assumption about parse-unknown; `green-generic-corrected` exits 0 after accepting only explicit parse-unknown/no-facts or obligation-bearing canonical facts. `6f350edbb` retains the historical checkpoints.

Root still must review that cumulative R8 source diff; its presence in the base does not approve it. **R8L owns historical receipt/source/native/ledger reconciliation.** R8G did not rerun every historical receipt. Unsupported macros/types/traits/cfg/effects remain unknown, bounded UNSAT does not establish arbitrary-workspace safety, CGA FnPtrCast remains address evidence, virtual calls remain candidates, and no complete `spur::main` capture exists. No notebook, graph CURRENT/publication, original CGA checkout/driver/launcher, global configuration/install or external messaging was accessed or changed.

Orientation and exact graph retrieval failed with analyst-query preparation and missing-artifact errors. `retrieval-fallback.json` records the failures and bounded raw fallback used for assigned source/build/evidence paths and exact failing listener-test locations. No graph rebuild was attempted.
