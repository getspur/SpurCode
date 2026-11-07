# M01 blocked before migration

M01 is **not implemented**. All three owned source files have been restored
byte-for-byte to dispatch base 7ff31e3fa22a21653879a24ef1544264bd84e5b6.
The ROOT --current-source verifier passed again after restoring them, including
all 93 frozen bindings. No assertion, fixture, production behavior, module wiring,
Cargo/dependency/feature, G03 archive, or predecessor artifact was changed.

## Concrete blocker for ROOT

The required owning-package command `scripts/spur-cargo test -p spur-core --lib
rebuild_singleflight -- --nocapture` exited 0 but executed **zero tests**:
0 passed, 0 failed, 0 ignored, 0 measured, **1601 filtered out**.
Current `crates/spur-core/src/server/handlers/mod.rs` declares only code_graph,
delegation, plan, and plan_execute. A search of current spur-core/src finds the
coordinator only within the assigned rebuild_singleflight.rs file. That file's
temporary observer was never compiled, independently corroborating that it is
outside the current module tree. Exact source hashes are in core-reachability.json;
the command, compiler output, test count and source bindings are retained in
before-network-core.{log,json} and before-network-core.source.diff.gz.

Executing this exact existing core helper in its owning package requires test
module wiring outside the three-file write scope, or ROOT must revise the native
acceptance requirement. Neither action was inferred or performed. No unrelated
coordinator fixture was substituted. This prevents the complete five-site native
baseline, so no constructor migration or actual POST was attempted.

Scope blocker audit bede36b3-65fc-4bb5-a127-d245c3e3a8f3 is persisted as comment
4657. Scope signal 4b2c96e7-7020-4765-b8d7-244367b7c3d0 is confirmed recorded,
late=false. The canonical issue bd-3txu remains in_progress under the original
assignee and singleton delegation; it was not closed or approved.

## Actual baseline results

Every heavy command used scripts/spur-cargo with SPUR_REMOTE=1,
SPUR_NO_LOCAL_FALLBACK=1 and SPUR_SYNC_FULL=1. Commands ran serially.

| Native target | Passed | Failed | Ignored | Measured | Filtered |
|---|---:|---:|---:|---:|---:|
| analyst library, overlay filter | 18 | 0 | 0 | 0 | 97 |
| analyst embedding integration, overlay filter | 0 | 0 | 0 | 0 | 29 |
| analyst overlay integration | 5 | 1 | 0 | 0 | 0 |
| CLI analyst_temporal_views | 14 | 0 | 0 | 0 | 0 |
| core library, rebuild_singleflight filter | 0 | 0 | 0 | 0 | 1601 |

The current analyst failure is
worktree_overlay_keeps_base_edges_to_reemitted_delta_targets, at
crates/spur-analyst/tests/overlay.rs:158: left 0, right 1. This reproduces the
historical bd-16eq symptom in an actual current execution. The original test and
production source identities were bound; no expectation was changed or failure
filtered, ignored, repaired, or claimed fixed. No after-migration run exists.

command-summary.json retains every executed test name/outcome and per-target
counts. Eight remote command attempts are retained: three sandbox infrastructure
exits 200 without compilation, two observer syntax compile failures 101, the
zero-test core exit 0, and the corrected analyst exit 101 / CLI exit 0 above.
The observer syntax mistake was accidental, corrected with parentheses, and
does not constitute a manufactured semantic RED. The first local runner setup
mistake raised FileNotFoundError before any build; see ENTRY.md.

## Native observations and identity

native-before.json contains **20 valid native records**, covering the existing
analyst manifest/counts, both actual artifact_with_file argument combinations,
and all 14 actual CLI helper inputs. Four of the five required construction-site
types are covered. The core fixture remains missing. Native Rust produced exact
Serde wire strings, Debug values including skipped node IDs, canonical artifact
BLAKE3 and capability digests, executable SHA-256, rustc -vV, package and build
environment identities. Python only parsed/joined/authenticated these outputs.

One analyst observation line interleaved with harness stdout and cannot be
parsed. Its raw bytes remain at before-observed-analyst.log:162 and it is
explicitly excluded, with no reconstruction or invented value. Other intact
observations cover both analyst artifact input combinations and the manifest
plus nested row counts. All 24 analyst and 14 CLI executed test names/outcomes
are present in the machine-readable summary and full logs.

Each command's .json metadata binds the exact argv, environment, times, source
HEAD and SHA-256 build-input files. Its .source.diff.gz losslessly retains the
temporary instrumentation patch against the dispatch base. observation-only.patch.gz
is the failed observer epoch; observation-corrected.patch.gz is the corrected
epoch. Neither instrumentation nor migration remains in source. The remote full
sync's actual source-tree hashes and provider rustflags are in the logs. Executable
and invocation identities belong to their respective temporary observation builds.

No native before/after equality or five-site conservation result is claimed.
root-post-candidates.json marks actual POST **not ready**. Provider/all-target
final checks were not run because the migration gate remains blocked. Local
scripts/spur-cargo fmt -- --check passed after restoring source.

## Entry and replay

See ENTRY.md and solver-comparison.json: both complete live PRE solver reloads
equal root-pre/proof.json.gz, with fingerprint
27b31490e5d9fa2a2ec7d497db9b8fa59635ca374e9d5e1a46e58ab417ed5653,
93 bindings, five sites, eight prospective cases, nine false controls and 42
variables. This remains prospective policy only.

INTENT UUID 79bb5c31-7bba-4a4a-819b-c422f9860e54 is persisted as comment 4654
and identically replayed idempotently. Exact request/response/replay are retained.
The final handoff audit records an incomplete, scope-blocked dispatch, not
completion or ROOT approval. SHASUMS authenticates the stable evidence packet;
handoff request/response/replay are added in a subsequent receipt-only commit.
