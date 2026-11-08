# M05 ROOT acceptance

The fifteen accessor/constructor sites preserve the observed behavior under the
explicit comparison supplement. ROOT independently reviewed the three-file
mechanical migration, authenticated the original and normalized worker trees,
and replayed all 51 worker controls and seven frozen comparison scopes. Eleven
additional ROOT probes passed. The production bytes remain exactly migration
`7c8eeb1433bc613e4bff01f3469133e58719b81f`; 142 other bound inputs are unchanged.

All six integration pairs conserve 1,036 records and 35 supplemental packets;
their 487 measured canonical, capability and public query outputs remain strict.
The original full acquisition conserves 1,043 records and 36 supplemental packets,
with 492 strict protected outputs including seven historical library records.
Every raw strict comparison remains **false**. Only the nine named transient
`overlay-empty` owner projections and the three precisely scoped metadata map
diagnostics receive the ordering exceptions declared in `ordering-pre`.
Persisted graph artifacts, public output ordering, full values, native row slices,
representation positions, multiplicities, and all other bytes stay strict.

ROOT found a reporting defect in the worker's full-scope reference index:
`captures.load(full=True)` interleaves references by acquisition folder, then
indexes them as records followed by supplements. This mislabels 42 reference
positions per full phase, including the seven library record references. It
does not affect the native inputs, normalization, comparator, controls or
comparison results. `replay.py` independently reconstructs the correct references
from the original immutable acquisition files. The two
`corrected-full-*-provenance.json.gz` files are authoritative for this reference
index only. Original worker files and all failures remain untouched. This is
an evidence correction, not a production or comparator repair.

ROOT actual POST fingerprint:
`d4bba2d2a7145c08b4fa7183b1218dd05397fb1f349f9cd38bd65cd9c2180618`.
It binds 293 source/evidence files and twelve actual case rows, using 42 compiler
variables. Jev gate is open, weakest 0.67. ROOT inspected the exact facts, verify
mode and selected consistency rule, and retained the three additional catalog
bindings for case uniqueness, distinct comparison lanes and exact cardinality.
`sol_d2d83b1036a64726` is SAT/pass;
`sol_a2fe41435dd24f8a` is UNSAT/fail. Both full results are reloaded and pinned.
The independent finite oracle verifies eight compiled predicates and rejects
thirteen individual false-claim/lane-alias controls. These are finite snapshot
checks; the solver does not inspect code or establish every possible execution.

The supplemental PRE pair was fully reloaded and compared to its complete
archives, then consumed/unpinned to respect the existing quota. The previous M04
accepted POST pair remains protected. No quota, confidence threshold or compiler
limit changed. The family solver persists a compiled generic request; an initial
ROOT verifier incorrectly compared that request directly to the family input.
That verifier assertion was corrected to use the existing independent compiled
predicate oracle. Solver inputs, receipts and verdicts did not change.

Canonical completion 4908 and worker RECORD 4906 are authenticated. The latter
records original checkpoint `76771c30e5855ce5aa717a7677b2eae465a58d63`; its normalized
tree is `eb57ebd43958be9c6f17da90cc6f1f57efcd1de8`. Both histories are retained.
The original ordering risks are addressed by measured unchanged-version repeats,
the explicit supplement, adversarial controls and this ROOT POST. The prior
acquisition lease risks ended with the completed and restored acquisitions.

There were no new native executions during comparison verification or ROOT
review. Both historical repeat exits remain 101. The existing
`parquet_macro_queries_reject_missing_source_manifest_rows` failure (`bd-yptp`)
still fails at stale graph hash before its intended missing-manifest assertion.
Earlier library 589 passes/three existing model-download ignores and all-target
checks retain their historical status. No workspace GREEN, byte equality or
unmeasured temporary-projection canonical hash is claimed.

ROOT approves this bounded M05 migration with the reference-index correction
above. The separate baseline bug stays open. This acceptance does not complete
the generic evidence architecture; M06 remains the next serial migration slice.

Replay from the repository root at the accepted M05 source:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M05/ROOT-final/verify-post.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M05/ROOT-final/replay.py
```

The first command verifies bound bytes and proof/control envelopes without new
native runs. The second replays the actual comparator/control code, checks the
nine archived generated artifacts without rewriting them, reconstructs the
corrected references, and reruns ROOT probes. It writes only the deterministic
ROOT replay outputs. The old worker `finalize.py --verify` must be run at its
own original checkpoint: its historical scope/main assertions intentionally do
not account for later ROOT documents or integration into main.
