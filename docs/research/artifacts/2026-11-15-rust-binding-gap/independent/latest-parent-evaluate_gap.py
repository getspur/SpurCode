"""Reproduce candidate diagnostics and isolated compiler controls; never write frozen facts."""
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

import duckdb

ART = Path(__file__).resolve().parent
ROOT = ART.parents[3]
REV = '61ba3d200439a541dce4c1617ca9277be7d438af'
SNAP = ART.parent / '2026-11-15-spurcode-project-quality/snapshot.duckdb'
EXT = ART.parent / '2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
sha = lambda b: hashlib.sha256(b).hexdigest()
assert duckdb.__version__ == '2.0.0'
assert sha(SNAP.read_bytes()) == 'c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
assert sha(EXT.read_bytes()) == 'e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028'
c = duckdb.connect(config={'allow_unsigned_extensions':'true','threads':'2','memory_limit':'1GB'})
c.execute("LOAD '" + str(EXT).replace("'", "''") + "'")
c.execute("ATTACH '" + str(SNAP).replace("'", "''") + "' AS frozen (READ_ONLY)")
receipts = []


def q(name, sql, params=None):
    params = params or []
    cur = c.execute(sql, params)
    names = [d[0] for d in cur.description]
    rows = [dict(zip(names, r)) for r in cur.fetchall()]
    receipts.append(dict(name=name, sql=sql, params=params, rows=rows))
    return rows


def command(args, env=None):
    r = subprocess.run(args, cwd=ROOT, env=env, capture_output=True, text=True, timeout=120)
    commands.append(dict(args=args, environment_overrides={'RUSTC_BOOTSTRAP':'1'} if env else {},
                         returncode=r.returncode, stdout=r.stdout, stderr=r.stderr))
    assert r.returncode == 0, r.stderr
    return r.stdout


# Inventory was schema-checked by binding_pilot.py. Explicitly re-check consumed schemas.
q('schema_first', "SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_catalog='frozen' AND table_name IN ('nodes','edges','inventory','sources','spans') ORDER BY table_name,ordinal_position")
q('runtime', 'SELECT version() AS version')
commands = []
manifest_paths = command(['git','ls-tree','-r','--name-only',REV]).splitlines()
manifest_paths = [p for p in manifest_paths if p == 'Cargo.toml' or p.endswith('/Cargo.toml')]
manifests = {}
manifest_hashes = {}
for p in manifest_paths:
    raw = subprocess.check_output(['git','show',REV+':'+p], cwd=ROOT)
    manifests[p] = tomllib.loads(raw.decode())
    manifest_hashes[p] = sha(raw)
workspace = manifests['Cargo.toml'].get('workspace', {})
packages = []
for p, m in manifests.items():
    if 'package' not in m:
        continue
    directory = str(Path(p).parent)
    member = directory == '.' or (any(fnmatch.fnmatchcase(directory, x) for x in workspace.get('members', []))
              and not any(fnmatch.fnmatchcase(directory, x) for x in workspace.get('exclude', [])))
    aliases = set()
    sections = [m] + list(m.get('target', {}).values())
    for section in sections:
        for k in ['dependencies', 'dev-dependencies', 'build-dependencies']:
            aliases.update(x.replace('-', '_') for x in section.get(k, {}))
    packages.append(dict(directory=directory, package=m['package']['name'],
                         crate_root=m.get('lib', {}).get('name', m['package']['name'].replace('-', '_')),
                         workspace_member=member, dependency_aliases=sorted(aliases)))
packages.sort(key=lambda x: (-len(x['directory']), x['directory']))
paths = [r[0] for r in c.execute("SELECT path FROM frozen.main.inventory WHERE suffix='.rs'").fetchall()]
mapping = []
for path in paths:
    package = next((p for p in packages if p['directory'] == '.' or path.startswith(p['directory']+'/')), None)
    mapping.append((path, package['package'] if package else None))
