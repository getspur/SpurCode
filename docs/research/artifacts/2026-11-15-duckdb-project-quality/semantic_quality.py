"""Independent frozen C++ controls and exact SQL source/span inspection."""
import hashlib
import json
from pathlib import Path

import duckdb

ART = Path(__file__).resolve().parent
controls = json.loads((ART / 'curated-cpp-controls.json').read_text())
c = duckdb.connect(str(ART / 'snapshot.duckdb'), read_only=True, config={'threads': '2', 'memory_limit': '1GB'})
queries = []


def query(name, sql, params=None):
    params = params or []
    cursor = c.execute(sql, params)
    names = [d[0] for d in cursor.description]
    rows = [dict(zip(names, r)) for r in cursor.fetchall()]
    queries.append({'name': name, 'sql': sql, 'params': params, 'rows': rows})
    return rows


query('schema_discovery', "SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('inventory','sources','nodes','edges','spans') ORDER BY table_name,ordinal_position")
hash_check = query('persisted_source_bytes', '''SELECT count(*) AS sources,
count(*) FILTER (WHERE sha256(source_bytes) <> sha256 OR encode(source) <> source_bytes) AS mismatching_sources
FROM sources''')[0]
line_checks = query('all_span_byte_and_line_bounds', '''SELECT count(*) AS spans,
count(*) FILTER (WHERE p.start_byte>p.end_byte OR p.end_byte>octet_length(s.source_bytes)) AS invalid_byte_bounds,
count(*) FILTER (WHERE p.start_line<1 OR p.end_line<p.start_line
 OR p.end_line>length(s.source)-length(replace(s.source,chr(10),''))+1) AS invalid_line_bounds
FROM spans p JOIN sources s USING(logical_path)''')[0]
results, wrapper_edges = [], []
for case in controls['cases']:
    rows = query(case['case_id'], '''SELECT n.node_id,n.stable_key,n.label,n.kind,p.start_byte,p.end_byte,
p.start_line,p.end_line,
substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,
          2*CAST(p.end_byte-p.start_byte AS BIGINT)) AS source_span_hex
FROM nodes n JOIN spans p ON n.logical_path=p.logical_path AND n.source_span_id=p.span_id
JOIN sources s ON n.logical_path=s.logical_path
WHERE n.logical_path=? AND regexp_extract(n.label,'([^:]+)$',1)=?
 AND n.kind<>'file' AND p.start_line<=? AND p.end_line>=?
ORDER BY p.start_line,n.node_id''', [case['path'], case['name_tail'], case['line'], case['line']])
    for row in rows:
        raw = bytes.fromhex(row['source_span_hex'])
        row['signature_bytes_present'] = case['source_signature'].encode() in raw
        row['span_sha256'] = hashlib.sha256(raw).hexdigest()
    results.append({'control': case, 'matching_nodes': rows,
                    'passed': len(rows) == 1 and rows[0]['signature_bytes_present']})
    if case['path'] == 'src/main/connection.cpp' and case['name_tail'] in ['Query', 'Prepare']:
        for row in rows:
            edges = query('wrapper-call-' + case['case_id'], '''SELECT e.*,dst.label AS local_target_name,
dst.kind AS local_target_kind FROM edges e LEFT JOIN nodes dst
 ON e.logical_path=dst.logical_path AND e.target_node_id=dst.node_id
WHERE e.logical_path=? AND e.source_node_id=? AND e.relation='calls'
ORDER BY e.edge_id''', [case['path'], row['node_id']])
            relevant = [e for e in edges if (e['target_label'] or '').split('::')[-1] == case['name_tail']]
            wrapper_edges.append({'case_id': case['case_id'], 'expected_receiver': 'context (ClientContext)',
                'expected_target_is_not_local_wrapper': True,
                'call_edge_observed': bool(relevant), 'edges': relevant,
                'incorrect_local_binding': [e for e in relevant if e['target_node_id'] is not None]})
report = {'revision': controls['revision'], 'source_hash_validation': hash_check,
          'span_bounds': line_checks,
          'curated_controls': {'total': len(results), 'passed': sum(r['passed'] for r in results),
                               'cases': results},
          'receiver_call_controls': wrapper_edges,
          'limitations': ['17 manually selected definitions are not repository-wide precision/recall',
                         'Bounds and byte equality do not establish AST or binding correctness',
                         'No cross-file target invented for unresolved receiver calls',
                         'Tree-sitter syntax-error and recovery counters are unavailable']}
(ART / 'semantic-quality.json').write_text(json.dumps(report, indent=2) + '\n')
(ART / 'semantic-queries.json').write_text(json.dumps(queries, indent=2) + '\n')
print(json.dumps({'source_hash_validation': hash_check, 'span_bounds': line_checks,
                  'curated_passed': report['curated_controls']['passed'], 'curated_total': len(results),
                  'wrapper_call_edges_observed': sum(r['call_edge_observed'] for r in wrapper_edges),
                  'incorrect_local_wrapper_bindings': sum(len(r['incorrect_local_binding']) for r in wrapper_edges)}, indent=2))
c.close()
