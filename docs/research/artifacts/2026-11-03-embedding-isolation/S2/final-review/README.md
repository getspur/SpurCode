# S2 final root verification

Candidate `251a1fb0627ed10849077a7424648cee0c8ae32c` fixes the two
shutdown ownership regressions. The expired idle close stays retryable; a
busy close deadline wakes active work and retains a cleanup owner. Queued
callers and a duplicate close return typed failures. Root changed no source.
The authored history is protected at
`spur/review-evidence/embedding-s2-attempt3-authored`.

Root verified the worker's raw/source evidence and the historical rejection,
then independently rebuilt and ran the full neutral-crate suite: **32 passed,
zero failed**. The exact output is `root-test.log.gz`; decoded observations
are in `root-test.json.gz`. There are 12 original case rows, four lifecycle
traces, two close rows, and the separately printed close/concurrency outcomes.
Both formerly stranded children were reaped and absent in `ps` before test
safety cleanup. Busy close returned `Timeout` in about 103 ms for its 100 ms
fixture deadline; its active request returned `Closed`. This is an observed
timing, not a hard real-time scheduling guarantee.

Root also ran normal-feature check, formatting and the normal/build dependency
tree. All passed; the neutral crate has no native inference/database package
in that dependency closure. Seven source hashes and three newly built
executable hashes are recorded in `identity.json`. The worker changed the
metadata key for executable hashes: root did not compare inherited executable
bytes before rebuilding, and makes no inherited-binary-match claim. The exact
source and fresh root runs are bound instead.

Five fresh Jev interpretations opened and were inspected with unchanged facts
and no unknowns. Root explicitly added discovered uniqueness/cardinality rules
where required; Jev itself compiles only one binding per call. All five actual
POST requests passed, with 16 independent controls rejected. Every result was
persisted and reloaded. `post.full.json.gz` retains complete original family
requests, Jev provenance, outputs and lowered reloads; `manifest.json` lists
all solve IDs and checksums.

| Actual-output binding | Passed receipt |
|---|---|
| Three close dispositions | `sol_de8454c9afa74cca` |
| Twelve request outcome cases | `sol_d4808993a1884a0e` |
| Reap/pipe-close before owned I/O join | `sol_868f4adfdaa24d7b` |
| Observed queue/restart counts | `sol_ad653b71d6a34246` |
| Source/executable identities | `sol_d335eb137eb6481c` |

The controls cover stranded closes, remote error/abort falsely publishing
vectors, erasure of an already committed result, missing/duplicate cases,
joining before reap/pipe closure, missing reap evidence, excess capacity or
generations, and stale/missing artifact identities. Unique and cardinality
controls fail their corresponding catalog bindings rather than merely an
unrelated tuple check.

`project_post.py` reproducibly derives finite facts from the raw observations
and an independently declared policy. Cleanup ordering uses observed event
indices and does not impose an order between reap and pipe closure. Trace
identity checks concern sent envelopes; incoming mismatch rejection is covered
by the passing S1/supervisor tests and reviewed Session validation. Run
`python3 verify.py` from this folder to check local evidence and immutable
historical source bindings. Later implementation changes must not rewrite
this evidence.

Rust verification used `SPUR_REMOTE=0 TMPDIR=/private/tmp
CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo`
with `test -p spur-embedding --features test-support -- --nocapture`,
`check -p spur-embedding`, `tree -p spur-embedding -e normal,build`, and
`fmt -p spur-embedding -- --check`. The runtime suite used the authorized
process-inspection permission. The worker's scoped Clippy and additional
all-target checks are preserved in the verified `../attempt3/` logs.

This acceptance covers the native-free supervisor and the declared finite
observations. It does not establish unbounded liveness, exhaustive scheduling,
native ORT teardown, real product embeddings, or final host dependency
isolation. Native execution/cleanup is S3; product models are S4; host routing
and packaged artifacts remain later gates. Product policy defaults remain
explicit rather than inferred from these test values.