c.execute('CREATE TEMP TABLE package_paths(path VARCHAR PRIMARY KEY, package VARCHAR)')
c.executemany('INSERT INTO package_paths VALUES (?,?)', mapping)
c.execute("CREATE TEMP VIEW calls AS SELECT e.*,p.package FROM frozen.main.edges e JOIN package_paths p ON p.path=e.logical_path WHERE e.relation='calls'")
c.execute("CREATE TEMP VIEW callables AS SELECT n.*,p.package FROM frozen.main.nodes n JOIN package_paths p ON p.path=n.logical_path WHERE kind IN ('function','method')")
c.execute('CREATE TEMP TABLE file_names AS SELECT logical_path,label,count(*) AS n FROM callables GROUP BY ALL')
c.execute('CREATE TEMP TABLE package_names AS SELECT package,label,count(*) AS n FROM callables GROUP BY ALL')
c.execute('CREATE TEMP TABLE global_names AS SELECT label,count(*) AS n FROM callables GROUP BY ALL')
c.execute("""CREATE TEMP TABLE features AS SELECT e.*, regexp_extract(target_label,'([^:]+)$',1) AS leaf,
coalesce(f.n,0) AS file_candidates,coalesce(p.n,0) AS package_candidates,coalesce(g.n,0) AS global_candidates
FROM calls e LEFT JOIN file_names f ON f.logical_path=e.logical_path AND f.label=regexp_extract(e.target_label,'([^:]+)$',1)
LEFT JOIN package_names p ON p.package=e.package AND p.label=regexp_extract(e.target_label,'([^:]+)$',1)
LEFT JOIN global_names g ON g.label=regexp_extract(e.target_label,'([^:]+)$',1)""")
q('row_conservation', "SELECT count(*) AS calls,count(DISTINCT (logical_path,edge_id)) AS unique_calls,count(*) FILTER(WHERE target_node_id IS NULL) AS unresolved,count(*) FILTER(WHERE package IS NULL) AS package_unknown FROM features")
for level in ['file', 'package', 'global']:
    q(level+'_candidate_multiplicity', f"""SELECT CASE WHEN {level}_candidates=0 THEN 'zero' WHEN {level}_candidates=1 THEN 'one' ELSE 'multiple' END AS multiplicity,
count(*) AS sites,sum({level}_candidates) AS candidate_pairs FROM features WHERE target_node_id IS NULL GROUP BY 1 ORDER BY 1""")
q('evidence_partition', """SELECT CASE WHEN target_label LIKE '%::%' THEN 'qualified_target_label'
WHEN scope_text IS NOT NULL AND length(trim(scope_text))>0 THEN 'scope_text_without_qualified_label'
WHEN receiver_text IS NOT NULL AND length(trim(receiver_text))>0 THEN 'receiver_without_scope_or_qualified_label'
ELSE 'bare_without_receiver_or_scope' END AS evidence,count(*) AS sites FROM features WHERE target_node_id IS NULL GROUP BY 1 ORDER BY 1""")
q('scope_top_30', "SELECT scope_text,count(*) AS sites FROM features WHERE target_node_id IS NULL AND scope_text IS NOT NULL GROUP BY 1 ORDER BY sites DESC,scope_text LIMIT 30")
q('name_join_fanout', "SELECT count(*) AS input_sites,count(*) FILTER(WHERE global_candidates=0) AS dropped_by_inner_join,sum(global_candidates) AS candidate_pairs,max(global_candidates) AS maximum_fanout FROM features WHERE target_node_id IS NULL")
q('no_candidate_top_20', "SELECT target_label,count(*) AS sites FROM features WHERE target_node_id IS NULL AND global_candidates=0 GROUP BY 1 ORDER BY sites DESC,target_label LIMIT 20")
q('single_candidate_with_receiver', "SELECT count(*) AS sites FROM features WHERE target_node_id IS NULL AND global_candidates=1 AND receiver_text IS NOT NULL")
q('package_coverage', "SELECT package,count(*) AS files FROM package_paths GROUP BY 1 ORDER BY package")
# Deterministic source-backed examples. One per evidence/candidate tier, bounded caller text.
examples = q('source_examples', """WITH ranked AS (SELECT *,row_number() OVER(PARTITION BY
CASE WHEN global_candidates=0 THEN 'zero' WHEN global_candidates=1 THEN 'one' ELSE 'multiple' END,
CASE WHEN target_label LIKE '%::%' THEN 'qualified' WHEN scope_text IS NOT NULL THEN 'scope' WHEN receiver_text IS NOT NULL THEN 'receiver' ELSE 'bare' END
ORDER BY logical_path,edge_id) AS rn FROM features WHERE target_node_id IS NULL)
SELECT e.logical_path,e.edge_id,e.target_label,e.receiver_text,e.scope_text,e.package,e.file_candidates,e.package_candidates,e.global_candidates,
n.label AS caller,p.start_line,p.end_line,
substring(hex(s.source_bytes),2*CAST(p.start_byte AS BIGINT)+1,2*CAST(least(p.end_byte-p.start_byte,6000) AS BIGINT)) AS caller_source_hex
FROM ranked e LEFT JOIN frozen.main.nodes n ON n.logical_path=e.logical_path AND n.node_id=e.source_node_id
LEFT JOIN frozen.main.spans p ON p.logical_path=n.logical_path AND p.span_id=n.source_span_id
LEFT JOIN frozen.main.sources s ON s.logical_path=e.logical_path WHERE rn=1 ORDER BY e.logical_path,e.edge_id""")
for e in examples:
    q('candidate_example_'+str(e['edge_id'])+'_'+e['logical_path'],
        'SELECT logical_path,node_id,label,kind,package FROM callables WHERE label=? ORDER BY logical_path,node_id LIMIT 5',
        [e['target_label'].split('::')[-1]])

