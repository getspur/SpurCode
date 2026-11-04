# N0 retry: incomplete, not review-ready

The test-first compile RED is commit 682198c73, with raw output in
`rust-gap-n0-retry-api-red.log`. The partial implementation introduces a
managed guard, synchronized close and leases for session and several value
paths. It is not an accepted or complete native ownership implementation.

The focused test command `SPUR_REMOTE=0 TMPDIR=/private/tmp
CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo
test -p spur-graph --test ort_shutdown -- --nocapture` exited 0, 4 passed
(including two inert child entry tests and two subprocess-driving tests),
with relevant ORT/DYLD/RUSTFLAGS/update overrides unset. Raw output is in
`rust-gap-n0-retry-partial-tests.log`. The executable SHA-256 was
`dbcc869359773f5a53bc0dc3a8c57be391941638fe250cee2e39a79f186de5a0`.
The source diff against 682198c73 at that test, including the subsequent
format-only test change and before adding these logs, had SHA-256
`3e4a272ccb7846995c622cf9eb6dcefbaebfb2779c9faee3b8cc64cf35dc00eb`.

An earlier graph `check -p spur-graph --all-targets` exited 0 with warnings;
its raw log is `rust-gap-n0-retry-all-targets.log`, and it precedes the final
session-finalization adjustment. `check -p spur-graph --no-default-features
--all-targets` exited 101 with eleven embedding-related unit-test compile
errors; see `rust-gap-n0-retry-nodefault.log`. Workspace `fmt --all -- --check`
and `git diff --check` exited 0. A standalone wrapper all-target check could
not complete because the separate temporary copy required uncached crates and
DNS failed; the workspace graph check did compile the enabled wrapper.

Unresolved: the async callback holds references to output names and borrowed
input backing after future cancellation; an environment lease alone does not
keep those borrows valid. The optional training session also uses the
managed environment without retaining its guard, in a file outside N0's
allowed edit list. The session allocator release order, failure finalization,
standalone ownership paths, actual release counts, full PGQ explicit-close
control, source-bound POST and negative controls remain to be verified. No
full runtime or application shutdown acceptance is claimed. Normal-exit RED
is retained for N1. See issue bd-3b6f audits 2412, 2414, 2415 and 2420 and
signals 6d541e26-c5e1-43cb-9b28-3ba73e8ba527 and
440dbdba-3f45-4c49-bcda-c0949fe916a7.

Additional checks: at source commit `7128551713b96807e2a63ddb52915c34157014b0`,
`check -p spur-graph --all-targets` exited 0; raw output is
`rust-gap-n0-retry-committed-check.log`. A subsequent test-only diff
SHA-256 `b16d8a5009e60ac964020dfc7e6c2128ee933b7c3cf550ebecdbd1d32a7b9ba5`
added explicit checked shutdown after a failed model load, successful inference,
and a real PGQ query. The focused test command exited 0, six passed, including
three inert child entries and three subprocess-driving tests. The raw log
`rust-gap-n0-retry-pgq.log` records inference output and PGQ result. Executable
SHA-256: `17f656d99b4634fe657c2babecf3b56ba522fe36d63d652fce092959b24b9244`.
Invalid model creation fails before native session creation; it does not test
failure after successful CreateSession during finalization. All other unresolved
items above still apply, except the explicit-close PGQ control was exercised.
