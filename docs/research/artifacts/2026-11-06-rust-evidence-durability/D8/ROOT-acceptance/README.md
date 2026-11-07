# D8 ROOT acceptance after empty-capability repair

The eight-line repair preserves `Unavailable` when an empty known Rust domain
has absent evidence on both inputs. A `README.md` shadow no longer invents
captured coverage. Actual captured empty evidence remains `Complete`.

ROOT independently verified the worker's committed source and audit trail.
INTENT 4347 and completion 4351 were persisted and replayed under canonical
`bd-3eyg`; the normalized result is byte-identical to authored receipt tip
`39c9297cb35a5ded5f26a6f54693ad414c4ac7f9`. Authored history is protected at
`spur/review-evidence/g3-d8-empty-repair-authored`. Both ROOT regression tests
and the original D8 test file remain unchanged. The preceding compiled RED
was one pass and one intended assertion failure; the repaired target is two
passes with no failures, ignores or filters.

The required worker targets report 87 passes, one independently established
macro fixture failure, zero ignores and 566 filtered unit cases. The macro
target remains 13 passed / 1 failed: `bd-xb5p`,
`parquet_macro_queries_reject_missing_source_manifest_rows`, fails on stale
graph metadata before the macro query. This target is not wholly GREEN and
the scoped repair does not change that fixture. All-target checking, scoped
formatting and complete diff checking passed.

ROOT resupplied its original 15 independent requests to repaired executable
SHA-256 `86229db1ac56849ab547e6140d1f9571b49310f43306144e1c666960df8b62ea`.
ROOT additionally materialized three explicit artifact pairs without mutation
fields and invoked the delivered replay entry point. These 18 native
invocations passed, with zero failures/ignores and 198 filtered entry tests.
The original 17 cases and 423 observations preserve their exact reviewed
inputs and outputs; the 300 non-counterexample matrix outputs are preserved.
Absent identity and absent non-Rust-shadow controls now return `Unavailable`;
captured empty returns `Complete` and authenticates against the empty inputs.
Legacy absence still reports unavailable source authentication.

Compilation occurred with a stable dirty source checkout at Git HEAD
`287d3020d`. Its complete 2,542-file byte inventory matches clean fix commit
`47d9619acadf3c1f276552a73355f1db0d5887a2`. The capture's original Git HEAD and
dirty status remain recorded; they have not been relabeled as a clean build.
Source, compiler, configuration and executable hashes were checked before
and after ROOT's native runs.

Fresh catalog/Jev POST fingerprint:
`2c96923bc95d01d6ccfa18cdf580721b12fbf4d3953c1bf2ec1f48e45251618b`.
It binds 2,620 source/evidence files, 19 exact observation groups and four
independent false claims. Jev opened the consistency request; ROOT added
case-key uniqueness and exact coverage bindings. All three requests were
validated by the family solver, persisted, pinned and reloaded completely:

| Receipt | Result |
| --- | --- |
| `sol_1333dbcec9c4429b` | SAT/pass, preserved native partition 0 |
| `sol_8ae82f413aae4b48` | SAT/pass, preserved native partition 1 |
| `sol_8119bed20fbb414f` | UNSAT/fail, false empty/site/origin claims |

The independent oracle re-evaluated all nine family/lowered predicates. Both
repair PRE envelopes also matched their archives and their six predicates
were independently checked. The false controls include the previously real
empty-capability upgrade, leaked old call groups in zero/Unknown replacements
and rebinding an untouched source's origin.

Run `python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/ROOT-acceptance/verify_post.py`
to regenerate inputs and verify the stored proof. For later code epochs, pass
`--source-revision` with this consumed committed source/evidence epoch. This
keeps preserved user working-tree edits and future implementation changes
distinct from the tested cohort.

These are finite native evidence/query checks. They do not establish cfg
activation, macro expansion, call-target correctness or complete panic
detection. D9's MCP cache identity and D10's fresh Rust notebook remain later
tasks. Approval/integration records are persisted separately in Beads and
`D8/local-integration/` after this evidence commit.
