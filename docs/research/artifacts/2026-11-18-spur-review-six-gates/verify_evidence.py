"""Verify archived experiment evidence; this does not qualify the live named app."""
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent
result = json.loads((ROOT / 'results.json').read_text())
notebook = json.loads((ROOT / 'experimental-notebook.ipynb').read_text())
app = json.loads((ROOT / 'app/app.ipynb').read_text())
observations = result['observations']
cells = {cell['id']: cell for cell in notebook['cells']}


def stdout(name):
    return json.loads(''.join(output['text'] for output in observations[name]['outputs']
                              if output['output_type'] == 'stream'
                              and output['name'] == 'stdout'))


pgq = stdout('pgq_grammar')
ports = stdout('python_ports')
origin = stdout('origin_b')
repo = stdout('repo')
collectors = stdout('collectors')
native = stdout('final_engine')
checks = {}
code = [cell for cell in notebook['cells'] if cell['cell_type'] == 'code']
checks['lab_thirteen_code_eleven_completed_two_idle'] = len(code) == 13 and sum(c.get('execution_count') is not None for c in code) == 11 and sum(not c['outputs'] and c.get('execution_count') is None for c in code) == 2
checks['no_unhandled_kernel_errors_in_capture'] = all(o['output_type'] != 'error' for c in code for o in c['outputs'])
checks['actual_named_20_engine'] = stdout('engine')['duckdb'] == '2.0.0' and len(stdout('engine')['loaded']) == 3
checks['grammar_initially_disabled'] = pgq['before'] == [] and 'PROPERTY' in pgq['red']
checks['grammar_enables_captured_context_paths'] = len(pgq['context_paths']) == 3 and len(pgq['syntax_paths']) == 1
checks['pgq_recursive_sql_parity'] = pgq['recursive_sql_parity'] is True
checks['grammar_reverse_failure_and_final_restore'] = 'MATCH' in pgq['reverse'] and pgq['after'] == ['duckpgq']
checks['plain_named_python_stubs_are_explicit'] = all(row['type'] == 'RuntimeError' and 'ports_unavailable' in row['message'] for row in ports['operations'])
checks['producer_authoring_rejected_atomically'] = observations['python_producer_insert_rejection']['isError'] and '9d6b13dd-e244-469e-a9e7-2ccb1be447b8' not in cells
checks['origin_async_and_thread_experiment'] = origin['phase_a'] != origin['phase_b'] and origin['async_preserved_A'] and origin['plain_thread_reports_B']
checks['deno_launch_and_consumer_remain_blocked'] = observations['deno_launch']['isError'] and observations['deno_consumer']['isError'] and all(cells[key]['execution_count'] is None and cells[key]['outputs'] == [] for key in ['b37deea2-2f07-4b8a-aed1-9f1fa5d7f41a', 'bbd700cf-f6b5-4a5f-bd6e-29cb4fb857b3'])
checks['real_full_tracked_inventory_and_diff'] = repo['tracked_head_entries'] == 18342 and len(repo['changed_files']) == 5
checks['base_head_matches_live_receipt'] = collectors['pr_receipt']['base_sha'] == repo['base'] and collectors['pr_receipt']['head_sha'] == repo['head']
checks['two_real_base_pinned_history_queries'] = len(collectors['history']) == 2 and all(row['base_revision'] == repo['base'] and row['authors'] for row in collectors['history'])
checks['live_saved_connection_inbox_result'] = observations['github_inbox']['status'] == 'succeeded' and '28731' in observations['github_inbox']['outputs'][0]
checks['automatic_pipeline_not_overclaimed'] = collectors['automatic_named_pipeline'] is False
checks['unknown_reviewers_not_empty_ownership_claim'] = collectors['pr_receipt']['requested_reviewers'] is None and collectors['current_reviewers'].startswith('unknown:')
checks['connection_metadata_restored'] = all(result['connection_restoration'][key] for key in ['provider_restored', 'credential_ref_restored', 'manifest_semantically_unchanged']) and result['connection_restoration']['status'] == 'ready'
checks['native_symbols_and_context_refs'] = native['cpp_symbols'] == 2 and native['context_row_refs'] == 12 and native['all_refs_exist']
checks['source_audit_identity'] = stdout('source_audit')['head'] == 'c686dcf6ce81d1cc517cd11f0db0edc7062ef9b7' and len(stdout('source_audit')['files']) == 6
manifest = app['metadata']['spur_app']
checks['app_manifest_and_minimal_capabilities'] = manifest['archetype'] == 'vanilla-widget' and manifest['capabilities']['active_output_scripts'] and not manifest['capabilities']['external_workspace'] and manifest['mcp_server'] is None
surface = next(c for c in app['cells'] if c['id'] == 'app-surface')
checks['app_frontend_declared'] = surface['metadata']['spur']['frontend'] == {'kind': 'custom', 'binds': [], 'emits': []}
rendered = surface['outputs'][0]['data']['text/html']
original = (ROOT.parent / '2026-11-18-spur-review-ux/prototype.html').read_text()
checks['app_saved_exact_reference_html'] = rendered == original and surface['execution_count'] is not None
checks['full_doctor_level_two'] = result['app']['doctor']['ok'] and result['app']['doctor']['level_reached'] == 2
checks['app_cascade_succeeded_without_live_ports'] = all(r['status'] == 'succeeded' for r in result['app']['cascade']['runs']) and result['app']['dag']['port_manifest'] == {}
checks['no_pack_or_browser_interaction_claim'] = result['app']['pack'] == 'not_run' and result['app']['host_browser_interactions'] == 'not_run'
solver = result['solver']
checks['app_solve_nine_pass_four_fail'] = solver['app_result']['status'] == 'unsat' and sum(r['outcome'] == 'pass' for r in solver['app_result']['rule_results']) == 9 and sum(r['outcome'] == 'fail' for r in solver['app_result']['rule_results']) == 4
checks['origin_consistency_passes_async_rejects_thread'] = solver['async_result']['outcome'] == 'pass' and solver['thread_result']['outcome'] == 'fail'
checks['two_gates_qualified_four_unqualified'] = [g['id'] for g in result['gates'] if g['qualification']] == ['R4', 'R5']
checks['factory_defaults_preserved'] = all(result['final_profiles'][language]['default_profile'] is None for language in ['python', 'deno'])
checks['no_pat_literals_in_captured_notebooks'] = not re.search(r'gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{30,}', json.dumps(notebook) + json.dumps(app))
verification = {'scope': 'Experiment/capture consistency, not complete application acceptance',
                'passed': sum(checks.values()), 'total': len(checks), 'checks': checks}
(ROOT / 'verification.json').write_text(json.dumps(verification, indent=2) + '\n')
assert all(checks.values()), verification
inventory = []
for path in sorted(ROOT.rglob('*')):
    if path.is_file() and path.name != 'capture-manifest.json' and not path.name.endswith('.spur-writer.lock') and '__pycache__' not in path.parts:
        inventory.append({'path': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size,
                          'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
(ROOT / 'capture-manifest.json').write_text(json.dumps({'schema_version': 1, 'files': inventory}, indent=2) + '\n')
print(json.dumps({'passed': verification['passed'], 'total': verification['total'], 'scope': verification['scope']}))
