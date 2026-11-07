# M01 attempt 2 — ready for ROOT POST/review

Canonical `bd-3txu`, delegation `993d3ca5b91c43b7`, assignee
`spur-worker-993d3ca5b91c43b7`. Keep the canonical issue `in_progress`.

Use G03 draft defaults to remove caller coupling to typed absence while
preserving every explicit fixture value and existing assertion.

## Source and ownership

- ROOT-owned recovery: safe fast-forward from dispatch `7ff31e3fa` to exact
  `1ef009a46a3e5c3a0c1fb7be9bd9eb3ff74dc84d`; 73 documentation/evidence paths,
  no Rust changes. See `root-recovery.json`. These are not worker implementation.
- Harness commit `c336c448a`: three-line documented path module importing the
  actual retained source and its three existing tests; no production wiring.
- Migration commit `cc76f9f6d1fbbce6c802eb45161754e1e90fd165`: five constructor
  sites in the three original files, five additions and ten removals. All
  non-evidence explicit values, IDs, wire controls and assertions are retained.
- Worker source delta is exactly four paths. [Unified review diff](source.diff), [full patch archive](source.diff.gz) and
  [tested source identities](final-source.json). No Cargo/dependency/feature,
  coordinator/cache, extractor, schema/carrier, G04 or M02 changes.
- All temporary native observers were removed before final checks. Original
  attempt1, original PRE, supplemental PRE and G03 archives remain unchanged.

## Native conservation

[Actual comparison and hashes](native-comparison.json): all five site kinds
and 19 distinct input cases are equal. This compares actual values, serde wire
bytes, Debug including skipped node IDs, and existing Rust canonical/capability
hashes. Python authenticates archive bytes with SHA-256; it computes no BLAKE3
and invents no native/expected passing values.

Before: 20 authenticated archived analyst/CLI records plus three fresh core
records. After: 24 intact records. The one-record difference is an additional
analyst duplicate; the old malformed/interleaved record stays excluded. The
nested row-count kind is derived only from the actual emitted row-count wire.
See `native-before.json`, `native-after.json`, `baseline-reuse.json` and the
per-epoch `.json`, `.log`, `.source.diff.gz` files. Each observation binds full
source, executable SHA-256, compiler, configuration and helper inputs.

The genuine core baseline executed before any migration via
`baseline-native-core`: three passes and three intact records. The migrated
core observation also has three passes. Its actual hash argument is `first`,
header/manifest are `test`, canonical hash is
`4b0322d189155953a389479c869eae996e9e9e77ad0eb14248df1c26cc65afaa`, capability
hash is `14000680dd80471e4b443c06d0d658e8f0d47bce56e63b98ed6179b41ba85267`.
The fixture still takes and preserves its argument; it was not hardcoded.

## Actual final checks

All builds/tests used `scripts/spur-cargo`, remote execution, no local fallback,
and `SPUR_SYNC_FULL=1`. Formatting ran locally. No failing remote run was hidden
by a local rerun. Counts and all 57 executed final test names are in
[command-summary.json](command-summary.json); raw logs remain authoritative.

| Target | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| Analyst library (`overlay`) | 18 | 0 | 0 | 97 |
| Analyst embedding filter | 0 | 0 | 0 | 29 |
| Analyst overlay integration | 5 | 1 | 0 | 0 |
| CLI `analyst_temporal_views` | 14 | 0 | 0 | 0 |
| Core `legacy_rebuild_singleflight` | 3 | 0 | 0 | 0 |
| Graph `generic_evidence_providers` | 16 | 0 | 0 | 0 |

Four-package `check --all-targets`: exit 0. Local `fmt -- --check`: exit 0.
Final full diff check against initial dispatch and source-only check against
recovered checkpoint: exit 0 after lossless evidence packaging. Warnings remain in the remote logs; the checks did not use
`-D warnings`. Final source hashes equal every final command's source snapshot.
The provider suite retains original None/Some(empty), malformed/scalar/metadata,
source-domain and native export controls.

**Inherited risk:** `worktree_overlay_keeps_base_edges_to_reemitted_delta_targets`
continues failing at `crates/spur-analyst/tests/overlay.rs:158`, left 0 / right 1
(`bd-16eq`). Before, observed-after and final outcomes/counts are unchanged.
Relevant original test/production identities are in
`inherited-failure-identities.json`. No assertion was skipped, ignored, repaired
or weakened. The analyst command exits 101; this is not a whole-suite GREEN.

## Operational observations, separate from semantics

- `baseline-core`: sandbox AWS connection failed, exit 200, zero tests, no local
  fallback. Network-authorized retry was used.
- `before-core`: full source upload completed, then wrapper refused changed
  revision/state, exit 1, zero tests. Worker checkpoint commits changed HEAD
  during upload. Subsequent runs held the checkout stable.
- `baseline-stable-core`: all three tests passed, but stderr/stdout transport
  interleaved all three observer records. Raw bytes remain intact; those records
  are excluded in `excluded-observations.json`, never reconstructed. Fresh
  `baseline-native-core` pre-serialized stdout observations solved transport.
- A preliminary formatting check identified formatting only in temporary
  observer code. The final observer-free formatting check passes.

## PRE, replay and handoff

Supplemental PRE passed with `--current-source` immediately after recovery and
before source/harness writes. Fingerprint
`e94c3ce57fd51766f36cda5f4e3ee6d6dc763a6ab307599c29519f9ad2b40126`:
116 bindings, one then-absent harness, 11 prospective cases, 12 independent false
controls, 51 variables; Jev open/0.75. Both complete live solver envelopes equal
archived proof (`solver-comparison.json`, `live-sat.json`, `live-unsat.json`).
Frozen PRE replay after edits omits `--current-source`; it remains prospective.

INTENT 4675, recovery checkpoint 4678 and native conservation checkpoint 4681
were persisted and identically replayed. The initial completion audit 4682 was superseded by a corrected final audit
after the post-commit evidence whitespace check described below. Audit receipts are saved
alongside this report. Source/evidence are committed; the final handoff verifies
a clean tree and leaves the canonical issue `in_progress`.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M01/worker/attempt2/compare.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M01/worker/attempt2/verify.py
```

[Actual ROOT POST candidates](root-post-candidates.json) bind final source,
native values/hashes, suite outcomes and inherited risk. No actual POST solver
receipt, self-approval, closure, merge, push or PR is claimed. Fresh actual ROOT
POST, independent review, inherited-risk decision and local integration remain
required.

## Lossless evidence packaging correction

The post-commit full diff check found trailing whitespace in raw observer
stdout and blank context lines of the stored patch. The earlier pre-commit
check did not cover those then-untracked files. `postcommit-whitespace-check.json`
retains the exact failure. No Rust source or test/native result changed.
`observed-analyst.log.gz` and `source.diff.gz` preserve the original exact decoded
bytes and hashes. `source.diff` is a valid zero-context unified review diff; the
full patch archive retains normal context. Replay resolves the archived logical
log transparently. No whitespace rule or assertion was disabled.
`evidence-packaging.json` maps old identities to their lossless archives, and
`pre-packaging-SHA256SUMS.json` preserves the earlier manifest.
