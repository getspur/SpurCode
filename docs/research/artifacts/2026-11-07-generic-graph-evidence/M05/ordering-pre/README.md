# M05 comparison-only supplemental PRE

ROOT independently verified diagnosis checkpoint
`279427cb13ad359120f3372797f871f05f13054a`: 81 original and 32 diagnostic manifest
files; frozen source excerpts; exact restoration to production `7c8eeb143`; both
new raw logs and all recorded Cargo diagnostic removal spans; 1,036 real records
and 17 binding packets of 145 compiled inputs per repeat. The same-version
old/old and new/new pairs each reproduce 18 owner-vector ordering differences
and three metadata-string ordering differences. All six strict comparisons stay
false; all 487 measured canonical/capability/public-query outputs agree.
The source trace ties these to HashSet caller iteration in a transient hydrated
owner projection and HashMap Debug iteration in three metadata diagnostics.
The transient projection's own canonical hash was not emitted: no claim about
that unmeasured hash is made.

Decision: permit a narrowly qualified **evidence comparison** relation, keeping
all raw/strict results and product code unchanged. This supersedes the original
PRE's strict observation relation only at the two boundaries below. It does not
relax the graph persistence contract, public output ordering, validation rules,
actual failures or the required ROOT actual POST/review. M05 remains unapproved;
main remains `0965b6073`.

1. In the nine named cases listed in `inputs.json`, only
   `macro-load/hydrated.output` and its matching `macro-context/return.input`
   may compare complete symbol rows under a permutation. The artifact must be
   the synthetic **`overlay-empty`** projection with nonempty unique owner IDs,
   not an arbitrary artifact. Require an exact same-run hydrated/context value
   pairing and multiplicities. Full row fields, owner file/IDs, typed evidence,
   skipped IDs and every other record/wire value remain exact.
2. Only `D7_NATIVE`, `filtered_corruption`, in
   `full_validation_rejects_filtered_out_corruption_and_actual_owner_changes`,
   at `value.value.outcome.error`, for the three listed Rust evidence tables,
   may compare the iteration order of complete expected/actual metadata map
   entries. Every key/value, expected/actual role, native entry bytes/formatting,
   table/path, error prefix/suffix/chain and every other packet field stays exact.
   Duplicate, malformed or nonstring maps reject. No generic error-string or
   whitespace normalization is authorized.

All three owner representations must agree **in position within each record**:
wire rows, actual serialized row byte slices and actual Rust Debug row slices.
Anchor their correspondence to authenticated native row representations; do not
implement a Python Rust Debug renderer or native canonical algorithm. Permuting
only one representation must reject. Preserve every byte outside the qualified
row slices, including JSON/Debug prefix, suffix and separators; a full JSON decode
alone must not hide whitespace, key-order or other byte changes. Likewise map
entry permutation must not hide other string encoding/formatting changes.

All canonical/capability identities and public query rows/scopes/availability,
including their ordering, remain strict. No missing/extra native record, changed
source/site/input, None/captured-empty substitution, owner mismatch, error or
multiplicity change may pass. Old strict comparisons remain available and false;
qualified logical conservation must be reported separately from byte equality.

The new code is limited to Python verification/control artifacts under
`M05/worker/attempt3-comparison/`. No Rust, tests, Cargo, old scripts/evidence,
approved plan or previous PRE edits. No additional native execution is needed or
authorized. Use the four frozen integration runs and the historical seven-record
library acquisitions at their real counts; do not relabel them as new runs.
Build meaningful controls first from copied real records. `inputs.json` enumerates
36 required negative categories and three positive categories; all must actually
execute against the qualified comparator, including the previously unmatched gaps,
not only the equal intersection. No fabricated product RED is claimed for the old
strict relation's expected rejection of newly permitted equivalences.

The unchanged macro fixture failure remains genuine exit 101 and is tracked as
`bd-yptp`. Earlier graph library 589 passes/3 model-download ignores and remaining
integration outcomes remain their actual historical results. No workspace GREEN,
production fix, main merge, approval or M06 is authorized here.

Supplement fingerprint:
`aee59c0f84c1eff55d1c0a7df1e39d2d62243168697b7f5fdd5ce84f517b4c8d`.
It binds 145 source/reference inputs, 119 archives/review artifacts, ten complete
policy cases and two distinct comparison lanes (36 compiler variables). Jev gate
is **open**, weakest 0.85. Catalog SAT/pass `sol_5b63c979810d426a` and UNSAT/fail
`sol_6372e8dc00004e79` are fully reloaded and pinned. The independent finite oracle
checks eight policy predicates and rejects eleven individual false policies.
This proves the declared policy model, not its yet-unimplemented comparator.

Original M05 PRE current receipts were fully reloaded/compared, the complete
archived six-request model independently checked (24 predicates), then only its
last active pair was consumed/unpinned to respect the existing cache quota.
All previous envelopes survive losslessly. Accepted M04 POST pins stay protected;
no quota/threshold/compiler limit changed. Workers reload only this new current
pair and compare full envelopes, not retired original/M03 IDs.

Replay after recovery to the supplied supplemental commit:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M05/ordering-pre/verify.py --current-source
```

The original PRE remains a historical source/implementation policy. ROOT owns
fresh actual POST and independent adversarial review of the qualified comparator
and all evidence before any M05 acceptance or local-main integration.
