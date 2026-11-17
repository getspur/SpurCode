"""Verify retained build evidence and reproduced gaps, not product readiness."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def read(path):
    return json.loads(path.read_bytes())


def tool_value(value):
    if 'structuredContent' in value:
        return value['structuredContent']
    return json.loads(next(c['text'] for c in value['content'] if c['type'] == 'text'))


def stdout_json(cell):
    text = ''.join(o.get('text', '') for o in cell['outputs'] if isinstance(o, dict)
                   and o.get('output_type') == 'stream')
    return json.loads(text)


def main():
    audit = read(HERE / 'audit.json')
    release = read(HERE / 'release/receipt.json')
    assert release['status'] == 'passed' and release['exit_code'] == 0
    assert release['rust_tests_mode'] == 'remote'
    assert release['source_head'] == audit['source_commit']
    assert release['unittests']['tests_run'] == 223
    assert release['unittests']['release_pass']
    assert all(release['unittests'][k] == 0 for k in
               ('failures', 'errors', 'skipped', 'expected_failures', 'unexpected_successes'))
    assert all(c['returncode'] == 0 for c in release['commands'])
    assert '61 passed; 0 failed; 0 ignored' in (HERE / 'release/core-notebook-tests.stdout').read_text()
    assert '4 passed; 0 failed; 0 ignored' in (HERE / 'release/rust-tests.stdout').read_text()
    package = Path(audit['build']['stable_binary'])
    assert hashlib.sha256(package.read_bytes()).hexdigest() == audit['build']['binary_sha256']
    assert hashlib.sha256(Path(str(package) + '.json').read_bytes()).hexdigest() == audit['build']['manifest_sha256']
    native = read(HERE / 'native-acceptance/evidence.json')
    assert native['status'] == 'passed'
    assert native['package_sha256'] == audit['build']['binary_sha256']
    assert native['function_anchors_checked'] == 28 and native['symbol_refs_checked'] == 70
    assert native['pointer_preserved_on_failures']
    cells = {c['id']: c for c in audit['notebook']['cells']}
    assert len([c for c in cells.values() if c['kind'] == 'code']) == 5
    assert all(c['status'] == 'success' for c in cells.values() if c['kind'] == 'code')
    observed = stdout_json(cells['7f9ef270-0121-4504-96fe-5593469c3647'])
    assert len(observed['ordinary_duplicate_noops']) == 3
    for case in observed['ordinary_duplicate_noops']:
        assert case['symbols'] == 2 and case['changes_on_identical_input'] == 2
        assert case['change_kinds'] == ['modified', 'added'] and not case['diagnostics']
    assert observed['notebook_coverage']['graph_result_status'] == 'ok'
    assert observed['notebook_coverage']['notebook_status'] == 'partial'
    assert observed['repeated_call_evidence']['source_call_expressions'] == 2
    assert len([e for e in observed['repeated_call_evidence']['edges'] if e['relation'] == 'calls']) == 1
    assert tool_value(audit['app']['doctor'])['ok']
    assert len(audit['app']['evidence_calls']) == 4
    assert all(not c['ok'] and 'fetch_arrow_table' in c['error'] for c in audit['app']['evidence_calls'])
    assert audit['app']['viewport']['iframe_height'] == 40
    assert audit['app']['viewport']['dashboard_height'] == 640
    resolution = read(HERE / 'spur-explore-audit-resolution-exact.json')
    assert not resolution['same_primary_id']
    assert resolution['subgraph_seed'][0]['stable_symbol_id'] != resolution['scorecard_top'][0]['stable_symbol_id']
    inventory = read(HERE / 'inventory.json')
    for relative, digest in inventory.items():
        assert hashlib.sha256((HERE / relative).read_bytes()).hexdigest() == digest, relative
    print(json.dumps({'audit_evidence': 'verified', 'release_gate': 'passed',
        'ordinary_review_gap_reproduced': 3, 'live_workbench_tool_failures': 4,
        'different_panel_targets': True, 'app_ready': False,
        'native_package_sha256': audit['build']['binary_sha256']}, indent=2))


if __name__ == '__main__':
    main()
