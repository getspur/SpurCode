"""Schema-first bounded pilot over immutable Rust facts and native2 functions."""
import hashlib
import json
from pathlib import Path
import duckdb

ART = Path(__file__).resolve().parent
SNAP = ART.parent / '2026-11-15-spurcode-project-quality/snapshot.duckdb'
EXT = ART.parent / '2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
assert duckdb.__version__=='2.0.0'
assert hashlib.sha256(SNAP.read_bytes()).hexdigest()=='c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
assert hashlib.sha256(EXT.read_bytes()).hexdigest()=='e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028'
c=duckdb.connect(config={'allow_unsigned_extensions':'true','threads':'2','memory_limit':'1GB'})
before_surface=c.execute("SELECT function_name FROM duckdb_functions() WHERE function_name LIKE 'spur_graph%' ORDER BY function_name").fetchall()
c.execute("SET temp_directory='"+str(ART/'spill').replace("'","''")+"'")
c.execute("LOAD '"+str(EXT).replace("'","''")+"'")
c.execute("ATTACH '"+str(SNAP).replace("'","''")+"' AS frozen (READ_ONLY)")
for name in ['inventory','sources','nodes','edges','spans','macro_invocations','macro_scopes']:
    c.execute(f'CREATE TEMP VIEW {name} AS SELECT * FROM frozen.main.{name}')
receipts=[]
receipts.append({'name':'native_surface_before_load','sql':"SELECT function_name FROM duckdb_functions() WHERE function_name LIKE 'spur_graph%' ORDER BY function_name",'params':[],'rows':[{'function_name':r[0]} for r in before_surface]})


def q(name,sql):
    cur=c.execute(sql);names=[d[0] for d in cur.description]
    rows=[dict(zip(names,row)) for row in cur.fetchall()]
    receipts.append({'name':name,'sql':sql,'params':[],'rows':rows})
    return rows


q('schema_first',"SELECT table_catalog,table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('nodes','edges','spans','inventory','sources') ORDER BY table_name,ordinal_position")
q('runtime', 'SELECT version() AS engine_version')
q('native_extension_function_surface',"SELECT function_name,function_type,parameters,parameter_types,return_type FROM duckdb_functions() WHERE function_name LIKE 'spur_graph%' ORDER BY function_name")
q('native_loaded_extensions',"SELECT extension_name,loaded,installed,extension_version FROM duckdb_extensions() WHERE loaded ORDER BY extension_name")
c.execute("CREATE TEMP VIEW rust_edges AS SELECT e.*,i.area FROM edges e JOIN inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='calls'")
q('call_partition', '''SELECT count(*) AS calls,
count(*) FILTER(WHERE target_node_id IS NOT NULL) AS local_targets,
count(*) FILTER(WHERE target_node_id IS NULL) AS unresolved,
count(*) FILTER(WHERE target_node_id IS NULL AND target_label IS NULL) AS unresolved_without_label,
count(*) FILTER(WHERE target_node_id IS NULL AND receiver_text IS NOT NULL AND length(trim(receiver_text))>0) AS unresolved_receiver_evidence,
count(*) FILTER(WHERE target_node_id IS NULL AND target_label LIKE '%::%') AS unresolved_qualified_label,
count(*) FILTER(WHERE target_node_id IS NULL AND import_path IS NOT NULL) AS unresolved_import_path,
count(*) FILTER(WHERE target_node_id IS NULL AND scope_text IS NOT NULL) AS unresolved_scope_text
FROM rust_edges''')
q('unresolved_top_labels_limit_30',"SELECT target_label,count(*) AS sites FROM rust_edges WHERE target_node_id IS NULL GROUP BY target_label ORDER BY sites DESC,target_label LIMIT 30")
q('qualified_roots_limit_30',"SELECT split_part(target_label,'::',1) AS root,count(*) AS sites FROM rust_edges WHERE target_node_id IS NULL AND target_label LIKE '%::%' GROUP BY root ORDER BY sites DESC,root LIMIT 30")
q('receiver_scope_examples_limit_20',"SELECT logical_path,edge_id,source_node_id,target_label,receiver_text,scope_text,confidence,bind_method,import_path FROM rust_edges WHERE target_node_id IS NULL ORDER BY hash(logical_path,edge_id) LIMIT 20")
q('rust_callable_kinds',"SELECT n.kind,count(*) AS nodes FROM nodes n JOIN inventory i ON i.path=n.logical_path WHERE i.suffix='.rs' GROUP BY n.kind ORDER BY nodes DESC")
q('callable_owner_examples_limit_20',"SELECT n.logical_path,n.node_id,n.label,n.kind,p.label AS parent_label,p.kind AS parent_kind FROM nodes n JOIN inventory i ON i.path=n.logical_path LEFT JOIN edges e ON e.logical_path=n.logical_path AND e.target_node_id=n.node_id AND e.relation='contains' LEFT JOIN nodes p ON p.logical_path=e.logical_path AND p.node_id=e.source_node_id WHERE i.suffix='.rs' AND n.kind IN ('function','method') ORDER BY hash(n.logical_path,n.node_id) LIMIT 20")
q('import_examples_limit_20',"SELECT e.logical_path,e.target_label,e.import_path,e.scope_text,count(*) AS sites FROM edges e JOIN inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='imports' GROUP BY ALL ORDER BY sites DESC,e.logical_path,e.target_label LIMIT 20")
q('enum_relation_kind_source',"SELECT n.logical_path,n.node_id,n.label,n.kind,p.start_byte,p.end_byte,p.start_line,p.end_line,substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,2*CAST(p.end_byte-p.start_byte AS BIGINT)) AS source_hex FROM nodes n JOIN spans p ON p.logical_path=n.logical_path AND p.span_id=n.source_span_id JOIN sources s ON s.logical_path=n.logical_path WHERE n.label='RelationKind' AND n.kind='enum' ORDER BY n.logical_path,n.node_id LIMIT 10")
(ART/'binding-pilot.json').write_text(json.dumps(receipts,indent=2,default=str)+'\n')
print(json.dumps({r['name']:r['rows'] for r in receipts if r['name'] in ['runtime','call_partition','rust_callable_kinds']},indent=2))
c.close()
