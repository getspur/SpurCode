"""Before/after checks of one shared native extractor, through fresh DuckDB2 SQL."""
import argparse
import hashlib
import json
from pathlib import Path

import duckdb

ART = Path(__file__).resolve().parent
SNAP = ART.parent/'2026-11-15-spurcode-project-quality/snapshot.duckdb'
BASELINE_SHA = 'e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028'
SOURCE_SHA = 'c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
parser = argparse.ArgumentParser()
parser.add_argument('--extension', type=Path, required=True)
parser.add_argument('--mode', choices=['baseline', 'fixed'], required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
assert duckdb.__version__ == '2.0.0'
assert sha(SNAP) == SOURCE_SHA
extension_sha = sha(args.extension)
assert (extension_sha == BASELINE_SHA) == (args.mode == 'baseline')
c = duckdb.connect(config={'allow_unsigned_extensions':'true','threads':'2','memory_limit':'1GB'})
c.execute("LOAD '"+str(args.extension.resolve()).replace("'","''")+"'")
c.execute("ATTACH '"+str(SNAP).replace("'","''")+"' AS frozen (READ_ONLY)")
receipts = []


def q(name, sql, params=None):
    params = params or []
    cur = c.execute(sql, params)
    rows = [dict(zip([d[0] for d in cur.description], r)) for r in cur.fetchall()]
    receipts.append(dict(name=name, sql=sql, params=params, rows=rows))
    return rows


q('schema', "SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('facts','sources','edges','inventory') ORDER BY table_name,ordinal_position")
q('runtime', 'SELECT version() AS version')
q('loaded_extension', "SELECT extension_name,loaded,extension_version FROM duckdb_extensions() WHERE extension_name='spur_graph'")
source = (ART/'control.rs').read_text()
c.execute('CREATE TEMP TABLE control AS SELECT spur_graph_file_facts(?,?) AS value', ['control.rs', source])
q('full_control_facts', 'SELECT value FROM control')
calls = q('control_calls', """WITH nodes AS (SELECT unnest(value.nodes) AS n FROM control),
edges AS (SELECT unnest(value.edges) AS e FROM control)
SELECT src.n.label AS caller, e.e.target_label, e.e.target_node_id, dst.n.label AS target_name,
owner.n.label AS target_owner, e.e.scope_text, e.e.receiver_text, e.e.bind_method, e.e.confidence
FROM edges e LEFT JOIN nodes src ON src.n.node_id=e.e.source_node_id
LEFT JOIN nodes dst ON dst.n.node_id=e.e.target_node_id
LEFT JOIN edges parent ON parent.e.target_node_id=dst.n.node_id AND parent.e.relation='contains'
LEFT JOIN nodes owner ON owner.n.node_id=parent.e.source_node_id
WHERE e.e.relation='calls' AND src.n.label IN ('observed_macro','observed_helper','observed_drop','observed_local_macro','observed_mixed_macro')
ORDER BY caller,e.e.target_label,e.e.scope_text NULLS FIRST""")


def matching(caller, leaf):
    return [r for r in calls if r['caller']==caller and r['target_label'].split('::')[-1]==leaf]


assert matching('observed_helper','helper')
assert all(r['target_name']=='helper' for r in matching('observed_helper','helper'))
assert matching('observed_drop','drop')
assert all(r['target_node_id'] is None for r in matching('observed_drop','drop'))
assert matching('observed_local_macro','new')
assert any(r['target_owner']=='LocalFactory' for r in matching('observed_local_macro','new'))
assert any(r['target_name']=='helper' for r in matching('observed_mixed_macro','helper'))
if args.mode == 'baseline':
    assert any(r['target_owner']=='LocalFactory' for r in matching('observed_macro','new'))
else:
    assert matching('observed_macro','new'), 'Retain the unresolved qualified call, do not drop evidence'
    assert all(r['target_node_id'] is None for r in matching('observed_macro','new'))
    mixed_new = matching('observed_mixed_macro','new')
    assert any(r['target_node_id'] is None for r in mixed_new), 'Retain Vec turbofish call'
    assert any(r['target_owner']=='LocalFactory' for r in mixed_new), 'Keep local constructor in same macro'

# Exhaust the constructor-review population, plus the previous six contract files.
c.execute("""CREATE TEMP TABLE selected_paths AS
SELECT DISTINCT e.logical_path FROM frozen.main.edges e
JOIN frozen.main.inventory i ON i.path=e.logical_path
WHERE i.suffix='.rs' AND e.relation='calls' AND e.bind_method='macro_body_singleton' AND e.target_label='new'
UNION SELECT logical_path FROM frozen.main.sources WHERE logical_path IN (
'crates/spur-graph-duckdb/src/lib.rs','crates/spur-graph-duckdb/src/file_facts.rs',
'crates/spur-graph/src/extract/tree_sitter.rs','crates/spur-acp/src/agent_model_catalog.rs',
'crates/spur-graph/src/schema.rs','crates/spur-graph/src/mcp/mod.rs')""")
q('selection', "SELECT count(*) AS files,(SELECT count(*) FROM frozen.main.edges e JOIN frozen.main.inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='calls' AND e.bind_method='macro_body_singleton' AND e.target_label='new') AS original_macro_constructor_edges FROM selected_paths")
c.execute('CREATE TEMP TABLE fresh AS SELECT s.logical_path,spur_graph_file_facts(s.logical_path,s.source) AS value FROM frozen.main.sources s JOIN selected_paths p USING(logical_path)')
parity = q('per_file_parity', """SELECT f.logical_path,
f.value.nodes IS NOT DISTINCT FROM old.value.nodes AS nodes_same,
f.value.spans IS NOT DISTINCT FROM old.value.spans AS spans_same,
f.value.macro_invocations IS NOT DISTINCT FROM old.value.macro_invocations AS invocations_same,
f.value.macro_scopes IS NOT DISTINCT FROM old.value.macro_scopes AS scopes_same,
f.value.edges IS NOT DISTINCT FROM old.value.edges AS edges_same,
len(f.value.edges) AS new_edges,len(old.value.edges) AS old_edges
FROM fresh f JOIN frozen.main.facts old USING(logical_path) ORDER BY f.logical_path""")
assert parity and all(all(r[k] for k in ['nodes_same','spans_same','invocations_same','scopes_same']) for r in parity)
if args.mode == 'baseline':
    assert all(r['edges_same'] for r in parity)
counts = q('selected_call_counts', """WITH old_edges AS (
SELECT e.* FROM frozen.main.edges e JOIN selected_paths p USING(logical_path)),
new_edges AS (SELECT f.logical_path,unnest(f.value.edges) AS e FROM fresh f)
SELECT 'baseline' AS version,count(*) AS calls,count(*) FILTER(WHERE target_node_id IS NOT NULL) AS local_targets
FROM old_edges WHERE relation='calls'
UNION ALL SELECT 'candidate',count(*),count(*) FILTER(WHERE e.target_node_id IS NOT NULL) FROM new_edges WHERE e.relation='calls'""")
real = q('real_constructor_control', """WITH f AS (
SELECT value FROM fresh WHERE logical_path='crates/spur-acp/src/capability_evidence.rs'),
nodes AS (SELECT unnest(value.nodes) AS n FROM f),edges AS (SELECT unnest(value.edges) AS e FROM f)
SELECT src.n.label AS caller,e.e.target_label,e.e.target_node_id,dst.n.label AS target_name,
owner.n.label AS target_owner,e.e.scope_text,e.e.bind_method
FROM edges e JOIN nodes src ON src.n.node_id=e.e.source_node_id
LEFT JOIN nodes dst ON dst.n.node_id=e.e.target_node_id
LEFT JOIN edges parent ON parent.e.target_node_id=dst.n.node_id AND parent.e.relation='contains'
LEFT JOIN nodes owner ON owner.n.node_id=parent.e.source_node_id
WHERE e.e.relation='calls' AND src.n.label='rejected_evidence_demotes_while_inconclusive_evidence_preserves_prior_routes'
AND regexp_extract(e.e.target_label,'([^:]+)$',1)='new' ORDER BY e.e.scope_text NULLS FIRST""")
assert real
if args.mode == 'baseline':
    assert any(r['target_owner']=='EvidenceEpoch' for r in real)
else:
    assert all(r['target_node_id'] is None for r in real), real
assert sha(SNAP) == SOURCE_SHA
result = dict(mode=args.mode,duckdb_version=duckdb.__version__,extension=str(args.extension.resolve()),
              extension_sha256=extension_sha,snapshot_sha256=SOURCE_SHA,control_sha256=sha(ART/'control.rs'),
              validator_sha256=sha(__file__),receipts=receipts,all_checks_passed=True,
              limits='Selected files plus controlled fixtures; no full-corpus precision, recall or performance claim.')
args.output.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(dict(mode=args.mode,all_checks_passed=True,files=len(parity),
                     changed_edge_lists=sum(not r['edges_same'] for r in parity),counts=counts,control_calls=calls),separators=(',',':')))
c.close()
