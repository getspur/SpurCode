# M6 runtime and Rust notebook observations

Ready for ROOT review on `bd-1pvg`, delegation `6abeb5630b4244d2`, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. Fresh ROOT Jev POST and approval remain
required. The issue is left in progress; G2/G3–G5 remain separate and incomplete.

## Authorized entry and identities

All five predecessor nodes were approved. The live issue/plan matched the exact
delegation label and assignee. Worker INTENT `563da938-0912-4f2b-81b9-acd9e6947b20`
persisted as comment `3578`; the identical replay returned `idempotent:true` with
that same comment. Complete requests/replies are in `entry/`. The fresh M6 PRE
verifier passed, and its three live lowered requests/results exactly matched the
archived reloads: SAT/SAT/UNSAT. No earlier architecture SAT is runtime evidence.

The unchanged 64-file production fingerprint is
`b633efdf819255ce175c3cf5dab318cae6908e4b07793cc5f43621882b9b643c`.
Exact normal macOS ARM64 debug artifacts were privately cloned from the approved
M5 installation:

| Artifact | SHA256 |
|---|---|
| Ordinary host | `06d245278b0298e8ce1f02ca5373fb3f3c0d9b10094e129b2870dca19accb033` |
| Ordinary helper | `e6f16fe0d1c4727a1f4172d7e21c11e6e5e03fc3456fb89e046d91962949bbf7` |
| ORT 1.22.0 archive | `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07` |
| Loaded ORT dylib | `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a` |

`before-final-native-identities.json` and `after-final-native-identities.json`
have identical 106-path maps, including actual source, binaries, model/tokenizer
files, original shared-cache files, private copies, runtime and archive. The
ordinary product runs also have their own before/after source/model snapshots.
Private HF revision metadata added for the test control is recorded separately;
the model bytes and read-only original caches were preserved.

No production source was repaired. One opt-in graph measurement test and focused
drivers/evidence were added. Milestones: `b1f204d7d`, `ba31107c0`, `895968f79`.
All Cargo commands used `scripts/spur-cargo`, one writer, `SPUR_REMOTE=0`,
`TMPDIR=/private/tmp`, target `/private/tmp/rust-gap-g1-acceptance-target`, pinned
archive and `/private/tmp/s6c-duckdb`. Formatting was local.

## Commands and actual outcomes

`run_checks.py STAGE` records exact argv/environment, exit, time, and compressed
unmodified streams. `test-counts-ignores-filters.json` lists named ignores and
actual libtest filters; no new assertion failure is called a native baseline.

| Gate | Actual result |
|---|---|
| Neutral client with test support | 48 passed; initial process-inspection EPERM retained, authorized rerun exit 0 |
| Default graph unit/integration targets | 105 invocations proceeded successfully; all-target command then exited 2 because Criterion rejected libtest `--test-threads` |
| Graph doctests | 2 passed |
| Graph examples | exit 0 |
| Criterion incremental | all cases succeeded, including 100,000-file and 20,000-commit workloads; subsequent overlay fixture failure retained |
| Overlay and Parquet remaining targets | exit 0 each with explicitly identified small private graph fixture; no repository CURRENT creation |
| Graph publication/batching fixture suite | 66 passed, 3 inherited model-download ignores; named legacy reference generator run separately below |
| Analyst embedding integrations | 29 passed |
| Analyst embedding unit scope | 9 passed, 1 named real-helper ignore, plus successful internal mode-test subprocess |
| Explicit same-owner query/v1 real acceptance | all three models, two lifecycles per model, exit 0 |
| CLI analyst temporal + graph temporal | 14 + 2 passed, exit 0; original five-row PGQ assertion unchanged, including repeated `serde::Serialize` |
| Separate distinct graph batch measurement | all three models, exit 0 |
| Explicit legacy real helper/reference/Parquet generator | all three models, exit 0; actual JSON/Parquet in `real-sidecars/` |
| Preserved direct-native wrapper workspace | 43 passed, exactly the eight historical abort failures, no ignores; doctests executed; exit 101 |

The last native suite used `--no-fail-fast`. Its eight failures exactly match
`historical-direct-native-baselines.json`: seven lifecycle cases and
`relocated_process_runs_ort_inference_then_pgq`. They remain real failures after
correct inference/PGQ output. New failures were not added to that list. The
separate broad historical O1 audit is not approved by these results.

