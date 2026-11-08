# M06B frozen comparison qualification — pending ROOT review

The complete frozen A/B dataset qualifies under ROOT's supplemental PRE.
`result.json` keeps **strict_equal=false**, **observed_root_equal=false**, and
**qualified_equal=true** separately. No M06 approval or actual POST is claimed.
Executed qualification controls: **66 positive permutations and 480 rejections**
(462 primary negatives plus eighteen identical-input anchor negatives). The
unchanged 46 native and eight source controls also pass.

Only this new `M06/worker/comparison/` directory changes relative to ROOT checkpoint
`81c5dde0ed92b6e3774ad878fd2608fa24cf1bca`. The worker fast-forwarded approved A
`2407d39cab150b418a41a7db3d0c1c92a4d81797` to that committed checkpoint after
canonical INTENT receipt 5012. This recovered B implementation
`2ee50a5fef63ff2b98cf6e965072ce228197fecc` without recreating or changing it.
Canonical task `bd-3qkw`, delegation `d8be6abacde441d8`, remains in progress.
Source tracker: `bd-hx77`; plan `b9e69ee5-ca31-4374-8f8d-404c58599647`.

## Relation and anchoring

`qualified.py` authenticates ROOT's committed PRE, all 153 current source inputs,
13 archive bindings, both entire packet manifests, and every PRE manifest member.
It then invokes the unchanged original bundle loader. Both packets authenticate
46 records (42 observations and four actual compiled records), one complete G03
supplement, all raw streams/frames/references/diagnostics, 153 compiler inputs per
compiled record, and native command outcomes. The independently checked saved
lineage binds compiler output bytes and each actual executable to its reference.
No native values are generated in Python.

Both arguments, their source rows and supplied reference indices are checked
against private copies of the frozen identities. The entire qualified comparison
view is anchored to approved A. Changing both arguments, shrinking both to an
equal subset, changing the reference phase, or substituting source rows cannot
manufacture acceptance. Synthetic controls operate on complete copied datasets;
their semantic mutations are checked without using raw payload hash mismatch as
a substitute for the semantic comparison.

The implementation uses the complete original `comparison_view`, including its
declared compiled-process diagnostics. It first verifies the original actual root
coordinates and root bytes against each phase's frozen roots, then invokes the
existing root view. It introduces no general root matcher or new root exception.
All other outer bytes remain part of the comparison.

Exactly `supplemental[0].cases[0|1].rust_negative_error` receive the new map view.
Each field contains its own expected and actual map: four distinct slots, each
with thirteen entries. The read-only M05 `native_slices.py` utility supplies only
lossless lexical parsing; no M05 acceptance policy is imported. ROOT's diagnostic
script is never used as an active comparator.

For every slot the comparison representation retains the complete outer text,
table path, role, map boundary bytes, each positional separator, all whole raw
entry slices, complete decoded pairs, cardinality and unique keys. Only the entry
sequence loses order. Equivalent escaping and changed within-entry whitespace
remain different raw entries. Malformed, duplicate, missing, extra, nonstring,
trailing or wrongly scoped content rejects. No native error, Debug, serde,
canonical or capability value is re-rendered. Every other dataset value remains
strict, including ordered lists, serialized bytes, presence, manifests, Results,
errors, input identity, owner/span data, run outcomes and ignore names.

## Test-first history and executed evidence

Test commit `bd09bf59f677f482d39f93dedfda661bf1f6b49d` contains the small scaffold
delegating qualification to the unchanged strict/root comparator.
`red.json` preserves the exact command, failing output and exit 1: a real
`AssertionError` naming both G03 fields. `identity.json` preserves passing A/A
and B/B identity controls with exit 0. The test body has remained byte-identical.
ACTION receipt 5014 binds that committed RED and the full PRE readbacks.
Fix commit `d40c0cbe3e36f930c7b82f6c189b936f9de78a26` contains the narrow comparator
and complete control reports. ACTION receipt 5016 binds that checkpoint and
report hashes to canonical `bd-3qkw`.
`replay-command.json` and `replay.log` retain the complete ordered replay with
exit 0, including byte-for-byte reproduction of all 546 qualification controls.

`replay.py` loads the committed scaffold in memory and executes that unchanged
test body again: the full-packet assertion fails and the identity test passes.
It then runs the landed implementation against the same test expectations.
The remaining commands reproduce the exact qualified report and full synthetic
control report, and authenticate all frozen bytes again.

`controls.json` records actual per-control outcomes and exact native references.
Positive controls use intact native entry slices: reverse/rotate in all four
slots on both phases, every adjacent transposition in each B slot, and both real
case permutations isolated separately. Negative controls exercise all four slots
on both phases for changed/missing/extra/duplicate/nonstring/malformed entries,
equivalent escapes, whitespace, boundaries, separators and delimiters. Additional
controls cover prefix/suffix/context/table and role changes, case/field/scope
swaps, extra cases, both-phase mutations, all 42 observation identities on each
phase, wire/row order, owner/manifest/canonical/capability/Result/error drift,
compiler/source/reference/provenance drift, and every run-result member.

