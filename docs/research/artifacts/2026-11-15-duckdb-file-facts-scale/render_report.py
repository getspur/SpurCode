"""Render the measured, versioned research record from completed results."""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
s = json.loads((OUT / 'summary.json').read_text())
inputs = s['inputs']
facts = s['fact_counts']


def direct_row(t):
    value = s['direct'][str(t)]
    return f"| {t} | {value['n']} | {value['median_seconds']:.3f} | {value['min_seconds']:.3f}–{value['max_seconds']:.3f} |"


def workflow_row(v):
    cold = s['workflow'][v]['cold']
    warm = s['workflow'][v]['same_connection_replay']
    restart = s['workflow'][v]['restart_replay']
    return f"| {v} | {cold['n']} | {cold['median_seconds']:.3f} ({cold['min_seconds']:.3f}–{cold['max_seconds']:.3f}) | {warm['median_seconds']:.3f} | {restart['median_seconds']:.3f} |"


incremental_rows = []
for name, value in sorted(s['incremental'].items()):
    incremental_rows.append(f"| {value['percent']}% | {value['variant']} | {value['missing']} | {value['timing']['wall_seconds']:.3f} | {value['restart_replay']['timing']['wall_seconds']:.3f} |")
phase_rows = '\n'.join(f"| {phase} | {seconds:.3f} |" for phase, seconds in s['cached_replay_trace'].items())
trace_total = sum(s['cached_replay_trace'].values())
export_share = 100 * s['cached_replay_trace'].get('parquet_export', 0) / trace_total
children_share = 100 * s['cached_replay_trace'].get('child_tables', 0) / trace_total

