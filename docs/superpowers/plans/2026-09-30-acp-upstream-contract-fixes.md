# ACP upstream contract fix plan

1. Read the existing audit and upstream evidence; confirm both local source bodies are unchanged. Discover rules and compile JEV candidates for current ID dispatch and initialization safety. Solve the current failing models and the intended passing models; preserve provenance and exact requests.
2. Add permanent native integration regressions and normal-input controls. Run the focused target remotely with `scripts/spur-cargo`; confirm failures are the two known defects. Commit the failing tests before production changes.
3. Use the advertised model option ID in configuration fallback. Reject unsupported protocol versions before capability caching/Ready and await existing shutdown. Keep changes crate-local.
4. Run focused tests to GREEN. JEV-compile observed post-fix facts and solve using the same rules; retain the original invalid fixtures as negative controls.
5. Run the crate suite and scoped clippy, inspect formatting and diff, request code review, and resolve material feedback. Commit fixes and verification evidence; close both beads issues only after the checks pass.

The user approved implementation and the TDD/JEV sequence. Existing unrelated edits in Cargo.lock and spur-jev must remain untouched.

Execution evidence: focused RED run produced 5 expected failures and 2 passing controls before production changes; committed as `a0e58a6e9`. The same target then passed all 7 tests. `scripts/spur-cargo test -p spur-acp` passed 653 tests with 1 ignored across 37 suites, including doctests. Formatting and `git diff --check` passed. Independent code review found no functional issues; its stale fallback-ID comment finding was corrected.

The remote all-target clippy check failed on 34 existing warnings in six test files (`child_stderr_piping`, `executor_events_roundtrip`, `load_session_error_propagation`, `nested_config_shape`, `orphan_sweep_e2e`, and `skip_permissions_config`). Each file is byte-identical to baseline `3295bce6b5c024528c5e8321f9a647d7e8957098`. This failure is retained in `.spur/scratch/acp-upstream-fixes-2026-09-30/clippy.log`; no local retry or lint suppression was used.

The focused remote check, `SPUR_REMOTE=1 scripts/spur-cargo clippy -p spur-acp --lib --test upstream_contracts -- -D warnings`, passes after correcting the new test's anonymous trait import and adding assertion messages. Review confirmed these corrections preserve test behavior.

Final remote rerun after those lint corrections: `scripts/spur-cargo test -p spur-acp --test upstream_contracts` passed all 7 tests. Approval accepts the documented pre-existing all-target lint failures; the changed library and regression target are lint-clean. Both approved contract fixes are complete.
