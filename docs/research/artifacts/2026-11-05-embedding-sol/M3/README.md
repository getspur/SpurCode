# M3: compatible local helper discovery and child cache

Implementation is ready for root review on `bd-2fkd`, delegation
`1d775afd778c44e8`, plan task `discovery-cache`. The issue remains `in_progress`.
Root's fresh Jev POST and node4 PRE are pending. Batching boundaries, full local
acceptance, three-model comparisons, packaging and Notebook acceptance are not
claimed by this packet.

The actual session resolved `gpt-6.1-sol` / `xhigh`, confirmed in its current
rollout `turn_context`; no model fallback or skipped selection occurred.
[dispatch.json](dispatch.json) preserves the authorized INTENT and identical
replay (comment 3427), seven reloaded PRE statuses, routing, and the first failed
lookup using the plan task key instead of canonical issue ID. That lookup failed
before authorization; the corrected fresh canonical issue authorized both calls.
No implementation preceded authorization and the M3 current-source verifier.

## Changes and milestones

- `9980f669d`: regression tests committed before implementation. The expanded
  [assertion RED](red-all.log) observed mandatory JSON rejection for discovery,
  cache, invalid path, metadata and deadline cases. Compilation succeeded; this
  was assertion RED, not a build error.
- `5ae93bf9c`: [production diff](production.diff). Neutral `LocalHelper` resolves
  an explicit absolute `SPUR_EMBEDDING_HELPER` or the platform sibling beside
  `current_exe`, preserving OS strings and argument vectors. New selections run
  strict bounded metadata validation before descriptor/model work. The graph
  adapter consumes M2's validated shared policy and removes duplicate JSON
  policy types. Complete legacy JSON retains its program/prefix semantics and
  precedence, including interpreter fixtures without a metadata command.
- Later test/evidence milestones add concrete error-type checks, exported
  per-case observations, and the normal-helper acceptance harness.

`SPUR_ORT_CACHE` is captured once into immutable child command configuration for
compatibility, descriptor and serving invocations. A nonempty explicit value
retains its meaning (relative values resolve against the captured current
directory); otherwise the base is `HOME/.spur/cache`, then `XDG_CACHE_HOME/spur`
when HOME is absent. Invalid supplied input fails with a typed error. Production
code never changes the parent environment or materializes the runtime cache.
The existing worker alone appends the reviewed runtime layout.

The supervisor's State/Gate/job/publication/cancel/shutdown mechanisms, native
runtime/model implementations, analyst ownership mechanisms and model registry
sources are unchanged. The analyst already calls the graph loader and M2 policy;
no analyst production glue was needed. All 14 inherited numeric choices remain
identical. Probe stdout uses the inherited frame cap with a checked `u64`
over-limit sentinel; stderr is discarded. Probe and descriptor share the one
effective startup deadline and existing caller-owned cleanup.

## Executed verification

All Cargo commands used `scripts/spur-cargo`, `SPUR_REMOTE=0`,
`TMPDIR=/private/tmp`, and
`CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`, one writer at a time.
DuckDB checks used `SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb`. Worker
builds/tests used the supplied `SPUR_ORT_ARCHIVE` path. Formatting ran locally.

| Command suffix after `scripts/spur-cargo` | Actual result | Evidence |
|---|---|---|
| `test -p spur-graph --test helper_discovery` | 5 grouped tests pass; 26 exported cases | [typed GREEN](green-discovery-typed.log), [cases](cases/) |
| `test -p spur-graph --test helper_discovery --test embedding_compatibility --test embedding_cancellation` | 12 pass | [graph GREEN](green-graph.log) |
| `test -p spur-embedding --features test-support` | 47 pass, 0 ignored | [neutral GREEN with process access](green-neutral-access.log) |
| `test -p spur-graph --lib skip` | 22 pass, 558 filtered | [disabled/ineligible checks](green-graph-skip.log) |
| `test -p spur-analyst --lib embedding::` | 9 pass; 1 pre-existing real sidecar acceptance ignored; 105 filtered | [analyst GREEN](green-analyst.log) |
| `test -p spur-analyst --test embedding warm_routes_modes_without_spurious_local_helpers -- --nocapture` | 1 pass, 28 filtered; Auto reachable/unavailable and Sidecar | [mode checks](green-modes.log) |
| `test -p spur-embedding-worker --test embedding_metadata stale_hash_and_profile_are_rejected_before_native_start -- --nocapture` | 1 pass, 10 filtered | [Hello checks](green-hello.log) |
| `test -p spur-embedding-worker --features test-support --test native_lifecycle native_initialization_failure_is_correlated_for_verified_identity -- --nocapture` | 1 pass, 6 filtered; correlated failure; raw wait status 256 (exit 1) | [initialization checks](green-native-init.log) |
| `test -p spur-graph --no-default-features --test embedding_compatibility` | 5 pass | [disabled-feature check](green-disabled.log) |
| `fmt -p spur-embedding -p spur-graph -- --check` | exit 0 | [format log](fmt.log) |

