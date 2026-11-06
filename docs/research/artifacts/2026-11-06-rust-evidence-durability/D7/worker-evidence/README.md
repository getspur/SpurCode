# D7 interrupted worker evidence

Canonical issue: `bd-bl34`. Delegation: `4c39671c389c48ed`.
Dispatch base: `ff910ce0503680d697dd0a2e5c3db2543e0574e2`.

Work stopped on an evidence authorization rejection, as required by the
dispatch ENTRY rule. `authorization-rejection.json` preserves the exact
`report_signal` request and returned error. ROOT was notified through the
accepted `report_progress` call. No alternate issue was attempted, and no
completion, approval, close, merge, or push is claimed.

Entry succeeded initially: the canonical issue was in progress with assignee
`spur-worker-4c39671c389c48ed`, matching plan/task/delegation labels, and D6
`bd-21oj` was closed. INTENT UUID
`701bd4b3-6ad5-47b4-8731-605c2a6c2999` persisted as comment `4118` and replayed
idempotently; complete requests/responses are in `D7-entry-audit.json`.

The ROOT PRE verifier passed before test changes:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7/root-pre/verify.py
```

Observed fingerprint:
`0e1e0171c6ece0c400a7ee97fc10774d8cb4fa734a34cca02856bbfdeda807ce`;
1,925 bindings, 20 prospective queries, 11 predicates. All three complete
live receipts were reloaded using `get_solve_result` and compared by JSON
value equality to each archived `reloaded` envelope in `root-pre/proof.json.gz`:
`sol_5aee4f82ef334308` SAT, `sol_68958d620d3947c6` SAT,
`sol_e23bc8fbc3b64cb2` UNSAT. The exact live envelopes are preserved in
`pre-live-receipts.json.gz`. These are prospective PRE results; no new query
implementation or runtime outcome is represented by them.

Test-first commits:

- `0331ce365e10d0692e4d9ef7fee5b30817b85dd8`: seed and adversarial tests.
- `1ec1a28d5`: correct the existing `TemporalIndex` import, mutate the actual
  owner kind invariant, and format the tests.

`tests-only.diff` contains the unified test diff against the dispatch base.
No production file has changed. The new tests remain uncompiled and unrun.

Both attempted compile-RED commands were:

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0 \
SPUR_CLOUD_BUILD_SH=/Users/kevintruong/project/SpurCode/scripts/cloud-build/build.sh \
scripts/spur-cargo test -p spur-graph --test rust_evidence_queries \
  zero_source_and_stale_oid_have_memory_disk_parity -- --exact --nocapture
```

The first attempt, inside the sandbox, exited **200** because neither AWS
endpoint was reachable. The authorized retry reached builder
`i-01d560dbad5bbb79c`, but source rsync timed out after 810 seconds and its
retry timed out after 60 seconds; the wrapper propagated **30**. Neither
attempt reached the compiler or test binary. These are infrastructure
failures, **not compile RED or assertion RED**. Raw logs and exits are
preserved under their original attempted-stage filenames. Original command
source hashes are in `D7-compile-red-sources.sha256`; current corrected test
hashes and unchanged production hashes are in `current-sources.sha256`.
The checkout remained immutable during each source upload. No local build
or fallback ran.

Read-only remote diagnosis found source-reclaim lock holder PID `837273`
(`flock`, age 29:05), with rsync child `837277` and rsync descendant `838578`;
PID `869983` was waiting on the same lock. The new D7 remote target was only
4.0K. No remote process was killed and no shared lock, source, or target
state was changed by the worker diagnosis.

The first risk-signal attempt using the tool's advertised `{kind}` schema
returned `missing field signal_id`. Retrying with the skill-required UUID,
severity, reason, and estimated_subtasks returned the authorization rejection
saved here. This is the stopping condition, not a rejected test result.

Remaining work requires ROOT dispatch repair first. Then obtain real compile
RED, add only signatures/default behavior, obtain assertion RED from the same
seed, implement the bounded production seam, finish acceptance cases and IO
instrumentation, run all required remote suites/checks, and export actual
native query responses and source-bound replay evidence. No runtime test
counts, test executable/toolchain hashes, GREEN, implementation completion
audit, or fresh POST exist yet. ROOT retains PRE/POST and approval ownership.
