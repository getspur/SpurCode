# S1 attempt 3: strict nested schema repair

**GREEN, root Jev POST review pending.** Strict intermediate deserialization uses empty struct variants to enforce their empty field sets while preserving the public unit variants and existing JSON. No supervisor or native inference work is included. [Unified code diff](code.diff); [actual POST candidates](post-candidate.json).

The carried-forward candidate is commit `d6f0b0cd9`; imported root RED `4d09e55b4`; variant test `d03fd911e`; implementation `c5831eea1`; lint-only expectation `05425b36b`. Original authored history and PRE evidence remain in the parent S1 directory. Source/build identity in the candidate is `05425b36b4ff6fb9b541608118b40fb1dc5902c2` before this evidence-only commit.

All commands below use `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo`.

| Command suffix | Result and evidence |
| --- | --- |
| `test -p spur-embedding --test root_wire_review -- --nocapture` | Exit 101 after successful compilation, 0 passed / 2 assertion failures. Both malformed variants decoded `Ok`; [red.log](red.log). |
| `test -p spur-embedding --test variant_schema -- --nocapture` | Exit 101 after successful compilation, both tests failed precisely on unknown unit-variant fields; [variant-red.log](variant-red.log). Other variant outcomes were retained. |
| `test -p spur-embedding -- --nocapture` | Exit 0; 10 original + 2 root + 2 variant tests passed; [green.log](green.log). |
| `fmt --package spur-embedding -- --check` | Exit 0; [fmt.log](fmt.log). |
| `clippy -p spur-embedding --all-targets -- -D warnings` | Exit 0; [clippy-green.log](clippy-green.log). Earlier lint failures and the intermediate rejected `allow` are retained in [clippy.log](clippy.log) and [clippy-allow.log](clippy-allow.log). |
| `check -p spur-embedding --all-targets` | Exit 0; [check.log](check.log). |
| `tree -p spur-embedding --edges normal,build` | Exit 0; no fastembed, ort, ort-sys or DuckDB in [dependency tree](dependency-tree.txt). |

The root compiled the PRE; the worker did not invoke Jev. The unmodified original [root PRE](../root-pre.json) has SHA-256 `e2b3a99f2c5eeb51441ac0ce872dcfdbbe2660be69f4c2658c84bcb6dba93884`; the root's prospective [repair PRE](../root-review/pre-repair.json) has SHA-256 `9cd9c178622558d77af5a834c7cb07f134c496a8617e5c9ea117d611e729142e`. The former selects `configuration.attribute_allowed_pair(envelope,receiver)` in verify mode with empty parameters and eight explicit `requires_any` obligations. The latter selects `data_integrity.mutually_consistent(case_policy)` in verify mode with empty parameters and two prospective reject outcomes. Both gates were inspected open by root. These are policy inputs, not actual behavior.

All seven persisted results were reloaded with `get_solve_result`; each complete lowered request equals the corresponding root artifact reload and its status equals the artifact's expected status. Full reloaded envelopes are saved beside this README:

| PRE case | Receipt | Status |
| --- | --- | --- |
| Contract | `sol_c206330cb5294103` | SAT |
| Wrong version | `sol_a10c326147e94f69` | UNSAT |
| Missing allocation guard | `sol_f73c1b3b273b42da` | UNSAT |
| Native dependency allowed | `sol_57804b1cbe844091` | UNSAT |
| Both unit cases reject | `sol_c552ee9e185a4ece` | SAT |
| Shutdown still accepts | `sol_ae996c56d5bd41de` | UNSAT |
| Closed still accepts | `sol_b20d44499efc4dae` | UNSAT |

POST handoff: [post-candidate.json](post-candidate.json) contains 43 actual observations extracted from the current test output, independent expected policy, seven disjoint partitions of at most seven cases, source/test/binary hashes, exact intended verification, and controls. Root should first enforce global exact identity, uniqueness, count and coverage; then compile/inspect/solve each partition and source/dependency bindings, persisting and reloading each receipt. Controls include omission, duplicate, stale source, injected ORT and outcome inversions including both repaired cases. The oversized header error must be combined with the passing read-probe assertion. These finite snapshots do not prove universal parser correctness or native cleanup. POST receipts remain pending root execution.
