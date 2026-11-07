# M01 ROOT actual conservation review and POST

Canonical `bd-3txu`, source `bd-3402`, ROOT review `bd-335d`, plan
`46b921e6-f999-4014-bd35-931b84eac903`. Recovered ROOT scope/PRE checkpoint:
`1ef009a46a3e5c3a0c1fb7be9bd9eb3ff74dc84d`. Reviewed normalized worker tree:
`05465d5063c7f8279b8eb1dacd4a5874d83267d0`.

ROOT verified the four-file migration: five struct-update draft additions replace
ten explicitly absent evidence/count fields, preserving all other bytes and
assertions. The three-line test-only harness imports the retained core source;
it executes that file's three original tests without production module/Cargo
wiring. Original harness c336c448a, migration cc76f9f6d and corrected evidence
62f20d360 are protected by review refs. ROOT-owned scope recovery is separate
from the worker implementation and original incomplete attempt evidence.

## Independent native and source replay

ROOT independently parsed every raw before/after native record and reproduced
the comparison:23 before records,24 after records,19 distinct input cases across
five site kinds. All actual values, serde wire bytes, Debug including skipped
node IDs and native canonical/capability hashes match. The nested count case is
projected from actual emitted row-count wire; no native value or BLAKE3 is
computed in Python. Different duplicate observation counts are retained; the
original malformed/interleaved lines remain excluded rather than reconstructed.

For all six observation epochs ROOT applied the archived patch to exact initial
Git bytes in a temporary directory, then checked every recorded source SHA.
The other source files and final command snapshots are authenticated against
the frozen reviewed Git epoch using batched Git blobs, so replay does not depend
on a retired worker checkout or later caller edits. Executable SHA, compiler,
configuration and exact raw log hashes are verified. This is an independent
replay of the worker's actual native executions, not an additional ROOT Rust test
run. Existing completed tests were not repeated without a changed concern.

ROOT rejected six mutations of real native values: overwrite the explicit CLI
header, upgrade absent evidence, lose a skipped node ID, alter the manifest's
explicit completeness, replace nullable counts with zero, and lose the core
helper's hash argument. Authentic comparisons are retained with their inputs
and exact value SHA-256 identities in actual-review.json.

## Actual tests and inherited risk

Final native executions:56 passed,1 failed,0 ignored,126 filtered. The core target
has3 passes before and after; CLI14 passes; graph providers16 passes. Analyst
library18 passes, overlay integration5 passes/1 failure; embedding filter has
zero executed and29 filtered. All-target check of four packages, local formatting,
source diff and complete archived diff checks pass. Final source equals tested
source snapshots. Existing negative/presence/source assertions remain unchanged.

The sole failure is the existing analyst
`worktree_overlay_keeps_base_edges_to_reemitted_delta_targets`, left0/right1,
tracked by open bd-16eq. Before and after names, statuses and exact failure detail
match. Relevant test/production bytes are identical to the initial dispatch.
ROOT accepts this inherited risk for the bounded behavior-preserving migration;
the bug stays open and no whole-suite GREEN or fix is claimed. No local fallback,
failure filtering, expectation change or production cache behavior change occurred.

ROOT authenticated genuine INTENT4675 by comparing its saved idempotent replay
and request to the live Beads audit (snapshotted here); an initial response is not
separately archived. Corrected completion4684 and its identical replay supersede
the earlier premature archive-whitespace claim. Lossless compressed logs/patches
preserve the original bytes; software source and native results were unchanged.

## Source-bound actual POST

Fingerprint `56d49b6557683c7ff9f59913cba2faf7bf08740b5f4b9407d985354a175cefd3`:
119 source bindings,33 archive bindings,15 actual groups and6 lanes (63 compiler
variables). The measured analyst failure and all_suites_green=false remain in
the facts. Jev selected verify/mutually_consistent on contract, gate open at
weakest0.66. ROOT inspected exact candidate facts and added catalog case/lane
uniqueness and exact group coverage; no confidence threshold was changed.

Pinned complete results: `sol_f516e7ea19f7477c` SAT/pass and
`sol_7df8fcfaca294dad` UNSAT/fail. Both full reloads match, independent finite
evaluation checks8 predicates, and all16 single false controls reject. These
results verify the finite conservation policy, including explicit inherited
failure; native/source review supplies the implementation evidence.

The consumed supplemental PRE pair was fully reloaded, compared to archives,
independently checked and then unpinned for quota space. Full reloads remain here;
accepted G03 POST pins stayed protected. No quota or software acceptance limit
was increased.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M01/ROOT-review/verify-post.py
```

Before successor source changes, add --current-source. Frozen Git/log/archive
replay remains valid after later migration steps. Official plan approval, scoped
risk acceptance and local-main integration are recorded separately in Beads.
