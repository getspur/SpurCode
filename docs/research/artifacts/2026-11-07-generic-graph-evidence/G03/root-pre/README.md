# G03 ROOT prospective PRE

Accepted predecessor: `8fb0d5059f8c387d1e4bfe833ac3a9d39b0c7c9c` on local main,
after independent G02 POST and official plan approval/integration. ROOT task:
`bd-363m`; generic architecture parent: `bd-2rg4`.

This PRE validates a prospective finite preparation policy. It does not prove
that the new APIs compile or implement the policy; worker TDD and native output,
then a fresh ROOT POST and independent review, are still required.

## Authorized boundary

Only `crates/spur-graph/src/schema.rs`, `src/store/parquet.rs`, and
`tests/generic_evidence_providers.rs` are production/test write paths. Evidence
belongs under the G03 directory. Current typed fields remain the only mutable
truth. No generic carrier, disk schema, wire tag, extractor/resolver, dependency,
cache, or collector cutover is included.

Introduce the exact artifact and manifest getter/setter/empty signatures in the
approved spec and `GraphArtifactRowCounts::empty()`. Getters return owned clones;
setters replace the corresponding existing Option value exactly. In this stage
they do not introduce new validation or a second decoded/mutable carrier. Tests
must distinguish absent and captured-empty evidence and metadata, and show that
mutating a getter result does not change the live artifact before replacement.
Existing validators, serialized omission/null behavior, canonical/capability
bytes, and all unrelated fields must remain unchanged.

`empty()` constructs an unchecked draft: current version helpers and native
empty-inventory source hash, empty collections and absent typed evidence. A
manifest draft has `complete=false`, `sidecar_complete=false`, zero counts with
optional counts absent, no shards, and existing writer defaults (`zstd-3` and
`PARQUET_ROW_GROUP_SIZE`, currently 16,384). These are existing defaults, not new
resource bounds. Empty drafts must not claim collection completeness.

The temporary `validate_legacy_evidence_export` is a genuine no-op on the current
typed-only artifact. Both old full/delta writers call it before disk writes;
existing writer validation stays intact. Future cutover supplies a real legacy
adapter guard. Do not add a permanent provider allowlist to the generic core.

## TDD and verification

A missing newly introduced signature is honest compile RED. The plan's
Some(empty) scenario must be tested, but do not manufacture a deliberately
broken setter stub to force an additional runtime RED. Any discovered existing
behavior failure still requires its genuine assertion RED before a repair.
Commit tests first, retain exact RED type, then implement the small API surface.

Required existing coverage: `generic_evidence_providers`, canonical-hash unit
tests, `parquet_roundtrip`, `parquet_schema_invariants`,
`macro_invocation_parquet`, and `rust_evidence_parquet`; graph all-target check,
local formatting and complete diff check. Run through `scripts/spur-cargo`,
remote by default with no local fallback after remote failure. Preserve named
pre-existing ignores and failures explicitly; never silently skip assertions.

Export actual native before/after artifact and metadata values/serialized bytes,
canonical hash/capability bytes, None/Some(empty), owned-clone replacement, draft
defaults, full/delta writer outputs, and retained source/metadata-negative
controls. Bind executable/compiler/config/fixture/source identities. Do not
implement hashes or expected native output in Python. Existing writer suites
and code review establish the guard call sites without adding an implementation
mirror test.

## Solver scope and retention

`prepare.py` rebinds 86 exact source/archive files to the accepted predecessor.
Eight policy rows and four distinct ownership lanes use 36 compiler variables.
Jev selected verify / `data_integrity.mutually_consistent` on `contract`, gate
open with weakest confidence 0.94. ROOT inspected the unchanged facts/binding,
then added catalog uniqueness for cases/lanes and exact case cardinality.

Persisted and pinned results: `sol_9e98a45d2f5e4cec` SAT/pass and
`sol_e947a41ec9294a43` UNSAT/fail. Complete reloads and an independent finite
oracle validate all eight compiler predicates; each of nine false controls is
also independently rejected in isolation. The UNSAT control is a false policy
snapshot, not a universal implementation proof.

To free cache space without increasing quota, ROOT reloaded both consumed G02
repair PRE receipts, compared their complete envelopes to archived proof, and
independently checked all eight predicates before releasing only those two
pins. Exact reloads are retained here. Accepted G02 POST pins remain protected.

Replay from any checkout containing the frozen epoch:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G03/root-pre/verify.py
```

Before worker source changes, add `--current-source` to check current bytes as
well. After changes, the normal replay still authenticates the frozen Git epoch;
the successor's source/native identity must be freshly bound for POST.
