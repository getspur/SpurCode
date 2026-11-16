import hashlib, importlib.util, json, os, pathlib, subprocess, sys
import duckdb
ROOT = pathlib.Path.cwd()
CRATE = ROOT / 'crates/spur-graph-duckdb'
EVIDENCE = pathlib.Path(__file__).resolve().parent
RELEASE = pathlib.Path('/private/tmp/spur-r7-65bb3605-release-1')
ART = pathlib.Path('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s')
FIX = ART.parent / '2026-11-15-rust-macro-qualification-fix'
sha = lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
release = json.loads((RELEASE / 'receipt.json').read_bytes())
assert release['status'] == 'passed' and release['exit_code'] == 0
package = pathlib.Path(release['test_environment']['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
os.environ.update(release['test_environment'])
sys.path.insert(0, str(CRATE / 'scripts'))
from check_release import approved_extension_sha, release_trust
from verify_package import verify_package
artifact = verify_package(package)
binary_sha = approved_extension_sha(package, **release_trust())
assert binary_sha == release['package']['binary']['sha256'] == sha(package)
published = json.loads((ART / 'published.json').read_bytes())
database = pathlib.Path(published['database'])
assert sha(database) == published['database_sha256']
reference_hashes = {str(p): sha(p) for p in [ART / 'published.json', ART / 'README.md', ART / 'inspect_published.py', FIX / 'control.rs']}
queries = []
with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    con.execute("ATTACH '" + str(database).replace("'", "''") + "' AS frozen (READ_ONLY)")
    def q(name, sql, params=()):
        cur = con.execute(sql, list(params))
        cols = [c[0] for c in cur.description]
        rows = [dict(zip(cols,r)) for r in cur.fetchall()]
        queries.append(dict(name=name,sql=sql,params=list(params),rows=rows))
        return rows
    info = con.execute('SELECT spur_graph_info()').fetchone()[0]
    assert info['extractor_id'] == release['release_trust']['source_identity']
    assert all(info[k] == artifact[k] for k in info)
    corpus = q('corpus', '''SELECT (SELECT count(*) FROM frozen.inventory) inventory,
      (SELECT count(*) FROM frozen.sources) sources, (SELECT count(*) FROM frozen.facts) current_facts,
      (SELECT count(*) FROM frozen.file_facts_cache) versioned_cache_keys''')[0]
    assert corpus == dict(inventory=4404,sources=4098,current_facts=4098,versioned_cache_keys=8196)
    meta = q('frozen_provenance','SELECT * FROM frozen.refresh_meta')[0]
    assert meta['extractor_sha'] == published['extension_sha256']
    assert meta['snapshot_commit'] == published['snapshot_commit']
    sample = 'crates/spur-acp/src/capability_evidence.rs'
    parity = q('fresh_native_cache_parity', '''SELECT s.logical_path,
      spur_graph_file_facts(s.logical_path,s.source) IS NOT DISTINCT FROM f.value all_five_collections_same
      FROM frozen.sources s JOIN frozen.facts f USING(logical_path) WHERE s.logical_path=?''',[sample])
    assert len(parity)==1 and parity[0]['all_five_collections_same']
    source = con.execute('SELECT source FROM frozen.sources WHERE logical_path=?',[sample]).fetchone()[0]
    con.execute('CREATE TEMP TABLE current_sample AS SELECT spur_graph_file_facts(?,?) value',[sample,source])
    real = q('current_real_vec_control', '''WITH ns AS (SELECT unnest(value.nodes) n FROM current_sample),
      es AS (SELECT unnest(value.edges) e FROM current_sample)
      SELECT n.n.label caller,e.e.target_label,e.e.scope_text,e.e.target_node_id,e.e.bind_method
      FROM es e JOIN ns n ON n.n.node_id=e.e.source_node_id WHERE e.e.relation='calls'
      AND e.e.target_label='new' AND n.n.label='rejected_evidence_demotes_while_inconclusive_evidence_preserves_prior_routes' ''')
    assert real and all(r['scope_text']=='Vec' and r['target_node_id'] is None for r in real)
    control_source = (FIX / 'control.rs').read_text()
    con.execute('CREATE TEMP TABLE validation_controls AS SELECT ?::VARCHAR logical_path,spur_graph_file_facts(?,?) value', ['control.rs','control.rs',control_source])
    assert con.execute('SELECT a.value IS NOT DISTINCT FROM b.value FROM validation_controls a JOIN frozen.validation_controls b USING(logical_path)').fetchone()[0]
    rows = q('current_controlled_calls', '''WITH ns AS (SELECT unnest(value.nodes) n FROM validation_controls),
       es AS (SELECT unnest(value.edges) e FROM validation_controls)
       SELECT src.n.label caller,e.e.target_label,e.e.scope_text,e.e.target_node_id,
              dst.n.label target_name,owner.n.label target_owner,e.e.bind_method
       FROM es e JOIN ns src ON src.n.node_id=e.e.source_node_id
       LEFT JOIN ns dst ON dst.n.node_id=e.e.target_node_id
       LEFT JOIN es parent ON parent.e.relation='contains' AND parent.e.target_node_id=dst.n.node_id
       LEFT JOIN ns owner ON owner.n.node_id=parent.e.source_node_id
       WHERE e.e.relation='calls' AND src.n.label IN
        ('observed_macro','observed_helper','observed_local_macro','observed_mixed_macro')
       ORDER BY caller,e.e.target_label,e.e.scope_text NULLS FIRST''')
    selected = lambda caller,leaf: [r for r in rows if r['caller']==caller and r['target_label']==leaf]
    bad=selected('observed_macro','new')
    assert bad and all(r['target_node_id'] is None and r['scope_text']=='Vec' for r in bad)
    assert any(r['target_name']=='helper' for r in selected('observed_helper','helper'))
    assert any(r['target_owner']=='LocalFactory' for r in selected('observed_local_macro','new'))
    assert any(r['target_owner']=='LocalFactory' for r in selected('observed_mixed_macro','new'))
    assert any(r['target_node_id'] is None for r in selected('observed_mixed_macro','new'))
    current_control = con.execute('SELECT value FROM validation_controls').fetchone()[0]
    current_sample = con.execute('SELECT value FROM current_sample').fetchone()[0]
    sys.path.insert(0,str(CRATE/'tests'))
    from test_file_facts import native_facts
    native_control,native_sample=native_facts([('control.rs',control_source),(sample,source)])
    assert native_control==current_control and native_sample==current_sample
    # Execute every SQL statement documented in the release contract.
    doc=(CRATE/'docs/release-contract.md').read_text()
    examples=doc.split('```sql\n',1)[1].split('```',1)[0]
    for statement in con.extract_statements(examples):
        q('documented_sql_example', statement.query)
    native_runtime=q('current_runtime','SELECT version() version')[0]
    platform=con.execute('PRAGMA platform').fetchone()[0]
assert sha(database)==published['database_sha256']
assert all(sha(p)==v for p,v in reference_hashes.items())
spec=importlib.util.spec_from_file_location('app',CRATE/'examples/app_consumer.py')
app=importlib.util.module_from_spec(spec);spec.loader.exec_module(app)
manifest=app.run_consumer(CRATE/'tests/fixtures/app_consumer.json',EVIDENCE/'headless',package,
    expected_extractor_id=info['extractor_id'])
assert manifest['artifact']['binary']['sha256']==binary_sha
assert manifest['replay']==dict(missing_keys=0,tables_unchanged=True)
for sid in ('base','head'):
    assert app.read_result(EVIDENCE/'headless',sid)['manifest']==manifest
assert manifest['runtime']['duckdb']=='2.0.0' and manifest['runtime']['platform']==platform
assert sha(package)==binary_sha
report=dict(status='passed',source_head=release['source_head'],source_identity=info['extractor_id'],
    package=str(package),binary_sha256=binary_sha,release_manifest_sha256=release['release_trust']['manifest_sha256'],
    runtime=release['runtime'],script_sha256=sha(__file__),reference_hashes=reference_hashes,
    database_sha256=sha(database),database_readonly=True,reference_extension_sha256=published['extension_sha256'],
    full_corpus_reingested=False,parity_scope=[sample,'control.rs'],native_oracle_parity=True,
    documented_sql_examples=5,headless_manifest=manifest,queries=queries)
(EVIDENCE/'current-acceptance.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(dict(status=report['status'],binary_sha256=binary_sha,source_identity=info['extractor_id'],
    corpus=corpus,parity_scope=report['parity_scope'],headless_run=manifest['run_id'],process=manifest['process'])))