The first neutral run encountered sandbox `EPERM` in `ps` process inspection;
[that failure remains preserved](green-neutral.log). The complete neutral suite
then passed with authorized access. Analyst socket checks also used authorized
access. Git milestone writes required access to the worktree index outside the
writable root and succeeded. No automatic approval rejection occurred.

The initial path test encountered macOS filesystem `EILSEQ` while creating a
non-Unicode filename, before invocation; [that failure is preserved](green-first.log).
The execution regression remains Linux-only and was not compiled or executed
on this macOS host. Space/Unicode paths did execute. No other-platform execution
or general non-Unicode native runtime support is claimed. Existing warnings are
retained in the logs; no environment failure is counted as GREEN.

## Actual normal helper

The ordinary worker was explicitly built with no test features and non-test
profiles using `build -p spur-embedding-worker --bin spur-embedding-worker
--message-format=json`. [Build records](normal-build.jsonl) and
[stderr](normal-build.stderr) are preserved. The frozen helper is:

`/private/tmp/M3 normal layout ü 7dhy7ilg/spur-embedding-worker`

SHA256: `aaa719bff37894bafe500332a99954612d035fc83d332ff70b3408f91c3fed63`.
Direct `--compatibility` returned 64 bytes, empty stderr, exit 0, version 1.24.0,
protocol 2, macOS/aarch64. The metadata-only cache directory was not created.

[run_normal.py](run_normal.py) copied the five available Nomic artifacts into a
private model cache, then exercised a copied graph host beside the ordinary
helper with no JSON/helper/cache override in the host. Both the actual graph
loader and neutral client produced the same finite 1×768 vector and closed
successfully. The graph cycle took 40,249 ms; the whole two-cycle test took
80.51 seconds. These are one-run observations, not latency guarantees.

The neutral trace records Hello/Embeddings/Closed validation and
`ExitStatus(unix_wait_status(0))`. Its actual serving arguments were
`33554432 64 65536 4194304 768 64 1 644080771`. The child-only cache was
`/private/tmp/M3 normal layout ü 7dhy7ilg/home/.spur/cache`. The worker created
only the private runtime at
`onnxruntime/1.22.0/2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a/libonnxruntime.dylib`,
whose bytes match the pinned SHA. The host's cache variable stayed absent.

[normal-observation.json](normal-observation.json) records build features,
commands/environment, metadata, model revision/artifact hashes, helper binary
inspection and native hash. [normal-actual.json](normal-actual.json) records the
real identity, policy, vectors and process trace. [Acceptance log](normal-acceptance.log)
records 1 pass, 0 ignored (the explicitly provisioned ignored test was selected
with `--ignored`). Shared source model bytes were hash-checked unchanged; only
private copies were used for inference. Installed binaries, global runtime/cache
destinations, notebooks, graph/CURRENT, main's dirty files and unrelated source
were not edited.

## Root review input

Run `python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M3/verify.py`.
It checks the captured results, private binaries/assets, exact error cases and
source manifest independently. [verification.json](verification.json) binds 51
current source files to fingerprint
`dd5361615d064068fe6a06c1d57d6c5858f5a76da83ffdb45f14d9b4d75302ba`.
Among the original 47 files, only neutral client/lib and the graph loader differ;
new discovery and focused test files are also hashed. Native/model/analyst
sources match the entry gate.

Root must compile and execute fresh POST checks against these actual
observations, assess the explicit platform limit, persist/reload the results,
and supply node4 PRE before approval. The worker tools expose no Jev compiler;
the exact mediation request was streamed via `report_progress`. No POST receipt
or approval is fabricated here.