`anchor-controls.json` additionally compares two identical mutated copies of A,
then two identical mutated copies of B. All eighteen controls preserve complete
reference anchoring and **strict=true/root-only=true**, yet reject qualification.
They cover a fixed map key's value, outer context, record identity/order, wire
bytes, manifest, run outcome, a missing supplement, and forged source rows with
matching forged compiled provenance. They demonstrate that equality of two
changed inputs cannot replace the frozen baseline identity.

`attempts/control-noop.json` records a stopped control-harness attempt: its
literal substring mutation was a no-op and never reached comparison. The fixture
was corrected without relaxing its rejection expectation. That attempt is not
counted as a completed control run.
`attempts/anchor-provenance.json` records a second fixture-development stop: a
copied G03 binding initially remained unmodified while the other copied source
bindings changed. The corrected fixture mutates all corresponding bindings;
strict/root identity expectations remain true and qualification remains false.

`original-replays.json` preserves the unchanged 46 native controls, eight source
controls and original B verifier, all exit 0. Both original comparison commands
still exit 1. Original scripts that deterministically rewrite reports are checked
after each execution against all **211 pre-existing M06 paths**; every byte stays
exact. Original strict reports, historical transport failure, command exits and
native captures remain untouched.

## PRE and frozen hashes

Fingerprint: `a276e43d891b0d1c1211c7f3155d27625e78b02bd5fc46dcf9fae69de86cce31`.
The unchanged verifier confirms 12 policies, two exact case lanes, 42 compiler
variables, eight compiled predicates and fourteen independent policy controls.
All twelve policy rows remain binding; none is weakened by this implementation.

Only current supplemental PRE receipts were reloaded, without retention changes:

| Solve ID | Status | Complete persisted envelope canonical SHA-256 |
| --- | --- | --- |
| `sol_bdbcea8d37514a18` | SAT/pass | `26ca3502446f3082eccffd213ed85a0c001c33c6b58ae0d7ce2d0e1eb857920e` |
| `sol_c2c294b2800b48de` | UNSAT/fail | `a53e1f3dd9dfa69281a415e2cb8fcf2bf3dc3ffbea246526d59c776a90d12d65` |

Their complete live envelopes are compressed losslessly under `entry/` and match
`comparison-pre/proof.json.gz` field for field. Jev stays OPEN at 0.86. These
receipts prove finite prospective policy, not parser correctness or native
acceptance. Original consumed source PRE receipts are read only from historical
archives by unchanged verification; no old solve ID is reloaded. M04 pins remain
untouched. No new solver, pin/unpin, threshold, quota or limit operation occurred.

`frozen-checks.json` binds all source/archive rows, packet/PRE hashes, complete live
readbacks and unchanged RED test bytes. Source-row digest:
`5a039b35ccc84c5af61841e83729be5fb4f21ce532a2cb54bcfc62916364d0bc`.
Archive-row digest:
`9b800bc59a1b01ae5ad9c3f84f8f2ac79d7024a1ca1afff8b00142b1542652ca`.
`frozen-files.json` inventories every old M06 path with SHA-256 and its original
Git blob. `manifest.json` hashes this directory except itself.

## Ordered replay

From repository root, one command performs the ordered verification below without
builds, fresh acquisition, solver calls or retention changes:

```sh
python3 -B docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison/replay.py
```

Order: frozen source/archive/PRE checks; unchanged ROOT diagnosis; unchanged PRE
verification; committed scaffold RED replay; unchanged tests with GREEN;
unchanged native/source controls and B verifier; strict and root-only failures;
complete qualified report; complete synthetic controls; repeated frozen checks;
`git diff --check`; new evidence manifest verification.

Individual deterministic reports can also be checked directly:

```sh
python3 -B -m unittest discover -s docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison -p test_qualified.py -v
python3 -B docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison/result.py
python3 -B docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison/controls.py
python3 -B docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison/frozen.py
git diff --check
```

The original comparison commands intentionally return 1; `replay.py` asserts
those failures rather than classifying them as a new test failure. Commands and
actual logs for new checks are retained alongside their deterministic reports.

## Limits and handoff

Finite controls do not exhaust thirteen-factorial permutations or arbitrary byte
mutations and do not prove general parser correctness. Native wrapper Err chains
each have one element, so no actual multi-element chain permutation is claimed.
The inherited selector bypass, optional G02 early return and combined byte/line
range probe remain explicit. The earlier 44 integration passes, 589 library
passes and three existing download ignores, plus final library/check/fmt success,
are inherited authenticated native evidence; this attempt did not rerun them.

No product/support/config/cache or native capture file changed. No build, graph
sync, environment workaround, install, push, PR, main merge, M07 or self-approval
occurred. Main and the user's existing paths remain untouched. ROOT reviewer
`bd-2u5v` owns actual POST, independent review, resolution of blocked signal
`45cc6321-a6e7-4350-93b1-53d731647fdd`, and the final decision. That signal remains
a pending acceptance condition. Canonical `bd-3qkw` stays in progress.

The exact authoritative evidence-only unified diff is:

```sh
git diff --binary --full-index --no-ext-diff --no-textconv 81c5dde0ed92b6e3774ad878fd2608fa24cf1bca HEAD -- docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/comparison
```

`review.diff.gz` is the equivalent staged Git patch with binary payloads, excluding
itself and `manifest.json` to avoid self-reference. The committed Git diff above
includes those packaging files as well and is authoritative.
