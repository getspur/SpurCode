# D8 native worker handoff

Implemented whole-file Rust evidence overlays in `spur-graph`. Canonical issue
`bd-syir`, delegation `e56f2571619948d7`, plan
`22d86ee0-2994-42b7-9463-305cb353c367`, task `D8-rust-evidence-overlay`.
The issue remains `in_progress` for ROOT's actual-output Jev POST and independent
review. This packet is worker evidence, not approval.

The production change is 85 added lines and one removed line in
`src/query_client.rs` and `src/rust_evidence/query.rs`. Original filters are
validated before routing. Explicit file requests select the active backend,
so stale OIDs cannot resurrect replaced or deleted sources. Aggregate queries
replace complete file and source-domain groups, preserve available Unknown
lanes, and distinguish unavailable coverage from deletion. Query projections
reuse `retained_origins` to preserve whole original inventories, retain only
referenced origins, sort deterministically and reject conflicting inventories.
No projected `RustEvidenceSet` or replacement checksum is manufactured.
The existing selected builder already handles physical deletions; extraction
and D3 canonical deletion behavior were left intact.

The original seed is byte-for-byte unchanged as the prefix of the test file.
ROOT's genuine assertion RED was verified against its immutable source epoch:
exit 101, 0 passed, 1 failed, 0 ignored/filtered, `Unavailable` versus `Complete`.
The expanded regression run in this attempt failed before Cargo with rsync
exit 30; it is transport evidence, not another assertion RED. No local build,
lock removal, job termination, target copying or infrastructure edit occurred.

| Commit | Purpose |
| --- | --- |
| `0d3072d8ecb83316f7e0b41dce6350d1562f0f2e` | Preserved original seed |
| `aa091403fa05ee2c54b561f15e174f8705bfbeb5` | Additional regressions and replay, committed before the fix |
| `010614145e9a41233a85c6c81db5b5df9394a444` | Whole-file overlay implementation |
| `8a6a2bbdc2e8f1a7bc1337ec046d56f19e6d43c5` | Native handoff instrumentation; production behavior unchanged |

`unified.diff.gz` contains the complete source/test diff from recovery
`936e13f000ba7c59aedbfe76308aa78c6f51af98` to the final native source epoch.
All remote runs used the prescribed wrapper/helper, full sync, rsync timeout
180 and no local fallback. Source and Git revision/status stayed fixed during
each upload/build. Exact commands, epochs, counts and hashes are in `checks.json`;
complete raw wrapper logs are compressed under `logs/`.

| Final required target | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| `rust_evidence_overlays` | 12 | 0 | 0 | 0 |
| `overlay_client` | 12 | 0 | 0 | 0 |
| `mcp_overlay_notfound_retry` | 4 | 0 | 0 | 0 |
| `rust_evidence_queries` | 8 | 0 | 0 | 0 |
| `rust_evidence_d7b` | 6 | 0 | 0 | 0 |
| `rust_evidence_pruning_d7b` | 3 | 0 | 0 | 0 |
| `rust_evidence_pruning_root_review` | 1 | 0 | 0 | 0 |
| `macro_invocation_queries` | 13 | 1 | 0 | 0 |
| `query_client::tests` | 26 | 0 | 0 | 566 |

The macro suite's sole failure is the previously demonstrated
`parquet_macro_queries_reject_missing_source_manifest_rows` (`bd-xb5p`), which
fails on stale graph content metadata before the macro query. The suite is not
wholly GREEN. `known-macro-baseline.json` identifies and hashes the preserved,
source-bound predecessor records. No other test failed. Graph `--all-targets`
check, scoped local formatting and complete diff checks passed. Existing
warnings remain recorded in the raw check log.

Across all native invocations in this attempt, including initial/final export
runs and 12 independent single-test replays: 24 invocations, 131 passed,
1 known baseline failure, 0 ignored, 698 filtered. The reused historical ROOT
seed RED is separate. Cargo check executes no tests.

`native-packet.tar.gz` holds 17 cases with original base/delta JSON artifacts,
raw source bytes, both Parquet directories, shadow paths, complete ordered
memory/disk/D5-reloaded outputs and executable/compiler/config identities.
`post-candidates.json.gz` adds active/deleted sets, original whole-file and
whole-origin payload digests, raw-byte OIDs and three independent invalid-output
controls. There are 423 ordered native observations. `initial-native-packet.tar.gz`
preserves the earlier 12-case capture at source `010614145`.

`independent-replays.tar.gz` preserves 12 actual arbitrary-input hook invocations:
normal zero/cfg/tuple/custom/legacy/mutation cases, origin conflict, old-site
injection into shadowed zero/cfg base groups, an untouched-origin rebinding and
invalid stored source binding. Injection results retain exact zero/Unknown
replacement groups. The rebound custom projection is deliberately tainted; the
independent oracle rejects its selection/provenance claim. Invalid stored source
metadata returns a native error. JSON mutations and disk directory mutations
are independent inputs; an unchanged supplied disk is expected to retain its
original value when only the JSON artifact is mutated.

Run the finite packet verifier from this checkout:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/worker-evidence/verify.py \
  --write-candidates /private/tmp/d8-post-candidates.json
```

It checks all 2,541 source/config/fixture bindings against immutable Git epoch
`8a6a2bbdc`, original seed preservation, all seven complete PRE/parent POST
envelopes, archive inventories, exact original group/provenance selection,
all 423 observations and the independent native replays. It does not rerun
historical source-bound PRE checks against modified code or perform ROOT's POST.

To replay independent inputs, extract the native packet to a private directory
and create a local JSON request with `base`/`delta` artifact paths, `shadowed`
paths and original `filters`. Optional `base_disk`, `delta_disk`, `base_source`
and `delta_source` are local directory paths. `base_mutations` and
`delta_mutations` are arrays of `{pointer, value}` JSON replacements.

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/worker-evidence/replay.py \
  --request /private/tmp/independent-d8-request.json \
  --capture /private/tmp/independent-d8-observation
```

The script uploads only the independent inputs into a new builder `/tmp`
directory and invokes the recorded native hook after checking source, config,
compiler and executable hashes. The instance is an availability hint and can be
supplied with `--instance`; an expired builder requires an independently recorded
rebuild through the authorized wrapper, not bypassed hash checks.

`custom_base`, `base_query_result`, `base_query_error` and
`base_result_mutations` additionally inject a backend query projection separately
from the stored artifacts. For example, use the identity case's first
`memory.ok` projection and replace `/files/0/calls/complete/evidence/sites` with
independently chosen old sites, or rebind `/files/1/origin_graph_content_hash`
and supply a corresponding `/origins` array. This surface represents an explicit
custom backend; its projection is not falsely validated as a stored artifact.

Full source authentication remains separate. Real captures authenticate each
original base/delta through Rust. Legacy absent carriers have only known native
physical byte/OID bindings and return evidence-authentication errors. The
additional declaration-only Unknown case is an explicit stored metadata mutation
of a separately authenticated selected capture: its available call lane remains
queryable, while full syntax authentication correctly rejects the mutation.
Both original input and exact authentication error are preserved. No cfg
activation, macro expansion, complete panic detection or call-target correctness
is claimed. `entry-and-progress.json.gz` records durable INTENT/progress replays;
`receipts-live.json.gz` preserves all seven exactly compared complete envelopes.
