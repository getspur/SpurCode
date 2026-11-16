"""Small, reproducible UX probes against a pinned local artifact; no build or install."""
from pathlib import Path
import collections
import datetime
import hashlib
import json
import os
import subprocess
import sys
import duckdb

ROOT = Path('/Users/kevintruong/project/SpurCode')
OUT = ROOT / 'docs/research/artifacts/2026-11-16-extension-agent-experience'
PROTOCOL = json.loads((ROOT / 'docs/research/artifacts/2026-11-15-duckdb-file-facts-scale/protocol.json').read_text())
EXTENSION = Path(PROTOCOL['extension'])
OUT.mkdir(parents=True, exist_ok=True)
artifact_sha = hashlib.sha256(EXTENSION.read_bytes()).hexdigest()
assert artifact_sha == PROTOCOL['extension_sha256']
assert duckdb.__version__ == '2.0.0'
def quote(value):
    return "'" + str(value).replace("'", "''") + "'"
def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

# An isolated, deterministic small database: reruns replace this experiment only.
db = OUT / 'journey.duckdb'
if db.exists():
    db.unlink()
con = duckdb.connect(str(db), config={'allow_unsigned_extensions': True})
con.execute('LOAD ' + quote(EXTENSION))
report = {
 'observed_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'runtime': {'python': sys.executable, 'duckdb': con.execute('SELECT version()').fetchone()[0],
             'scope': 'separate native Python process; not managed notebook SQL'},
 'artifact': {'path': str(EXTENSION), 'sha256': artifact_sha,
              'build_commit_from_prior_protocol': PROTOCOL['artifact_build_commit']},
 'source_head_at_observation': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
 'source_files': {str(p.relative_to(ROOT)): digest(p) for p in [
    ROOT/'crates/spur-graph-duckdb/README.md',
    ROOT/'crates/spur-graph-duckdb/src/lib.rs',
    ROOT/'crates/spur-graph-duckdb/src/file_facts.rs',
    ROOT/'crates/spur-graph-duckdb/examples/file_facts.sql']},
}
docs = con.execute("""SELECT function_name, parameters, parameter_types, description, examples
FROM duckdb_functions() WHERE starts_with(function_name, 'spur_graph_') ORDER BY function_name""").fetchall()
report['function_guidance'] = [{'name':r[0], 'parameters':r[1], 'parameter_types':r[2],
                                'description':r[3], 'examples':r[4]} for r in docs]
assert {r[0] for r in docs} == {'spur_graph_symbols','spur_graph_file_facts','spur_graph_symbol_changes'}
SRC = 'fn helper() {} fn entry() { helper(); remote_call(); }'
symbols = con.execute("SELECT unnest(spur_graph_symbols('src/demo.rs', ?), recursive := true)", [SRC])
names = [x[0] for x in symbols.fetchall()]
assert {'helper','entry'} <= set(names)
report['first_result'] = {'symbol_names': names}
facts = con.execute("SELECT spur_graph_file_facts('src/demo.rs', ?)", [SRC]).fetchone()[0]
report['single_file'] = {
 'list_counts': {k:len(v) for k,v in facts.items()},
 'edge_relations': dict(collections.Counter(e['relation'] for e in facts['edges'])),
 'call_evidence': [{k:e[k] for k in ['source_node_id','target_node_id','relation','target_label','confidence','bind_method']}
                   for e in facts['edges'] if 'call' in e['relation']],
}
assert any(e['target_label'] == 'remote_call' and e['target_node_id'] is None for e in facts['edges'])
other = con.execute("SELECT spur_graph_file_facts('src/copy.rs', ?)", [SRC]).fetchone()[0]
report['local_id_collision'] = sorted(set(n['node_id'] for n in facts['nodes']) & set(n['node_id'] for n in other['nodes']))
assert report['local_id_collision'], 'Fixture should demonstrate why bare local IDs cannot identify repository nodes'
errors = []
for path in ['../outside.rs', 'assets/input.xyz']:
    try:
        con.execute('SELECT spur_graph_file_facts(?, ?)', [path,SRC]).fetchone()
        raise AssertionError('Expected invalid path failure: ' + path)
    except duckdb.Error as exc:
        errors.append({'path':path, 'error':str(exc), 'connection_alive':con.execute('SELECT 42').fetchone()[0] == 42})
