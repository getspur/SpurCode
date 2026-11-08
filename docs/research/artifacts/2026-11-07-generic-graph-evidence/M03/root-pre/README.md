# M03 prospective six-constructor PRE

ROOT issue `bd-1hk7`; accepted predecessor/local main:
`698bb2a12d457866d7facc9a5bbc19ce07714095`.

Fingerprint `5fad21dc4009ee6976cfa5f93a3bc877b0b21df62c5e90180a64a1d61b882a82`
binds 131 current source files, including the six exact literals, draft APIs,
execution routes, benchmark baseline configuration, three existing sample-corpus
files, and accepted M02 POST inputs/proof. Nine prospective preservation cases,
two ownership lanes and 33 compiler variables have no synthesis unknowns. Jev
1.14 opens at weakest confidence 0.87. Complete pinned/reloaded receipts:

- `sol_9da019eed1cd4dfc`: SAT/pass, intended migration.
- `sol_064efa3db9a646bc`: UNSAT/fail, invalid migration.

The independent oracle checks eight predicates and rejects ten individual false
claims. Native M03 observations are pending; PRE is not implementation acceptance.

## Exact source changes

Only these three files may change, besides M03 worker evidence:

- `crates/spur-graph/benches/parquet.rs`: `search_pruning_benchmark_artifact`,
  `traversal_benchmark_artifact`, `symbol_search_benchmark_artifact`,
  `current_query_benchmark_artifact`.
- `crates/spur-graph/src/git_walk.rs`: `empty_graph_artifact`.
- `crates/spur-graph/src/mcp/mod.rs`: the `GraphIndexArtifact` inside
  `tests::overlay_client_for_backend_skips_wrap_when_worktree_matches_index`.

All twelve named typed fields are `None`. Replace only them with six
`..GraphIndexArtifact::empty()` tails. Preserve every other byte/expression,
including the explicit production temporal header, current manifest/hash calls,
benchmark `bench` and test headers/hashes, clones, node IDs, vector capacities,
selector inputs, rows, edge fields and assertions. Keep the explicit fields in
`empty_graph_artifact`; do not replace its whole body or change walk/reducer/cache
behavior. There is no generic carrier/schema/collector cutover, dependency/Cargo,
performance, new capacity or M04 change. No manufactured semantic RED or mirror
test is needed for this refactor; unchanged original tests guard it.

## Genuine execution and fixture identity

Use `scripts/spur-cargo` with `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1
SPUR_SYNC_FULL=1` for builds/checks/tests. Native before/after records must reach
all six actual site kinds, with unchanged real inputs and multiplicities. Small
wire/Debug/skipped IDs can be preserved directly; large values may use native
digest/length bindings. Compare native canonical/capability identity as well.
Python authenticates/joins actual exports and does not invent native hashes or
expected passing data. Temporary observers/fixture preparation may live only in
the three approved files and must be removed before final checks.

Existing library tests reach the production helper and MCP fixture, for example
`git_walk::tests::ordinal_reducer_buffers_out_of_order_results_until_gap_closes`
and `mcp::tests::overlay_client_for_backend_skips_wrap_when_worktree_matches_index`.
Verify actual names/calls and record nonzero execution. The final full library
suite and graph all-target check remain required.

The Parquet benchmark is a custom Criterion `harness=false` target. Use a remote
test proxy, for example:

```sh
scripts/spur-cargo test --release -p spur-graph --bench parquet -- --quick bench_exact_search_row_group_pruning
```

`scripts/spur-cargo bench` routes locally and is not the permitted native route.
Criterion setup calls other groups even when measurements are filtered; prove
which four constructors actually ran rather than assuming from a filter/zero
test result. Preserve the 16,384 rows times eight row groups, 512-noise/traversal
sizes, loop bounds, thresholds and existing assertions. Observe outside timing.

`load_fixture` has an existing `SPUR_GRAPH_PERF_FIXTURE` override. The configured
default is an external `/Volumes/Projects/...` artifact and otherwise falls back
to building the whole repository. For repeatable native conservation, ROOT allows
the existing override to a temporary read-only Parquet artifact built through
ordinary native graph APIs from the bound, unchanged
`crates/spur-graph/tests/fixtures/sample_corpus` inputs. Prepare it natively in the
approved temporary observation context if necessary. Pin the actual fixture and
input source identities; use exactly the same data in both phases. Do not change
baseline configuration, production defaults or workload constants, fabricate a
replacement fixture in Python, add a persistent harness/dependency/flag, or claim
the default route passed when it failed or was unexecuted. This establishes
constructor conservation, not a performance improvement.

After restoring observers, run the unfiltered graph library suite, graph
all-target check, scoped local formatting and full source/evidence diff checks.
Retain existing negative controls. Record command argv/environment, exact counts,
raw logs, compiled source/HEAD and native identities. Keep source/HEAD stable
during each remote command. Actual failures/ignores stay named; no local fallback,
silent filtering, threshold tuning or infrastructure failure labelled compiler
or assertion RED. M02's open `bd-3dkq`, `bd-1izf`, `bd-3umt` and separate analyst
`bd-16eq` remain open. Diagnose/report a new blocker without broadening this slice.

## Replay and retention

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M03/root-pre/verify.py --current-source
```

Omit `--current-source` after migration to authenticate the frozen PRE. ROOT
released only the consumed M01 accepted POST pins after complete live equality
and eight independent predicates; exact envelopes are archived. The accepted
M02 POST pins remain protected. No persistence quota or solver limit changed.
