"""Fresh-process extraction diagnostic for valid local macro qualifier variants."""
import argparse
import hashlib
import json
from pathlib import Path
import duckdb

ap=argparse.ArgumentParser()
ap.add_argument('--extension',type=Path,required=True)
ap.add_argument('--output',type=Path,required=True)
a=ap.parse_args()
art=Path(__file__).resolve().parent
source=(art/'scope-probes.rs').read_text()
assert duckdb.__version__=='2.0.0'
c=duckdb.connect(config={'allow_unsigned_extensions':'true'})
c.execute("LOAD '"+str(a.extension.resolve()).replace("'","''")+"'")
facts=c.execute('SELECT spur_graph_file_facts(?,?)',['scope-probes.rs',source]).fetchone()[0]
nodes={n['node_id']:n for n in facts['nodes']}
parents={e['target_node_id']:e['source_node_id'] for e in facts['edges'] if e['relation']=='contains'}
callers=['plain','spaced','block_before_separator','block_after_separator','line_before_separator','via_self']
rows=[]
for e in facts['edges']:
    caller=nodes[e['source_node_id']]['label']
    if e['relation']!='calls' or caller not in callers or e['target_label']!='new': continue
    target=nodes.get(e['target_node_id'])
    owner=nodes.get(parents.get(e['target_node_id']))
    rows.append(dict(caller=caller,scope=e['scope_text'],target=e['target_node_id'],target_name=target['label'] if target else None,owner=owner['label'] if owner else None,bind_method=e['bind_method']))
checks={caller:any(r['caller']==caller and r['owner']=='Factory' for r in rows) for caller in callers}
out=dict(duckdb_version=duckdb.__version__,extension=str(a.extension.resolve()),extension_sha256=hashlib.sha256(a.extension.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(source.encode()).hexdigest(),rows=rows,expected_local_constructor_preserved=checks,all_positive_controls_passed=all(checks.values()),facts=facts)
a.output.write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps({k:v for k,v in out.items() if k!='facts'},separators=(',',':')))
c.close()
