import collections
import fnmatch
import hashlib
import json
import pathlib
import subprocess
import tomllib

OUT = pathlib.Path(__file__).resolve().parent
ROOT = pathlib.Path('/Users/kevintruong/project/SpurCode')
ART = ROOT / 'docs/research/artifacts/2026-11-15-rust-binding-gap'
SNAP = ROOT / 'docs/research/artifacts/2026-11-15-spurcode-project-quality/snapshot.duckdb'
REV = '61ba3d200439a541dce4c1617ca9277be7d438af'
import duckdb
assert duckdb.__version__ == '2.0.0'
c = duckdb.connect(':memory:', config={'threads':'2','memory_limit':'1GB','temp_directory':str(OUT/'spill')})
c.execute('ATTACH ? AS frozen (READ_ONLY)', [str(SNAP)])
receipts=[]
def q(name,sql,params=None):
    result=c.execute(sql,params or [])
    rows=[dict(zip([d[0] for d in result.description],r)) for r in result.fetchall()]
    receipts.append(dict(name=name,sql=sql,params=params or [],rows=rows))
    (OUT/'recomputed.json').write_text(json.dumps(receipts,indent=2)+'\n')
    return rows

q('schema_first',"SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' ORDER BY table_name,ordinal_position")
q('runtime', 'SELECT version() AS version')
q('raw_counts',"SELECT count(*) AS calls,count(DISTINCT (logical_path,edge_id)) AS unique_keys,count(*) FILTER(WHERE target_node_id IS NULL) AS unresolved,count(*) FILTER(WHERE target_node_id IS NOT NULL) AS local_targets,100.0*count(*) FILTER(WHERE target_node_id IS NOT NULL)/count(*) AS local_target_pct FROM frozen.main.edges WHERE relation='calls' AND ends_with(logical_path,'.rs')")
q('hint_counts',"SELECT count(*) FILTER(WHERE receiver_text IS NOT NULL) AS receiver,count(*) FILTER(WHERE scope_text IS NOT NULL) AS scope,count(*) FILTER(WHERE contains(target_label,'::')) AS qualified,count(*) FILTER(WHERE target_label IS NULL) AS null_labels,count(*) FILTER(WHERE receiver_text IS NOT NULL AND trim(receiver_text)='') AS empty_receiver,count(*) FILTER(WHERE scope_text IS NOT NULL AND trim(scope_text)='') AS empty_scope FROM frozen.main.edges WHERE relation='calls' AND ends_with(logical_path,'.rs') AND target_node_id IS NULL")
q('evidence_partition',"SELECT CASE WHEN contains(target_label,'::') THEN 'qualified' WHEN length(trim(scope_text))>0 THEN 'scope' WHEN length(trim(receiver_text))>0 THEN 'receiver' ELSE 'bare' END AS evidence,count(*) AS sites FROM frozen.main.edges WHERE relation='calls' AND ends_with(logical_path,'.rs') AND target_node_id IS NULL GROUP BY 1 ORDER BY 1")
def git(*args):
    return subprocess.check_output(['git',*args],cwd=ROOT)
paths=git('ls-tree','-r','--name-only',REV).decode().splitlines()
manifest_paths=[p for p in paths if p=='Cargo.toml' or p.endswith('/Cargo.toml')]
manifest_bytes={p:git('show',REV+':'+p) for p in manifest_paths}
manifests={p:tomllib.loads(raw.decode()) for p,raw in manifest_bytes.items()}
workspace=manifests['Cargo.toml'].get('workspace',{})
packages=[]
for path,m in manifests.items():
    if 'package' not in m: continue
    directory=str(pathlib.PurePosixPath(path).parent)
    aliases=set()
    for s in [m,*m.get('target',{}).values()]:
        for kind in ('dependencies','dev-dependencies','build-dependencies'):
            aliases.update(k.replace('-','_') for k in s.get(kind,{}))
    member=directory=='.' or (any(fnmatch.fnmatchcase(directory,p) for p in workspace.get('members',[])) and not any(fnmatch.fnmatchcase(directory,p) for p in workspace.get('exclude',[])))
    packages.append(dict(directory=directory,package=m['package']['name'],crate_root=m.get('lib',{}).get('name',m['package']['name'].replace('-','_')),workspace_member=member,dependency_aliases=sorted(aliases)))