Environment/invocation failures remain under their own names: sandbox socket/ps
EPERM; initial parallel-link ENOSPC; Criterion argument rejection; missing
overlay fixture; unconfigured explicit reference generator and consequent test
mutex poisoning; interrupted private-HOME build; native build ENOSPC and an
unavailable section-vector assertion during disk exhaustion. Only the task's
new private `.rustup`/`.cargo` directories were reclaimed. Corrected invocations
and independent reruns are preserved alongside those failures.

## Vectors, publication and actual faults

Six ordinary product lifecycles returned finite 768-dimensional unit vectors
within `1e-4`, matching the separately pinned **query** references. Maximum query
delta was `3.706607812348217e-9`. Three direct ordinary-helper observations pin
the exact model/tokenizer/profile identities, repeat distinct preformatted
document batches, and observe raw helper waitpid status 0 after Closed. Six
ordinary host waits are raw 0 and Python returncode 0. The normal CLI does not
expose its helper's raw wait status; helper absence is recorded separately.

Same-owner Rust query/v1 tests explicitly reuse one owner for query and v1
multi-text requests. v1 still performs sequential single-text operations.
`runtime_measurement.rs` uses a separate graph owner and actual distinct
two-text batches. These are separate experiments and reference sets.

Real child processes abort before a reply, mid-frame, after a committed reply,
and on shutdown: supervisor traces contain raw `unix_wait_status(6)`.
Truncated normal exit 0, malformed JSON, generation/model/request/row/dimension
mismatch, nonfinite data, deadlines, queue pressure, cancellation, restart
exhaustion, blocked writes, close races and shutdown/reap failures were exercised.
Ten second-chunk fault exports record one earlier validated chunk and no logical
batch result. Independent DuckDB reads of five actual fault Parquet files retain
two committed vectors and a NULL third vector, with model/source identity intact.
Pending output is never silently certified. Raw status 9 records containment
kill where appropriate, and never clean native shutdown.

The actual ordinary CLI also built a private graph and analyst DB with both
documented no-embedding flags, an absent adjacent helper, and a missing explicit
helper override: exit 0. Public socket v1/mode tests and real sockets passed.

## Native dependency boundary

Actual `nm`, `otool`, loader streams and six live host/helper `vmmap` pairs are
preserved. The ordinary host has DuckDB symbols and no FastEmbed/ORT symbols or
loaded ONNX runtime; the helper has FastEmbed and no DuckDB/PGQ dependency/load.
Fresh normal/build trees corroborate the observed artifacts and the approved M5
normal-feature build identities. Product copies contain the two ordinary
executables; fault binaries, test-support and owner-test-hooks are excluded.
Test-only controls/fixtures are identified separately.

Already exited metadata probes yielded `vmmap` 255. Those failed observations
are retained and are not counted as live dependency inspection. Each of the six
actual inference helpers and six hosts has a successful live map.

## Measurements and comparison limits

macOS ARM64, ordinary debug artifacts, prepopulated private disk assets;
OS page cache was uncontrolled. Session cold includes discovery/identity work
and model/session initialization. Initial ordinary-product measurements ran
alongside serial Cargo checks, which is recorded as background load. RSS uses
`ps` samples at the inherited 20 ms interval plus sampling-command overhead;
these are sampled peaks. Combined RSS is a simultaneous sum, not a sum of
independent process peaks. Raw samples and workloads are in the JSON ledgers.

| Model | Product first request (s, two sessions) | Product warm query (ms) | Distinct graph warm 2-row batch (ms / rows/s) | Peak host / helper / combined RSS (MiB) |
|---|---|---|---|---|
| Nomic | 55.398 / 46.889 | 16.557 / 12.767 | 63.31 / 31.59 | 44.78 / 1726.72 / 1771.36 |
| CodeRank | 57.603 / 51.038 | 19.932 / 12.004 | 29.11 / 68.70 | 44.88 / 1731.31 / 1776.19 |
| Jina | 56.443 / 58.327 | 9.607 / 11.271 | 25.43 / 78.64 | 45.06 / 2013.47 / 2058.39 |

Graph session-cold two-row batches were 26.451 / 26.466 / 30.750 seconds,
after separate measured discovery. Graph checked shutdown was 31–37 ms;
ordinary-product shutdown was 34–92 ms.

An available **test-only in-process reference executable** ran the same two
preformatted document inputs with matching vectors and raw exit 0: Nomic
1089.67 ms / 1160.48 MiB sampled RSS; CodeRank cache-complete retry 529.41 ms /
1666.67 MiB; Jina 472.96 ms / 1398.00 MiB. It measures total process/model/batch/
shutdown and supplies no reused-session warm timing. Its loader/identity work
differs from the supervised path. The observed debug cold latency and sampled
memory cost are higher on the supervised path; no performance improvement or
optimal-default claim is made.