report = f'''# Larger DuckDB file-facts notebook benchmark

The integrated V2 function successfully processed the larger frozen source plan
through actual Rust notebook executions. Multi-threading and cache reuse work;
cold extraction remains substantial. The observed one-to-four-thread median
ratio is **{s['direct_speedup_1_to_4']:.2f}×**. Full-workflow timings vary widely
on this busy development host, so they are observations rather than an isolated
capacity guarantee or proof that DuckDB outperforms an equally batched Rust pipeline.

## Version and workload

Main snapshot: `{s['protocol']['main_head']}`. Runtime code matches native build
`{s['protocol']['artifact_build_commit']}`; only README differs in those crate trees.
The immutable loaded extension SHA-256 is
`{s['protocol']['extension_sha256']}`. No production code was changed for these experiments.

Rust cells 046 onward in `duckdb-temporal-performance.ipynb` launch the pinned
native Python DuckDB **2.0.0** runtime. The notebook's managed SQL runtime remains
separate. Source staging uses one persistent Git batch process and typed Parquet,
with no per-row SQL insertion. Source preparation took **{inputs['preparation_seconds']:.3f}s**.

The plan covers **{inputs['rows']:,} blob/path entries, {inputs['source_bytes']:,} bytes**
from the frozen 1,987-commit DuckDB prefix at `{inputs['frozen_head']}`.
The source Parquet file has **{inputs['row_groups']} row groups** of at most 128 rows.
This is a historical-source plan with one selected logical path per old blob/language
entry, not every historical path occurrence or full temporal reconstruction.

One historical `.hpp` fixture contains invalid UTF-8. Every key is retained;
**{inputs['valid_utf8_rows']:,} valid UTF-8 inputs ({inputs['valid_utf8_bytes']:,} bytes)**
can be supplied to the VARCHAR API. The rejected source has a NULL source value,
explicit `source_status=encoding_error`, an error message and its original bytes
in a sparse BLOB column. Its raw bytes were read back from Parquet and verified
against its Git blob ID. Caller lineage retains the encoding diagnostic; the
generic cache labels this NULL input `null_input`. No lossy decoding was used.

The expanded pilot independently matched all five native collections for
**{s['native_pilot_exact_rows']} valid text inputs**, plus checked the rejected input's
NULL behavior. Full runs produce **{facts['nodes']:,} nodes, {facts['edges']:,} edges and
{facts['spans']:,} spans**, with zero native extraction errors for valid inputs.
The one NULL fact result belongs to the encoding-rejected input. This DuckDB
corpus has no Rust macro facts; macro fields retain typed empty lists, and their
nonempty behavior was covered by the prior native/SQL contract.

## Controlled direct extraction

Each thread count has three observations. The balanced order was
`1/2/4, 4/1/2, 2/4/1`. Every run used the same input, native artifact, typed full-facts
kernel and output fields, with a fresh database/fact cache and warm OS pages.
Query timing excludes preparation, Python startup, export, oracle and parity checks.

| Threads | Observations | Median seconds | Observed range |
|---|---:|---:|---:|
{direct_row(1)}
{direct_row(2)}
{direct_row(4)}

These are shared-host measurements. Load snapshots and later per-run load averages
were recorded; substantial contention and heterogeneous core scheduling can affect
wall time and CPU throughput. Ranges are min/max observations, not confidence bounds.
Process RSS is a lifetime peak, including work earlier in that process, and must
not be interpreted as an isolated phase allocation measurement. The 4 GB DuckDB
setting does not bound native Rust AST/fact allocations.

## Full workflow and durable cache

The generic workflow is the committed example. The Parquet variant preserves
the materialized missing-key boundary but semi-joins it into the Parquet-backed
staged source view before calling the scalar. The whole workload includes cache
population, five separate child tables, caller occurrences and seven Parquet exports.
Variants ran in `generic/parquet/parquet/generic` order, with two observations each.

| Workflow | Observations | Cold midpoint seconds (range) | Same-connection replay midpoint | Reopened-cache replay midpoint |
|---|---:|---:|---:|---:|
{workflow_row('generic')}
{workflow_row('parquet')}

The observed midpoint ratio is **{s['workflow_speedup']:.2f}×**, with wide variation;
the full ranges are more informative than a single favorable pair. Both replay
forms materialize **zero missing/extracted rows**, including after database reopen.
The experimental variant is specific to the known Parquet input view and does not
change the generic example's arbitrary-relation interface.

## Incremental probes

These are single diagnostic observations per variant and miss fraction, not
repeated performance estimates. Deterministic hash buckets evict cache keys from
a copied complete cache; input content and caller lineage stay unchanged. The
observed key counts can differ from exactly 1%/10%. Every refresh reconstructs the
same full typed cache, child tables and exports, then survives a zero-miss restart.

| Requested bucket fraction | Variant | Missing-key results | Refresh seconds | Reopened replay seconds |
|---|---|---:|---:|---:|
{chr(10).join(incremental_rows)}

This confirms that cache misses control native input work. At small miss counts,
rebuilding/exporting the complete cached dataset leaves a substantial fixed cost.
`extracted` counts materialized missing-key results; it is not an exactly-once
parser-call guarantee under DuckDB TRY retries.

## Where cached replay spends time

One additional complete-cache diagnostic executes the real SQL statement by
statement, with zero extraction, then validates the same typed outputs. It is
excluded from the cold timing medians; Python dispatch adds some diagnostic overhead.

| Phase | Accumulated seconds |
|---|---:|
{phase_rows}

Child-table rebuilds account for **{children_share:.1f}%**, and Parquet export for
**{export_share:.1f}%**, of that diagnostic's accumulated statement time. These are
concrete next targets: incrementally maintain unchanged
fact/child tables and publish only changed fact partitions, while refreshing caller
lineage independently. A zero fact-cache miss count does not imply unchanged
commit/parent occurrence lineage, so skipping the whole workflow would lose data.
No such production optimization is implemented by this benchmark.

## Profiles, correctness and limits

The first serial operator profile attributes 0.246s to Parquet scan and 130.395s
elapsed worker time to the scalar projection, with zero temporary-file spill.
Projection includes native extraction and typed result construction; it does not
isolate parsing from conversion. Operator elapsed times are distinct from OS
`getrusage` CPU time. A separate cold four-thread sample was captured successfully
and excluded from timing medians. The release binary is stripped and `atos` did
not recover individual Rust names, so no per-function hotspot is claimed.

Every full/partial result matched the reference in both directions of typed
`EXCEPT ALL`, including list order and NULLs. **{s['typed_parquet_checks']} cache/child/lineage
Parquet round trips preserved values and DuckDB types in connections without
loading the extension. Every same-connection and reopened-cache replay processed
zero misses. All source bytes, input keys and artifacts retain version/hash lineage.

There were two operational observations: the first preparation attempt correctly
failed strict UTF-8 decoding before diagnostics were added; its logs remain under
`attempts/`. One notebook MCP call timed out at 300s while its cell subsequently
finished successfully; saved process results and cell readback recovered completion.
Claims here rely on child exits, saved result status and parity checks, rather than
transport status alone. No user processes were stopped to quiet the shared host.

Evidence: [summary](artifacts/2026-11-15-duckdb-file-facts-scale/summary.json),
[protocol](artifacts/2026-11-15-duckdb-file-facts-scale/protocol.json),
[runner](artifacts/2026-11-15-duckdb-file-facts-scale/benchmark.py),
[inputs](artifacts/2026-11-15-duckdb-file-facts-scale/inputs.json),
[cached trace](artifacts/2026-11-15-duckdb-file-facts-scale/cached-trace/result.json),
[profile assessment](artifacts/2026-11-15-duckdb-file-facts-scale/profile-control/assessment.json),
[artifact hashes](artifacts/2026-11-15-duckdb-file-facts-scale/manifest-sha256.json).
Large databases, source/fact Parquet and raw profile output remain local with hashes.
Reproduction requires the persisted frozen source plan/repository and pinned native
runtime/artifact named in the protocol, plus fresh output run names.
'''
path = ROOT / 'docs/research/2026-11-15-duckdb-file-facts-scale.md'
path.write_text(report)
print(json.dumps({'report': str(path), 'direct_speedup': s['direct_speedup_1_to_4'],
                  'workflow_midpoint_ratio': s['workflow_speedup'],
                  'cached_export_share_percent': export_share,
                  'typed_parquet_checks': s['typed_parquet_checks']}))
