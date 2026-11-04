# S2 root review: shutdown ownership remains red

The review rejects approval of candidate
`0eb801f3068b4e9eba4fcaf58d24d15311e4e98b`, delegation
`c4f7313a79eb4003`, canonical issue `bd-14ak`. S3 remains gated.
Root changed no production source. Test-first regression commit:
`2983ccaa4f72a0da415ed918dfaec08e549fe638`.

The original worker evidence verified: 30 byte-preserved logs, six source
hashes, 12 case rows, four traces and 22 recorded PRE comparisons. Both
inherited executable hashes matched before the independent rebuild. The
independent root baseline passed all 29 tests (15 supervisor and 14 protocol)
and produced another 12 cases/four traces. The build paths changed, so root
records its own executable hashes separately in `root-identity.json`.

Two additional real-process tests compiled and failed at their intended
assertions, exit 101, zero passed/two failed:

| Case | Observed result before test safety cleanup |
|---|---|
| Expired close of an idle live worker | First close returned `Timeout`, retry returned `Closed`; the child remained alive and no reap occurred during the three-second observation window. |
| Close deadline behind a hung active request | Close returned `Timeout`; the request remained unfinished and the child remained alive/unreaped during the three-second window. Manual cancellation by the test then released it. |

The tests use the real fault executable and `ps` process inspection. Their
manual cancellation/client Drop is safety cleanup, and is explicitly excluded
from the success predicates. Raw events and observations are preserved in
`root-close-red.log.gz` and `root-close-red.json.gz`. The busy-case `elapsed_ns`
ends after the observation window; it is not the duration of the shutdown
call itself. The fixture's request policy is five seconds, close caller budget
100 ms and observation window three seconds, reusing existing fixture choices.
These are test inputs, not product defaults or real-time OS guarantees.

The causal source path is `acquire(shutdown=true)`: it marks `gate.closed`
before checking the deadline or waiting for the active permit. Its early
timeout returns do not transfer cleanup responsibility or signal the active
request. Every later close immediately rejects the closed gate. Thus the
first case has no shutdown cleanup path; the second relies on an unrelated
request deadline, caller cancellation or client Drop.

Root ran fresh catalog discovery and Jev PRE before adding the regressions.
The finite allowed `(case, disposition)` tuples require cleanup rather than
stranding. The positive PRE passed (`sol_cf65ebea7b324138`); a deliberately
stranded control was rejected (`sol_717f6fb1188d4abe`). Jev PRE opened and the
exact `data_integrity.mutually_consistent` binding was inspected.

Root then compiled fresh Jev POST from the actual `ps`, reap and completion
observations. Its gate opened with unchanged policy/facts and no unknowns.
All three actual verification requests were UNSAT/fail:

- Both regressions: `sol_bccd43a8de1b4cc8`.
- Expired idle close alone: `sol_c8c35b198e56453a`.
- Busy close deadline alone: `sol_ae375b78ea7e44c4`.

All five receipts were persisted and reloaded. The archive
`solve-review.full.json.gz` retains exact Jev inputs/provenance, original
family requests, solver outputs and complete lowered reloads. The manifest
binds every local evidence file. Run `python3 verify.py` from this folder.
Source verification uses the immutable candidate/test Git objects, so a later
implementation fix does not rewrite or invalidate this historical rejection.
This is a rejected actual-output POST; no whole-task acceptance is implied,
and no unbounded liveness claim follows from this finite observation window.

Both Rust runs used `SPUR_REMOTE=0 TMPDIR=/private/tmp
CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo`.
The baseline suffix was `test -p spur-embedding --features test-support --
--nocapture`; the new RED suffix was `test -p spur-embedding --features
test-support --test root_close_review -- --nocapture`. Local formatting and
the staged diff check passed. The runs required the already authorized
process-inspection permission; the actual cargo exit codes are preserved.

The next attempt must preserve the original implementation/tests and these
two failing tests, repair close ownership with test-first changes, rerun the
full neutral-crate suite and export fresh runtime/source/build observations
for root POST. An expired idle close may leave a usable retry or initiate
owned asynchronous cleanup; it must not strand the child behind a closed
gate. A close that cannot drain active work by its deadline must explicitly
fail pending work and initiate termination/reaping/IO completion. Keep caller
return deadlines separate from eventual observed OS cleanup. Test queued
waiters and concurrent/duplicate close interactions too. No native runtime
or downstream integration changes belong in S2.
