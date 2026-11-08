# M03 worker evidence — bd-3dil

This is worker evidence for ROOT review. It is not ROOT POST, approval, issue
closure, a main merge, or authorization for M04. Canonical issue `bd-3dil`, plan
`0856024d-c567-442b-8865-ba5e7851b775`, task `M03-caller-preparation`, delegation
`f69c5315b70c4340`. The runtime issue was `in_progress`, assigned to
`spur-worker-f69c5315b70c4340`, with the matching delegation label and no
supersession. Runtime assignment/plan snapshots are in `runtime-pre.json.gz`.

The source change is exactly six `..GraphIndexArtifact::empty()` tails replacing
twelve named typed `None` fields in the three approved files. Every other source
byte, expression, assertion, workload, threshold, clone, and explicit header/hash
is preserved. `migrate.py` checks this against the archived original source;
`source-only.diff` is the source-only unified diff. Production
`empty_graph_artifact` keeps its full explicit literal. There are no schema,
GraphFacts, carrier, dependency, lockfile, collector, reducer, cache, default,
performance, other-batch, or product-test changes.

| Epoch | Commit |
|---|---|
| Accepted M02/local main | `698bb2a12d457866d7facc9a5bbc19ce07714095` |
| Explicit ROOT M03 PRE/original base | `3b3def64ab3fb81458aafe3145a90ce93a8069f5` |
| Initial intent evidence; original full-library compiled HEAD | `31d445dd8f38f11f305b4166695901d736005fd4` |
| Native BEFORE compiled HEAD | `433cf167b1c51db3e841d8ea81dd00720f5fdc1d` |
| Complete native baseline checkpoint | `9075d035d5416d53964a306d208c8a1698cb7ee5` |
| Minimal source migration; native AFTER compiled HEAD | `4179448927de9d8f03c312cafa5819f2acf0abb5` |
| Native equality checkpoint; final checks compiled HEAD | `9d1772515bdb869aeb3a4db1fe219f528ecf9017` |

Final validation evidence is committed at
`09546e21945102a13e88d769a4b7dc4e076558db`. The final receipt commit ID is
reported in the handoff. These commits
retain the original history; uniquely named `refs/evidence/M03-f69c5315b70c4340/*`
refs preserve the important epochs too. All changes after the source migration
commit are under this worker evidence directory.

PRE fingerprint
`5fad21dc4009ee6976cfa5f93a3bc877b0b21df62c5e90180a64a1d61b882a82`
authenticated 131 bindings, six sites, nine prospective cases, two lanes, 33
variables, eight independent predicates and ten false controls. The complete live
envelopes of `sol_9da019eed1cd4dfc` (SAT/pass) and `sol_064efa3db9a646bc`
(UNSAT/fail) exactly equal the frozen `proof.json.gz` reloaded values. See
`runtime-pre.json.gz` and `pre-verification.json`. ROOT PRE was reverified with
`--current-source` immediately before migration, after removing BEFORE observers.
PRE is prospective. No solver pins, quota, or retention settings were changed;
accepted M02 POST protection and intentionally retired M01 POST history remain
untouched.

| Actual constructor kind | BEFORE calls | AFTER calls |
|---|---:|---:|
| `search_pruning_benchmark_artifact` | 1 | 1 |
| `traversal_benchmark_artifact` | 4 | 4 |
| `symbol_search_benchmark_artifact` | 1 | 1 |
| `current_query_benchmark_artifact` | 1 | 1 |
| Production `empty_graph_artifact` | 1 | 1 |
| MCP overlay-skip test literal | 1 | 1 |

`before-native.json.gz` and `after-native.json.gz` contain genuine native exports,
full compiled source readbacks, exact command/source epochs, real arguments,
caller locations and named thread/case identities. Small wire/Debug/skipped-ID
values are complete; large ones carry native BLAKE3, SHA256 and byte length.
Native canonical content and Rust capability identities, both absence getters,
all eleven vector lengths/capacities, and multiplicities match. Numeric thread IDs
and sequence numbers are retained; the join compares named cases and resolves
shifted source locations to actual function declarations. No native hash or
canonical algorithm is implemented in Python. `equality-join.json` records the
join and thirteen independently rejected mutations of copied actual records.
`join.py` and `verify.py` replay those checks.

The production helper was observed immediately after its literal construction,
with empty collections, before the existing reducer mutates it. The owning
library cases are the exact existing
`git_walk::tests::ordinal_reducer_buffers_out_of_order_results_until_gap_closes`
and
`mcp::tests::overlay_client_for_backend_skips_wrap_when_worktree_matches_index`.
Both phases ran both cases and all their original assertions.