packages.sort(key=lambda x:(-len(x['directory']),x['directory']))
saved=json.loads((ART/'package-catalog.json').read_text())
manifest_hashes={p:hashlib.sha256(raw).hexdigest() for p,raw in manifest_bytes.items()}
assert manifest_hashes==saved['manifest_sha256']
assert packages==saved['packages']
(OUT/'independent-package-catalog.json').write_text(json.dumps(dict(revision=REV,manifest_sha256=manifest_hashes,packages=packages),indent=2)+'\n')
rs_paths=[r[0] for r in c.execute("SELECT DISTINCT path FROM frozen.main.inventory WHERE suffix='.rs'").fetchall()]
mapping=[]
for path in rs_paths:
    ancestors=[p for p in packages if p['directory']=='.' or path.startswith(p['directory']+'/')]
    nearest=max(ancestors,key=lambda p:len(p['directory']),default=None)
    mapping.append((path,nearest['package'] if nearest else None))
c.execute('CREATE TEMP TABLE package_paths(path VARCHAR PRIMARY KEY,package VARCHAR)')
c.executemany('INSERT INTO package_paths VALUES (?,?)',mapping)
q('package_sanity',"SELECT count(*) AS inventory_paths,count(DISTINCT path) AS unique_paths,count(*) FILTER(WHERE package IS NULL) AS unknown_paths FROM package_paths")
q('unmapped_calls',"SELECT count(*) AS missing_inventory FROM frozen.main.edges e LEFT JOIN package_paths p ON p.path=e.logical_path WHERE relation='calls' AND ends_with(e.logical_path,'.rs') AND p.path IS NULL")
c.execute("CREATE TEMP TABLE callable AS SELECT n.logical_path,n.node_id,n.label,n.kind,p.package FROM frozen.main.nodes n LEFT JOIN package_paths p ON n.logical_path=p.path WHERE ends_with(n.logical_path,'.rs') AND n.kind IN ('function','method')")
c.execute("CREATE TEMP TABLE unresolved AS SELECT e.*,p.package,list_last(string_split(e.target_label,'::')) AS leaf FROM frozen.main.edges e LEFT JOIN package_paths p ON e.logical_path=p.path WHERE e.relation='calls' AND ends_with(e.logical_path,'.rs') AND e.target_node_id IS NULL")
q('package_unknown_calls',"SELECT count(*) AS calls,count(*) FILTER(WHERE package IS NULL) AS unknown_package FROM frozen.main.edges e LEFT JOIN package_paths p ON e.logical_path=p.path WHERE e.relation='calls' AND ends_with(e.logical_path,'.rs')")
for scope,keys,on in [('file','logical_path,label','u.logical_path=n.logical_path AND u.leaf=n.label'),('package','package,label','u.package=n.package AND u.leaf=n.label'),('global','label','u.leaf=n.label')]:
    c.execute(f'CREATE TEMP TABLE {scope}_names AS SELECT {keys},count(*) AS candidates FROM callable GROUP BY {keys}')
    c.execute(f'CREATE TEMP TABLE {scope}_candidates AS SELECT u.logical_path,u.edge_id,u.receiver_text,coalesce(n.candidates,0) AS candidates FROM unresolved u LEFT JOIN {scope}_names n ON {on}')
    rows=q(scope+'_multiplicity',f"SELECT CASE WHEN candidates=0 THEN 'zero' WHEN candidates=1 THEN 'one' ELSE 'multiple' END AS multiplicity,count(*) AS sites,sum(candidates) AS pairs FROM {scope}_candidates GROUP BY 1 ORDER BY 1")
    assert sum(r['sites'] for r in rows)==227414
    q(scope+'_row_conservation',f'SELECT count(*) AS rows,count(DISTINCT (logical_path,edge_id)) AS keys FROM {scope}_candidates')
