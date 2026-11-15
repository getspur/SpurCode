"""One frozen tracked snapshot; batch Git reads, extension SQL extraction, typed tables."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import threading
import time

import duckdb
import _duckdb

ART = Path(__file__).resolve().parent
EXT = ART.parent / '2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
EXPECTED_EXT = 'e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028'
inventory_bytes = (ART / 'tracked-inventory.json').read_bytes()
inventory = json.loads(inventory_bytes)
repo, revision = inventory['repo'], inventory['revision']
assert duckdb.__version__ == '2.0.0'
assert hashlib.sha256(EXT.read_bytes()).hexdigest() == EXPECTED_EXT
dbpath = ART / 'snapshot.duckdb'
assert not dbpath.exists(), 'Use a fresh output directory; never overwrite an experiment'
started = time.monotonic()
timings = {}


def progress(stage, **extra):
    row = {'stage': stage, 'elapsed_seconds': time.monotonic() - started, **extra}
    (ART / 'ingest-progress.json').write_text(json.dumps(row, indent=2) + '\n')
    print(json.dumps(row), flush=True)


def area(path):
    prefix = path.split('/')[0]
    return {'src': 'core', 'extension': 'extensions', 'third_party': 'third_party',
            'test': 'tests', 'tools': 'tools', 'benchmark': 'benchmarks'}.get(prefix, 'other')


c = duckdb.connect(str(dbpath), config={'allow_unsigned_extensions': 'true', 'threads': '2',
                                     'memory_limit': '1GB'})
c.execute("SET temp_directory='" + str(ART / 'spill').replace("'", "''") + "'")
c.execute("LOAD '" + str(EXT).replace("'", "''") + "'")
c.execute('CREATE TABLE inventory(path VARCHAR,git_blob_oid VARCHAR,bytes BIGINT,suffix VARCHAR,area VARCHAR,status VARCHAR,error VARCHAR)')
c.executemany('INSERT INTO inventory VALUES (?,?,?,?,?,?,?)', [
    (r['path'], r['git_blob_oid'], r['bytes'], r['suffix'], area(r['path']),
     'symlink' if r['mode'] == '120000' else 'non_blob' if r['type'] != 'blob' else 'pending', None)
    for r in inventory['files']])
# Discovery is retained before any data-quality query. Function capability probes
# use every exact path, so special basenames are not guessed from suffixes.
schema = c.execute('SELECT table_name,column_name,data_type FROM information_schema.columns ORDER BY table_name,ordinal_position').fetchall()
(ART / 'schema-before-ingest.json').write_text(json.dumps(schema, indent=2) + '\n')
c.execute("CREATE TABLE admission AS SELECT path, TRY(spur_graph_file_facts(path,'')) IS NOT NULL AS accepted FROM inventory WHERE status='pending'")
c.execute("UPDATE inventory SET status='path_admission_failed',error='Exact-path empty-source extension probe returned NULL under TRY; reason samples retained separately' WHERE path IN (SELECT path FROM admission WHERE NOT accepted)")
admitted = {r[0] for r in c.execute('SELECT path FROM admission WHERE accepted').fetchall()}
samples = []
for path, suffix in c.execute("SELECT min(path),suffix FROM inventory WHERE status='path_admission_failed' GROUP BY suffix ORDER BY suffix").fetchall():
    try:
        c.execute('SELECT spur_graph_file_facts(?,?)', [path, '']).fetchone()
        samples.append({'path': path, 'suffix': suffix, 'error': 'probe unexpectedly succeeded on retry'})
    except Exception as exc:
        samples.append({'path': path, 'suffix': suffix, 'error': str(exc)})
(ART / 'path-admission-errors.json').write_text(json.dumps(samples, indent=2) + '\n')
timings['inventory_and_path_admission'] = time.monotonic() - started
progress('path_admission_complete', tracked=len(inventory['files']), accepted=len(admitted))
c.execute('CREATE TABLE sources(logical_path VARCHAR,source VARCHAR,source_bytes BLOB,file_oid VARCHAR,sha256 VARCHAR)')
by_oid = {}
for row in inventory['files']:
    if row['path'] in admitted:
        by_oid.setdefault(row['git_blob_oid'], []).append(row)
env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
p = subprocess.Popen(['git', '-C', repo, 'cat-file', '--batch'], stdin=subprocess.PIPE,
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
writer_errors = []


def send():
    try:
        p.stdin.write(('\n'.join(by_oid) + '\n').encode('ascii'))
        p.stdin.close()
    except BaseException as exc:
        writer_errors.append(repr(exc))


t = threading.Thread(target=send)
t.start()
source_rows, failures = [], []
for oid, rows in by_oid.items():
    header = p.stdout.readline().decode('ascii').strip().split()
    assert len(header) == 3 and header[:2] == [oid, 'blob'], header
    length = int(header[2])
    data = p.stdout.read(length)
    assert len(data) == length and p.stdout.read(1) == b'\n'
    assert hashlib.sha1(b'blob ' + str(length).encode() + b'\0' + data).hexdigest() == oid
    error, status = None, 'ready'
    try:
        if b'\0' in data:
            status, error = 'binary', 'NUL byte in accepted source path'
        else:
            text = data.decode('utf-8')
    except UnicodeDecodeError as exc:
        status, error = 'non_utf8', str(exc)
    for row in rows:
        assert length == row['bytes'], row['path']
        if status == 'ready':
            source_rows.append((row['path'], text, data, oid, hashlib.sha256(data).hexdigest()))
        else:
            failures.append((status, error, row['path']))
t.join()
stderr = p.stderr.read().decode()
assert p.wait() == 0 and not writer_errors, (stderr, writer_errors)
c.executemany('INSERT INTO sources VALUES (?,?,?,?,?)', source_rows)
c.execute("UPDATE inventory SET status='ready' WHERE path IN (SELECT logical_path FROM sources)")
if failures:
    c.executemany('UPDATE inventory SET status=?,error=? WHERE path=?', failures)
assert c.execute("SELECT count(*) FROM inventory WHERE status='pending'").fetchone()[0] == 0
timings['git_blob_read_hash_utf8_and_staging'] = time.monotonic() - started - sum(timings.values())
del source_rows
progress('source_staging_complete', sources=c.execute('SELECT count(*) FROM sources').fetchone()[0])
# One set-shaped extraction; TRY isolates a bad file instead of dropping its row
# or aborting the corpus. Error retries below diagnose only failed inputs.
c.execute('CREATE TABLE facts AS SELECT logical_path,TRY(spur_graph_file_facts(logical_path,source)) AS value FROM sources')
c.execute("UPDATE inventory SET status='extracted' WHERE path IN (SELECT logical_path FROM facts WHERE value IS NOT NULL)")
for path, in c.execute('SELECT logical_path FROM facts WHERE value IS NULL ORDER BY logical_path').fetchall():
    try:
        c.execute('SELECT spur_graph_file_facts(logical_path,source) FROM sources WHERE logical_path=?', [path]).fetchone()
        error = 'NULL under TRY; retry succeeded'
    except Exception as exc:
        error = str(exc)
    c.execute("UPDATE inventory SET status='parse_failed',error=? WHERE path=?", [error, path])
timings['extension_sql_extraction'] = time.monotonic() - started - sum(timings.values())
progress('extraction_complete', files=c.execute("SELECT count(*) FROM inventory WHERE status='extracted'").fetchone()[0])
for collection in ['nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']:
    c.execute(f'CREATE TABLE {collection} AS SELECT logical_path,item.* FROM facts,UNNEST(value.{collection}) AS u(item)')
timings['typed_collection_materialization'] = time.monotonic() - started - sum(timings.values())
schema = c.execute('SELECT table_name,column_name,data_type FROM information_schema.columns ORDER BY table_name,ordinal_position').fetchall()
(ART / 'schema.json').write_text(json.dumps(schema, indent=2) + '\n')
counts = {table: c.execute(f'SELECT count(*) FROM {table}').fetchone()[0]
          for table in ['inventory', 'sources', 'facts', 'nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes']}
statuses = dict(c.execute('SELECT status,count(*) FROM inventory GROUP BY status').fetchall())
assert counts['inventory'] == len(inventory['files']) == sum(statuses.values())
assert counts['sources'] == counts['facts'] == statuses.get('extracted', 0) + statuses.get('parse_failed', 0)
assert statuses.get('ready', 0) == 0
c.execute('CHECKPOINT')
c.close()
receipt = {'status': 'completed', 'repo': repo, 'revision': revision, 'counts': counts,
           'statuses': statuses, 'timings_seconds': timings,
           'tracked_inventory_sha256': hashlib.sha256(inventory_bytes).hexdigest(),
           'extension_sha256': EXPECTED_EXT, 'duckdb': duckdb.__version__,
           'native_engine_sha256': hashlib.sha256(Path(_duckdb.__file__).read_bytes()).hexdigest(),
           'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
           'resources': {'threads': 2, 'memory_limit': '1GB', 'reason': 'Controlled experiment on memory-pressured 24GB host; disk spill enabled; not a production setting'},
           'host': platform.platform(), 'database_bytes': dbpath.stat().st_size,
           'limitations': ['Single pinned snapshot, no temporal-history claim', 'Path-admission rejects remain in denominator; exact errors sampled by suffix', 'TRY extraction failures retain rows and exact retry errors', 'Tree-sitter syntax-error/recovery counters are not exposed by this extension', 'File-local extraction does not supply repository-wide binding']}
(ART / 'ingest-result.json').write_text(json.dumps(receipt, indent=2) + '\n')
progress('completed', **counts)
