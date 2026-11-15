"""Rust macro scope/source/owner/reference diagnostics; no inferred bindings."""
import json
from pathlib import Path
import duckdb

art = Path(__file__).resolve().parent
c = duckdb.connect(str(art / 'snapshot.duckdb'), read_only=True,
                   config={'threads':'2','memory_limit':'1GB'})
c.execute("SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('macro_scopes','macro_invocations','sources','nodes') ORDER BY table_name,ordinal_position").fetchall()
source_cte = '''WITH source_info AS MATERIALIZED (SELECT logical_path,file_oid,
octet_length(source_bytes) AS bytes,length(source)-length(replace(source,chr(10),''))+1 AS lines FROM sources)'''
queries = [
('scope_source_integrity', source_cte + ''' SELECT count(*) AS scopes,
count(*) FILTER(WHERE s.logical_path IS NULL) AS missing_sources,
count(*) FILTER(WHERE m.file_path IS DISTINCT FROM m.logical_path) AS path_mismatches,
count(*) FILTER(WHERE m.source_oid IS DISTINCT FROM s.file_oid) AS oid_mismatches,
count(*) FILTER(WHERE array_length(m.byte_range) IS DISTINCT FROM 2 OR m.byte_range[1] IS NULL OR m.byte_range[2] IS NULL OR m.byte_range[1]>m.byte_range[2] OR m.byte_range[2]>s.bytes) AS invalid_byte_ranges,
count(*) FILTER(WHERE array_length(m.line_range) IS DISTINCT FROM 2 OR m.line_range[1] IS NULL OR m.line_range[2] IS NULL OR m.line_range[1]<1 OR m.line_range[1]>m.line_range[2] OR m.line_range[2]>s.lines) AS invalid_line_ranges
FROM macro_scopes m JOIN inventory i ON i.path=m.logical_path
LEFT JOIN source_info s ON s.logical_path=m.logical_path WHERE i.suffix='.rs' '''),
('invocation_owner_references', '''SELECT count(*) AS invocations,
count(*) FILTER(WHERE caller_stable_symbol_id IS NOT NULL) AS nonnull_caller_ids,
count(*) FILTER(WHERE caller_stable_symbol_id IS NOT NULL AND NOT EXISTS(
SELECT 1 FROM nodes n WHERE n.logical_path=m.logical_path AND n.stable_key=m.caller_stable_symbol_id)) AS caller_ids_without_matching_node
FROM macro_invocations m JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs' '''),
('scope_owner_references', '''SELECT count(*) AS scopes,count(*) FILTER(WHERE NOT EXISTS(
SELECT 1 FROM nodes n WHERE n.logical_path=m.logical_path AND n.stable_key=m."scope".stable_symbol_id)) AS scope_ids_without_matching_node
FROM macro_scopes m JOIN inventory i ON i.path=m.logical_path WHERE i.suffix='.rs' '''),
('definition_reference_integrity', source_cte + ''' SELECT count(*) AS definition_candidates,
count(*) FILTER(WHERE s.logical_path IS NULL) AS definition_sources_absent,
count(*) FILTER(WHERE m.resolution.definition.source_oid IS DISTINCT FROM s.file_oid) AS definition_oid_mismatches,
count(*) FILTER(WHERE array_length(m.resolution.definition.byte_range) IS DISTINCT FROM 2 OR m.resolution.definition.byte_range[1] IS NULL OR m.resolution.definition.byte_range[2] IS NULL OR m.resolution.definition.byte_range[1]>m.resolution.definition.byte_range[2] OR m.resolution.definition.byte_range[2]>s.bytes) AS invalid_definition_byte_ranges,
count(*) FILTER(WHERE array_length(m.resolution.definition.line_range) IS DISTINCT FROM 2 OR m.resolution.definition.line_range[1] IS NULL OR m.resolution.definition.line_range[2] IS NULL OR m.resolution.definition.line_range[1]<1 OR m.resolution.definition.line_range[1]>m.resolution.definition.line_range[2] OR m.resolution.definition.line_range[2]>s.lines) AS invalid_definition_line_ranges
FROM macro_invocations m JOIN inventory i ON i.path=m.logical_path
LEFT JOIN source_info s ON s.logical_path=m.resolution.definition.file_path
WHERE i.suffix='.rs' AND m.resolution.definition IS NOT NULL''')]
results = {}
receipts = []
for name,sql in queries:
    cur=c.execute(sql);row=dict(zip([x[0] for x in cur.description],cur.fetchone()))
    results[name]=row;receipts.append({'name':name,'sql':sql,'params':[],'result':row})
report={'results':results,'queries':receipts,
        'scope':'Diagnostics of explicit metadata references only. A present source/owner/definition candidate does not establish compiler expansion or semantic binding correctness.'}
(art/'macro-metadata-audit.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(results,indent=2));c.close()
