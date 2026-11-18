"""Check the archived evaluation evidence; this is not native app acceptance."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
results = json.loads((ROOT / 'results.json').read_text())
notebook = json.loads((ROOT / 'experimental-notebook.ipynb').read_text())
observations = results['observations']


def stdout(key):
    return json.loads(''.join(o['text'] for o in observations[key]['outputs']
                              if o['output_type'] == 'stream' and o['name'] == 'stdout'))


analysis = stdout('analysis_2_corrected')
native = stdout('native_load')
bridge = stdout('named_python_bridge')
parser = stdout('duckpgq_parser_probe')
paths = stdout('recursive_sql_control')
cells = {c['id']: c for c in notebook['cells']}
code = [c for c in notebook['cells'] if c['cell_type'] == 'code']
solver = results['solver']
checks = {}
checks['captured_fixture_identity'] = hashlib.sha256((ROOT / 'fixture/design-data.json').read_bytes()).hexdigest() == results['fixture']['fixture_sha256']
checks['nine_code_cells_seven_completed_one_error_one_idle'] = (
    len(code) == 9
    and sum(c.get('execution_count') is not None for c in code) == 7
    and sum(any(o['output_type'] == 'error' for o in c['outputs']) for c in code) == 1
    and sum(not c['outputs'] and c.get('execution_count') is None for c in code) == 1
)
checks['named_deno_stays_selected_and_unexecuted'] = (
    cells['b5df3bec-d78b-4b47-9576-d04df9af7a91']['metadata']['spur']['kernel_profile'] == 'review-ui'
    and not cells['b5df3bec-d78b-4b47-9576-d04df9af7a91']['outputs']
    and results['observations']['deno_failure']['isError']
)
checks['named_python_20_selected'] = cells['4d69f0ae-4d07-4ef3-9e14-d1e734b58f5b']['metadata']['spur']['kernel_profile'] == 'review-duckdb2' and analysis['duckdb_runtime'] == '2.0.0'
checks['distinct_python_processes'] = stdout('analysis_155')['pid'] != analysis['pid']
checks['snapshot_and_ref_integrity'] = analysis['head'] == 'd29b26bb92ac940a0807be0f43cc385b31623839' and analysis['all_context_row_refs_exist'] and analysis['resolved_context_row_refs'] == 12
checks['captured_domain_counts'] = (analysis['changed_files_sql'], analysis['source_evidence'], analysis['components'], analysis['historical_people_candidates']) == (5, 21, 6, 7)
checks['existing_preparation_no_installs'] = all(
    results['preparation'][name]['receipt']['manager'] is None
    and not results['preparation'][name]['receipt']['owned']
    and not results['preparation'][name]['receipt']['capabilities']['ports_arrow']
    for name in ['python', 'duckdb2']
)
checks['three_extensions_loaded_and_hashed'] = len(native['loaded']) == 3 and all(r[1] for r in native['loaded']) and native['all_artifact_hashes_match']
checks['native_cpp_probe_and_scope'] = native['cpp_probe_symbol_count'] == 2 and native['spur_graph_info']['resolution_scope'] == 'single_file'
checks['disabled_python_publication_is_observed'] = not bridge['published'] and bridge['exception_type'] == 'NameError' and not analysis['spur_in_globals']
checks['graph_parser_failure_retained'] = observations['duckpgq_query_failure']['status'] == 'failed' and all(a['status'] == 'failed' and 'PROPERTY' in a['message'] for a in parser['attempts'])
checks['recursive_control_preserves_candidates_and_witnesses'] = len(paths['full_context_paths']) == 3 and len(paths['syntax_only_paths']) == 1 and paths['candidate_paths_hidden'] == 2 and all(p[4] for p in paths['full_context_paths'])
checks['standalone_html_control_is_explicit'] = observations['standalone_html_control']['status'] == 'succeeded' and any('text/html' in o.get('data', {}) for o in cells['ae6ed920-5551-4dd7-a6f7-2872fdb6e4f1']['outputs'])
checks['factory_defaults_preserved'] = all(results['final_profiles'][family]['default_profile'] is None for family in ['deno', 'python'])
checks['no_native_app_or_port_pipeline_claim'] = not results['evaluation_doctor']['ok'] and not results['final_dag']['nodes'] and not results['final_dag']['port_manifest']
checks['solve_engine_pass_seven_bindings'] = solver['engine_result']['status'] == 'sat' and solver['engine_result']['outcome'] == 'pass' and len(solver['engine_result']['rule_results']) == 7
checks['solve_app_fails_six_unqualified_gates'] = solver['app_result']['status'] == 'unsat' and solver['app_result']['outcome'] == 'fail' and sum(r['outcome'] == 'fail' for r in solver['app_result']['rule_results']) == 6
checks['solve_requests_are_exact_nonpersistent_catalog_requests'] = all(solver[key]['persist'] is False and all(r['rule_id'] == 'configuration.requires_any' for r in solver[key]['rules']) for key in ['engine_request', 'app_request'])
checks['no_architecture_ownership_or_ai_overclaim'] = paths['architecture']['alignment'] == 'not_evaluated_no_policy' and paths['people']['formal_ownership'] == 'not_verified' and paths['ai_review'] == 'not_run'

verification = {'scope': 'Evidence consistency, not end-to-end app acceptance', 'passed': sum(checks.values()), 'total': len(checks), 'checks': checks}
(ROOT / 'verification.json').write_text(json.dumps(verification, indent=2) + '\n')
assert all(checks.values()), verification
manifest = {'schema_version': 1, 'files': []}
for path in sorted(ROOT.rglob('*')):
    if path.is_file() and path.name != 'capture-manifest.json':
        manifest['files'].append({'path': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size,
                                  'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
(ROOT / 'capture-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({'passed': verification['passed'], 'total': verification['total'], 'scope': verification['scope']}))
