"""Frozen Rust controls, test diagnostics and real macro evidence via DuckDB SQL."""
import hashlib
import json
from pathlib import Path

import duckdb

ART = Path(__file__).resolve().parent
controls = json.loads((ART / 'curated-rust-controls.json').read_text())
c = duckdb.connect(str(ART / 'snapshot.duckdb'), read_only=True,
                   config={'threads': '2', 'memory_limit': '1GB'})
queries = []


def query(name, sql, params=None):
    params = params or []
    cursor = c.execute(sql, params)
    names = [d[0] for d in cursor.description]
    rows = [dict(zip(names, r)) for r in cursor.fetchall()]
    queries.append({'name': name, 'sql': sql, 'params': params, 'rows': rows})
    return rows


query('schema_first', "SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('inventory','sources','facts','nodes','edges','spans','macro_invocations','macro_scopes') ORDER BY table_name,ordinal_position")
source_hashes = query('all_source_byte_hashes', '''SELECT count(*) AS sources,
count(*) FILTER(WHERE sha256(source_bytes)<>sha256 OR encode(source)<>source_bytes) AS mismatches
FROM sources''')[0]
span_bounds = query('span_bounds_by_suffix', '''WITH source_info AS MATERIALIZED (
SELECT logical_path,octet_length(source_bytes) AS byte_length,
 length(source)-length(replace(source,chr(10),''))+1 AS line_count FROM sources)
SELECT i.suffix,count(*) AS spans,
count(*) FILTER(WHERE p.start_byte IS NULL OR p.end_byte IS NULL OR p.start_byte>p.end_byte OR p.end_byte>s.byte_length) AS invalid_byte_bounds,
count(*) FILTER(WHERE p.start_line IS NULL OR p.end_line IS NULL OR p.start_line<1 OR p.end_line<p.start_line OR p.end_line>s.line_count) AS invalid_line_bounds
FROM spans p JOIN source_info s USING(logical_path) JOIN inventory i ON i.path=p.logical_path
GROUP BY i.suffix ORDER BY spans DESC''')
rust_coverage = query('rust_coverage', '''SELECT status,count(*) AS files,sum(bytes) AS bytes
FROM inventory WHERE suffix='.rs' GROUP BY status ORDER BY status''')
yield_rows = query('rust_yield', '''WITH yield AS (
SELECT logical_path,count(*) FILTER(WHERE kind<>'file') AS symbols FROM nodes GROUP BY logical_path)
SELECT count(*) AS admitted_rust_files,sum(y.symbols) AS symbols,
count(*) FILTER(WHERE y.symbols=0) AS file_only
FROM inventory i JOIN yield y ON i.path=y.logical_path WHERE i.suffix='.rs' AND i.status='extracted' ''')[0]
binding = query('rust_call_binding', '''SELECT count(*) AS calls,
count(*) FILTER(WHERE e.target_node_id IS NOT NULL) AS local_targets,
count(*) FILTER(WHERE e.target_node_id IS NULL) AS unresolved
FROM edges e JOIN inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='calls' ''')[0]
query('rust_node_kinds', '''SELECT n.kind,count(*) AS nodes FROM nodes n JOIN inventory i
ON i.path=n.logical_path WHERE i.suffix='.rs' GROUP BY n.kind ORDER BY nodes DESC''')
query('rust_call_binding_by_crate', '''SELECT i.area,count(*) AS calls,
count(*) FILTER(WHERE e.target_node_id IS NOT NULL) AS local_targets,
count(*) FILTER(WHERE e.target_node_id IS NULL) AS unresolved
FROM edges e JOIN inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='calls'
GROUP BY i.area ORDER BY calls DESC''')

definitions = []
for case in controls['positive_definitions'] + controls['negative_raw_string_definitions']:
    rows = query(case['case_id'], '''SELECT n.node_id,n.stable_key,n.label,n.kind,
p.start_byte,p.end_byte,p.start_line,p.end_line,
substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,
          2*CAST(p.end_byte-p.start_byte AS BIGINT)) AS source_span_hex
FROM nodes n JOIN spans p ON n.logical_path=p.logical_path AND n.source_span_id=p.span_id
JOIN sources s ON n.logical_path=s.logical_path
WHERE n.logical_path=? AND regexp_extract(n.label,'([^:]+)$',1)=?
 AND n.kind<>'file' AND p.start_line<=? AND p.end_line>=?
ORDER BY p.start_line,n.node_id''', [case['path'], case['name_tail'], case['line'], case['line']])
    for row in rows:
        raw = bytes.fromhex(row['source_span_hex'])
        # AST spans exclude indentation; original bytes are never normalized.
        row['signature_bytes_present'] = case['source_signature'].lstrip().encode() in raw
        row['span_sha256'] = hashlib.sha256(raw).hexdigest()
    expected = case.get('expected_nodes', 1)
    passed = len(rows) == expected and (expected == 0 or rows[0]['signature_bytes_present'])
    definitions.append({'control': case, 'matching_nodes': rows, 'passed': passed})

macro_controls = []
for case in controls['local_macro_invocations']:
    rows = query('macro-' + case['case_id'], '''SELECT m.*,s.file_oid,
substring(hex(s.source_bytes),2*CAST(m.byte_range[1] AS BIGINT)+1,
 2*CAST(m.byte_range[2]-m.byte_range[1] AS BIGINT)) AS invocation_hex
FROM macro_invocations m JOIN sources s USING(logical_path)
WHERE m.logical_path=? AND m.normalized_macro_path=?
 AND m.line_range[1]<=? AND m.line_range[2]>=?
ORDER BY m.site_id''', [case['path'], case['macro_path'], case['line'], case['line']])
    for row in rows:
        row['source_oid_matches'] = row['source_oid'] == row['file_oid']
        row['signature_bytes_present'] = case['source_signature'].lstrip().encode() in bytes.fromhex(row['invocation_hex'])
    macro_controls.append({'control': case, 'evidence': rows,
        'passed': len(rows) == 1 and rows[0]['source_oid_matches'] and rows[0]['signature_bytes_present']})

