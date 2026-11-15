"""Prepare typed DuckDB source inputs; run by the Rust notebook, outside query timing."""
import hashlib
import json
import subprocess
import time
from collections import Counter
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

ROOT = Path('/Users/kevintruong/project/SpurCode')
OUT = ROOT / 'docs/research/artifacts/2026-11-15-duckdb-file-facts/frozen-source'
OLD = ROOT / 'docs/research/artifacts/2026-11-14-duckdb-full-temporal'
PLAN = OLD / 'extension-comparison/reachable372-extension-r0/plan.json'
REPO = OLD / 'legacy-comparison/reachable372-cli-j4-cache64m-r0.git'
EXPECTED_PLAN = '33a1fc2e674c3108d6522aa207876ac1dce5bea5f1c0633b0f5f744c68a88dc2'

start = time.perf_counter()
OUT.mkdir(exist_ok=True)
raw = PLAN.read_bytes()
assert hashlib.sha256(raw).hexdigest() == EXPECTED_PLAN
plan = json.loads(raw)
unique = {}
for batch in plan['batches']:
    for task in batch['tasks']:
        key = (task['blob_oid'], task['source_path'])
        assert key not in unique
        unique[key] = task
tasks = sorted(unique.values(), key=lambda t: (t['blob_oid'], t['source_path']))
rows = []
with subprocess.Popen(['git', '--git-dir', str(REPO), 'cat-file', '--batch'],
                      stdin=subprocess.PIPE, stdout=subprocess.PIPE) as proc:
    for task in tasks:
        proc.stdin.write((task['blob_oid'] + '\n').encode('ascii'))
        proc.stdin.flush()
        oid, kind, length = proc.stdout.readline().decode('ascii').strip().split()
        assert oid == task['blob_oid'] and kind == 'blob'
        assert int(length) == task['byte_length']
        data = proc.stdout.read(int(length))
        assert len(data) == int(length) and proc.stdout.read(1) == b'\n'
        calculated = hashlib.sha1(b'blob ' + str(len(data)).encode('ascii') + b'\0' + data).hexdigest()
        assert calculated == oid
        rows.append({'blob_oid': oid, 'logical_path': task['source_path'],
                     'source': data.decode('utf-8'), 'language': task['language'],
                     'byte_length': len(data)})
    proc.stdin.close()
    assert proc.wait() == 0
schema = pa.schema([('blob_oid', pa.string()), ('logical_path', pa.string()),
                    ('source', pa.string()), ('language', pa.string()),
                    ('byte_length', pa.uint64())])
pq.write_table(pa.Table.from_pylist(rows, schema=schema), OUT / 'sources.parquet',
               row_group_size=128, compression='zstd')
# Pilot includes size quantiles and the largest input in every source language.
# Its size is an experiment setting, not a runtime admission policy.
pilot_keys = set()
for language in sorted({r['language'] for r in rows}):
    candidates = sorted((r for r in rows if r['language'] == language),
                        key=lambda r: (r['byte_length'], r['blob_oid'], r['logical_path']))
    for quantile in range(16):
        r = candidates[quantile * (len(candidates) - 1) // 15]
        pilot_keys.add((r['blob_oid'], r['logical_path']))
pilot = [r for r in rows if (r['blob_oid'], r['logical_path']) in pilot_keys]
pq.write_table(pa.Table.from_pylist(pilot, schema=schema), OUT / 'pilot.parquet',
               row_group_size=1, compression='zstd')
result = {'frozen_head': plan['head'], 'plan_sha256': EXPECTED_PLAN,
          'source_key': ['blob_oid', 'logical_path'], 'rows': len(rows),
          'source_bytes': sum(r['byte_length'] for r in rows),
          'languages': dict(Counter(r['language'] for r in rows)),
          'row_group_size': 128, 'row_groups': pq.ParquetFile(OUT / 'sources.parquet').num_row_groups,
          'pilot_rows': len(pilot), 'pilot_source_bytes': sum(r['byte_length'] for r in pilot),
          'preparation_seconds': time.perf_counter() - start,
          'scope': 'One logical path per distinct old temporal blob/language plan entry; not every historical blob/path occurrence.',
          'artifacts': {name: hashlib.sha256((OUT / name).read_bytes()).hexdigest()
                        for name in ['sources.parquet', 'pilot.parquet']}}
(OUT / 'inputs.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
