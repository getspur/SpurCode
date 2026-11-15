"""Bounded source evidence for accepted C++ translation units with no symbols."""
import json
from pathlib import Path

import duckdb

art = Path(__file__).resolve().parent
c = duckdb.connect(str(art / 'snapshot.duckdb'), read_only=True,
                   config={'threads': '2', 'memory_limit': '1GB'})
c.execute("SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('nodes','inventory','sources') ORDER BY table_name,ordinal_position").fetchall()
sql = '''WITH yield AS (
 SELECT logical_path,count(*) FILTER(WHERE kind<>'file') AS symbols
 FROM nodes GROUP BY logical_path)
SELECT i.path,i.bytes,i.area,substring(s.source,1,6000) AS source_excerpt,
length(s.source) AS source_characters
FROM inventory i JOIN sources s ON i.path=s.logical_path
JOIN yield y ON y.logical_path=i.path
WHERE i.status='extracted' AND i.suffix='.cpp' AND y.symbols=0
ORDER BY i.bytes DESC,i.path LIMIT 5'''
cursor = c.execute(sql)
columns = [v[0] for v in cursor.description]
rows = [dict(zip(columns, r)) for r in cursor.fetchall()]
result = {'sql': sql, 'display_limit': 5, 'excerpt_characters_limit': 6000, 'rows': rows,
          'note': 'File-only is a diagnostic, not automatic failure. Inspect source to distinguish symbol-free translation units from missed definitions.'}
(art / 'file-only-cpp-all-scopes.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'sample_count': len(rows), 'files': [{k:r[k] for k in ['path','bytes','source_characters']} for r in rows]}))
c.close()