# Freeze semantic expectations before compiling or extracting controls.
expectations = dict(receiver_stdout='3:999\n', receiver_mir_targets=['Vec::<u8>::len','Domain::len'],
                    generated_label_bodies_before=0, generated_label_bodies_after=1,
                    limits='Isolated controls, not a workspace compiler run or accuracy estimate.')
(ART/'compiler-expectations.json').write_text(json.dumps(expectations, indent=2)+'\n')
fixture = '''struct Domain;
impl Domain { fn len(&self) -> usize { 999 } }
fn observed(v: Vec<u8>) -> usize { v.len() }
fn expected(d: Domain) -> usize { d.len() }
fn main() { println!("{}:{}", observed(vec![1,2,3]), expected(Domain)); }
'''
(ART/'receiver-control.rs').write_text(fixture)
command(['rustc','--version','--verbose'])
command(['rustc','--edition=2021','--crate-name','receiver_control','--emit=mir',str(ART/'receiver-control.rs'),'-o',str(ART/'receiver-control.mir')])
command(['rustc','--edition=2021','--crate-name','receiver_control',str(ART/'receiver-control.rs'),'-o',str(ART/'receiver-control')])
assert command([str(ART/'receiver-control')]) == expectations['receiver_stdout']
mir = (ART/'receiver-control.mir').read_text()
assert all(t in mir for t in expectations['receiver_mir_targets'])

prior = json.loads((ART.parent/'2026-11-15-spurcode-project-quality/rust-quality.json').read_text())
pieces = {d['control']['name_tail']:bytes.fromhex(d['matching_nodes'][0]['source_span_hex']).decode()
          for d in prior['definition_results'] if d['control']['name_tail'] in ['NativeLabel','labels']}
invocation = bytes.fromhex(prior['macro_controls']['cases'][0]['evidence'][0]['invocation_hex']).decode()
enum = next(r for r in json.loads((ART/'binding-pilot.json').read_text()) if r['name']=='enum_relation_kind_source')['rows'][0]
enum_text = bytes.fromhex(enum['source_hex']).decode()
slots = re.findall(r'(\w+)\s*=>\s*"([^"]+)"', invocation)
assert len(slots) == 14
original_macro_file = subprocess.check_output(['git','show',REV+':crates/spur-graph-duckdb/src/file_facts.rs'], cwd=ROOT)
assert original_macro_file[4967:4968] == b';'
macro_fixture = '\n\n'.join([enum_text,pieces['NativeLabel'],pieces['labels'],invocation+';',
    'fn main() {\n'+''.join('println!("{}", RelationKind::'+variant+'.label());\n' for variant,_ in slots)+'}'])
