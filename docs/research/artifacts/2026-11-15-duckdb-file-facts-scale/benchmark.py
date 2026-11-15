"""Frozen typed file-facts experiments, launched through Rust notebook cells."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import resource
import re
import shutil
import statistics
import subprocess
import sys
import time

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
PROTOCOL = json.loads((OUT / 'protocol.json').read_text())
COLLECTIONS = ['nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']
KEY = 'blob_oid,logical_path,extractor_sha'


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + '\n')


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def prepare():
    import pyarrow as pa
    import pyarrow.parquet as pq
    start = time.perf_counter()
    raw = Path(PROTOCOL['plan']).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == PROTOCOL['plan_sha256']
    (OUT / 'input-plan.json').write_bytes(raw)
    plan = json.loads(raw)
    tasks = sorted((t for b in plan['batches'] for t in b['tasks']),
                   key=lambda t: (t['blob_oid'], t['source_path']))
    assert len({(t['blob_oid'], t['source_path']) for t in tasks}) == len(tasks)
    rows = []
    encoding_errors = []
    with subprocess.Popen(['git', '--git-dir', PROTOCOL['repository'], 'cat-file', '--batch'],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
        for task in tasks:
            proc.stdin.write((task['blob_oid'] + '\n').encode('ascii'))
            proc.stdin.flush()
            oid, kind, length = proc.stdout.readline().decode('ascii').strip().split()
            assert oid == task['blob_oid'] and kind == 'blob'
            assert int(length) == task['byte_length']
            data = proc.stdout.read(int(length))
            assert len(data) == int(length) and proc.stdout.read(1) == b'\n'
            assert hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == oid
            try:
                source = data.decode('utf-8')
                status, error, raw_bytes = 'ok', None, None
            except UnicodeDecodeError as decode_error:
                source, status, error, raw_bytes = None, 'encoding_error', str(decode_error), data
                encoding_errors.append({'blob_oid': oid, 'logical_path': task['source_path'],
                                        'byte_length': len(data), 'error': error,
                                        'raw_sha256': hashlib.sha256(data).hexdigest()})
            rows.append({'blob_oid': oid, 'logical_path': task['source_path'],
                         'source': source, 'language': task['language'], 'byte_length': len(data),
                         'source_status': status, 'source_error': error, 'raw_invalid_bytes': raw_bytes})
        proc.stdin.close()
        assert proc.wait() == 0
    schema = pa.schema([('blob_oid', pa.string()), ('logical_path', pa.string()),
                        ('source', pa.string()), ('language', pa.string()), ('byte_length', pa.uint64()),
                        ('source_status', pa.string()), ('source_error', pa.string()),
                        ('raw_invalid_bytes', pa.binary())])
    pq.write_table(pa.Table.from_pylist(rows, schema=schema), OUT / 'sources.parquet',
                   row_group_size=PROTOCOL['row_group_size'], compression='zstd')
    pilot_keys = set()
    for language in sorted({r['language'] for r in rows}):
        choices = sorted((r for r in rows if r['language'] == language and r['source'] is not None),
                         key=lambda r: (r['byte_length'], r['blob_oid'], r['logical_path']))
        if not choices:
            continue
        for quantile in range(4):
            row = choices[quantile * (len(choices) - 1) // 3]
            pilot_keys.add((row['blob_oid'], row['logical_path']))
    pilot = [r for r in rows if (r['blob_oid'], r['logical_path']) in pilot_keys or r['source'] is None]
    pq.write_table(pa.Table.from_pylist(pilot, schema=schema), OUT / 'pilot.parquet',
                   row_group_size=1, compression='zstd')
    result = {'rows': len(rows), 'source_bytes': sum(r['byte_length'] for r in rows),
              'frozen_head': plan['head'], 'plan_sha256': PROTOCOL['plan_sha256'],
              'languages': dict(Counter(r['language'] for r in rows)),
              'row_group_size': PROTOCOL['row_group_size'],
              'row_groups': pq.ParquetFile(OUT / 'sources.parquet').num_row_groups,
              'max_source_bytes': max(r['byte_length'] for r in rows),
              'pilot_rows': len(pilot), 'pilot_bytes': sum(r['byte_length'] for r in pilot),
              'valid_utf8_rows': sum(r['source'] is not None for r in rows),
              'valid_utf8_bytes': sum(r['byte_length'] for r in rows if r['source'] is not None),
              'encoding_errors': encoding_errors,
              'preparation_seconds': time.perf_counter() - start,
              'sha256': {n: digest(OUT / n) for n in ['sources.parquet', 'pilot.parquet']}}
    save(OUT / 'inputs.json', result)
    print(json.dumps(result))


def measured(con, sql):
    before = resource.getrusage(resource.RUSAGE_SELF)
    start = time.perf_counter()
    con.execute(sql).fetchall()
    wall = time.perf_counter() - start
    after = resource.getrusage(resource.RUSAGE_SELF)
    user, system = after.ru_utime - before.ru_utime, after.ru_stime - before.ru_stime
    return {'wall_seconds': wall, 'user_seconds': user, 'sys_seconds': system,
            'average_cpu_cores': (user + system) / wall,
            'process_lifetime_peak_rss_bytes': after.ru_maxrss,
            'minor_faults': after.ru_minflt - before.ru_minflt,
            'major_faults': after.ru_majflt - before.ru_majflt,
            'voluntary_context_switches': after.ru_nvcsw - before.ru_nvcsw,
            'involuntary_context_switches': after.ru_nivcsw - before.ru_nivcsw}


def connect(database, threads, input_name='sources.parquet', load_extension=True):
    import duckdb
    assert duckdb.__version__ == '2.0.0'
    con = duckdb.connect(str(database), config={'allow_unsigned_extensions': True})
    con.execute(f'SET threads = {threads}')
    con.execute('SET memory_limit = ' + quote(PROTOCOL['memory_limit']))
    if load_extension:
        assert digest(PROTOCOL['extension']) == PROTOCOL['extension_sha256']
        con.execute('LOAD ' + quote(PROTOCOL['extension']))
    con.execute('CREATE TEMP VIEW inputs AS SELECT * FROM read_parquet(' + quote(OUT / input_name) + ')')
    return con


def counts(con, table):
    return dict(zip(COLLECTIONS, con.execute('SELECT ' + ','.join(
        'coalesce(sum(len(facts.' + c + ')),0)' for c in COLLECTIONS) + ' FROM ' + table).fetchall()[0]))


def exact(con, current, baseline):
    con.execute('ATTACH ' + quote(OUT / baseline / 'facts.duckdb') + ' AS baseline (READ_ONLY)')
    reference = 'SELECT * FROM baseline.facts'
    result = con.execute('SELECT (SELECT count(*) FROM ((' + current + ') EXCEPT ALL (' + reference + '))),'
                         '(SELECT count(*) FROM ((' + reference + ') EXCEPT ALL (' + current + ')))').fetchall()[0]
    con.execute('DETACH baseline')
    assert result == (0, 0), result
    return result


def staged(con):
    con.execute('CREATE TEMP VIEW staged_sources AS SELECT blob_oid,logical_path,source,'
                + quote(PROTOCOL['extension_sha256']) + '::VARCHAR extractor_sha FROM inputs')
    con.execute("CREATE TEMP VIEW source_lineage AS SELECT 'historical-source-plan-entry' occurrence_kind, "
                'blob_oid,logical_path,' + quote(PROTOCOL['extension_sha256']) + '::VARCHAR extractor_sha,'
                'source_status,source_error,byte_length FROM inputs')


def sql_workflow(variant):
    sql = (ROOT / 'crates/spur-graph-duckdb/examples/file_facts.sql').read_text()
    if variant == 'parquet':
        needle = 'FROM file_facts_missing;'
        assert sql.count(needle) == 1
        replacement = 'FROM staged_sources s SEMI JOIN file_facts_missing m USING (' + KEY + ');'
        sql = sql.replace(needle, replacement)
    return sql


def workflow_observation(con, sql):
    timing = measured(con, sql)
    missing, extracted = [con.execute('SELECT count(*) FROM file_facts_' + table).fetchall()[0][0]
                          for table in ['missing', 'extracted']]
    assert missing == extracted
    return {'timing': timing, 'missing': missing, 'extracted': extracted}


def verify_parquet(database, folder):
    import duckdb
    results = []
    with duckdb.connect() as con:
        con.execute('SET memory_limit = ' + quote(PROTOCOL['memory_limit']))
        con.execute('ATTACH ' + quote(database) + ' AS saved (READ_ONLY)')
        for suffix in ['cache', 'occurrences', *COLLECTIONS]:
            left = 'SELECT * FROM saved.file_facts_' + suffix
            right = 'SELECT * FROM read_parquet(' + quote(folder / ('file-facts-' + suffix + '.parquet')) + ')'
            diffs = con.execute('SELECT (SELECT count(*) FROM ((' + left + ') EXCEPT ALL (' + right + '))),'
                                '(SELECT count(*) FROM ((' + right + ') EXCEPT ALL (' + left + ')))').fetchall()[0]
            assert diffs == (0, 0), (suffix, diffs)
            ltypes = [(r[0], r[1]) for r in con.execute('DESCRIBE ' + left).fetchall()]
            rtypes = [(r[0], r[1]) for r in con.execute('DESCRIBE ' + right).fetchall()]
            assert ltypes == rtypes, suffix
            results.append({'table': suffix, 'differences': diffs, 'types_equal': True})
    return results


def experiment(args):
    folder = OUT / args.name
    folder.mkdir(exist_ok=False)
    os.chdir(folder)
    database = folder / 'facts.duckdb'
    if args.mode == 'incremental':
        shutil.copy2(OUT / args.seed / 'facts.duckdb', database)
    result = {'name': args.name, 'mode': args.mode, 'threads': args.threads,
              'variant': args.variant, 'extractor_sha256': PROTOCOL['extension_sha256'],
              'artifact_build_commit': PROTOCOL['artifact_build_commit'],
              'main_code_snapshot': PROTOCOL['main_head'], 'memory_limit': PROTOCOL['memory_limit']}
    result['logical_cpus'] = os.cpu_count()
    result['system_load_start'] = list(os.getloadavg())
    input_name = 'pilot.parquet' if args.mode == 'pilot' else 'sources.parquet'
    con = None
    try:
        con = connect(database, args.threads, input_name)
        result['inputs'] = con.execute('SELECT count(*),sum(byte_length) FROM inputs').fetchall()[0]
        result['input_statuses'] = con.execute('SELECT source_status,count(*) FROM inputs GROUP BY source_status ORDER BY source_status').fetchall()
        if args.mode in ('pilot', 'extract', 'profile'):
            con.execute("PRAGMA enable_profiling='json'")
            con.execute('PRAGMA profiling_output=' + quote(folder / 'extraction-profile.json'))
            result['extraction'] = measured(con, '''CREATE TABLE facts AS
                SELECT blob_oid,logical_path,TRY(spur_graph_file_facts(logical_path,source)) facts FROM inputs''')
            con.execute('PRAGMA disable_profiling')
            result['errors'] = con.execute('SELECT count(*) FROM facts WHERE facts IS NULL').fetchall()[0][0]
            result['native_extraction_errors'] = con.execute('''SELECT count(*) FROM facts f JOIN inputs i
                USING(blob_oid,logical_path) WHERE f.facts IS NULL AND i.source_status='ok' ''').fetchall()[0][0]
            rejected = con.execute('''SELECT count(*) FROM facts f JOIN inputs i USING(blob_oid,logical_path)
                WHERE i.source_status='encoding_error' AND f.facts IS NOT NULL''').fetchall()[0][0]
            assert rejected == 0
            result['fact_counts'] = counts(con, 'facts')
            result['export'] = measured(con, "COPY (SELECT * FROM facts ORDER BY blob_oid,logical_path) TO 'facts.parquet' (FORMAT PARQUET)")
            if args.mode == 'pilot':
                assert result['native_extraction_errors'] == 0
                sys.path.insert(0, str(ROOT / 'crates/spur-graph-duckdb/tests'))
                assert digest(PROTOCOL['oracle']) == PROTOCOL['oracle_sha256']
                os.environ['SPUR_FILE_FACTS_ORACLE'] = PROTOCOL['oracle']
                from test_file_facts import native_facts
                fixtures = con.execute('SELECT logical_path,source FROM inputs WHERE source IS NOT NULL ORDER BY blob_oid,logical_path').fetchall()
                expected = native_facts(fixtures)
                actual = [r[0] for r in con.execute('''SELECT f.facts FROM facts f JOIN inputs i USING(blob_oid,logical_path)
                    WHERE i.source IS NOT NULL ORDER BY blob_oid,logical_path''').fetchall()]
                assert expected == actual
                result['independent_native_exact_rows'] = len(expected)
            elif args.baseline:
                result['exact_differences'] = exact(con, 'SELECT * FROM facts', args.baseline)
        else:
            staged(con)
            expected_missing = result['inputs'][0]
            if args.mode == 'incremental':
                result['eviction'] = measured(con, 'DELETE FROM file_facts_cache WHERE '
                    '(blob_oid,logical_path,extractor_sha) IN (SELECT ' + KEY + ' FROM staged_sources '
                    'WHERE hash(blob_oid,logical_path) % 100 < ' + str(args.miss_percent) + ')')
                remaining = con.execute('SELECT count(*) FROM file_facts_cache').fetchall()[0][0]
                expected_missing -= remaining
                result['simulated_miss_percent'] = args.miss_percent
            sql = sql_workflow(args.variant)
            (folder / 'workflow.sql').write_text(sql)
            result['workflow_sql_sha256'] = hashlib.sha256(sql.encode()).hexdigest()
            result['cold' if args.mode == 'workflow' else 'incremental'] = workflow_observation(con, sql)
            observation = result.get('cold', result.get('incremental'))
            assert observation['missing'] == expected_missing
            save(folder / 'progress.json', {'phase': 'ingestion_complete', 'observation': observation})
            result['statuses'] = con.execute('SELECT status,count(*) FROM file_facts_cache GROUP BY status ORDER BY status').fetchall()
            assert sum(n for _, n in result['statuses']) == result['inputs'][0]
            result['fact_counts'] = counts(con, 'file_facts_cache')
            result['child_counts'] = {c: con.execute('SELECT count(*) FROM file_facts_' + c).fetchall()[0][0] for c in COLLECTIONS}
            assert result['child_counts'] == result['fact_counts']
            result['occurrences'] = con.execute('SELECT count(*) FROM file_facts_occurrences').fetchall()[0][0]
            assert result['occurrences'] == result['inputs'][0]
            result['encoding_diagnostic_occurrences'] = con.execute('''SELECT count(*) FROM file_facts_occurrences
                WHERE source_status='encoding_error' AND source_error IS NOT NULL
                  AND extraction_status='null_input' ''').fetchall()[0][0]
            expected_rejections = dict(result['input_statuses']).get('encoding_error', 0)
            assert result['encoding_diagnostic_occurrences'] == expected_rejections
            result['same_connection_replay'] = workflow_observation(con, sql)
            assert result['same_connection_replay']['extracted'] == 0
            save(folder / 'progress.json', {'phase': 'replay_complete', 'replay': result['same_connection_replay']})
            if args.baseline:
                result['exact_differences'] = exact(con, 'SELECT blob_oid,logical_path,facts FROM file_facts_cache', args.baseline)
            con.close()
            con = connect(database, args.threads)
            staged(con)
            result['restart_replay'] = workflow_observation(con, sql)
            assert result['restart_replay']['extracted'] == 0
            save(folder / 'progress.json', {'phase': 'restart_complete', 'replay': result['restart_replay']})
        con.close()
        con = None
        if args.mode in ('workflow', 'incremental'):
            result['typed_parquet_without_extension'] = verify_parquet(database, folder)
        result['status'] = 'passed'
    except BaseException as error:
        result['status'] = 'failed'
        result['error'] = repr(error)
        raise
    finally:
        if con is not None:
            con.close()
        result['system_load_end'] = list(os.getloadavg())
        save(folder / 'result.json', result)
        print(json.dumps(result))


def summarize():
    runs = {}
    for path in OUT.glob('*/result.json'):
        value = json.loads(path.read_text())
        if 'mode' in value:
            runs[path.parent.name] = value
    assert runs and all(r['status'] == 'passed' for r in runs.values())
    reference = runs['d-t1-r0']
    assert all(r['inputs'] == reference['inputs'] and r['fact_counts'] == reference['fact_counts']
               for n, r in runs.items() if n != 'pilot')
    assert all(r.get('exact_differences', [0, 0]) == [0, 0] for r in runs.values())
    def stats(values):
        return {'n': len(values), 'wall_seconds': [v['wall_seconds'] for v in values],
                'median_seconds': statistics.median(v['wall_seconds'] for v in values),
                'min_seconds': min(v['wall_seconds'] for v in values),
                'max_seconds': max(v['wall_seconds'] for v in values),
                'cpu_seconds': [v['user_seconds'] + v['sys_seconds'] for v in values],
                'average_cpu_cores': [v['average_cpu_cores'] for v in values],
                'process_lifetime_peak_rss_bytes': [v['process_lifetime_peak_rss_bytes'] for v in values]}
    summary = {'protocol': PROTOCOL, 'inputs': json.loads((OUT / 'inputs.json').read_text()),
               'fact_counts': reference['fact_counts'], 'errors': reference['errors'],
               'native_extraction_errors': reference['native_extraction_errors'],
               'native_pilot_exact_rows': runs['pilot']['independent_native_exact_rows'],
               'direct': {}, 'workflow': {}, 'incremental': {},
               'all_compared_facts_exact': True, 'run_names': sorted(runs),
               'script_sha256': digest(__file__)}
    for threads in PROTOCOL['threads']:
        candidates = [r for r in runs.values() if r['mode'] == 'extract' and r['threads'] == threads]
        assert len(candidates) == PROTOCOL['observations_per_thread']
        summary['direct'][str(threads)] = stats([r['extraction'] for r in candidates])
    for variant in ['generic', 'parquet']:
        candidates = [r for r in runs.values() if r['mode'] == 'workflow' and r['variant'] == variant]
        assert len(candidates) == 2
        summary['workflow'][variant] = {phase: stats([r[phase]['timing'] for r in candidates])
                                        for phase in ['cold', 'same_connection_replay', 'restart_replay']}
        assert all(r[phase]['extracted'] == 0 for r in candidates for phase in ['same_connection_replay', 'restart_replay'])
    for name, r in runs.items():
        if r['mode'] == 'incremental':
            summary['incremental'][name] = {'variant': r['variant'], 'percent': r['simulated_miss_percent'],
                                           'missing': r['incremental']['missing'], 'timing': r['incremental']['timing'],
                                           'restart_replay': r['restart_replay']}
    summary['direct_speedup_1_to_4'] = summary['direct']['1']['median_seconds'] / summary['direct']['4']['median_seconds']
    summary['workflow_speedup'] = summary['workflow']['generic']['cold']['median_seconds'] / summary['workflow']['parquet']['cold']['median_seconds']
    summary['typed_parquet_checks'] = sum(len(r.get('typed_parquet_without_extension', [])) for r in runs.values())
    summary['all_replays_extract_zero'] = all(r[p]['extracted'] == 0 for r in runs.values()
        if r['mode'] in ('workflow', 'incremental') for p in ['same_connection_replay', 'restart_replay'])
    assert len(summary['incremental']) == 4
    for percent in (1, 10):
        pair = [r for r in summary['incremental'].values() if r['percent'] == percent]
        assert len(pair) == 2 and pair[0]['missing'] == pair[1]['missing']
    summary['cached_replay_trace'] = runs['cached-trace']['phase_seconds']
    summary['profile_assessment'] = json.loads((OUT / 'profile-control/assessment.json').read_text())
    summary['environment_snapshots'] = json.loads((OUT / 'environment-snapshots.json').read_text())
    import duckdb
    import _duckdb
    import platform
    summary['runtime'] = {'duckdb_version': duckdb.__version__,
                          'native_module_sha256': digest(_duckdb.__file__),
                          'python_version': sys.version,
                          'machine': platform.machine(), 'os': platform.system(),
                          'logical_cpus': os.cpu_count()}
    with duckdb.connect() as con:
        rejected = con.execute('''SELECT blob_oid,logical_path,raw_invalid_bytes FROM read_parquet('''
                               + quote(OUT / 'sources.parquet') + ''') WHERE source_status='encoding_error' ''').fetchall()
    summary['raw_encoding_bytes_verified'] = []
    for oid, path, raw in rejected:
        assert hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest() == oid
        summary['raw_encoding_bytes_verified'].append({'blob_oid': oid, 'logical_path': path,
                                                       'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest()})
    assert len(rejected) == len(summary['inputs']['encoding_errors'])
    assert digest(OUT / 'sources.parquet') == summary['inputs']['sha256']['sources.parquet']
    save(OUT / 'summary.json', summary)
    manifest = []
    for p in sorted(OUT.rglob('*')):
        if p.is_file() and p.name != 'manifest-sha256.json':
            manifest.append({'path': str(p.relative_to(OUT)), 'bytes': p.stat().st_size, 'sha256': digest(p)})
    save(OUT / 'manifest-sha256.json', manifest)
    print(json.dumps(summary))


def trace_cached(args):
    """Attribute one cached replay by statement; not a cold timing observation."""
    folder = OUT / args.name
    folder.mkdir(exist_ok=False)
    os.chdir(folder)
    database = folder / 'facts.duckdb'
    shutil.copy2(OUT / args.seed / 'facts.duckdb', database)
    result = {'name': args.name, 'mode': 'trace', 'threads': args.threads,
              'variant': args.variant, 'statements': [],
              'note': 'Cached diagnostic split by real SQL statements; excluded from cold timing medians.'}
    con = None
    try:
        con = connect(database, args.threads)
        staged(con)
        sql = sql_workflow(args.variant)
        for index, statement in enumerate(con.extract_statements(sql)):
            text = statement.query
            clean = re.sub(r'--[^\n]*', '', text).strip().lower()
            if clean.startswith('copy'):
                phase = 'parquet_export'
            elif 'create or replace table file_facts_occurrences' in clean:
                phase = 'lineage'
            elif re.search(r'create or replace table file_facts_(nodes|edges|spans|macro_)', clean):
                phase = 'child_tables'
            elif 'create or replace temp table file_facts_extracted' in clean:
                phase = 'extraction'
            else:
                phase = 'staging_validation_cache_setup'
            timing = measured(con, text)
            result['statements'].append({'index': index, 'phase': phase, 'sql': text, 'timing': timing})
        result['inputs'] = con.execute('SELECT count(*),sum(byte_length) FROM inputs').fetchall()[0]
        result['fact_counts'] = counts(con, 'file_facts_cache')
        result['extracted'] = con.execute('SELECT count(*) FROM file_facts_extracted').fetchall()[0][0]
        assert result['extracted'] == 0
        if args.baseline:
            result['exact_differences'] = exact(con, 'SELECT blob_oid,logical_path,facts FROM file_facts_cache', args.baseline)
        con.close()
        con = None
        result['typed_parquet_without_extension'] = verify_parquet(database, folder)
        phases = {}
        for statement in result['statements']:
            phase = statement['phase']
            phases[phase] = phases.get(phase, 0) + statement['timing']['wall_seconds']
        result['phase_seconds'] = phases
        result['status'] = 'passed'
    except BaseException as error:
        result['status'] = 'failed'
        result['error'] = repr(error)
        raise
    finally:
        if con is not None:
            con.close()
        save(folder / 'result.json', result)
        print(json.dumps(result))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['prepare', 'pilot', 'extract', 'profile', 'workflow', 'incremental', 'trace', 'summarize'])
    parser.add_argument('--name')
    parser.add_argument('--threads', type=int, default=4)
    parser.add_argument('--variant', choices=['generic', 'parquet'], default='generic')
    parser.add_argument('--baseline')
    parser.add_argument('--seed', default='w-generic-r0')
    parser.add_argument('--miss-percent', type=int, choices=[1, 10], default=1)
    args = parser.parse_args()
    if args.mode == 'prepare':
        prepare()
    elif args.mode == 'summarize':
        summarize()
    elif args.mode == 'trace':
        assert args.name
        trace_cached(args)
    else:
        assert args.name
        experiment(args)
