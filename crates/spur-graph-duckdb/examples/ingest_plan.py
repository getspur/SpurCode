"""Checkpointed frozen-HEAD ingestion through the native SPUR DuckDB extension."""

import hashlib
import json
import argparse
from pathlib import Path
import resource
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone

import duckdb

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--repo', type=Path, required=True)
parser.add_argument('--catalog', type=Path, required=True)
parser.add_argument('--plan', type=Path, required=True)
parser.add_argument('--extension', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--source-storage', choices=('persistent', 'git'), default='persistent',
                    help='Keep source in DuckDB, or use only transient batches and retain it in Git')
args = parser.parse_args()
ROOT = args.output.resolve()
ROOT.mkdir(parents=True, exist_ok=True)
REPO = args.repo.resolve()
EXTENSION = args.extension.resolve()
PLAN_PATH = args.plan.resolve()
CATALOG_PATH = args.catalog.resolve()
PLAN = json.loads(PLAN_PATH.read_text())
HEAD = PLAN['head']
assert duckdb.__version__ == '2.0.0'
assert PLAN['row_limit'] == duckdb.__standard_vector_size__
if not PLAN['batches'] or not any(batch['tasks'] for batch in PLAN['batches']):
    raise SystemExit('A plan with at least one supported source task is required.')
EXTRACTOR = hashlib.sha256(EXTENSION.read_bytes()).hexdigest()
PLAN_HASH = hashlib.sha256(PLAN_PATH.read_bytes()).hexdigest()
CATALOG_HASH = hashlib.sha256(CATALOG_PATH.read_bytes()).hexdigest()
RUN = hashlib.sha256((HEAD + EXTRACTOR + PLAN_HASH + CATALOG_HASH).encode()).hexdigest()
START = time.perf_counter()


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def max_rss_bytes():
    maximum = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return maximum if sys.platform == 'darwin' else maximum * 1024


def progress(**values):
    values.update(run_id=RUN, head=HEAD, extractor_sha256=EXTRACTOR,
                  wall_seconds=time.perf_counter() - START,
                  timestamp=datetime.now(timezone.utc).isoformat(),
                  max_rss_bytes=max_rss_bytes())
    temporary = ROOT / 'head-progress.json.tmp'
    temporary.write_text(json.dumps(values, indent=2) + '\n')
    temporary.replace(ROOT / 'head-progress.json')
    print(json.dumps(values), flush=True)


connection = duckdb.connect(str(ROOT / 'duckdb-spur-graph.duckdb'), config={'allow_unsigned_extensions': True})
connection.execute(f'LOAD {quote(EXTENSION)}')
symbol_type = connection.execute("SELECT return_type FROM duckdb_functions() WHERE function_name='spur_graph_symbols'").fetchall()[0][0]
connection.execute('CREATE TABLE IF NOT EXISTS files(head VARCHAR,path VARCHAR,mode VARCHAR,kind VARCHAR,blob_oid VARCHAR,byte_length UBIGINT,language VARCHAR,PRIMARY KEY(head,path))')
if args.source_storage == 'persistent':
    connection.execute('CREATE TABLE IF NOT EXISTS blobs(blob_oid VARCHAR PRIMARY KEY, content BLOB)')
connection.execute('CREATE TABLE IF NOT EXISTS blob_flags(blob_oid VARCHAR PRIMARY KEY,binary_prefix BOOLEAN)')
connection.execute(f'CREATE TABLE IF NOT EXISTS parse_cache(blob_oid VARCHAR,language VARCHAR,extractor_sha VARCHAR,representative_path VARCHAR,symbols {symbol_type},status VARCHAR,error VARCHAR,input_bytes UBIGINT,parsed_at TIMESTAMPTZ,PRIMARY KEY(blob_oid,language,extractor_sha))')
connection.execute('CREATE TABLE IF NOT EXISTS tasks(run_id VARCHAR,head VARCHAR,batch_id INTEGER,blob_oid VARCHAR,language VARCHAR,path VARCHAR,input_bytes UBIGINT,PRIMARY KEY(run_id,blob_oid,language))')
connection.execute('CREATE TABLE IF NOT EXISTS batches(run_id VARCHAR,batch_id INTEGER,language VARCHAR,source_rows UBIGINT,input_bytes UBIGINT,parse_seconds DOUBLE,wall_seconds DOUBLE,max_rss_bytes UBIGINT,completed_at TIMESTAMPTZ,PRIMARY KEY(run_id,batch_id))')
connection.execute('CREATE TABLE IF NOT EXISTS runs(run_id VARCHAR PRIMARY KEY,head VARCHAR,extractor_sha VARCHAR,plan_sha VARCHAR,status VARCHAR,started_at TIMESTAMPTZ,completed_at TIMESTAMPTZ)')
connection.execute('BEGIN')
connection.execute(f'CREATE TEMP TABLE incoming_catalog AS SELECT ?::VARCHAR AS head,path,mode,kind,blob_oid,byte_length,language FROM read_json_auto({quote(CATALOG_PATH)})', [HEAD])
existing_paths = connection.execute('SELECT count(*) FROM files WHERE head=?', [HEAD]).fetchall()[0][0]
if existing_paths:
    old_minus_new = connection.execute('SELECT count(*) FROM ((SELECT * FROM files WHERE head=?) EXCEPT ALL (SELECT * FROM incoming_catalog))', [HEAD]).fetchall()[0][0]
    new_minus_old = connection.execute('SELECT count(*) FROM ((SELECT * FROM incoming_catalog) EXCEPT ALL (SELECT * FROM files WHERE head=?))', [HEAD]).fetchall()[0][0]
    if old_minus_new or new_minus_old:
        raise SystemExit('catalog conflicts with the persisted inventory for this frozen HEAD')
