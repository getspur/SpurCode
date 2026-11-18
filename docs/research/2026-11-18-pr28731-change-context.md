# PR #28731: changed files in the tracked repository context

The review starts with the five changed files and five changed C++ functions,
then maps them to unchanged code, shared contracts, build components and test
configuration. The context inventory contains **all 18,342 tracked head files**,
with base blobs used to verify which surrounding files are unchanged.

[Experimental notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-cpp-extraction-gap-lab.ipynb)
· [Evidence](artifacts/2026-11-18-pr-repository-context-map/)
· [Code context diagram](artifacts/2026-11-18-pr-repository-context-map/code-context.svg)
· [Configuration diagram](artifacts/2026-11-18-pr-repository-context-map/configuration-context.svg)

## The five-file map

| Changed file | Repository context | What a reviewer should inspect |
|---|---|---|
| `first_last_any.cpp` | CMake component `duckdb_aggr_distr`; unchanged optimizer callback dispatcher; unchanged aggregate-function metadata contract | Registration and use of `RewriteOrderedFirst`; metadata propagation to other consumers |
| `relation_statistics_helper.cpp` | CMake component `duckdb_optimizer_relation_statistics`; unchanged public wrapper bodies; unchanged join-order client, extractor and optimizer tests | Paths into the changed internal helper, including clients behind overloaded declarations |
| `stats_dependent_tests.json` | Shared skip list inherited by unchanged `disable_optimizer.json` and `verify_serializer.json` | Effective test selection changes without edits to those inheriting files |
| `verify_aggregate_state_export.json` | Listed in the unchanged Makefile test matrix; interpreted by the existing configuration loader | New regression exclusion under aggregate-state-export verification |
| `distinct_on_stats_propagation.test` | Added statistics regression; referenced by both changed skip lists and inherited configurations | Test intent and configuration coverage, separately from code-call evidence |

## 1. Incoming code context reaches unchanged clients

Pinned source references identify:

- `src/optimizer/join_order/relation_manager.cpp:623,856`.
- `src/optimizer/relation_statistics/relation_statistics_extractor.cpp:124`.
- Seven references in `test/optimizer/relation_statistics.cpp`.

These reference `RelationStatisticsHelper::ExtractAggregationStats`. Its two
implementation wrappers have **identical body SHA256 values before and after
the PR**, although their line positions move from 554/562 to 688/696. Both
forward into the changed `ExtractAggregationStatsInternal` at line 558.

The retained native syntax graph also maps the internal helper to the added
`HasBaseColumnSource`, `HasUniqueGroups` and `GetValuePreservingColumn` functions.
The notebook displays 15 incoming paths within two native call hops.

The useful review path is therefore:

```text
unchanged join-order / extractor / test client
  → public aggregate-statistics API reference
  → unchanged wrapper body inside the changed source file
  → changed internal implementation and new helper functions
```

The first hop remains **source-reference candidate evidence**: overload and
declaration/definition binding is incomplete. The wrapper-to-helper portion is
retained bound syntax evidence. The app should show that seam, rather than
silently dropping the unchanged client or inventing a resolved call path.

## 2. The two changed C++ areas share a contract

The unchanged `src/include/duckdb/function/aggregate_function.hpp` defines
`properties.single_value_identity`, its reader and setter at lines 353 and
401–402. In the changed aggregate rewrite, line 57 propagates that property to
the rewritten function. The changed statistics implementation reads it at
line 664.

Scanning the full tracked tree also finds an **unchanged execution consumer**:
`src/execution/operator/set/physical_recursive_cte_keyed.cpp:42`. The unchanged
`minmax.cpp:380,404` provides other producers of the same contract.

This connects aggregate-function definitions, optimizer statistics and a
recursive-CTE execution component as a **shared-contract review neighborhood**.
It does not establish that this PR changes the recursive-CTE runtime path.
That consumer is a concrete place to inspect, with an exact source witness.

The CMake declarations corroborate the component boundaries:

| Context file | Declared target |
|---|---|
| `first_last_any.cpp`, `minmax.cpp` | `duckdb_aggr_distr` |
| `aggregate_rewrite.cpp` | `duckdb_optimizer` |
| `join_order/relation_manager.cpp` | `duckdb_optimizer_join_order` |
| Statistics helper/extractor/operator sources | `duckdb_optimizer_relation_statistics` |
| `physical_recursive_cte_keyed.cpp` | `duckdb_operator_set` |
| Existing optimizer test source | `test_optimizer` |

These are build-declared components, not an assertion that an architectural
policy has been checked.

## 3. Callback context matters beyond ordinary calls

The changed `RewriteOrderedFirst` is registered at four source locations through
`SetDirectRewriteCallback`. The unchanged optimizer dispatcher in
`src/optimizer/aggregate_rewrite.cpp:58` invokes
`GetDirectRewriteCallback()(input)`.

The notebook displays this as a **possible callback-dispatch relationship**,
with the registration and dispatch witnesses. It is distinct from a directly
bound call and from observed runtime execution. This is a missing relationship
type that a call-only graph would omit from a PR review.

## 4. Unchanged configuration files have an effective effect

The added regression is explicitly listed in the two changed skip configurations.
Parsed inheritance produces these additional paths:

```text
unchanged disable_optimizer.json ─┐
                                 ├→ changed stats_dependent_tests.json
unchanged verify_serializer.json ┘    → added SQL regression (skipped)
```

The aggregate-state-export configuration also skips the new regression. Thus the
shared list plus three execution/verification configurations select the new test
for exclusion. These are declared configuration effects; this investigation did
not execute the DuckDB regression suite.

The unchanged `test/helpers/test_config.cpp:602–617` explains the inheritance:
the loader reads the inherited configuration and inserts its skipped tests into
the current configuration. This is a concrete non-call-graph dependency.

## What the notebook now supports

- Select any of the five changed files and query its incoming, outgoing,
  component, configuration and shared-contract relationships through DuckDB.
- Inspect direct and two-relation unchanged context for every changed file;
  these typed neighborhoods are explicitly separate from call-path reachability.
- Inspect exact source references, revision/blob identities and relationship
  provenance. All relationship endpoints join to the full tracked-file catalog.
- Distinguish bound syntax, qualified/unqualified reference candidates, callback
  candidates, shared-contract participation and parsed build/configuration facts.
- Keep unchanged symbol bodies visible even when their containing file changes.

The file catalog contains every tracked head entry. Text relationship discovery
scans 17,289 UTF-8 blobs; 1,053 non-UTF-8 blobs retain inventory metadata. The
evidence map records 332 relationships, including build hierarchy outside the
immediate change neighborhood. It is a selected, typed context map, not a
complete semantic graph for every tracked file.

The C++ binding gaps in the earlier proposal remain relevant. The required
app-facing result is a **change-to-repository context graph with source witnesses
and evidence types**, not a larger extraction-count summary. Future repository
resolution should improve these paths while preserving their uncertainty and
their build/test/configuration context.

The extension/app contract should therefore return context paths from changed
file and symbol seeds in one immutable repository snapshot, retaining unchanged
forwarders and file vertices whose languages lack symbol extraction. Call,
callback, shared-contract, build and configuration providers need separate
relationship types. Acceptance should include the three concrete paths above:
unchanged clients through wrappers, metadata-contract consumers, and effective
configuration changes through unchanged inheriting files.
