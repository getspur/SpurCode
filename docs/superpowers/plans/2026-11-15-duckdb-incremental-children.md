# Incremental File-Facts Children Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-11-15-duckdb-incremental-children.md`
**Formal @spec cells:** none (markdown contract)
**Design issue:** `bd-1phw` (close after committing this approved design)

**Goal:** Maintain five typed child tables by changed cache key while preserving independently refreshed caller lineage.

**Architecture:** A durable key-only checkpoint records completed child expansion.
Refresh new/retried keys and remove orphans in the existing transaction. Missing
checkpoint/child tables trigger one cache-driven bootstrap. Full exports retain
their current behavior and schemas.

**Tech Stack:** DuckDB 2.0.0 SQL, Python real-engine tests, existing native V2 artifact; parent-owned Rust notebook validation.

---

### Task 1: Independent test-first incremental contract

**Task ID:** `incremental_children_contract`
**Files:** modify `crates/spur-graph-duckdb/tests/test_file_facts_pipeline.py` only.
**Depends on:** none.
**Worker:** codex / gpt-6.1-sol / xhigh, as requested by user.

Add focused real-engine regressions. A small fixture uses the existing RUST source,
distinct logical paths and extractor labels; preserve all existing tests. Helpers
may compare every child to its complete cache UNNEST, in both directions, including
native fields, types and ordinals. They must not reproduce the delta algorithm.

Representative contract assertions within the same connection after initial run:

```python
before = self.rows("SELECT table_name, table_oid FROM duckdb_tables() WHERE table_name IN "
                   "('file_facts_nodes','file_facts_edges','file_facts_spans',"
                   "'file_facts_macro_invocations','file_facts_macro_scopes') ORDER BY table_name")
self.run_pipeline()
after = self.rows("SELECT table_name, table_oid FROM duckdb_tables() WHERE table_name IN "
                  "('file_facts_nodes','file_facts_edges','file_facts_spans',"
                  "'file_facts_macro_invocations','file_facts_macro_scopes') ORDER BY table_name")
self.assertEqual(before, after)
self.assertEqual(self.scalar('SELECT count(*) FROM file_facts_child_refresh'), 0)
self.assertEqual(self.scalar('SELECT count(*) FROM file_facts_child_removed'), 0)
```

After reopen, assert zero deltas, checkpoint coverage and exact child content;
do not assume internal table OID numbers persist across connections.

Also test lineage-only duplicate/root/merge changes, a new path/version without
altering existing rows, cache-key deletion with no source restaging, and deletion
then retry of the same valid key (no duplicate old children). Test NULL/error/empty
parents' checkpoints, empty staging, migration of pre-existing full-cache/child
tables with no checkpoint, and repair when one child table is dropped. Exact
current snapshot exports remain required.

Run and retain intended RED against unmodified SQL:

```sh
SPUR_GRAPH_EXTENSION=/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension \
  /private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover \
  -s crates/spur-graph-duckdb/tests -p test_file_facts_pipeline.py -v
```

Failures must be missing incremental behavior, not a bad metadata API or fixture.
Commit RED test contract, record exact command/log and scope, do not merge/close.

### Task 2: Incremental SQL and documentation

**Task ID:** `incremental_children_sql`
**Files:** modify `crates/spur-graph-duckdb/examples/file_facts.sql` and `crates/spur-graph-duckdb/README.md` only.
**Depends on:** `incremental_children_contract` approved.
**Worker:** codex / gpt-6.1-sol / xhigh.

Implement the transition in the spec. Keep the original source anti-join and
extraction materialization boundary. Preserve the cache, all five child schemas,
caller occurrence schema and all seven Parquet paths. Maintain key-only durable
`file_facts_child_state`; expose TEMP refresh/removed key relations. Do not invent
an all-payload hash, quota, new native API or source-provider requirement. Handle
bootstrap, delete/retry and missing-child repair before steady delta updates.

Keep caller lineage rebuilding outside the child delta condition. State, children,
cache and occurrences remain transactional; exports still follow commit. Explain
checkpoint ownership/reset, legacy bootstrap, supported cache deletion/retry and
the remaining full snapshot cost in README.

Run the approved contract GREEN and actual engine regressions:

```sh
SPUR_GRAPH_EXTENSION=/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension \
SPUR_FILE_FACTS_ORACLE=/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/reference_file_facts \
PYTHONPATH=crates/spur-graph-duckdb/tests \
  /private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest \
  test_file_facts_pipeline test_file_facts test_extension test_temporal -v
```

Re-run the small catalog snapshot requests using `persist:false` as POST evidence;
state their finite scope. Retain exact source/artifact hashes and command logs.
No Rust change/build is required; any necessary build must use `scripts/spur-cargo`.
Do not weaken the test-first contract, merge or close.

### Task 3: Independent source and contract review

**Task ID:** `incremental_children_review`
**Files:** no planned writes.
**Depends on:** `incremental_children_sql` approved.
**Worker:** codex / gpt-6.1-sol / xhigh.

Review the full branch diff against this spec, including no-op behavior, complete
key identity, retry/deletion semantics, bootstrap/repair, NULL/error/empty parent
coverage, unchanged list ordinals/types, independent caller multiplicity and
transaction boundaries. Read the test/solver/runtime evidence; distinguish finite
snapshot checks from runtime transition coverage. Report concrete blocking findings
with file:line and reproduction, or explicit none. No source edits or expensive
duplicate builds; no merge/close. Parent owns final integration and notebook timing.

### Parent integration and validation

After approval, combine the plan on its dedicated branch, retain user dirty files,
and compare frozen old/new SQL on copied 19,604-key caches. Measure bootstrap
separately, then repeated zero-miss runs and matched 1%/10% hash-bucket updates.
Verify lineage-only changes with zero child refresh, all typed fact/child outputs
against a full-UNNEST reference, seven Parquet round trips and zero re-extraction
after reopen. Trace child-maintenance phase separately from exports and include
host load; no assumed timing threshold. Commit the measured report and close the
implementation epic only after required work is actually complete.

No worker touches parser code, Cargo, prior benchmark artifacts, notebook files
or user changes. Signal any needed out-of-scope edit before proceeding.

### Review correction R1: active-schema layout detection

Independent review `bd-18w2` reproduced a hardcoded `main` predicate in the first
implementation. Non-default-schema replays refresh all keys; an intact `main`
layout can mask a missing child table elsewhere and leave it empty. The parent
independently reproduced both failures on the pinned native engine. No original
candidate is integrated into main or benchmarked before correction.

Follow-up plan `d119066b-25a1-4644-8159-cf65c8487953`, epic `bd-22ye`, starts at
`8d3f7a285`. `schema_layout_fix` (`bd-94e5`) owns only SQL and pipeline tests:
commit an intended failing multi-schema regression before the minimal fix, adapt
test metadata helpers for persistent active-schema versus TEMP relations, and run
the full real-engine regression suite. `schema_layout_review` (`bd-82z4`) performs
an independent read-only review afterward. Both use codex/gpt-6.1-sol/xhigh.
The parent refreezes the corrected candidate, integrates verified bytes, then
runs the previously prepared notebook comparison.
