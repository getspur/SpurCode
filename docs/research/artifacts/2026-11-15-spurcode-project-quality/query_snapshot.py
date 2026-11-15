"""Execute notebook-authored SQL against the frozen native DuckDB2 snapshot."""
import json
from pathlib import Path
import sys
import time

import duckdb

art = Path(__file__).resolve().parent
assert duckdb.__version__ == '2.0.0'
queries = json.load(sys.stdin)
c = duckdb.connect(str(art / 'snapshot.duckdb'), read_only=True,
                   config={'threads': '2', 'memory_limit': '1GB'})
results = []
for q in queries:
    started = time.perf_counter()
    cursor = c.execute(q['sql'])
    names = [r[0] for r in cursor.description]
    rows = [dict(zip(names, r)) for r in cursor.fetchall()]
    result = {'name': q['name'], 'sql': q['sql'], 'rows': rows,
              'wall_seconds': time.perf_counter() - started}
    results.append(result)
    print(json.dumps(result, default=str), flush=True)
(art / 'notebook-sql-exploration.json').write_text(json.dumps(results, indent=2, default=str) + '\n')
c.close()
