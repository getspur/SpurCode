"""Additional SQL evidence: exact lexical owner hints remain candidates, never bindings."""
import hashlib
import json
from pathlib import Path
import duckdb

ART = Path(__file__).resolve().parent
SNAP = ART.parent/'2026-11-15-spurcode-project-quality/snapshot.duckdb'
assert duckdb.__version__ == '2.0.0'
assert hashlib.sha256(SNAP.read_bytes()).hexdigest() == 'c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
c = duckdb.connect(config={'threads':'2','memory_limit':'1GB'})
c.execute("ATTACH '"+str(SNAP).replace("'","''")+"' AS frozen (READ_ONLY)")
receipts = []


def q(name, sql):
    cur = c.execute(sql)
    rows = [dict(zip([d[0] for d in cur.description],r)) for r in cur.fetchall()]
    receipts.append(dict(name=name, sql=sql, params=[], rows=rows))
    return rows


q('schema_first', "SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('nodes','edges','macro_invocations') ORDER BY table_name,ordinal_position")
packages = json.loads((ART/'package-catalog.json').read_text())['packages']
paths = [r[0] for r in c.execute("SELECT path FROM frozen.main.inventory WHERE suffix='.rs'").fetchall()]
mapping = [(path,next((p['package'] for p in packages if p['directory']=='.' or path.startswith(p['directory']+'/')),None)) for path in paths]
c.execute('CREATE TEMP TABLE package_paths(path VARCHAR PRIMARY KEY,package VARCHAR)')
c.executemany('INSERT INTO package_paths VALUES (?,?)', mapping)
c.execute("""CREATE TEMP TABLE owners AS SELECT DISTINCT n.logical_path,n.node_id,n.label,p.package,parent.label AS owner_label,parent.kind AS owner_kind
FROM frozen.main.nodes n JOIN package_paths p ON p.path=n.logical_path
JOIN frozen.main.edges e ON e.logical_path=n.logical_path AND e.target_node_id=n.node_id AND e.relation='contains'
JOIN frozen.main.nodes parent ON parent.logical_path=e.logical_path AND parent.node_id=e.source_node_id
WHERE n.kind IN ('function','method') AND parent.kind IN ('impl','trait')""")
c.execute('CREATE TEMP TABLE owner_counts AS SELECT package,owner_label,label,count(*) AS n FROM owners GROUP BY ALL')
c.execute("""CREATE TEMP TABLE calls AS SELECT e.*,p.package,coalesce(o.n,0) AS owner_candidates
FROM frozen.main.edges e JOIN package_paths p ON p.path=e.logical_path
LEFT JOIN owner_counts o ON o.package=p.package AND o.owner_label=e.scope_text AND o.label=e.target_label
WHERE e.relation='calls' AND e.target_node_id IS NULL""")
q('exact_owner_hint_candidates', "SELECT CASE WHEN owner_candidates=0 THEN 'zero' WHEN owner_candidates=1 THEN 'one' ELSE 'multiple' END AS multiplicity,count(*) AS sites,sum(owner_candidates) AS pairs FROM calls GROUP BY 1 ORDER BY 1")
c.execute('CREATE TEMP TABLE owner_sample AS SELECT * FROM calls WHERE owner_candidates=1 ORDER BY logical_path,edge_id LIMIT 12')
q('exact_owner_hint_samples_limit_12', """SELECT e.logical_path,e.edge_id,e.target_label,e.receiver_text,e.scope_text,e.owner_candidates,n.label AS caller,p.start_line,p.end_line,
substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,2*CAST(least(p.end_byte-p.start_byte,6000) AS BIGINT)) AS caller_source_hex
FROM owner_sample e JOIN frozen.main.nodes n ON n.logical_path=e.logical_path AND n.node_id=e.source_node_id
JOIN frozen.main.spans p ON p.logical_path=n.logical_path AND p.span_id=n.source_span_id
JOIN frozen.main.sources s ON s.logical_path=e.logical_path
ORDER BY e.logical_path,e.edge_id LIMIT 12""")
q('owner_candidate_top_15', "SELECT scope_text,target_label,count(*) AS sites FROM calls WHERE owner_candidates=1 GROUP BY 1,2 ORDER BY sites DESC,scope_text,target_label LIMIT 15")
c.execute("CREATE TEMP TABLE crate_sample AS SELECT * FROM calls WHERE target_label LIKE 'crate::%' ORDER BY logical_path,edge_id LIMIT 12")
q('explicit_crate_paths_limit_12', """SELECT e.logical_path,e.edge_id,e.target_label,e.receiver_text,e.scope_text,n.label AS caller,p.start_line,p.end_line,
substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,2*CAST(least(p.end_byte-p.start_byte,6000) AS BIGINT)) AS caller_source_hex
FROM crate_sample e JOIN frozen.main.nodes n ON n.logical_path=e.logical_path AND n.node_id=e.source_node_id
JOIN frozen.main.spans p ON p.logical_path=n.logical_path AND p.span_id=n.source_span_id
JOIN frozen.main.sources s ON s.logical_path=e.logical_path
ORDER BY e.logical_path,e.edge_id LIMIT 12""")
# Exact same spelling only. Aliases/shadowing/features are not resolved by this classifier.
deps = [(p['package'],alias) for p in packages for alias in p['dependency_aliases']]
c.execute('CREATE TEMP TABLE dependency_roots(package VARCHAR,alias VARCHAR)')
c.executemany('INSERT INTO dependency_roots VALUES (?,?)', deps)
roots = [(p['crate_root'],) for p in packages if p['workspace_member']]
c.execute('CREATE TEMP TABLE workspace_roots(alias VARCHAR)')
c.executemany('INSERT INTO workspace_roots VALUES (?)', roots)
q('explicit_spelling_partition', """WITH hints AS (SELECT *,split_part(coalesce(nullif(scope_text,''),CASE WHEN target_label LIKE '%::%' THEN target_label ELSE NULL END),'::',1) AS root FROM calls)
SELECT CASE WHEN root IN ('std','core','alloc') THEN 'std_core_alloc_spelling'
WHEN root='crate' THEN 'crate_spelling'
WHEN root IN (SELECT alias FROM workspace_roots) THEN 'workspace_crate_spelling'
WHEN EXISTS(SELECT 1 FROM dependency_roots d WHERE d.package=h.package AND d.alias=h.root) THEN 'declared_dependency_alias_spelling'
WHEN root IS NOT NULL THEN 'other_scope_or_qualified_spelling' ELSE 'no_scope_or_qualified_spelling' END AS evidence,count(*) AS sites
FROM hints h GROUP BY 1 ORDER BY 1""")
(ART/'owner-analysis.json').write_text(json.dumps(receipts,indent=2)+'\n')
assert hashlib.sha256(SNAP.read_bytes()).hexdigest() == 'c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
print(json.dumps({r['name']:r['rows'] for r in receipts if r['name'] in ['exact_owner_hint_candidates','explicit_spelling_partition']},separators=(',',':')))
c.close()