connection.execute('INSERT INTO files SELECT * FROM incoming_catalog ON CONFLICT DO NOTHING')
task_count = sum(len(batch['tasks']) for batch in PLAN['batches'])
connection.execute('INSERT INTO tasks SELECT ?,?,b.batch_id,t.blob_oid,t.language,t.path,t.byte_length FROM (SELECT unnest(batches) AS b FROM read_json_auto(?)) batch,unnest(b.tasks) AS rows(t) ON CONFLICT DO NOTHING', [RUN, HEAD, str(PLAN_PATH)])
planned_keys = connection.execute('SELECT count(*) FROM tasks WHERE run_id=?', [RUN]).fetchall()[0][0]
if planned_keys != task_count:
    raise SystemExit('duplicate parse keys in plan')
invalid_tasks = connection.execute("SELECT count(*) FROM tasks t WHERE t.run_id=? AND NOT EXISTS (SELECT 1 FROM files f WHERE f.head=t.head AND f.path=t.path AND f.blob_oid=t.blob_oid AND f.language=t.language AND f.byte_length=t.input_bytes AND f.kind='blob' AND f.mode IN ('100644','100755'))", [RUN]).fetchall()[0][0]
if invalid_tasks:
    raise SystemExit('task metadata does not match the supported catalog path')
omitted_keys = connection.execute('SELECT count(*) FROM files f ANTI JOIN tasks t ON f.blob_oid=t.blob_oid AND f.language=t.language AND t.run_id=? WHERE f.head=? AND f.language IS NOT NULL', [RUN, HEAD]).fetchall()[0][0]
if omitted_keys:
    raise SystemExit('supported catalog keys missing from plan')
connection.execute("INSERT INTO runs VALUES(?,?,?,?,'running',current_timestamp,NULL) ON CONFLICT(run_id) DO UPDATE SET status='running'", [RUN, HEAD, EXTRACTOR, PLAN_HASH])
connection.execute('COMMIT')
parse_misses = connection.execute('SELECT count(*) FROM tasks t ANTI JOIN parse_cache p ON t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=? WHERE t.run_id=?', [EXTRACTOR, RUN]).fetchall()[0][0]
persistent_miss = ' OR NOT EXISTS (SELECT 1 FROM blobs b WHERE b.blob_oid=t.blob_oid)' if args.source_storage == 'persistent' else ''
pending = connection.execute('SELECT t.batch_id,t.blob_oid,t.language,t.path,t.input_bytes FROM tasks t WHERE t.run_id=? AND (NOT EXISTS (SELECT 1 FROM parse_cache p WHERE t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=?) OR NOT EXISTS (SELECT 1 FROM blob_flags f WHERE f.blob_oid=t.blob_oid)' + persistent_miss + ') ORDER BY t.batch_id,t.blob_oid', [RUN, EXTRACTOR]).fetchall()
supported_paths = connection.execute('SELECT count(*) FROM files WHERE head=? AND language IS NOT NULL', [HEAD]).fetchall()[0][0]
progress(state='initialized', supported_paths=supported_paths, unique_tasks=task_count, pending_tasks=parse_misses, source_reads=len(pending))
by_batch = {}
for row in pending:
    by_batch.setdefault(row[0], []).append(row)

