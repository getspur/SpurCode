"""Recheck empty-probe notebook rejects on actual immutable serialized inputs.

This parses repository data only; it never opens/edits/runs a live notebook.
The primary snapshot database stays read-only and Rust-only metrics stay fixed.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading

import duckdb

ART = Path(__file__).resolve().parent
inventory = json.loads((ART / 'tracked-inventory.json').read_text())
with duckdb.connect(str(ART / 'snapshot.duckdb'), read_only=True) as original:
    original.execute("SELECT column_name,data_type FROM information_schema.columns WHERE table_name='inventory' ORDER BY ordinal_position").fetchall()
    paths = {r[0] for r in original.execute("SELECT path FROM inventory WHERE status='path_admission_failed' AND suffix='.ipynb'").fetchall()}
by_oid = {}
for row in inventory['files']:
    if row['path'] in paths:
        by_oid.setdefault(row['git_blob_oid'], []).append(row)
env = {k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
p = subprocess.Popen(['git','-C',inventory['repo'],'cat-file','--batch'],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
writer_errors = []


def send():
    try:
        p.stdin.write(('\n'.join(by_oid) + '\n').encode())
        p.stdin.close()
    except BaseException as exc:
        writer_errors.append(repr(exc))


t = threading.Thread(target=send)
t.start()
sources = []
for oid, rows in by_oid.items():
    header = p.stdout.readline().decode().strip().split()
    assert len(header)==3 and header[:2]==[oid,'blob'],header
    n = int(header[2]);data = p.stdout.read(n)
    assert len(data)==n and p.stdout.read(1)==b'\n'
    assert hashlib.sha1(b'blob '+str(n).encode()+b'\0'+data).hexdigest()==oid
    for row in rows:
        assert row['bytes']==n
        sources.append((row['path'],data.decode('utf-8'),oid))
t.join()
assert p.wait()==0 and not writer_errors
extension = ART.parent / '2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
assert hashlib.sha256(extension.read_bytes()).hexdigest()=='e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028'
c = duckdb.connect(config={'allow_unsigned_extensions':'true','threads':'2','memory_limit':'1GB'})
c.execute("LOAD '"+str(extension).replace("'","''")+"'")
c.execute('CREATE TABLE inputs(path VARCHAR,source VARCHAR,file_oid VARCHAR)')
c.executemany('INSERT INTO inputs VALUES (?,?,?)',sources)
sql = 'CREATE TABLE recheck AS SELECT path,TRY(spur_graph_file_facts(path,source)) AS value FROM inputs'
c.execute(sql)
cur = c.execute('''SELECT path,value IS NOT NULL AS accepted_on_actual_source,
array_length(value.nodes) AS nodes,array_length(value.edges) AS edges,
array_length(value.spans) AS spans,array_length(value.macro_invocations) AS macro_invocations,
array_length(value.macro_scopes) AS macro_scopes FROM recheck ORDER BY path''')
names = [v[0] for v in cur.description]
rows = [dict(zip(names,row)) for row in cur.fetchall()]
for row in rows:
    if not row['accepted_on_actual_source']:
        try:
            c.execute('SELECT spur_graph_file_facts(path,source) FROM inputs WHERE path=?',[row['path']]).fetchone()
            row['error']='NULL under TRY; retry succeeded'
        except Exception as exc:
            row['error']=str(exc)
summary = {'rechecked':len(rows),'accepted_on_actual_source':sum(r['accepted_on_actual_source'] for r in rows),
           'remaining_failures':sum(not r['accepted_on_actual_source'] for r in rows),
           'additional_nodes':sum(r['nodes'] or 0 for r in rows),
           'additional_macro_invocations':sum(r['macro_invocations'] or 0 for r in rows)}
report = {'revision':inventory['revision'],'summary':summary,'sql':sql,'rows':rows,
          'scope':'Separate diagnostic recheck, not merged into primary snapshot. Original Rust-only counts unchanged.',
          'finding':'Empty-source admission is not a reliable capability test for formats requiring nonempty valid input, including notebooks.'}
(ART / 'admission-recheck.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(summary))
c.close()
