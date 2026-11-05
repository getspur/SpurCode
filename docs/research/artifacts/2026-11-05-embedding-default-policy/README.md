# Shared default policy: work in progress

Base: `091bb89af74ff76902f8b836bc7f5f76cf91196f`. The committed recovery
verifier exited 0 before edits: 40 source files, fingerprint
`9bed966557a71fd0ab2a181d4ad9bad096e73d52ff64f7df95789dc410f62ca2`,
10 stage-A receipts (1 SAT / 9 UNSAT), four historical budget controls and
34 independently evaluated catalog predicates. All 10 stage-A receipts were
live reloaded with the expected status. Historical control IDs remain pinned:
`sol_f2502a750d4640c3`, `sol_10287aebb84d4b56`,
`sol_816b8915a0804dba`, `sol_8af864ebabdc4768`.

Issue `bd-tr9g`, delegation `801a997d9b9d4e64`: INTENT audit
`f9b12ea6-17eb-4eb1-a2fa-35735c18685f` persisted and replayed idempotently.
Test-only commit `fc33b1e808b46b700b0cd6b05d0aa201ba1a9df8` restores the
regression from `49e855e8e80757aa361b5c9323fffc593f53db01`.

## Assertion RED

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo test -p spur-analyst --lib default_local_policy_does_not_require_json_configuration -- --nocapture
```

Exit 101: one test failed, 111 filtered out; the child assertion reports
`default local helper policy must be available: environment variable not found`.
Full output is `red.log`. The initial sandbox git commit failed with index.lock
EPERM; the same commit succeeded with authorized escalation.

## Measurement (exploratory, not default selection)

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo build -p spur-embedding-worker
python3 -B docs/research/artifacts/2026-11-05-embedding-default-policy/measure.py
```

Both exited 0. The source-built debug helper hash, model identities, timings,
experimental bounds, sizes and process status are in `measurement.log`.
The measurement used a private copy of the Nomic cache and a private runtime
cache. The 64 short texts yielded 64 768-dimensional rows, and the helper
closed with exit 0. The materialized runtime SHA-256 was
`2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`;
the archive SHA-256 was
`cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`.
This single fixture cannot determine universal capacities or timeout defaults.

No production implementation, GREEN, POST or successor PRE has been completed.

## Attempt 3 preflight and fresh RED

The current dispatch is `bd-tr9g` / `8ae38a29d099477d`, attempt 3 of plan
`6144d93c-6c55-47c2-932c-97990c518458`. INTENT audit
`32a8f7aa-3f12-4d1e-8a87-106ef97022f4` was persisted as comment 3362 and
replayed idempotently. The verifier passed at base `091bb89af`; all ten
stage-A solver receipts reloaded with the expected status. Checkpoint test and
evidence paths were recovered as commit `5b1b62462`.

The analyst command above was rerun on this attempt: exit 101, one assertion
failure, 111 filtered out, because the JSON environment variable was absent.
See `attempt3-red.log`. The worker compatibility command above also exited 101,
one assertion failure, because `--compatibility` was treated as missing limits;
see `attempt3-compatibility-red.log`. These are assertion failures, not build
errors. Both runs emitted existing compiler warnings.

`off_mode_ignores_invalid_helper_configuration` passed on attempt 3 (exit 0,
1 passed, 112 filtered), using the same local environment and
`scripts/spur-cargo test -p spur-analyst --lib
off_mode_ignores_invalid_helper_configuration -- --nocapture`. Its log is
`attempt3-off.log`; it is a preservation test, not implementation GREEN.

The graph's default batch is 64 and its section-body eligibility threshold is
4096 bytes. The batch override has no corresponding maximum, and symbol text,
section titles and analyst queries do not share that body threshold. The wire
format bounds the serialized JSON frame separately from raw text bytes. A
default policy must address splitting and oversized individual inputs.

Private measurement cache artifact sizes (five files read by the worker) were
548026095 bytes for Nomic revision `e9b6763023c676ca8431644204f50c2b100d9aab`,
548431020 bytes for CodeRank revision `151669b173750250e611e1be00c812f112ea6020`,
and 644080771 bytes for Jina revision `516f4baf13dec4ddddda8631e019b5737c8bc250`.
These are observed snapshot sizes, not a bound for future revisions.

The inspected `resource.request_within_limit` catalog rule was run without
persistence for the three measured fixtures: rows 64, dimensions 768 and
response frames 608390/609126/609992 bytes against experimental limits 64,
768 and 16777216; SAT. Changing the Jina frame to 16777217 returned UNSAT.
The same calls with `persist: true` both failed with `solver artifact quota
exceeded (ArtifactCount): limit 512, attempted 513`. They are not durable
receipts or a product policy gate. The exact inputs and failure were sent to
root with `report_progress`; root mediation is pending.

The independent worker metadata regression in `2b89aac40` ran with the same
environment and `scripts/spur-cargo test -p spur-embedding-worker --test
compatibility -- --nocapture`. It exited 101 at the assertion: the helper
reported `expected seven protocol limits and artifact budget`. A preceding
compile failure from a missing dev dependency is recorded separately and is
not an assertion RED.

`coderank-normal-measurement.log` and `jina-measurement.log` contain additional
64-short-text measurements using the normal helper hash above. The first
CodeRank measurement (`coderank-measurement.log`) used a different helper
hash after the test build and is retained only as exploratory evidence. The
normal helper was rebuilt before repeating it (`worker-rebuild.log`). Jina's
observed descriptor, Hello and cold-request times were approximately 73, 73
and 71 seconds, respectively. These remain observations of one fixture, not
upper bounds or chosen defaults.