q('fanout',"SELECT count(*) AS input_sites,count(*) FILTER(WHERE candidates=0) AS dropped,sum(candidates) AS pairs,max(candidates) AS maximum_fanout,count(*) FILTER(WHERE candidates=1 AND receiver_text IS NOT NULL) AS singleton_with_receiver FROM global_candidates")
c.execute("CREATE TEMP TABLE owners AS SELECT DISTINCT n.logical_path,n.node_id,n.label,n.package,parent.label AS owner_label,parent.kind AS owner_kind FROM callable n JOIN frozen.main.edges e ON n.logical_path=e.logical_path AND n.node_id=e.target_node_id AND e.relation='contains' JOIN frozen.main.nodes parent ON parent.logical_path=e.logical_path AND parent.node_id=e.source_node_id WHERE parent.kind IN ('impl','trait')")
c.execute('CREATE TEMP TABLE owner_names AS SELECT package,owner_label,label,count(*) AS candidates FROM owners GROUP BY package,owner_label,label')
c.execute('CREATE TEMP TABLE owner_candidates AS SELECT u.*,coalesce(n.candidates,0) AS candidates FROM unresolved u LEFT JOIN owner_names n ON n.package=u.package AND n.owner_label=u.scope_text AND n.label=u.target_label')
owner=q('owner_multiplicity',"SELECT CASE WHEN candidates=0 THEN 'zero' WHEN candidates=1 THEN 'one' ELSE 'multiple' END AS multiplicity,count(*) AS sites,sum(candidates) AS pairs FROM owner_candidates GROUP BY 1 ORDER BY 1")
assert sum(r['sites'] for r in owner)==227414
q('owner_conservation','SELECT count(*) AS rows,count(DISTINCT (logical_path,edge_id)) AS keys FROM owner_candidates')
c.execute('CREATE TEMP TABLE dependency_roots(package VARCHAR,alias VARCHAR)')
c.executemany('INSERT INTO dependency_roots VALUES (?,?)',[(p['package'],d) for p in packages for d in p['dependency_aliases']])
c.execute('CREATE TEMP TABLE workspace_roots(alias VARCHAR)')
c.executemany('INSERT INTO workspace_roots VALUES (?)',[(p['crate_root'],) for p in packages if p['workspace_member']])
q('spelling_partition',"WITH hints AS (SELECT *,split_part(coalesce(nullif(scope_text,''),CASE WHEN contains(target_label,'::') THEN target_label END),'::',1) AS root FROM unresolved) SELECT CASE WHEN root IN ('std','core','alloc') THEN 'std_core_alloc_spelling' WHEN root='crate' THEN 'crate_spelling' WHEN root IN (SELECT alias FROM workspace_roots) THEN 'workspace_crate_spelling' WHEN EXISTS(SELECT 1 FROM dependency_roots d WHERE d.package=h.package AND d.alias=h.root) THEN 'declared_dependency_alias_spelling' WHEN root IS NOT NULL THEN 'other_scope_or_qualified_spelling' ELSE 'no_scope_or_qualified_spelling' END AS evidence,count(*) AS sites FROM hints h GROUP BY 1 ORDER BY 1")
q('null_target_hints_top',"SELECT target_label,count(*) AS sites FROM unresolved GROUP BY 1 ORDER BY 2 DESC,1 LIMIT 30")
q('owner_positive_samples',"SELECT logical_path,edge_id,target_label,receiver_text,scope_text,package,candidates FROM owner_candidates WHERE logical_path='crates/spur-acp/examples/compat_spike.rs' AND candidates=1 ORDER BY edge_id LIMIT 12")
q('singleton_candidate_targets',"SELECT u.logical_path,u.edge_id,u.target_label,u.receiver_text,u.scope_text,n.logical_path AS candidate_path,n.node_id AS candidate_node,n.label FROM unresolved u JOIN global_candidates g USING(logical_path,edge_id) JOIN callable n ON n.label=u.leaf WHERE g.candidates=1 AND u.logical_path='crates/spur-acp/examples/compat_spike.rs' AND u.target_label IN ('skip','args') ORDER BY edge_id")
q('macro_piece_nodes',"SELECT n.logical_path,n.node_id,n.label,n.kind,s.start_byte,s.end_byte,s.start_line,s.end_line FROM frozen.main.nodes n JOIN frozen.main.spans s ON s.logical_path=n.logical_path AND s.span_id=n.source_span_id WHERE (n.logical_path='crates/spur-graph/src/schema.rs' AND n.label='RelationKind' AND n.kind='enum') OR (n.logical_path='crates/spur-graph-duckdb/src/file_facts.rs' AND n.label IN ('NativeLabel','labels')) ORDER BY n.logical_path,n.node_id")
q('labels_macro_invocations',"SELECT logical_path,raw_macro_path,normalized_macro_path,byte_range,line_range,resolution FROM frozen.main.macro_invocations WHERE logical_path='crates/spur-graph-duckdb/src/file_facts.rs' AND normalized_macro_path='labels' ORDER BY byte_range[1]")
c.close()
for r in receipts:
    if r['name'] not in ('schema_first',):print(r['name'],json.dumps(r['rows']))
print('PACKAGE_COUNT',len(packages),'MANIFEST_COUNT',len(manifest_paths))
