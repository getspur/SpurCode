"""Diagnostic lexical marker coverage; never substitutes a C++ parser."""
import json
from pathlib import Path

import duckdb

art = Path(__file__).resolve().parent
c = duckdb.connect(str(art / 'snapshot.duckdb'), read_only=True,
                   config={'threads': '2', 'memory_limit': '1GB'})
schema = c.execute("SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('nodes','inventory','sources') ORDER BY table_name,ordinal_position").fetchall()
pattern = r'(?m)^[ \t]*TEST_CASE[ \t]*\('
base = '''WITH yield AS (
 SELECT logical_path,count(*) FILTER(WHERE kind<>'file') AS symbols
 FROM nodes GROUP BY logical_path), marked AS (
 SELECT i.path,i.bytes,coalesce(y.symbols,0) AS symbols,
 array_length(regexp_extract_all(s.source,?)) AS marker_lines
 FROM inventory i JOIN sources s ON i.path=s.logical_path
 LEFT JOIN yield y ON i.path=y.logical_path
 WHERE i.area='tests' AND i.suffix='.cpp' AND i.status='extracted')'''
summary_sql = base + ''' SELECT count(*) AS admitted_cpp_test_files,
count(*) FILTER(WHERE marker_lines>0) AS files_with_test_case_marker,
count(*) FILTER(WHERE marker_lines>0 AND symbols=0) AS file_only_with_test_case_marker,
sum(marker_lines) FILTER(WHERE symbols=0) AS candidate_marker_lines_in_file_only_sources
FROM marked'''
cur = c.execute(summary_sql, [pattern]);summary = dict(zip([d[0] for d in cur.description], cur.fetchone()))
sample_sql = base + ' SELECT * FROM marked WHERE marker_lines>0 AND symbols=0 ORDER BY bytes DESC,path LIMIT 20'
cur = c.execute(sample_sql, [pattern]); names = [d[0] for d in cur.description]
samples = [dict(zip(names, row)) for row in cur.fetchall()]
result = {'summary': summary, 'sample_limit': 20, 'samples': samples,
          'queries': [{'sql': summary_sql, 'params': [pattern]}, {'sql': sample_sql, 'params': [pattern]}],
          'manual_counterexamples': ['test/api/test_relation_api.cpp','test/api/capi/v1/test_capi_data_chunk.cpp','test/api/capi/v1/test_capi_values.cpp'],
          'interpretation': 'Marker lines are lexical candidates, not parsed test definitions or a recall estimate. The three manually inspected counterexamples contain real TEST_CASE bodies yet yield only a file node. Preserve these misses separately from the 17 ordinary-definition controls.'}
(art / 'cpp-test-macro-coverage.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(summary))
c.close()
