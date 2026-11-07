# G02 repair prospective PRE

Source epoch: 256c6c6ea5bb8456e1f40d7e4e8928472e0c2211.
Fingerprint:
e3eb81c395a6fc90ed0c9d584cd0e05482f8f20d1c3c10a6e459fe04783c0694.

The current implementation fails two independently observed runtime assertions.
This prospective target model binds 84 source/archive files, 8 repair policy
cases and 9 individually rejected false controls. It does not assert repair
success or replace the eventual actual-output POST.

Jev jev-1.14.0 returned open, weakest 0.96, selecting mutually_consistent
with subject repair_contract and exact facts. The executed catalog request
adds declared case/provider uniqueness and exact coverage bindings. No
unknowns, empty subjects, thresholds or solver limits changed; 33 variables.

| Proposed model | Receipt | Result |
| --- | --- | --- |
| Intended repair | sol_803d23cd91fb4603 | SAT/pass |
| False repair claims / provider alias | sol_b175348537b343ed | UNSAT/fail |

Both complete envelopes are pinned/reloaded. The independent oracle evaluates
8 compiled predicates and all 9 single false controls. Initial attempt1 PRE
receipts sol_1edb0703733b4de2 and sol_137ccfd26e414299 were completely reloaded,
compared with their archives and independently checked (8 predicates), then
unpinned because this repair PRE supersedes them. Their original requests and
full envelopes remain in the archive; do not require those historical IDs
to remain live. Accepted G01 POST receipts remain retained.

## Explicit rework scope

Relative to crates/spur-graph:
- src/evidence/providers/rust.rs
- src/evidence/providers/mod.rs if a shared query helper is necessary
- tests/generic_evidence_providers.rs
- tests/rust_evidence_contract.rs, solely the measured golden-version cohort

New worker evidence belongs under G02/rework/attempt2. Existing worker/ and
ROOT-review/ archives are immutable. Other provider modules and the generic
core remain unchanged unless ROOT reviews a concrete additional failure.

Fix authoritative physical query-domain coverage without changing old capture
eligibility. Separate query projections with legitimate unavailable provenance
from complete stored extension validation. Keep Root's final three regression
functions unchanged. Correct the older contradictory worker assertion that
expects exclusion of the actual physical source; retain positive cell/virtual
source controls. No fabricated source origin/length, unknown completeness
upgrade, new parser, schema/extractor/dependency change or backend/storage
wiring is allowed.

The predecessor's entire native graph/set values differ from historical
packets only at the recorded manifest-version paths. Update expected values
for the current cohort only; preserve old archive bytes and omission checks,
and keep every other exact comparison and all original mutation checks active.

Required unfiltered suites include generic_evidence_providers,
rust_evidence_contract, macro_invocation_artifacts and generic_evidence_contract.
Run the graph all-target check, local formatting and full diff check. Fresh
native export/replay should use only the provider test target to avoid the
prior multi-target stderr interleaving. Reuse the original inputs but write new
outputs/config/command identities under attempt2; bind current source bytes.

Worker entry runs verify.py --current-source before changes and reloads only
the two current repair receipt IDs with pin omitted, comparing full envelopes.
ROOT actual-output POST and independent review remain required before G03.