Current catalog checks compare complete measured integer rows with inherited
request/shutdown/dimension limits: SAT `sol_895fb3f7841a46bf`; three exceeded-bound
controls are UNSAT `sol_75e4f0843f7c481f`, `sol_13a99f87cfc14ec9`,
`sol_4f5049a2c54947cc`. All four were persisted/reloaded/pinned. The independent
oracle checks 12 predicates, exact fixed facts, lowerings and models. No new
capacity/deadline was selected, and finite SAT does not prove future performance.

## New Notebook MCP acceptance

Notebook skill was discovered/read through the catalog before operations.
New notebook: `/Users/kevintruong/project/spur-notebook/embedding-runtime-acceptance-bd-1pvg-563da938.ipynb`,
ID `nb-08896ffe4a694c270f8311ef`. Eight cells and canonical tool-exported outputs
are in `notebook-document.json`; mutations targeted this new notebook only.

- `a34d42a0-…`: actual Rust kernel arithmetic probe, output 42.
- `db133caf-…`: actual Rust UnixStream client, ordinary Nomic/helper inference,
  real helper SIGABRT, rejected next output, live host ping, retained prior vector.
- `36c97aa6-…`: actual Rust runs the unchanged built default CLI PGQ regression
  before and after a fresh real helper abort; both raw waits 0. Failure reply
  16.327 ms; post-failure host shutdown raw 256/code 1, socket removed, child gone.
- `846127fb-…`: actual Rust observes native runtime loaded and no response available
  before SIGABRT during pending first inference. Typed failure in 54.682 ms,
  host ping survives, explicit failure on close, child absent.

SQL SELECT 42 also succeeded before/after the helper failure. The notebook SQL
kernel's DuckDB 1.5.5 lacks DuckPGQ: the initial native SQL PGQ setup failed and
the extension catalog returned zero rows. The unchanged PGQ regression was
actually executed from Rust using its built default CLI test artifact; no native
notebook-SQL PGQ success is claimed. Rust `catch_unwind` only cleans up Rust test
assertions; the native abort boundary is the separately executed helper.

## Retained risks and ROOT handoff

The original test-only CodeRank in-process control failed an offline HF lookup
then aborted on native teardown (raw 6 / Python -6). This is a **separate risk**,
not a ninth historical graph baseline. Comment `3585` and risk signal
`8cdcf291-0a24-4fde-a64e-0d1ef18bdd8f` record source/risk evidence. A missing pinned
revision ref was added only inside the private cache; its separately labeled
retry exited 0. The original failure remains, and no native production repair
or broad O1 approval was attempted.

PyArrow's full nullable fixed-size-vector Parquet read fails with
`Expected all lists to be of size=768 but index 3 had size=0`, including an
authorized rerun. DuckDB independently reads the correct committed/NULL rows.
That reader interoperability limitation remains explicit; it is not called a
passing PyArrow check or repaired without a bounded ROOT gate.

```sh
python3 -B docs/research/artifacts/2026-11-05-embedding-runtime-acceptance/verify.py
python3 -B docs/research/artifacts/2026-11-05-embedding-runtime-acceptance/prepare_post.py
```

`verify.py` exits 0 while explicitly accounting for eight native failures and
the retained risks. It independently rejects stale source, forged vector,
missing observation and duplicate identity controls. `post-inputs.json` keeps
the complete required task binding separate from actual observations and hashes
for ROOT's fresh Jev POST/adversaries. Worker success and transport receipts do
not constitute ROOT acceptance. No release/publication, user install overwrite,
service restart, main merge, old notebook edit, or external notebook source/
graph/CURRENT edit was performed. Linux/Windows/optimized-release execution and
general native memory safety/complete panic detection are not established.

Final worker MCP handoff acknowledgement is unavailable: the read-only issue
refresh, final audit `54d206da-5fa7-4a3a-a36d-448872de457f`, and its identical replay
received no response within the observation allowance. Persistence is **unknown**;
`entry/final-audit-*.json` preserves the exact payload and outcomes. The initial
INTENT/replay (comment 3578) and earlier risk audit (3585) were actually persisted.
ROOT should inspect Beads for that UUID before any idempotent retry through an
authorized worker context. No successful final persistence or new status
transition is inferred from the timeout. The later progress call was also
unacknowledged/interrupted; this does not change the saved runtime outcomes.

