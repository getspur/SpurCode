# G03 ROOT independent review and POST

Canonical worker issue `bd-3t8y`, source task `bd-36nv`, ROOT review `bd-1cz3`,
plan `7c25f4ce-dee1-45fa-91fb-6e8b16db82bb`. Dispatch base:
`bb1f602b3b2f35943a3c4546adad8d9883528709`. Reviewed normalized worker tree:
`f42a03ec511b67b31c7824dff7dad7930941e679`.

The worker changed exactly the approved three Rust source/test paths: 58 lines
in schema.rs, 82 in store/parquet.rs, and 238 in the existing provider tests.
All changes are additions. Artifact/manifest typed fields remain the sole
mutable truth, getters clone, setters replace each Option, empty constructors
produce unchecked drafts, and both old writers invoke the temporary no-op guard
before creating files. Existing validators and all old tests remain intact.
No generic carrier, disk/wire schema, extractor/resolver or dependency change.

## Evidence and independent execution

ROOT authenticated the complete worker archive, original UUID INTENT 4629,
completion 4633 and correction 4635 with identical idempotent replay. Original
test-first commits precede implementation 9022ea0be; the API-only RED records
39 E0599 errors. Its exact test-first commit has unchanged predecessor production
source. Original TDD/implementation/evidence refs are protected separately from
the orchestrator's normalization.

Worker verification: 61 passed, zero failed/ignored, 587 filtered (the explicit
canonical-hash unit filter). This includes providers16, canonical5, macro
Parquet9, roundtrip9, schema invariants4 and Rust Parquet18. All-target check,
formatting and full-base diff check passed. Failed network/upload/partial-sync
attempts remain archived and classified separately; full-sync remote retries
include the unchanged frozen embedded fixtures. No local fallback was used.

ROOT independently executed the unchanged final provider target remotely:
16 passed, zero failed/ignored/filtered, exit0, 135.3 seconds. The recorded
command used two exact package-local request/config copies; their originals,
SHA-256 identities and invocation are archived here. All 27 compiled source
identities in both native exports equal the reviewed Git epoch. The raw native
log and separately extracted exact JSON are authenticated, rather than generated
from expectations.

Four supplied authentic cases cover empty and nonempty Rust/macro providers.
Two changed-source controls preserve stored validation while failing exact-byte
source validation. Missing captured length fails stored and source validation.
A duplicated scalar mismatch also fails after native checksum rehashing. The
independent request contains eight cases; these are cases inside the test run,
not eight additional test functions. Existing G03 native execution additionally
checks four persisted Rust/macro checksum forgeries across two fixture cohorts.

For each cohort the actual getter/setter artifact and manifest roundtrip is
exact in value, serialized bytes, canonical hash and capability. None remains
distinct from captured-empty, and a cloned value changes live state only when
replaced. Drafts use native empty source identity/current version helpers and
existing writer defaults, with completeness false and absent typed lanes.
Full/delta reads preserve typed evidence. Existing Parquet output sorts symbol
and edge rows; the full graph comparison accounts for that order, and does not
claim byte identity between in-memory and readback row ordering.

## Fresh mathematical POST

POST fingerprint:
`c77181dff69198cc462ce880946cb2220333a1d9aaf67bfe8202b5681f5b051f`.
The projection binds 87 source files and 14 native/log/archive inputs, deriving
15 actual groups plus four ownership lanes (57 compiler variables). Jev selected
verify / mutually_consistent on contract, gate open at weakest0.85. ROOT inspected
the exact candidate, then added catalog case/lane uniqueness and exact coverage.

Pinned, fully reloaded results: `sol_ece7d695b2394700` SAT/pass and
`sol_a52c9a65c5f94664` UNSAT/fail. An independent finite oracle validates all
eight compiler predicates and rejects each of 16 false controls in isolation.
The solver verifies the exact projected finite facts; native execution and source
review supply the software evidence.

ROOT reloaded the consumed G03 PRE pair, compared complete envelopes and checked
all eight predicates independently before releasing those two pins for quota
space. Their full reloads are archived here. Accepted G02 POST pins remain
protected. No solver quota or confidence threshold was changed.

Replay the native/source/archive projection and independent predicates:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G03/ROOT-review/verify-post.py
```

Before successor edits, add `--current-source`. Future replay authenticates the
frozen reviewed Git epoch, complete native log and archives. ROOT approval and
local-main integration are recorded separately on the canonical plan/issues;
this packet is their evidence, not a worker self-approval.
