# Incremental file-facts child tables

User approval follows the notebook benchmark in `bd-189b` and authorizes this
implementation and its validation. The zero-miss trace spent 2.655 seconds
(72.5%) rebuilding child tables, 0.875 seconds exporting, and 0.00123 seconds in
the empty extraction statement. Optimize child maintenance as one change.

## Contract

Keep the existing `staged_sources`, `source_lineage`, native function, cache key
`(blob_oid, logical_path, extractor_sha)`, five child schemas/list ordinals,
parent statuses and seven typed Parquet snapshot names. Full exports remain in
this step. Keep whole-file native parsing and the original missing-key boundary.

The workflow owns derived child tables and a new durable key-only checkpoint
`file_facts_child_state`. Cache keys identify immutable content/path/extractor;
direct updates to cache payloads or derived rows are outside the contract.
Deleting a cache key to retry it remains supported. One writer, no surrounding
transaction, and database commit before non-atomic snapshot publication remain
the execution model.

## Transition

Let C be cache keys after this run's insertion, S the prior child checkpoint,
and M the newly extracted keys (`file_facts_extracted`, including retries).
For an established, intact child layout:

```
refresh = (C minus S) union M
removed = S minus C
delete old child rows for refresh union removed
insert each native list independently for refresh keys in C
next checkpoint = C
```

All five children, cache/state changes and caller occurrences commit in the same
transaction. No cache hit hashes every wide facts payload or recreates the child
tables. Unchanged keys retain their rows, IDs and list positions. NULL/error/empty
parents are checkpointed even when they produce no child rows. Each child key
remains the full cache key plus its collection-local `fact_ordinal`.
Materialize selected refresh facts before the five independent UNNEST expansions,
so a replay expands zero facts and a partial update expands only selected keys.

If the checkpoint or any child table is missing, do one cache-driven bootstrap
or repair of all five children, with no re-extraction of cached inputs. This also
upgrades databases written by the old full-rebuild SQL. Dropping the checkpoint
is the explicit rebuild mechanism; manually editing checkpoint rows is unsupported.
Schema-incompatible cache changes still require migration/fresh storage.

Required diagnostic TEMP relations expose the applied refresh and removed key
sets as `file_facts_child_refresh` and `file_facts_child_removed`. Both have the
three cache-key columns; duplicate keys are forbidden. A bootstrap refresh set is
C. On an ordinary replay, both sets are empty. Durable state is key-only.

Caller lineage always refreshes from the current caller relation independently
of these sets. Added duplicate/merge/root occurrences must appear even when
refresh is empty. Deleted cached keys must become `not_cached` in occurrences and
lose all derived children when not restaged. A deleted/retried key must replace
its prior children rather than append duplicates.

## Validation

Use the existing native DuckDB 2.0.0 runtime and immutable extension artifact
`e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`.
Independent SQL/native function tests remain unchanged. Add test-first pipeline
regressions covering no-op physical child-table reuse and zero deltas, reopen,
lineage-only changes, new/path/versioned keys, deletion without restaging,
deletion/retry without duplicate rows, all parent statuses, legacy bootstrap,
missing-child repair, empty staging and full typed/list-ordinal parity against a
simple full-UNNEST reference. Existing pipeline and typed Parquet checks must pass.

Catalog PRE evidence is retained at
`docs/research/artifacts/2026-11-15-duckdb-incremental-children/solve-pre-small.json`.
The implemented `data_integrity.unique` and `data_integrity.foreign_key` rules
accept the valid finite snapshot and reject duplicate children, orphan children
and a missing NULL-fact parent's checkpoint. Keys are injective labels for full
tuples in this small witness. This verifies supplied snapshots, not SQL transitions
or transaction behavior. Earlier invalid requests are retained separately. No
runtime quota, threshold, hash-all-facts policy or timing target is introduced.

After integration, the parent benchmarks the old and new SQL through Rust cells
on copies of the frozen 19,604-key cache. Record one-time bootstrap separately;
compare steady zero-miss and partial updates with exact cache/child/lineage/type
parity, independent caller-lineage changes and unchanged seven-file publication.
Use matched repeated observations, record shared-host load and phase time, and
report measured effects without assuming a fixed speedup.

## Scope

Implementation touches only `crates/spur-graph-duckdb` SQL/tests/README. No native
Rust, Cargo, source parser, Parquet partition protocol or generic input-provider
change. Parent research documents/artifacts/notebook are separate validation work.
