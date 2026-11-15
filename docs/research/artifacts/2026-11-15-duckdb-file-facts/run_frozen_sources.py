"""Actual DuckDB 2 queries launched by Rust notebook cells; no per-row SQL ingestion."""
import argparse
import hashlib
import json
import os
import resource
import sys
import time
from pathlib import Path

import duckdb

ROOT = Path('/Users/kevintruong/project/SpurCode')
ART = ROOT / 'docs/research/artifacts/2026-11-15-duckdb-file-facts'
WORKTREE = Path('/private/tmp/spur-file-facts-integration-20261115')
EXT = ART / 'integration/spur_graph.duckdb_extension'
OUT = ART / 'frozen-source'
COLLECTIONS = ['nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def run(con, sql):
    before = resource.getrusage(resource.RUSAGE_SELF)
    start = time.perf_counter()
    con.execute(sql).fetchall()
    elapsed = time.perf_counter() - start
    after = resource.getrusage(resource.RUSAGE_SELF)
    return {'wall_seconds': elapsed, 'user_seconds': after.ru_utime - before.ru_utime,
            'sys_seconds': after.ru_stime - before.ru_stime,
            'process_peak_rss_bytes': after.ru_maxrss,
            'note': 'macOS process lifetime peak RSS, not a query-only peak or a memory-limit guarantee'}


parser = argparse.ArgumentParser()
parser.add_argument('mode', choices=['pilot', 'extract', 'workflow'])
parser.add_argument('--threads', type=int, default=1)
parser.add_argument('--name', required=True)
parser.add_argument('--baseline')
parser.add_argument('--materialize-input', action='store_true')
parser.add_argument('--parallel-missing-source', action='store_true')
args = parser.parse_args()
assert duckdb.__version__ == '2.0.0'
folder = OUT / args.name
folder.mkdir(exist_ok=False)
os.chdir(folder)
extractor_sha = hashlib.sha256(EXT.read_bytes()).hexdigest()
result = {'mode': args.mode, 'name': args.name, 'threads': args.threads,
          'duckdb': duckdb.__version__, 'extractor_sha256': extractor_sha,
          'integration_commit': 'f0218c673', 'memory_limit': '4GB',
          'input': 'pilot.parquet' if args.mode == 'pilot' else 'sources.parquet'}
try:
    with duckdb.connect(str(folder / 'facts.duckdb'),
                        config={'allow_unsigned_extensions': True}) as con:
        con.execute('LOAD ' + quote(EXT))
        con.execute(f'SET threads = {args.threads}')
        con.execute("SET memory_limit = '4GB'")
        con.execute('CREATE VIEW inputs AS SELECT * FROM read_parquet(' + quote(OUT / result['input']) + ')')
        result['inputs'] = con.execute('SELECT count(*), sum(byte_length) FROM inputs').fetchall()[0]
        result['runtime'] = con.execute('SELECT version(), current_setting(\'threads\')').fetchall()[0]
        result['materialized_input'] = args.materialize_input
        if args.materialize_input:
            result['input_materialization'] = run(con, 'CREATE TEMP TABLE materialized_inputs AS SELECT * FROM inputs')
            con.execute('CREATE OR REPLACE VIEW inputs AS SELECT * FROM materialized_inputs')
        if args.mode in ('pilot', 'extract'):
            con.execute("PRAGMA enable_profiling='json'")
            con.execute('PRAGMA profiling_output=' + quote(folder / 'extraction-profile.json'))
            result['extraction'] = run(con, '''CREATE TABLE facts AS
                SELECT blob_oid, logical_path,
                       TRY(spur_graph_file_facts(logical_path, source)) facts FROM inputs''')
            con.execute('PRAGMA disable_profiling')
            result['errors'] = con.execute('SELECT count(*) FROM facts WHERE facts IS NULL').fetchall()[0][0]
            assert result['errors'] == 0
            result['fact_counts'] = dict(zip(COLLECTIONS, con.execute(
                'SELECT ' + ', '.join('sum(len(facts.' + c + '))' for c in COLLECTIONS) + ' FROM facts'
            ).fetchall()[0]))
            result['parquet_export'] = run(con, "COPY (SELECT * FROM facts ORDER BY blob_oid,logical_path) TO 'facts.parquet' (FORMAT PARQUET)")
            if args.mode == 'pilot':
                sys.path.insert(0, str(WORKTREE / 'crates/spur-graph-duckdb/tests'))
                os.environ['SPUR_FILE_FACTS_ORACLE'] = str(ART / 'integration/reference_file_facts')
                from test_file_facts import native_facts
                fixtures = con.execute('SELECT logical_path, source FROM inputs ORDER BY blob_oid,logical_path').fetchall()
                expected = native_facts(fixtures)
                actual = [r[0] for r in con.execute('SELECT facts FROM facts ORDER BY blob_oid,logical_path').fetchall()]
                assert actual == expected, 'frozen source pilot differs from independent native oracle'
                result['independent_native_exact_rows'] = len(expected)
            if args.baseline:
                con.execute('ATTACH ' + quote(OUT / args.baseline / 'facts.duckdb') + ' AS baseline (READ_ONLY)')
                differences = con.execute('''SELECT
                    (SELECT count(*) FROM ((SELECT * FROM facts) EXCEPT ALL (SELECT * FROM baseline.facts))),
                    (SELECT count(*) FROM ((SELECT * FROM baseline.facts) EXCEPT ALL (SELECT * FROM facts)))''').fetchall()[0]
                assert differences == (0, 0), differences
                result['exact_baseline_differences_both_directions'] = differences
        else:
            con.execute('CREATE TEMP TABLE staged_sources AS SELECT blob_oid,logical_path,source,?::VARCHAR extractor_sha FROM inputs', [extractor_sha])
            # Each occurrence here represents a source-plan entry, not historical commit ancestry.
            con.execute("CREATE TEMP TABLE source_lineage AS SELECT 'frozen-source-plan-entry' occurrence_kind, blob_oid,logical_path,extractor_sha FROM staged_sources")
            workflow = (WORKTREE / 'crates/spur-graph-duckdb/examples/file_facts.sql').read_text()
            result['parallel_missing_source_experiment'] = args.parallel_missing_source
            if args.parallel_missing_source:
                # Retain the physical missing-key boundary; feed the native projection
                # from the parallel Parquet scan, filtered by that materialized key set.
                # This is an experiment for this known input view, not a generic API change.
                needle = 'FROM file_facts_missing;'
                assert workflow.count(needle) == 1
                replacement = '''FROM (SELECT *, ''' + quote(extractor_sha) + '''::VARCHAR extractor_sha FROM inputs) s
                    SEMI JOIN file_facts_missing m USING (blob_oid,logical_path,extractor_sha);'''
                workflow = workflow.replace(needle, replacement)
                (folder / 'experimental-workflow.sql').write_text(workflow)
                result['experimental_sql_sha256'] = hashlib.sha256(workflow.encode()).hexdigest()
            result['workflow_cold'] = run(con, workflow)
            result['cold_missing'] = con.execute('SELECT count(*) FROM file_facts_missing').fetchall()[0][0]
            assert result['cold_missing'] == result['inputs'][0]
            result['statuses'] = con.execute('SELECT status,count(*) FROM file_facts_cache GROUP BY status ORDER BY status').fetchall()
            assert sum(n for _, n in result['statuses']) == result['inputs'][0]
            assert all(status in ('ok', 'empty_source', 'empty_facts') for status, _ in result['statuses'])
            result['fact_counts'] = {c: con.execute('SELECT count(*) FROM file_facts_' + c).fetchall()[0][0] for c in COLLECTIONS}
            result['workflow_cache_replay'] = run(con, workflow)
            result['replay_missing'] = con.execute('SELECT count(*) FROM file_facts_missing').fetchall()[0][0]
            result['replay_extracted'] = con.execute('SELECT count(*) FROM file_facts_extracted').fetchall()[0][0]
            assert result['replay_missing'] == result['replay_extracted'] == 0
            if args.baseline:
                con.execute('ATTACH ' + quote(OUT / args.baseline / 'facts.duckdb') + ' AS baseline (READ_ONLY)')
                differences = con.execute('''SELECT
                    (SELECT count(*) FROM ((SELECT blob_oid,logical_path,facts FROM file_facts_cache) EXCEPT ALL (SELECT * FROM baseline.facts))),
                    (SELECT count(*) FROM ((SELECT * FROM baseline.facts) EXCEPT ALL (SELECT blob_oid,logical_path,facts FROM file_facts_cache)))''').fetchall()[0]
                assert differences == (0, 0), differences
                result['exact_baseline_differences_both_directions'] = differences
    result['artifact_bytes'] = {p.name: p.stat().st_size for p in folder.iterdir() if p.is_file()}
    result['status'] = 'passed'
except BaseException as error:
    result['status'] = 'failed'
    result['error'] = repr(error)
    raise
finally:
    (folder / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
