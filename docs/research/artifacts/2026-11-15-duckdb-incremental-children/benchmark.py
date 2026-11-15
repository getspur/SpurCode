"""Matched old/new SQL benchmark on copied frozen caches, invoked by Rust notebook."""
import argparse
from contextlib import chdir
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import re
import shutil
import statistics
import subprocess
import time

import duckdb

ART = Path(__file__).resolve().parent
ROOT = ART.parents[3]
PREVIOUS = ROOT / 'docs/research/artifacts/2026-11-15-duckdb-file-facts-scale'
PROTOCOL = json.loads((PREVIOUS / 'protocol.json').read_text())
KEY = 'blob_oid,logical_path,extractor_sha'
CHILDREN = ['nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n')


def measure(con, sql, folder):
    before = resource.getrusage(resource.RUSAGE_SELF)
    load_start = os.getloadavg()
    start = time.perf_counter()
    with chdir(folder):
        con.execute(sql).fetchall()
    wall = time.perf_counter() - start
    after = resource.getrusage(resource.RUSAGE_SELF)
    return {'wall_seconds': wall, 'user_seconds': after.ru_utime - before.ru_utime,
            'sys_seconds': after.ru_stime - before.ru_stime,
            'process_lifetime_peak_rss_bytes': after.ru_maxrss,
            'load_start': load_start, 'load_end': os.getloadavg()}


def connect(path):
    con = duckdb.connect(str(path), config={'allow_unsigned_extensions': True})
    con.execute('SET threads=4')
    con.execute("SET memory_limit='4GB'")
    con.execute('LOAD ' + quote(PROTOCOL['extension']))
    con.execute('CREATE TEMP VIEW inputs AS SELECT * FROM read_parquet(' + quote(PREVIOUS / 'sources.parquet') + ')')
    con.execute('CREATE TEMP VIEW staged_sources AS SELECT blob_oid,logical_path,source,'
                + quote(PROTOCOL['extension_sha256']) + '::VARCHAR extractor_sha FROM inputs')
    lineage(con, False)
    return con


def lineage(con, extra):
    columns = ('blob_oid,logical_path,' + quote(PROTOCOL['extension_sha256'])
               + '::VARCHAR extractor_sha,source_status,source_error,byte_length')
    sql = "CREATE OR REPLACE TEMP VIEW source_lineage AS SELECT 'source-plan-entry' occurrence_kind," + columns
    sql += ',NULL::VARCHAR parent_oid FROM inputs'
    if extra:
        sql += " UNION ALL SELECT 'synthetic-merge-probe' occurrence_kind," + columns
        sql += ",p.parent_oid FROM (SELECT * FROM inputs WHERE source_status='ok' ORDER BY blob_oid,logical_path LIMIT 1) i"
        sql += " CROSS JOIN (VALUES ('probe-parent-1'),('probe-parent-2'),('probe-parent-2')) p(parent_oid)"
    con.execute(sql)


def key_count(con, name):
    return con.execute('SELECT count(*) FROM ' + name).fetchall()[0][0]


def table_oids(con):
    names = ','.join(quote('file_facts_' + name) for name in CHILDREN)
    return con.execute('SELECT table_name,table_oid FROM duckdb_tables() '
                       'WHERE database_name=current_database() AND schema_name=current_schema() '
                       'AND table_name IN (' + names + ') ORDER BY table_name').fetchall()


def difference(con, left, right):
    ltypes = [(r[0], r[1]) for r in con.execute('DESCRIBE ' + left).fetchall()]
    rtypes = [(r[0], r[1]) for r in con.execute('DESCRIBE ' + right).fetchall()]
    assert ltypes == rtypes, (ltypes, rtypes)
    value = con.execute('SELECT (SELECT count(*) FROM ((' + left + ') EXCEPT ALL (' + right + '))),'
                        '(SELECT count(*) FROM ((' + right + ') EXCEPT ALL (' + left + ')))').fetchall()[0]
    assert value == (0, 0), value
    return value


