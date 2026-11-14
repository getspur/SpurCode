# Bounded temporal finalization

`bounded_temporal_finalize.py` finalizes existing immutable output from
`derive_temporal_pairs.py`. It loads no native extension and performs no parsing
or matching. The source generation, its failed status, and its pair shards remain
unchanged. Python 3.11+ and DuckDB are required; the fixture and full experiment
used DuckDB 2.0.0.

## Configuration and admission basis

Use absolute paths in a JSON configuration:

```json
{
  "source_generation": "/data/experiment/temporal-derived/gen-SOURCE_HASH",
  "metadata": {
    "commits": "/data/experiment/commits.parquet",
    "comparisons": "/data/experiment/comparisons.parquet",
    "changes": "/data/experiment/changes.parquet",
    "parse_tasks": "/data/experiment/parse_tasks.parquet",
    "symbol_pairs": "/data/experiment/symbol_pairs.parquet",
    "change_pairs": "/data/experiment/change_pairs.parquet"
  },
  "comparison_counts": "/data/experiment/complete-comparison-counts.parquet",
  "output": "/data/experiment/bounded-finalization",
  "memory_limit": "384MB",
  "threads": 1,
  "max_partition_candidates": 160435,
  "max_output_bytes": 2684354560,
  "min_free_bytes": 536870912
}
```

These limits reproduce one measured experiment; they are not portable defaults.
Its 496,163 pairs emitted 29,314,796 unique-pair changes, expanding to 158,721,052
occurrence candidates. The measured maximum for an entire commit, including all
parents, was 160,435 candidates. Batches contain complete contiguous commit
ordinal ranges and cannot split an oversized commit to evade this ceiling.
`memory_limit` limits DuckDB managed memory, not total process RSS; the two full
run attempts peaked at approximately 555MB and 594MB RSS. One DuckDB thread and
`temp_directory=''` disable a spill-based retry. The 2.5GiB output cap and 512MiB
free-space reserve were chosen from measured available storage. Measured output
and free space are checked before and after output writes and before publishing
completion; a failed check retains artifacts and records failure.

The source must have a hash-bound `gen-*` directory, `inputs.json`,
`current-status.json`, `.lock`, and every expected `receipts/NNNNNN.json` and pair
Parquet shard. The helper verifies source receipts, checksums, actual pair IDs,
and the original row/byte budgets. The six metadata exports must have exactly
the original schemas and ordered typed rows of the semantic metadata DuckDB
file bound by `source_generation/inputs.json`; this is verified with bounded
2048-row cursor batches rather than another metadata copy.

The comparison-count Parquet must contain `comparison_id BIGINT` and
`raw_candidates BIGINT`: exactly one nonnegative, non-NULL count for **every**
metadata comparison, including zero-work comparisons. Each count is the sum of
completed pair emission lengths across that comparison's supported occurrences.
The full experiment's complete basis had 117,658 rows and a candidate sum of
158,721,052; its SHA-256 was
`0c79d9a1f024c1897c770307875877d21535aa404051333d1f4c727c51f61b05`.
The helper checks exact comparison-ID coverage while planning and reconciles
each batch against actual emissions before folding. Keep the count-generation
script and its provenance receipt alongside this resource basis.

## Plan, execute, and restart

From the repository root, select a Python environment with DuckDB installed:

```sh
PYTHON=/path/to/venv/bin/python
CONFIG=/absolute/path/configuration.json
PROGRAM=crates/spur-graph-duckdb/examples/bounded_temporal_finalize.py
"$PYTHON" "$PROGRAM" --configuration "$CONFIG" > /path/to/reviewed-plan.json
"$PYTHON" "$PROGRAM" --configuration "$CONFIG" --execute
# Resume using the same controller, configuration, and immutable inputs:
"$PYTHON" "$PROGRAM" --configuration "$CONFIG" --execute
```

The first command only reads inputs and prints the plan. Review its hashes,
commit ranges, candidate ceiling, and storage policy before execution. Execution
uses a shared source lock and an exclusive output lock. Re-running verifies
terminal receipts and data before reuse; a verified completed run does zero new
thin projections and folds. Corrupt completed checkpoints fail closed.
Unreceipted interrupted files move into a recorded `orphans/` directory before
only that unit is recomputed; no verified checkpoint is overwritten or deleted.

The generation binds controller/configuration descriptors, DuckDB version,
original semantic inputs, metadata equivalence, source receipts/shards, resource
basis, and policies. Absolute descriptor paths participate in this hash. Moving
the controller or inputs therefore creates a different generation even when
their bytes match. Resume a frozen experiment with its original controller and
paths; promotion does not adopt or rewrite its existing generation.

## Output contract

`output/gen-HASH/` contains immutable `inputs.json`, `plan.json`, per-unit
`receipts/`, compact ZSTD Parquet, and `temporal.duckdb` with views over metadata,
source payloads, and folded pointers. All referenced input files must remain
available at their bound paths. The plan's `batches` contain `partition_id`,
`ordinal_start`, `ordinal_end`, `candidate_rows`, and `commit_count`.

| File | Typed columns |
| --- | --- |
| `thin/emissions-NNNNNN.parquet` | `pair_id BIGINT`, `emitted_ordinal BIGINT`, `stable_id UBIGINT`, `change_kind VARCHAR`, nullable `previous_id UBIGINT` |
| `thin/pair-state-NNNNNN.parquet` | `pair_id BIGINT`, `status VARCHAR`, nullable `raw_change_count BIGINT`, `emitted_count BIGINT`, `diagnostic_count BIGINT` |
| `thin/occurrences.parquet` | `commit_ordinal UINTEGER`, nullable `parent_ordinal UINTEGER`, `pair_id BIGINT`, `change_id BIGINT` |
| `folds/part-NNNNNN.parquet` | `commit_ordinal UINTEGER`, nullable `parent_ordinal UINTEGER`, `stable_id UBIGINT`, `pair_id BIGINT`, `emitted_ordinal BIGINT`, `change_id BIGINT`, `change_kind VARCHAR`, nullable `previous_id UBIGINT` |

Stable IDs must round-trip as 16 lowercase hexadecimal digits. Folding groups
by `(commit, parent, stable_id)`, retains the first pointer ordered by
`(change_id, emitted_ordinal)`, and turns conflicting `(kind, previous_id)`
variants into `modified` with a NULL previous ID. Parent-specific duplicate
snapshots remain present. Payload/token lists stay outside grouped state;
`symbol_snapshots` retrieves the chosen native payload with `list_extract`.
`temporal_edges` uses thin folded pointers for symbol and rename edges.

`result.json` is published only on completion and includes native-compatible
snapshot/edge counts, diagnostic/status counts, candidate/fold reconciliation,
source generation, policies, and zero extraction/matching call counts.
Generation and output-root `current-status.json` describe the latest attempt;
after a failed retry they are authoritative even if an earlier immutable
`result.json` remains. stdout emits checkpoint progress and the final result
with resource measurements. Independent full-target validation is a separate
step; helper completion alone does not establish full native parity.
