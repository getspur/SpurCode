# S2 shutdown ownership rework

**GREEN, root Jev POST review pending.** Rationale: a timed-out close retains responsibility for failing active work and reclaiming its child, while an already expired idle close leaves a usable retry. The full crate diff is `worker.diff.gz`; prior evidence and original test/fix history remain in the neighboring directories. Carry-forward commit `eb1886d0b`, reviewer test commit `3282b2fd4` (imported from `2983ccaa4`), concurrency test commit `4cc0fbb47`, production fix `373aeb536`. The original attempt's source commits and RED evidence are preserved in `../review/` and the supplied protected refs.

All test/check commands below used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo`; fmt used the same environment except the target is unnecessary. The real-process tests used authorized escalation for `ps`. Outputs are byte-preserved, gzip compressed under `logs/`; `identity.json` binds source, raw log, compiler and executable hashes. `observations.json` is extracted from the final log, with actual monotonic offsets, PIDs, generation/request IDs, wait statuses, and lifecycle events. These are fault fixtures, not real embedding acceptance.

| Command suffix | Result |
|---|---|
| `test -p spur-embedding --features test-support --test root_close_review -- --nocapture` before fix | exit 101, 0 passed, 2 runtime assertion failures (`red`) |
| same with `busy_close_wakes` filter before fix | exit 101, 0 passed, 1 runtime assertion failure (`queue-red`) |
| `test -p spur-embedding --features test-support -- --nocapture` | exit 0, 32 passed including 3 close, 15 supervisor and 14 protocol/schema tests (`final`) |
| `check -p spur-embedding --all-targets --features test-support` | exit 0 (`check`) |
| `clippy -p spur-embedding --all-targets --features test-support -- -D warnings` | initial exit 101 for imported test's missing assertion message; corrected, exit 0 (`clippy`, `clippy-final`) |
| `tree -p spur-embedding -e normal,build` | exit 0; serde, serde_json, thiserror, uuid and transitive dependencies only (`deps`) |
| `fmt --check` | exit 0 (`fmt`) |

The idle test records `Timeout`, then `Ok(())`, a correlated `Closed`, normal wait status, and `ps` absence before Drop. The busy test records `Timeout`, `Err(Closed)` for the request, kill, wait, pipe closure and I/O join; `ps` confirms absence before the test's manual cancellation. The separate concurrent-close test records a queued `Err(Closed)`, duplicate `Err(Closed)`, active `Err(Closed)`, and reap before safety cancellation. Its observed events include I/O join. The printed busy elapsed values distinguish the close call from the subsequent observation. Neither test proves a hard real-time OS reap bound.

The root-authored PRE and failed historical POST are preserved in `../root-review/solve-review.full.json.gz`. `reload-check.json` records five fresh exact full-reload and request comparisons; `../root-review/verify.py` verifies the historical archive. The root executed Jev; this worker did not. The historical failed POST must not be mistaken for this fix's POST. `expected-policy.json` states the independent policy; `post-candidates.json` contains the exact family request candidate, rejected controls and bounded projection from current observations. The root must freshly compile/inspect Jev and solve/persist/reload actual-output POST and negatives before approval. No self-approval, S3 work, native inference, or downstream integration is claimed.
