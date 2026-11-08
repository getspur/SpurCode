# M06 supplemental comparison PRE

This is a new, explicit comparison policy for the two frozen G03 metadata
diagnostics. It does not change product behavior, generalize error comparison,
or inherit M05 exceptions. M06 remains unapproved.

Fingerprint: `a276e43d891b0d1c1211c7f3155d27625e78b02bd5fc46dcf9fae69de86cce31`.
It binds all 153 final source inputs at migration `2ee50a5fef63ff2b98cf6e965072ce228197fecc`,
both complete native packet manifests/indices, ROOT's lossless diagnosis, and
the unchanged formatter/producer/comparator. Twelve prospective policy rows and
two case lanes use 42 compiler variables. Native results are outside this finite
policy proof; runtime conservation still requires the implementation and controls.

Jev is open at confidence 0.86 with unchanged configuration. ROOT checked its
family, verify mode, full facts, single consistency binding and no unknowns, then
added the catalog's two uniqueness bindings and exact policy cardinality.
`sol_bdbcea8d37514a18` is SAT/pass and `sol_c2c294b2800b48de` is UNSAT/fail.
Both complete executed responses and compiled reloads are in `proof.json.gz`.
The independent oracle checks eight compiled predicates and fourteen individual
false-policy, case-alias and policy-cardinality controls. These are finite policy
checks, not proof that an arbitrary parser is correct or that native tests passed.

The source/acquisition PRE pins were retained through implementation and then
consumed by ROOT into the required actual POST review pipeline, as authorized in
`../root-pre/README.md#review-and-retention`. Before release, both complete live
envelopes matched their lossless archive by canonical SHA-256. The exact quota
failure and readbacks are in `pre-consumption.json`. No solver quota, cache file,
threshold or compiler limit changed. M04 POST pins remain protected. Do not reload
the consumed original M06 or older M05 IDs. Workers may reload only these two
current supplemental PRE IDs and must never change retention. ROOT retains the
new pins through comparison implementation, then may consume them after complete
reload and lossless archival for the required actual POST.

## Exact authorized relation

First authenticate the entire immutable A/B packets and actual compiled source
and executable correspondence. Compare the complete dataset. Only
`supplemental[0].cases[0].rust_negative_error` and
`supplemental[0].cases[1].rust_negative_error` may use the new view. Each field
contains a separately anchored expected map and actual map, each of 13 entries.
The existing actual per-case temporary-root span mapping remains unchanged.

The new view may permute identical whole raw entry slices. It must preserve
duplicate-free string keys/values, complete pairs, cardinality, exact entry
encodings, map delimiters and positional separator bytes, map roles, all outer
error bytes, table names and context. Bind each map to its own immutable baseline
field/fixture so changing both phases cannot manufacture identity. Reject malformed,
duplicate, missing, extra or nonstring entries, equivalent escaping, altered
within-entry whitespace, role/case swaps and any change outside these four maps.
Full native serde/Debug/canonical/capability/Result/errors, inputs, wire ordering,
presence, manifests, multiplicity and run outcomes remain strict everywhere else.

ROOT's `../ROOT-B-review/diagnose.py` authenticates all four maps and proves the
only two remaining differences are entry permutations, with all 42 actual site
records strict-equal. It is a diagnostic, not an active qualified comparator.
The unchanged Rust source formats HashMaps with `{:?}`; it promises no stable
entry order. No new native execution is needed to resolve this saved-data gap.

## Evidence-only follow-up

Implement only under `M06/worker/comparison/`. No product source, existing test,
capture, archive, A/B comparator, policy or ROOT file may change. No builds or
native reruns. Preserve both original false comparisons, errors and raw streams.
Commit a test first that genuinely fails its whole-dataset qualification assertion
using the original comparator; then implement the narrow relation. Keep strict
false and qualified status separate. Use complete before/after datasets, not an
equal subset. Include both real case permutations, meaningful copied rejection
controls for all policy categories, and unchanged 46 native/eight source controls.
Capture actual per-control outcomes and leave any unexecuted category explicit.

Deliver a deterministic read-only replay, hashes and an audit on canonical
`bd-3qkw`. ROOT owns actual POST, independent review, signal resolution, approval
and local-main merge. No M07 or whole-refactor completion is authorized here.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/ROOT-B-review/diagnose.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/comparison-pre/verify.py
```