(ART/'macro-control.rs').write_text(macro_fixture)
expanded = command(['rustc','--edition=2021','--crate-name','macro_control','-Zunpretty=expanded',str(ART/'macro-control.rs')],
                   env={**os.environ,'RUSTC_BOOTSTRAP':'1'})
(ART/'macro-expanded.rs').write_text(expanded)
command(['rustc','--edition=2021','--crate-name','macro_control',str(ART/'macro-control.rs'),'-o',str(ART/'macro-control')])
macro_output = command([str(ART/'macro-control')])
assert macro_output == ''.join(label+'\n' for _,label in slots)

controls = {}
for name, source in [('receiver-control',fixture),('macro-control',macro_fixture),('macro-expanded',expanded)]:
    facts = q(name+'_extraction', 'SELECT spur_graph_file_facts(?,?) AS facts', [name+'.rs', source])[0]['facts']
    nodes = {n['node_id']:n for n in facts['nodes']}
    spans = {s['span_id']:s for s in facts['spans']}
    raw = source.encode()
    def node_source(n):
        span = spans[n['source_span_id']]
        return raw[span['start_byte']:span['end_byte']].decode()
    generated = [dict(node=n, source=node_source(n)) for n in facts['nodes']
                 if n['label']=='label' and n['kind'] in ['function','method'] and 'match self' in node_source(n)]
    calls = [dict(edge=e,caller=nodes.get(e['source_node_id'],{}).get('label'),
                  target=nodes.get(e['target_node_id']),target_source=node_source(nodes[e['target_node_id']]) if e['target_node_id'] in nodes else None)
             for e in facts['edges'] if e['relation']=='calls' and e['target_label']=='len']
    controls[name] = dict(nodes=len(facts['nodes']),edges=len(facts['edges']),macro_invocations=len(facts['macro_invocations']),
                          generated_label_bodies=generated,len_calls=calls)
assert len(controls['macro-control']['generated_label_bodies']) == 0
assert len(controls['macro-expanded']['generated_label_bodies']) == 1
controls['macro_fixture_provenance'] = dict(revision=REV, enum=enum, invocation_sha256=sha(invocation.encode()),
    trait_sha256=sha(pieces['NativeLabel'].encode()), macro_sha256=sha(pieces['labels'].encode()),
    adaptation='Concatenate exact frozen enum span (without its preceding derive attributes), trait, macro and invocation plus its following source semicolon at byte 4967; append a new main. No external dependencies. Expanded source has separate provenance.')
(ART/'package-catalog.json').write_text(json.dumps(dict(revision=REV,manifest_sha256=manifest_hashes,packages=packages),indent=2)+'\n')
(ART/'binding-analysis.json').write_text(json.dumps(receipts,indent=2)+'\n')
(ART/'compiler-controls.json').write_text(json.dumps(dict(expectations=expectations,commands=commands,results=controls),indent=2)+'\n')
(ART/'execution-provenance.json').write_text(json.dumps(dict(script_sha256=sha(Path(__file__).read_bytes()),
    source_revision=REV, duckdb_version=duckdb.__version__,snapshot_sha256=sha(SNAP.read_bytes()),
    extension_sha256=sha(EXT.read_bytes())),indent=2)+'\n')
assert sha(SNAP.read_bytes()) == 'c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e'
print(json.dumps(dict(complete=True,receipts=len(receipts),receiver=controls['receiver-control'],
    macro_before=len(controls['macro-control']['generated_label_bodies']),macro_after=len(controls['macro-expanded']['generated_label_bodies'])),separators=(',',':')))
c.close()