report['error_recovery'] = errors
report['null_empty_try'] = {
 'null_source_is_null': con.execute("SELECT spur_graph_file_facts('src/null.rs', NULL) IS NULL").fetchone()[0],
 'empty_source_is_nonnull': con.execute("SELECT spur_graph_file_facts('src/empty.rs', '') IS NOT NULL").fetchone()[0],
 'unsupported_try_is_null': con.execute("SELECT TRY(spur_graph_file_facts('assets/input.xyz', ?)) IS NULL", [SRC]).fetchone()[0]
}
assert all(report['null_empty_try'].values())
delta = con.execute("""SELECT spur_graph_symbol_changes('src/demo.rs', 'src/demo.rs', 'M',
 spur_graph_symbols('src/demo.rs', 'fn old_name() {}'),
 spur_graph_symbols('src/demo.rs', 'fn new_name() {}'))""").fetchone()[0]
report['temporal_pair'] = {'changes':[{'kind':x['change_kind'], 'name':x['symbol']['entity_name']} for x in delta['changes']],
                           'diagnostics':delta['diagnostics'], 'scope':'one supplied file/version pair; no Git discovery'}

# Use the repository's exact promoted workflow. This includes statuses, child tables, and typed exports.
con.execute('CREATE TEMP TABLE staged_sources(blob_oid VARCHAR, logical_path VARCHAR, source VARCHAR, extractor_sha VARCHAR)')
con.executemany('INSERT INTO staged_sources VALUES (?, ?, ?, ?)', [
 ('demo-blob','src/demo.rs',SRC,artifact_sha),
 ('demo-blob','src/copy.rs',SRC,artifact_sha),
 ('empty-blob','src/empty.rs','',artifact_sha),
 ('bad-blob','assets/input.xyz',SRC,artifact_sha),
 ('null-blob','src/null.rs',None,artifact_sha)])
con.execute("""CREATE TEMP TABLE source_lineage AS
 SELECT 'demo-repository' repository, 'demo-commit' commit_oid, p.parent_oid, s.blob_oid, s.logical_path, s.extractor_sha
 FROM staged_sources s CROSS JOIN (VALUES ('parent-a'), ('parent-b')) p(parent_oid)""")
workflow = (ROOT / 'crates/spur-graph-duckdb/examples/file_facts.sql').read_text()
previous = Path.cwd()
os.chdir(OUT)
try:
    con.execute(workflow)
    first = con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0]
    status = con.execute('SELECT status, count(*) FROM file_facts_cache GROUP BY status ORDER BY status').fetchall()
    con.execute(workflow)
    second = con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0]
    refresh = con.execute('SELECT count(*) FROM file_facts_child_refresh').fetchone()[0]
    parent_rows = con.execute('SELECT count(*) FROM file_facts_cache').fetchone()[0]
    occurrence_rows = con.execute('SELECT count(*) FROM file_facts_occurrences').fetchone()[0]
    assert first == 5 and second == 0 and refresh == 0
    assert parent_rows == 5 and occurrence_rows == 10
    report['cache_workflow'] = {'first_missing_keys':first, 'replay_missing_keys':second,
      'replay_child_refresh_keys':refresh, 'cache_parents':parent_rows,
      'lineage_occurrences':occurrence_rows, 'statuses':status,
      'error_text_retained_in_cache':False, 'workflow_sha256':digest(ROOT / 'crates/spur-graph-duckdb/examples/file_facts.sql')}
finally:
    os.chdir(previous)
con.close()
with duckdb.connect(str(db), read_only=True) as fresh:
    report['fresh_connection'] = {
      'registered_graph_functions':fresh.execute("SELECT count(*) FROM duckdb_functions() WHERE starts_with(function_name, 'spur_graph_')").fetchone()[0],
      'cached_parent_rows_readable_without_load':fresh.execute('SELECT count(*) FROM file_facts_cache').fetchone()[0]}
assert report['fresh_connection']['registered_graph_functions'] == 0
assert report['fresh_connection']['cached_parent_rows_readable_without_load'] == 5
report['portable_parquet'] = [
 {'path':str(p), 'bytes':p.stat().st_size, 'sha256':digest(p)} for p in sorted(OUT.glob('*.parquet'))]
report['checks_passed'] = True
(OUT / 'native-results.json').write_text(json.dumps(report, indent=2) + '\n')
# Keep the notebook's output readable; full typed signatures and artifact hashes live in the JSON.
summary = {k:v for k,v in report.items() if k not in ['source_files','portable_parquet','function_guidance']}
summary['function_guidance'] = [{'name':r['name'],'has_description':bool(r['description']),
                                 'example_count':len(r['examples'])} for r in report['function_guidance']]
summary['parquet_file_count'] = len(report['portable_parquet'])
print(json.dumps(summary, indent=2))
