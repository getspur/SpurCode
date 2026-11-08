# M06 prospective PRE and implementation handoff

ROOT approved the grounding at `9799dc20f974d54cb33e02bafff4cc3b71d9ed12`
after checking 46 files, 138 combined source ranges, exact three-document scope,
and canonical RECORD 4931/completion 4934. Grounding is merged at
`98cd1c097787f83b46e75039afb1eb3e7b05e95a`. No M06 product code has changed.

PRE fingerprint:
`a99d2c132816f945b50cee8c45476c3cc743a2669f69253db8931d9e187e0b4d`.
It binds 153 source/reference inputs, three grounding documents, twelve complete
policy rows and four distinct acquisition lanes, using 48 compiler variables.
`sol_e556e670e1904bad` is SAT/pass;
`sol_aa9e9b76e042467c` is UNSAT/fail. Both complete compiled envelopes are
reloaded and pinned in `proof.json.gz`. The independent oracle checks eight
compiled predicates and fourteen individual false-claim/lane/cardinality
controls. Native observations remain null: this is prospective policy, not
native acceptance or M06 completion.

Jev remains blocked at completeness confidence 0.48, below its unchanged 0.6
threshold. ROOT independently checked that all policy rows/cells and enum values
are present and declared, with no unknowns. The absent native measurements are
outside these policy facts. The direct catalog route verifies this explicit
finite model; no rephrasing, threshold change or fabricated open gate occurred.

The earlier quota failure, automatic unpin rejection and nonpersistent checks
remain archived, together with the original blocked checkpoint documents.
The user then explicitly approved continuing M6 in response to the request to
release only the two archived M05 POST pins. ROOT again compared both complete
live envelopes to their lossless archives, then retired only
`sol_d2d83b1036a64726` and `sol_a2fe41435dd24f8a`.
`retirement-approval.json` records that authorization. M04 POST pins
`sol_20d1c51504e24bf7` and `sol_3a83a9eb74604722` remain protected.
No cache file, quota or compiler limit was changed.

## Authorized mechanical change

Only four operations in three product files:

- S1: `schema.rs::validate_rust_evidence_sources` obtains its owned Option via
  `rust_evidence_set()?`, preserving the exact absence error and validation order.
- S2: `schema.rs::validate_macro_invocations` obtains its owned Option via
  `macro_evidence_set()?`, preserving None success and every subsequent check.
- S3/S4: the existing private artifact helpers in `search.rs` and `selector.rs`
  remove only the two explicit typed None members and add the final
  `..GraphIndexArtifact::empty()` tail. All other expressions and vector order
  remain exact.

These are six field occurrence migrations. Eight compatibility declarations,
defaults and getter/setter self-accesses remain; replacing their internals with
recursive calls is forbidden. Preserve old raw wire controls, all explicit
synthetic test header/version/hash values, GraphFacts, canonical/omission rules
and None versus Some(empty). G03 getters clone and setters assign; no semantic
bugfix, new storage truth, cache change or generic carrier cutover is authorized.

## Reversible native acquisition

The exact policy and case inventory are in `inputs.json#acquisition_policy`.
Temporary capture at the two existing cfg(test) helper returns must emit the
original native artifact, serialized bytes, Debug, canonical/capability outcome,
actual fixture input and current test identity, returning the identical object.
Temporary capture-only tests may be appended to
`tests/rust_evidence_contract.rs` and `tests/macro_invocation_artifacts.rs`,
using their existing physical fixtures and directly invoking the public schema
wrappers. Original test bodies/assertions and production validator bodies are
not probe-edit targets.

Capture real None, empty, nonempty and source/manifest/owner/caller negatives
with complete errors and actual inputs. An upstream rejection or a case name
does not prove a later branch ran. Keep any unreachable coverage explicit.
The unchanged remainder of each validator is also source-checked.

Acquire actual before/after native bytes/ordered values with matching input,
site, case and multiplicity; retain source, command, compiler and executable
identities from the actual compiled variant. Never inject expected source hashes
as native observations or relabel G03 historical metadata as M06 provenance.
Prefer framed stdout with exact raw diagnostic/removal spans. Optional export
environment/path transport must be demonstrated before relying on it.

Remove all temporary hooks and restore both support test files byte-for-byte.
The final product diff contains only the four operations above. Preserve
temporary patches, compiled variants and raw captures as bounded compressed
evidence. M05's transient owner/error ordering exceptions do not apply to M06.
Any unexplained comparison gap returns to ROOT with the strict failure intact.

All builds/tests/checks use `scripts/spur-cargo`, remote with no local fallback;
formatting is local. The mechanical refactor has no fabricated semantic RED.
Use genuine existing baseline results and meaningful copied-native mutation
controls, then exact before/after conservation and required GREEN/check/fmt.
A true bugfix is a separate decision requiring a failing regression.

## Review and retention

Workers reload only the two current M06 PRE IDs and compare complete envelopes.
Do not reload retired M05/M03/original PRE IDs. Workers make no retention changes.
ROOT retains these pins through implementation and, after complete live reload
and lossless archival, may consume this PRE for the required actual POST.
M04 pins remain protected. ROOT owns actual POST, independent adversarial review,
approval, local-main merge and the serial M07 successor.

Entry verification, before any source edits:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/root-pre/verify.py
```

After source migration, `--historical` explicitly checks baseline source bytes
from the bound Git epoch; it does not certify changed working-tree/native inputs.
Those require the separate before/after acquisition and actual POST. No runtime
success, performance gain or completed architecture is claimed by this PRE.