def verify(con, expected_occurrences, candidate):
    checks = {}
    for name in CHILDREN:
        expected = 'SELECT ' + KEY + ',fact_ordinal,item.* FROM file_facts_cache,'
        expected += 'UNNEST(facts.' + name + ') WITH ORDINALITY u(item,fact_ordinal)'
        checks[name] = difference(con, 'SELECT * FROM file_facts_' + name, expected)
    expected = "SELECT l.*,coalesce(c.status,'not_cached') extraction_status FROM source_lineage l LEFT JOIN file_facts_cache c USING (" + KEY + ')'
    checks['occurrences'] = difference(con, 'SELECT * FROM file_facts_occurrences', expected)
    assert key_count(con, 'file_facts_occurrences') == expected_occurrences
    if candidate:
        checks['checkpoint'] = difference(con, 'SELECT ' + KEY + ' FROM file_facts_child_state',
                                         'SELECT ' + KEY + ' FROM file_facts_cache')
    return checks


def observe(con, sql, folder, variant, condition, expected_missing):
    before = table_oids(con)
    timing = measure(con, sql, folder)
    missing, extracted = [key_count(con, 'file_facts_' + suffix) for suffix in ['missing', 'extracted']]
    assert missing == extracted == expected_missing
    result = {'variant': variant, 'condition': condition, 'timing': timing,
              'missing': missing, 'extracted': extracted}
    if variant == 'new':
        assert table_oids(con) == before
        result['child_refresh'] = key_count(con, 'file_facts_child_refresh')
        result['child_removed'] = key_count(con, 'file_facts_child_removed')
        assert result['child_refresh'] == expected_missing and result['child_removed'] == 0
    return result


def trace(con, sql, folder):
    result = {'statements': [], 'phase_seconds': {}}
    for index, statement in enumerate(con.extract_statements(sql)):
        clean = re.sub(r'--[^\n]*', '', statement.query).strip().lower()
        if clean.startswith('copy'):
            phase = 'parquet_export'
        elif 'create or replace table file_facts_occurrences' in clean:
            phase = 'lineage'
        elif (re.search(r'file_facts_(child_|nodes|edges|spans|macro_)', clean)
              and not clean.startswith('insert into file_facts_cache')):
            phase = 'child_maintenance'
        elif 'create or replace temp table file_facts_extracted' in clean:
            phase = 'extraction'
        else:
            phase = 'staging_validation_cache_setup'
        timing = measure(con, statement.query, folder)
        result['statements'].append({'index': index, 'phase': phase,
                                     'sql': statement.query, 'timing': timing})
        result['phase_seconds'][phase] = result['phase_seconds'].get(phase, 0) + timing['wall_seconds']
    assert key_count(con, 'file_facts_extracted') == 0
    return result


def verify_snapshots(folder):
    # This fresh reader never loads the extension: nested facts must stay portable.
    results = {}
    with duckdb.connect() as con:
        con.execute("SET memory_limit='4GB'")
        con.execute('ATTACH ' + quote(folder / 'facts.duckdb') + ' AS saved (READ_ONLY)')
        for suffix in ['cache', 'occurrences', *CHILDREN]:
            left = 'SELECT * FROM saved.file_facts_' + suffix
            right = 'SELECT * FROM read_parquet(' + quote(folder / ('file-facts-' + suffix + '.parquet')) + ')'
            results[suffix] = difference(con, left, right)
    return results


parser = argparse.ArgumentParser()
parser.add_argument('--candidate-sql', required=True)
parser.add_argument('--name', required=True)
args = parser.parse_args()
assert duckdb.__version__ == '2.0.0'
assert hashlib.sha256(Path(PROTOCOL['extension']).read_bytes()).hexdigest() == PROTOCOL['extension_sha256']
folder = ART / args.name
folder.mkdir(exist_ok=False)
old = subprocess.check_output(['git', 'show', '71430cd5b:crates/spur-graph-duckdb/examples/file_facts.sql'], cwd=ROOT, text=True)
new = Path(args.candidate_sql).read_text()
(folder / 'old.sql').write_text(old)
(folder / 'new.sql').write_text(new)
sql = {'old': old, 'new': new}
result = {'name': args.name, 'native_sha256': PROTOCOL['extension_sha256'],
          'runtime': {'duckdb': duckdb.__version__, 'python': platform.python_version(),
                      'platform': platform.platform(), 'logical_cpus': os.cpu_count(),
                      'threads': 4, 'memory_limit': '4GB'},
          'input_sha256': hashlib.sha256((PREVIOUS / 'sources.parquet').read_bytes()).hexdigest(),
          'seed_cache_sha256': hashlib.sha256((PREVIOUS / 'w-generic-r0/facts.duckdb').read_bytes()).hexdigest(),
          'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
          'sql_sha256': {k: hashlib.sha256(v.encode()).hexdigest() for k, v in sql.items()},
          'note': 'Fresh copied caches; bootstrap separate; old/new sequential on shared host. Process RSS includes both contexts and prior validation.',
          'observations': [], 'bootstrap': {}}