The benchmark used the existing `SPUR_GRAPH_PERF_FIXTURE` override. Temporary
native preparation called ordinary `build_facts`, `artifact_from_facts` and
`write_artifact_parquet` on the three unchanged PRE-bound `sample_corpus` files,
then made the artifact read-only. BEFORE created it; AFTER reused exactly the
same path and all fifteen file bytes. Both native processes proved six loads of
that actual input. Native exports include every fixture/corpus file's SHA256,
length and base64 bytes; `before-native-summary.json` gives a compact inventory.
The input contains three graph files, 60 symbols and 111 edges. The constructor
workloads remain 16,384 rows times eight groups (131,072 symbols), 512 noise rows,
516 traversal symbols/edges and the original capacities (including 1,024 for the
grown traversal vectors). Preparation/serialization happened outside timed
regions. The default external fixture/repository fallback route was **not run**.
This is value conservation, with no performance claim.

| Validation | BEFORE | AFTER/final |
|---|---|---|
| Unfiltered graph `--lib` | 589 passed, 0 failed, 3 ignored, 0 filtered | 589 passed, 0 failed, 3 ignored, 0 filtered |
| Exact owning library cases | 2 passed, 0 failed, 590 filtered | 2 passed, 0 failed, 590 filtered |
| Existing provider/accessor negatives | 16 passed, 0 failed, 0 ignored/filtered | 16 passed, 0 failed, 0 ignored/filtered |
| Remote release Criterion test proxy | hit/miss: 2 Success; four actual helper kinds | hit/miss: 2 Success; four actual helper kinds |
| Graph `check --all-targets` | not required/run | passed (exit 0) |
| Local package-scoped `fmt -- --check` | source preserved | passed |

Criterion is `harness=false`: the two `Testing ... / Success` cases and actual
constructor exports establish execution. No zero-libtest or compile-only count
is offered as native acceptance. Existing malformed metadata, source binding,
opaque/unsupported, duplicate, captured-empty, None and accessor/native writer
controls remain unchanged. The optional existing environment-driven replay probe
is included in its suite's named results; it is not used for six-site coverage.
`command-summary.json` retains every named test status, ignore reason, count,
filter and native case count; raw output remains in lossless `*.log.gz` files.

Every build/test/check used `scripts/spur-cargo`, `SPUR_REMOTE=1`,
`SPUR_NO_LOCAL_FALLBACK=1`, `SPUR_SYNC_FULL=1`; the accepted observation/final
commands also used `SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0` to preserve full output.
The benchmark argv was exactly the remote release test proxy in ROOT PRE.
Formatting stayed local. Each `*.command.json` stores exact argv, controlling
environment, UTC start/end, actual exit, HEAD and SHA256/length bindings for all
131 source inputs before/after the command. Source and HEAD stayed stable for
every command. Native source readbacks authenticate the exact observer overlays;
`*-unobserved-source.json.gz`, all observer patches and `observe.py` preserve the
temporary context. All observation/preparation edits were removed before final
checks; final source bytes equal the minimal migration commit.

All diagnostic history is retained:

- `baseline-lib`: sandbox AWS endpoint failure, exit 200, zero compiler/tests;
  network-enabled retry passed remotely. There was no local fallback.
- `before-owning-interleaved`: both original cases passed, but stderr native JSON
  interleaved with libtest stdout. Those records are rejected for conservation.
  Exact source/patch/log are retained; locked stdout observers produced the
  accepted repeat.
- `before-bench-observer-compile-failure`: exit 101 from an observer-only JSON
  macro expression, zero benchmark execution. The observer hash expression was
  corrected and the same remote route passed. This is a real compiler failure,
  not infrastructure failure or semantic runtime RED. No original assertion or
  constructor value was changed to resolve it.
- The initial audit used the real plan task ID where the tool expected an issue
  ID; it returned issue-not-found and did not persist. Canonical `bd-3dil` audit
  succeeded (comment 4750), including identical successful replays. ACTION audit
  comment 4752 likewise has its returned first receipt and identical replay.
  Full payloads/receipts are in `intent-audit.json` and `action-audit.json`.
  RECORD audit comment 4757 persisted with its identical successful replay;
  `record-audit.json` preserves both complete returned receipts. Its digests bind
  validation commit `09546e21945102a13e88d769a4b7dc4e076558db`, including that
  epoch's manifest. The final manifest additionally binds these receipts.

The three existing model-download ignores remain named in both full library logs:
`coderank_embed_smoke_constructs_model_and_returns_expected_dimensions`,
`fastembed_smoke_constructs_model_and_returns_expected_dimensions`, and
`jina_code_smoke_constructs_model_and_returns_expected_dimensions` (all under
`store::lance_sections::tests`). Existing dead-code warnings remain visible.
M02's six core/worker failures remain open under `bd-3dkq`, `bd-1izf`, `bd-3umt`;
separate analyst issue `bd-16eq` is also open. `inherited-issues.json` archives
live read-only issue responses. They were not rerun or repaired here, and no
common cause is inferred.

Replay with `python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M03/worker/verify.py`.
`SHA256SUMS.json` binds the archive. `root-post-candidates.json` supplies observed
source/native/result bindings for ROOT to independently derive fresh Jev POST.
ROOT actual-source/native-bound POST and independent review remain required
before approval and M04.
