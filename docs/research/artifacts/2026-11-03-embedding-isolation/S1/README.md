# S1 embedding protocol — bd-339u

Status: **GREEN, root Jev POST review pending**. The root-owned POST compile, inspection, solve, persistence and reload have not been performed by this worker. This is not an approval or a native lifecycle result.

The native-free crate defines versioned frames, model identities, configurable limits and correlated request/reply validation, so a later supervisor can exchange bounded messages with an isolated worker.

## Provenance

Base `1a50b5b304a90bd9ef64de619390fbed8a50e9b9`; code/test source head `56f8fec24` before this evidence-only commit. [code.diff](code.diff) is the unified code/test diff from that base. [post-candidate.json](post-candidate.json) records full source and test-binary SHA-256 identities, rustc version, explicit test limits, independent expected policy, verbatim runtime observations and candidate POST intent/negative controls. [test-output.txt](test-output.txt) is the full local test output; [dependency-tree.txt](dependency-tree.txt) is the normal/build closure.

[root-pre.json](root-pre.json) is the brain's unmodified PRE artifact, SHA-256 `e2b3a99f2c5eeb51441ac0ce872dcfdbbe2660be69f4c2658c84bcb6dba93884`. The brain compiled Jev; this worker did not. Its gate was open, family `configuration`, mode `verify`, rule `configuration.attribute_allowed_pair` for `(envelope, receiver)` with empty parameters; eight independently added catalog `requires_any` bindings cover the declared obligations. The PRE encodes prospective policy, not observed implementation behavior. This worker called `get_solve_result` on all four IDs, compared the entire returned lowered request to the artifact reload, and found each equal:

| Case | Receipt | Reload status |
|---|---|---|
| Declared contract | `sol_c206330cb5294103` | SAT |
| Wrong internal version | `sol_a10c326147e94f69` | UNSAT |
| Missing allocation guard | `sol_f73c1b3b273b42da` | UNSAT |
| Native dependency allowed | `sol_57804b1cbe844091` | UNSAT |

## Test-first record

All cargo commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo`.

| Test commit | RED command after test/scaffold | Observed RED | Implementation commit |
|---|---|---|---|
| `f2cdc88ca` | `test -p spur-embedding --test protocol` | exit 101, unresolved imports for the new API (compile RED, not assertion failure) | `b7cd1ee9f` |
| `4620569eb` | `test -p spur-embedding --test protocol pending_requests_are_bounded_and_consumed_only_on_valid_completion` | exit 101, missing `max_pending` and `complete_reply` (compile RED) | `9ce934b6a` |
| `e407832ef` | `test -p spur-embedding --test protocol embed_requires_completed_hello_and_shutdown_closes_session` | exit 101, runtime assertion failure: embed accepted before Hello | `a3c239afe` |

The first full run after the pending-limit implementation also exposed the older test retaining two completed requests; it was corrected to consume replies, then the full suite passed. `da05b1b9f` adds boundary and observed-case exports; `56f8fec24` resolves lint-only issues.

| Verification | Result |
|---|---|
| `test -p spur-embedding -- --nocapture` | exit 0; 10 tests passed; doc tests passed |
| `tree -p spur-embedding -e normal,build` | exit 0; no FastEmbed, ORT, ort-sys, DuckDB or libduckdb-sys |
| `fmt -p spur-embedding -- --check` | exit 0 |
| `clippy -p spur-embedding --all-targets -- -D warnings` | exit 0 after correcting initially reported crate-local lints |

## POST handoff and limits

Use the exact `jev_post_intent`, `independent_policy`, `actual_observations`, source hashes and test binary hash in [post-candidate.json](post-candidate.json), together with the raw dependency tree. Compile and inspect the actual-output binding and solve it with persistence/reload. Apply the listed inverted/missing/duplicate/dependency-injection controls; never substitute the PRE design facts for runtime observations. The header-only probe's passing assertion shows the read was not attempted; its oversized error alone would not establish that property. The finite cases are a bounded projection, not a proof over every possible input. `Closed` encodes the shutdown acknowledgement; actual cleanup is the later native worker's responsibility.
