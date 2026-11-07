# G02 independent ROOT review: changes required

Reviewed normalized worker source f7fbd8b2b1fb86564162c04fadf0e509fdd20020.
Canonical issue bd-ode3, plan 73be9add-0d91-4416-a3bf-a3dbdb5e4810.
ROOT issue bd-2gjo. G02 remains unapproved; local main remains accepted G01.
No production code was changed during ROOT review.

Worker packet authentication passed: 3 providers, 11 mutation cases and
24 compiled-source bindings. Full source identities also match Git blobs
at the reviewed worker epoch after ROOT added tests. Original test, RED,
implementation, native-test and evidence commits remain protected by
spur/review-evidence/generic-G02-original-* refs in their actual TDD order.
The worker's in-progress status, INTENT 4581, checkpoint 4586, completion
4589/4592 and both risk/scope signals were independently read.

## Two genuine regressions

Independent test commits: 295d6de4c, then corrected parity control 256c6c6ea.
The corrected remote run exited 101: 1 passed, 2 failed, 0 ignored, 10 filtered.

1. RustEvidenceProvider.source_domain excludes the real authoritative file
   physical#cell:real.rs although the existing typed query reports its source
   as Unavailable. Query source ownership must survive; this does not authorize
   broadening the existing collector's capture policy.
2. query_rust_evidence normalizes a validated query projection as a stored
   captured extension. A legitimate Unavailable source with None byte length
   and origin fails source_length. Preserve that coverage and typed result
   without inventing provenance or relaxing full stored-source validation.

The captured-source parity control passes. The initial broader parity test
also compared a missing .rs file against the old macro API's vacuous Complete.
The explicit generic contract requires Unavailable for an absent source.
ROOT corrected that oracle, retained its raw failing run, and added an explicit
Unavailable guard. This is not a third production defect or an instruction
to upgrade missing coverage to Complete.

The initial remote run encountered spot preemption before rustc. The wrapper's
built-in recovery launched i-0d81ed4c6240d4317 and completed the native assertions.
No local fallback, token reset, quota change, cache copying or unrelated job
termination was used.

## Historical required-suite failures

The approved predecessor 0c6318bc619dad602ae11032cf45fd6239b46957 independently
reproduces both failures in rust_evidence_contract. Its original 18-test file
and all production bytes are unchanged. An observation-only harness includes
that file, runs the three d2 tests plus one native export, and reports
2 passed, 2 failed, 0 ignored, 15 filtered.

Complete native graph/set comparison with the frozen D1 packets found exactly:
- empty graph: $.manifest_version only;
- empty set: no differences;
- nonempty graph: $.manifest_version only;
- nonempty set: $.origins[0].manifest_version only.

Current native version:
37eee2e2e0d9213d4d9ff13ccbacae861859bd592305fb88dbe54e4e09024626.
Historical version:
bf1ad64f405c4cd5222120dad7b0d5461d0b7d2f52f9aeb0bbe79c07ae61d524.

ROOT authorizes narrow version-cohort expectation updates in
tests/rust_evidence_contract.rs. Preserve frozen D1 archives and original
wire omission checks, assert the measured version fields, compare every other
field exactly, and execute all existing forgery/mutation assertions. Do not
rewrite snapshots, remove comparisons, ignore/filter tests or weaken validators.

## Repair handoff

Fresh prospective repair PRE: rework-pre/README.md. It binds the failing
source epoch and actual native records; it is not a successful implementation
POST. Only the four listed source/test paths and new G02 rework evidence may
change. ROOT owns actual-output POST and review after the repair.

Replay:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/ROOT-review/verify-findings.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/ROOT-review/rework-pre/verify.py --current-source
```

Omit --current-source after implementation to authenticate the historical
review epoch, then independently bind the new compiled source. Raw logs are
unchanged gzip archives authenticated by command metadata; baseline replay
uses recorded source identities and Git blobs, not a surviving temporary VM
or working tree.
