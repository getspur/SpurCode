# M01 scope repair

ROOT issue `bd-z7na`; canonical worker issue `bd-3txu`; plan
`46b921e6-f999-4014-bd35-931b84eac903`. Attempt1 delegation
`e61d8717f1124e34` stopped correctly at its three-file boundary. Its normalized
checkpoint is `1c80f84cbd1059aa39a9436613c62d847537c4e0`. No constructor migration
was implemented, so approval and actual POST are withheld.

ROOT read the genuine scope signal
`4b2c96e7-7020-4765-b8d7-244367b7c3d0` (comment4659), scope audit4657 and
incomplete handoff4661. The core source has no production module declaration.
Its old library filter returned zero passed/failed/ignored and1,601 filtered,
so it did not execute the fixture. The retained file depends only on existing
spur-graph/tokio/futures/std APIs and contains three original unit tests.

The corrected fourth write path is a test-only integration target:
`crates/spur-core/tests/legacy_rebuild_singleflight.rs`. It imports the actual
retained file with a path module, allowing its existing tests and helper to
execute without connecting the old coordinator to the application. Production
module wiring, Cargo/dependencies, source copying and cache behavior are outside
scope. The three original caller files still receive only mechanical migration
and meaningful native observation. Existing assertions remain unchanged.

Use `scripts/spur-cargo test -p spur-core --test legacy_rebuild_singleflight` for
the actual core baseline and after-migration run. Merely observing a zero-test
library filter cannot satisfy acceptance. Introduce/compile the harness before
migrating the fixture; capture exact native before/after bytes/values and source
identities. New harness execution is prospective at this repair checkpoint.

Attempt1's20 valid native records cover the other four construction-site kinds.
The actual current analyst overlay failure reproduces historical bd-16eq:
left0 versus right1. It remains an explicitly preserved baseline risk, with no
out-of-scope repair, skip or expectation change. Analyst library18 passed,
overlay5 passed/1 failed, CLI14 passed, core0 executed. No migration or after
comparison was claimed. One interleaved, unparseable observation line was
explicitly excluded; its bytes remain in the log and must not be reconstructed.

This scope repair is part of the authorized generic carrier migration. It keeps
the same serial DAG and worker preference (Codex/gpt-6.1-sol/xhigh). ROOT will
bind the amended plan, current module/source bytes, original incomplete evidence
and new-path absence in a supplemental prospective PRE before retrying M01.
The original PRE/archive remains historical and unchanged. Main remains the
approved G03 commit `c1e11019ab6b0eb32d062c30f17ac8d970fcd89d` with user edits
preserved; no M01 implementation is merged.