child = None
if pending:
    child = subprocess.Popen(['git', '-C', str(REPO), 'cat-file', '--batch'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def send_objects():
        try:
            child.stdin.write(('\n'.join(row[1] for row in pending) + '\n').encode('ascii'))
        finally:
            child.stdin.close()

    sender = threading.Thread(target=send_objects)
    sender.start()

try:
    for batch_id, tasks in by_batch.items():
        batch_started = time.perf_counter()
        source_bytes = sum(row[4] for row in tasks)
        assert len(tasks) <= PLAN['row_limit'] and source_bytes <= PLAN['byte_budget']
        progress(state='parsing', batch_id=batch_id, language=tasks[0][2], source_rows=len(tasks), input_bytes=source_bytes, first_path=tasks[0][3])
        content = []
        for _, oid, _, _, expected_size in tasks:
            header = child.stdout.readline().decode('ascii').split()
            assert len(header) == 3 and header[0] == oid and header[1] == 'blob', header
            size = int(header[2]);assert size == expected_size
            payload = child.stdout.read(size)
            assert len(payload) == size and child.stdout.read(1) == b'\n'
            content.append((oid, payload))
        connection.execute('BEGIN')
        blob_columns = [[row[0] for row in content], [row[1] for row in content]]
        if args.source_storage == 'persistent':
            connection.execute('INSERT INTO blobs SELECT unnest(?::VARCHAR[]),unnest(?::BLOB[]) ON CONFLICT DO NOTHING', blob_columns)
        else:
            connection.execute('CREATE OR REPLACE TEMP TABLE blobs AS SELECT blob_oid,first(content) AS content FROM (SELECT unnest(?::VARCHAR[]) AS blob_oid,unnest(?::BLOB[]) AS content) GROUP BY blob_oid', blob_columns)
        # Match git_walk::is_binary's existing byte-prefix contract. These flags
        # are content facts, independent of grammar and extraction version.
        connection.execute('INSERT INTO blob_flags SELECT unnest(?::VARCHAR[]),unnest(?::BOOLEAN[]) ON CONFLICT DO NOTHING', [blob_columns[0], [b'\x00' in row[1][:8192] for row in content]])
        connection.execute('CREATE OR REPLACE TEMP TABLE ready_batch AS SELECT t.*,TRY(decode(b.content)) AS source FROM tasks t JOIN blobs b USING(blob_oid) ANTI JOIN parse_cache p ON t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=? WHERE t.run_id=? AND t.batch_id=?', [EXTRACTOR, RUN, batch_id])
        parse_started = time.perf_counter()
        connection.execute('CREATE OR REPLACE TEMP TABLE evaluated_batch AS SELECT blob_oid,language,path,input_bytes,source IS NULL AS invalid_utf8,TRY(spur_graph_symbols(path,source)) AS symbols FROM ready_batch')
        parse_seconds = time.perf_counter() - parse_started
        connection.execute("INSERT INTO parse_cache SELECT blob_oid,language,?,path,symbols,CASE WHEN invalid_utf8 THEN 'encoding_error' WHEN symbols IS NULL THEN 'extension_error' ELSE 'completed' END,CASE WHEN invalid_utf8 THEN 'source is not valid UTF-8' WHEN symbols IS NULL THEN 'extension error; source retained for reproduction' ELSE NULL END,input_bytes,current_timestamp FROM evaluated_batch ON CONFLICT DO NOTHING", [EXTRACTOR])
        batch_seconds = time.perf_counter() - batch_started
        connection.execute('INSERT INTO batches VALUES(?,?,?,?,?,?,?,?,current_timestamp) ON CONFLICT DO NOTHING', [RUN, batch_id, tasks[0][2], len(tasks), source_bytes, parse_seconds, batch_seconds, max_rss_bytes()])
        connection.execute('COMMIT')
        connection.execute('CHECKPOINT')
        done = connection.execute('SELECT count(*),count(*) FILTER(WHERE p.status<>\'completed\'),coalesce(sum(array_length(p.symbols)),0) FROM tasks t JOIN parse_cache p ON t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=? WHERE t.run_id=?', [EXTRACTOR, RUN]).fetchall()[0]
        progress(state='checkpoint', batch_id=batch_id, completed_tasks=done[0], error_tasks=done[1], unique_symbol_rows=done[2], batch_parse_seconds=parse_seconds, batch_wall_seconds=batch_seconds)
finally:
    if child is not None:
        if child.poll() is None:
            child.kill()
        sender.join()
        stderr = child.stderr.read().decode(errors='replace')
        code = child.wait()
        if code not in (0, -9):
            raise RuntimeError(f'Git object reader failed: {code}: {stderr}')

connection.execute(f"CREATE OR REPLACE VIEW symbol_facts AS SELECT blob_oid,language,extractor_sha,unnest(symbols,recursive:=true) FROM parse_cache WHERE status='completed' AND extractor_sha={quote(EXTRACTOR)}")
connection.execute(f"CREATE OR REPLACE VIEW head_symbols AS SELECT f.head,f.path,f.blob_oid,f.language,unnest(p.symbols,recursive:=true) FROM files f JOIN parse_cache p ON f.blob_oid=p.blob_oid AND f.language=p.language AND p.extractor_sha={quote(EXTRACTOR)} WHERE f.head={quote(HEAD)} AND p.status='completed'")
connection.execute(f"CREATE OR REPLACE VIEW ingestion_errors AS SELECT f.path,f.blob_oid,f.language,p.status,p.error FROM files f JOIN parse_cache p ON f.blob_oid=p.blob_oid AND f.language=p.language AND p.extractor_sha={quote(EXTRACTOR)} WHERE f.head={quote(HEAD)} AND p.status<>'completed'")
counts = connection.execute('SELECT count(*),count(*) FILTER(WHERE p.status=\'completed\'),count(*) FILTER(WHERE p.status<>\'completed\'),count(*) FILTER(WHERE array_length(p.symbols)=0),coalesce(sum(array_length(p.symbols)),0) FROM tasks t JOIN parse_cache p ON t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=? WHERE t.run_id=?', [EXTRACTOR, RUN]).fetchall()[0]
missing = connection.execute('SELECT count(*) FROM tasks t ANTI JOIN parse_cache p ON t.blob_oid=p.blob_oid AND t.language=p.language AND p.extractor_sha=? WHERE t.run_id=?', [EXTRACTOR, RUN]).fetchall()[0][0]
assert missing == 0 and counts[0] == task_count
missing_paths = connection.execute('SELECT count(*) FROM files f ANTI JOIN parse_cache p ON f.blob_oid=p.blob_oid AND f.language=p.language AND p.extractor_sha=? WHERE f.head=? AND f.language IS NOT NULL', [EXTRACTOR, HEAD]).fetchall()[0][0]
if missing_paths:
    raise SystemExit('supported catalog paths missing from parse cache')
missing_flags = connection.execute('SELECT count(*) FROM tasks t ANTI JOIN blob_flags f USING(blob_oid) WHERE t.run_id=?', [RUN]).fetchall()[0][0]
if missing_flags:
    raise SystemExit('current task blobs missing binary-prefix flags')
if args.source_storage == 'persistent':
    missing_sources = connection.execute('SELECT count(*) FROM tasks t ANTI JOIN blobs b USING(blob_oid) WHERE t.run_id=?', [RUN]).fetchall()[0][0]
    if missing_sources:
        raise SystemExit('current task blobs missing requested persistent source')
path_counts = connection.execute('SELECT count(*),count(*) FILTER(WHERE language IS NOT NULL) FROM files WHERE head=?', [HEAD]).fetchall()[0]
languages = connection.execute('SELECT language,count(*) AS symbol_rows,count(DISTINCT path) AS paths_with_symbols FROM head_symbols GROUP BY language ORDER BY symbol_rows DESC').fetchall()
path_symbols = connection.execute('SELECT count(*) FROM head_symbols').fetchall()[0][0]
connection.execute(f'COPY (SELECT * FROM parse_cache WHERE extractor_sha={quote(EXTRACTOR)}) TO {quote(ROOT / "head-parse-cache.parquet")} (FORMAT PARQUET)')
connection.execute(f'COPY head_symbols TO {quote(ROOT / "head-symbols.parquet")} (FORMAT PARQUET)')
connection.execute(f'COPY blob_flags TO {quote(ROOT / "blob-flags.parquet")} (FORMAT PARQUET)')
connection.execute('UPDATE runs SET status=?,completed_at=current_timestamp WHERE run_id=?', ['completed' if counts[2] == 0 else 'completed_with_errors', RUN])
connection.execute('CHECKPOINT')
usage = resource.getrusage(resource.RUSAGE_SELF)
summary = {'state':'completed' if counts[2] == 0 else 'completed_with_errors','head':HEAD,'engine':duckdb.__version__,'run_id':RUN,'extractor_sha256':EXTRACTOR,'inventory_paths':path_counts[0],'supported_paths':path_counts[1],'unique_tasks':counts[0],'completed_parse_tasks':counts[1],'error_tasks':counts[2],'zero_symbol_tasks':counts[3],'unique_symbol_rows':counts[4],'path_symbol_rows':path_symbols,'missing_tasks':missing,'missing_supported_paths':missing_paths,'new_parse_tasks_this_run':parse_misses,'source_reads_this_run':len(pending),'source_storage':args.source_storage,'languages':[{'language':r[0],'symbol_rows':r[1],'paths_with_symbols':r[2]} for r in languages],'wall_seconds':time.perf_counter()-START,'user_seconds':usage.ru_utime,'sys_seconds':usage.ru_stime,'max_rss_bytes':max_rss_bytes(),'database':str(ROOT/'duckdb-spur-graph.duckdb'),'scope':'Frozen HEAD tracked regular files supported by the SPUR language registry; symbol facts and completion, not full temporal/call-graph ingestion.'}
connection.close()
(ROOT/'head-ingestion-result.json').write_text(json.dumps(summary,indent=2)+'\n')
progress(**summary)

if counts[2]:
    raise SystemExit('Ingestion completed with errors; inspect ingestion_errors before accepting the output.')