connections = {}
try:
    for variant in ['old', 'new']:
        target = folder / variant
        target.mkdir()
        shutil.copy2(PREVIOUS / 'w-generic-r0/facts.duckdb', target / 'facts.duckdb')
        con = connect(target / 'facts.duckdb')
        connections[variant] = con
        result.setdefault('workload', {})[variant] = con.execute(
            'SELECT count(*),sum(byte_length),count(*) FILTER (WHERE source_status=\'ok\') FROM inputs').fetchall()[0]
        result['bootstrap'][variant] = measure(con, sql[variant], target)
        assert key_count(con, 'file_facts_missing') == 0
        if variant == 'new':
            assert key_count(con, 'file_facts_child_refresh') == key_count(con, 'file_facts_cache')
        verify(con, 19604, variant == 'new')
    for variant in ['old', 'new', 'new', 'old', 'old', 'new']:
        result['observations'].append(observe(connections[variant], sql[variant], folder / variant,
                                              variant, 'warm', 0))
        save(folder / 'progress.json', result)
    for variant in ['new', 'old']:
        con = connections[variant]
        lineage(con, True)
        row = observe(con, sql[variant], folder / variant, variant, 'lineage_only', 0)
        row['verification'] = verify(con, 19607, variant == 'new')
        result['observations'].append(row)
        lineage(con, False)
    for percent, order in [(1, ['old', 'new']), (10, ['new', 'old'])]:
        for variant in order:
            con = connections[variant]
            con.execute('DELETE FROM file_facts_cache WHERE (blob_oid,logical_path,extractor_sha) IN '
                        '(SELECT ' + KEY + ' FROM staged_sources WHERE hash(blob_oid,logical_path)%100<' + str(percent) + ')')
            expected = 19604 - key_count(con, 'file_facts_cache')
            row = observe(con, sql[variant], folder / variant, variant, 'partial_' + str(percent), expected)
            row['verification'] = verify(con, 19604, variant == 'new')
            result['observations'].append(row)
            save(folder / 'progress.json', result)
    result['warm'] = {}
    for variant in ['old', 'new']:
        values = [r['timing']['wall_seconds'] for r in result['observations']
                  if r['variant'] == variant and r['condition'] == 'warm']
        result['warm'][variant] = {'seconds': values, 'median': statistics.median(values)}
    result['trace'] = {}
    for variant in ['new', 'old']:
        result['trace'][variant] = trace(connections[variant], sql[variant], folder / variant)
        verify(connections[variant], 19604, variant == 'new')
    # Close writers before independently opening either database or its snapshots.
    for con in connections.values():
        con.close()
    connections.clear()
    result['reopened'] = {}
    result['typed_parquet_without_extension'] = {}
    for variant in ['new', 'old']:
        target = folder / variant
        con = connect(target / 'facts.duckdb')
        connections[variant] = con
        # Check durable state before replay can repair or recreate it.
        verify(con, 19604, variant == 'new')
        result['reopened'][variant] = observe(con, sql[variant], target, variant, 'reopened', 0)
        verify(con, 19604, variant == 'new')
        con.close()
        del connections[variant]
        result['typed_parquet_without_extension'][variant] = verify_snapshots(target)
    with duckdb.connect() as con:
        con.execute("SET memory_limit='4GB'")
        for variant in ['old', 'new']:
            con.execute('ATTACH ' + quote(folder / variant / 'facts.duckdb') + ' AS ' + variant + ' (READ_ONLY)')
        result['old_new_exact'] = {suffix: difference(con, 'SELECT * FROM old.file_facts_' + suffix,
                                                    'SELECT * FROM new.file_facts_' + suffix)
                                   for suffix in ['cache', 'occurrences', *CHILDREN]}
    result['status'] = 'passed'
except BaseException as error:
    result['status'] = 'failed'
    result['error'] = repr(error)
    raise
finally:
    for con in connections.values():
        con.close()
    save(folder / 'result.json', result)
    print(json.dumps(result))