macro_totals = query('rust_macro_totals', '''SELECT
(SELECT count(*) FROM macro_invocations m JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs') AS invocations,
(SELECT count(*) FROM macro_scopes m JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs') AS scopes''')[0]
macro_distribution = query('rust_macro_context_resolution', '''SELECT m.test_context,m.execution_phase,
m.resolution.kind AS resolution_kind,count(*) AS sites
FROM macro_invocations m JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs'
GROUP BY ALL ORDER BY sites DESC''')
macro_bounds = query('rust_macro_bounds_and_source_identity', '''WITH source_info AS MATERIALIZED (
SELECT logical_path,file_oid,octet_length(source_bytes) AS byte_length,
 length(source)-length(replace(source,chr(10),''))+1 AS line_count FROM sources)
SELECT count(*) AS invocations,
count(*) FILTER(WHERE m.file_path IS DISTINCT FROM m.logical_path) AS file_path_mismatches,
count(*) FILTER(WHERE m.source_oid IS DISTINCT FROM s.file_oid) AS source_oid_mismatches,
count(*) FILTER(WHERE array_length(m.byte_range) IS DISTINCT FROM 2 OR m.byte_range[1] IS NULL OR m.byte_range[2] IS NULL OR m.byte_range[1]>m.byte_range[2] OR m.byte_range[2]>s.byte_length) AS invalid_byte_bounds,
count(*) FILTER(WHERE array_length(m.line_range) IS DISTINCT FROM 2 OR m.line_range[1] IS NULL OR m.line_range[2] IS NULL OR m.line_range[1]<1 OR m.line_range[1]>m.line_range[2] OR m.line_range[2]>s.line_count) AS invalid_line_bounds
FROM macro_invocations m JOIN source_info s USING(logical_path)
JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs' ''')[0]

# Lexical discovery is deliberately separate from the frozen semantic controls:
# these markers can occur inside comments/raw fixture strings and do not count AST tests.
marker = r'(?m)^[ \t]*#\[(?:test|tokio::test|async_std::test)(?:\]|\()'
test_markers = query('rust_test_marker_file_diagnostics', '''WITH yield AS (
SELECT logical_path,count(*) FILTER(WHERE kind<>'file') AS symbols FROM nodes GROUP BY logical_path), marked AS (
SELECT i.path,coalesce(y.symbols,0) AS symbols,array_length(regexp_extract_all(s.source,?)) AS marker_lines
FROM inventory i JOIN sources s ON i.path=s.logical_path LEFT JOIN yield y ON i.path=y.logical_path
WHERE i.suffix='.rs' AND i.status='extracted')
SELECT count(*) AS files,count(*) FILTER(WHERE marker_lines>0) AS files_with_test_markers,
count(*) FILTER(WHERE marker_lines>0 AND symbols=0) AS file_only_with_test_markers,
sum(marker_lines) AS candidate_test_attribute_lines FROM marked''', [marker])[0]
query('file_only_rust_examples_limit_20', '''WITH yield AS (
SELECT logical_path,count(*) FILTER(WHERE kind<>'file') AS symbols FROM nodes GROUP BY logical_path)
SELECT i.path,i.bytes FROM inventory i JOIN yield y ON i.path=y.logical_path
WHERE i.suffix='.rs' AND i.status='extracted' AND y.symbols=0 ORDER BY i.bytes DESC,i.path LIMIT 20''')

positive = [r for r in definitions if 'expected_nodes' not in r['control']]
negative = [r for r in definitions if 'expected_nodes' in r['control']]
result = {'revision': controls['revision'], 'source_hashes': source_hashes,
          'span_bounds_by_suffix': span_bounds, 'rust_coverage': rust_coverage,
          'rust_yield': yield_rows, 'rust_call_binding': binding,
          'ordinary_definition_controls': {'passed': sum(r['passed'] for r in positive), 'total': len(positive)},
          'raw_string_negative_controls': {'passed': sum(r['passed'] for r in negative), 'total': len(negative)},
          'definition_results': definitions,
          'macro_controls': {'passed': sum(r['passed'] for r in macro_controls), 'total': len(macro_controls), 'cases': macro_controls},
          'rust_macro_totals': macro_totals, 'rust_macro_distribution': macro_distribution,
          'rust_macro_integrity': macro_bounds, 'test_marker_diagnostics': test_markers,
          'limits': ['Selected semantic controls, not exhaustive recall/precision',
                     'Lexical test attributes may be comments or raw-string fixture text',
                     'Macro evidence does not establish expansion into generated function bodies',
                     'Non-null file-local call targets are not a global or semantically proven binding',
                     'Non-Rust spans, especially notebook virtual sources, require format-specific interpretation']}
(ART / 'rust-quality.json').write_text(json.dumps(result, indent=2) + '\n')
(ART / 'rust-quality-queries.json').write_text(json.dumps(queries, indent=2) + '\n')
print(json.dumps({k: result[k] for k in ['source_hashes','rust_coverage','rust_yield','rust_call_binding',
    'ordinary_definition_controls','raw_string_negative_controls','rust_macro_totals','rust_macro_integrity','test_marker_diagnostics']}, indent=2))
print('Local macro controls:', result['macro_controls']['passed'], '/', result['macro_controls']['total'])
c.close()
