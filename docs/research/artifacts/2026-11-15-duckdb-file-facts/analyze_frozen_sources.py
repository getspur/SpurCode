"""Aggregate measured notebook results and verify exported typed data without extension."""
import hashlib
import json
import statistics
from pathlib import Path

import duckdb

ROOT = Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-duckdb-file-facts')
OUT = ROOT / 'frozen-source'


def load(name):
    value = json.loads((OUT / name / 'result.json').read_text())
    assert value['status'] == 'passed'
    return value


runs = {n: load(n) for n in ['pilot-r0', 't1-r0', 't4-r0', 't4-r1', 't1-r1',
                            't4-table-r0', 't4-table-r1', 'workflow-t4', 'workflow-t4-r1',
                            'workflow-parquet-r0', 'workflow-parquet-r1']}
for name, r in runs.items():
    if name != 'pilot-r0':
        assert r['fact_counts'] == runs['t1-r0']['fact_counts']


def metrics(names, field):
    values = [runs[n][field] for n in names]
    return {'wall_seconds': [v['wall_seconds'] for v in values],
            'midpoint_seconds': statistics.median(v['wall_seconds'] for v in values),
            'cpu_seconds': [v['user_seconds'] + v['sys_seconds'] for v in values],
            'average_cpu_cores': [(v['user_seconds'] + v['sys_seconds']) / v['wall_seconds'] for v in values],
            'process_lifetime_peak_rss_bytes': [v['process_peak_rss_bytes'] for v in values]}


summary = {'inputs': json.loads((OUT / 'inputs.json').read_text()),
           'native_artifact': json.loads((ROOT / 'integration/artifact-manifest.json').read_text()),
           'fact_counts': runs['t1-r0']['fact_counts'], 'pilot_exact_native_rows': 58,
           'serial_parquet': metrics(['t1-r0', 't1-r1'], 'extraction'),
           'parallel_parquet': metrics(['t4-r0', 't4-r1'], 'extraction'),
           'four_threads_materialized_table': metrics(['t4-table-r0', 't4-table-r1'], 'extraction'),
           'generic_workflow_cold': metrics(['workflow-t4', 'workflow-t4-r1'], 'workflow_cold'),
           'generic_workflow_replay': metrics(['workflow-t4', 'workflow-t4-r1'], 'workflow_cache_replay'),
           'experimental_parquet_workflow_cold': metrics(['workflow-parquet-r0', 'workflow-parquet-r1'], 'workflow_cold'),
           'experimental_parquet_workflow_replay': metrics(['workflow-parquet-r0', 'workflow-parquet-r1'], 'workflow_cache_replay')}
summary['parallel_extraction_speedup'] = summary['serial_parquet']['midpoint_seconds'] / summary['parallel_parquet']['midpoint_seconds']
summary['experimental_workflow_speedup'] = summary['generic_workflow_cold']['midpoint_seconds'] / summary['experimental_parquet_workflow_cold']['midpoint_seconds']
summary['all_cache_replays_extracted_zero'] = all(runs[n]['replay_extracted'] == 0 for n in runs if n.startswith('workflow'))
summary['all_compared_facts_exact'] = all(r.get('exact_baseline_differences_both_directions', [0, 0]) == [0, 0] for r in runs.values())

# Check the physical scan partition explanation with the actual pinned engine.
with duckdb.connect() as con:
    con.execute('CREATE TEMP TABLE materialized AS SELECT * FROM read_parquet(?)', [str(OUT / 'sources.parquet')])
    cursor = con.execute("PRAGMA storage_info('materialized')")
    columns = [d[0] for d in cursor.description]
    info = [dict(zip(columns, row)) for row in cursor.fetchall()]
    summary['materialized_input_row_groups'] = sorted({r['row_group_id'] for r in info})

verified = []
for name in ['workflow-t4', 'workflow-t4-r1', 'workflow-parquet-r0', 'workflow-parquet-r1']:
    # No LOAD: exported data and stored standard DuckDB types need no extension.
    with duckdb.connect() as con:
        path = str(OUT / name / 'facts.duckdb').replace("'", "''")
        con.execute("ATTACH '" + path + "' AS saved (READ_ONLY)")
        for suffix in ['cache', 'occurrences', 'nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']:
            parquet = str(OUT / name / ('file-facts-' + suffix + '.parquet')).replace("'", "''")
            left = 'SELECT * FROM saved.file_facts_' + suffix
            right = "SELECT * FROM read_parquet('" + parquet + "')"
            difference = con.execute('SELECT (SELECT count(*) FROM ((' + left + ') EXCEPT ALL (' + right + '))),'
                                     '(SELECT count(*) FROM ((' + right + ') EXCEPT ALL (' + left + ')))').fetchall()[0]
            assert difference == (0, 0), (name, suffix, difference)
            ltypes = con.execute('DESCRIBE ' + left).fetchall()
            rtypes = con.execute('DESCRIBE ' + right).fetchall()
            assert [(r[0], r[1]) for r in ltypes] == [(r[0], r[1]) for r in rtypes]
            verified.append({'run': name, 'table': suffix, 'typed_exact_differences': difference})
summary['parquet_roundtrips_without_extension'] = verified

scans = {}
for name in ['t1-r0', 't4-r0', 't4-table-r0']:
    profile = json.loads((OUT / name / 'extraction-profile.json').read_text())
    records = []
    def visit(node):
        if 'SCAN' in node.get('operator_name', node.get('type', '')):
            records.append({k: node.get(k) for k in ['type', 'operator_name', 'intermediate_rows', 'rows_scanned', 'row_groups_scanned', 'total_row_groups_to_scan', 'timing', 'extra_info']})
        for child in node.get('children', []):
            visit(child)
    for root in profile.get('operator', [profile]):
        visit(root)
    scans[name] = records
summary['profile_scans'] = scans
summary['experiment_scripts'] = {n: hashlib.sha256((ROOT / n).read_bytes()).hexdigest()
                                 for n in ['prepare_frozen_sources.py', 'run_frozen_sources.py', 'analyze_frozen_sources.py']}
summary['limits'] = [
    'Bounded frozen source plan, not full temporal history or whole-repository resolution.',
    'Cold refers to a fresh fact cache; OS source pages are warm. Source preparation/build/oracle/exact comparisons are outside query timings.',
    'Each timing uses the same typed full-facts native kernel; no native-versus-DuckDB engine superiority claim.',
    'Two observations per comparison, ordered 1/4/4/1 for direct extraction; generic/candidate/candidate/generic for workflow.',
    'Four-thread extraction uses more aggregate CPU and higher process lifetime peak RSS than one thread.',
    'Experimental Parquet SEMI JOIN variant is specific to the known input view; the generic SQL example is unchanged.',
    'All cached versions and child tables are rebuilt/exported by the generic example; production admission/backpressure and atomic file publication remain separate.'
]
(OUT / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
manifest = []
for p in sorted(OUT.rglob('*')):
    if p.is_file() and p.name != 'manifest-sha256.json':
        data = p.read_bytes()
        manifest.append({'path': str(p.relative_to(OUT)), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
(OUT / 'manifest-sha256.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({k: summary[k] for k in ['parallel_extraction_speedup', 'experimental_workflow_speedup', 'materialized_input_row_groups', 'all_cache_replays_extracted_zero', 'all_compared_facts_exact']}))
print('Typed Parquet round trips without extension:', len(verified))
