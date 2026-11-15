"""Explicit positive/negative controls for the frozen six-file real cohort."""
import json
from pathlib import Path

ART = Path(__file__).resolve().parent
corpus = ART / 'corpus-real-r1'
comparison = ART / 'comparison-real-r0'
report = json.loads((comparison / 'report.json').read_text())
native = {r['case_id']: r for r in map(json.loads, (comparison / 'native.jsonl').read_text().splitlines())}
candidate = {r['case_id']: r for r in map(json.loads, (comparison / 'candidate.jsonl').read_text().splitlines())}
spec = json.loads((corpus / '.retrieval/spec.json').read_text())
requests = {r['case_id']: r for r in spec['requests']}
checks = []


def check(name, condition):
    checks.append({'name': name, 'passed': bool(condition)})


check('complete_29_request_matrix', set(native) == set(candidate) == set(requests) and len(requests) == 29)
for case, total in [('exact-function', 1), ('prefix', 1), ('substring-struct', 5),
                    ('glob', 1), ('limited', 10), ('empty', 0)]:
    row = native['real-search-' + case]
    check('search_' + case, row['error'] is None and row['body']['total_matches'] == total)
check('limit_and_truncation', len(native['real-search-limited']['body']['candidates']) == 1
      and native['real-search-limited']['body']['truncated'] is True)
check('positive_search_identity', native['real-search-exact-function']['body']['candidates'][0]['entity_name'] == 'source_offset')
for name, start, end in [('source_offset', 105, 111), ('register', 595, 607), ('extract_file_facts', 369, 410)]:
    for context in [0, 2]:
        case = f'real-read-{name}-{context}'
        row = native[case]
        body = row['body'] or {}
        lines = (corpus / body['file']).read_bytes().splitlines(keepends=True)
        expected_range = {'start': max(1, start - context), 'end': min(len(lines), end + context)}
        exact = b''.join(lines[expected_range['start'] - 1:expected_range['end']]).decode('utf-8')
        check(case + '_native_exact_range_bytes', row['error'] is None and body['range'] == expected_range
              and body['source'] == exact and f'fn {name}' in body['source'])
        other = candidate[case]['body'] or {}
        check(case + '_candidate_exact_range_bytes', candidate[case]['error'] is None
              and other['range'] == expected_range and other['source'] == exact
              and other['file_oid'] == body['file_oid'])
        if context:
            check(case + '_native_context_field', body.get('context_lines') == context)
for direction in ['callers', 'callees']:
    for visibility in ['false', 'true']:
        original = f'real-{direction}-extract_file_facts-{visibility}'
        check(original + '_incomplete_selector_negative', native[original]['body'] is None
              and native[original]['error']['code'] == -32004)
        qualified = f'real-{direction}-qualified-extract_file_facts-{visibility}'
        check(qualified + '_positive_selector', native[qualified]['error'] is None)
        row = native[f'real-{direction}-source_offset-{visibility}']['body']
        check(f'{direction}_positive_local_{visibility}', len(row[direction]) >= 1)
    qualified = native[f'real-{direction}-qualified-extract_file_facts-true']['body']
    check(direction + '_qualified_nonempty_unresolved', len(qualified[direction]) >= 1
          and qualified['counts_by_kind']['unresolved'] >= 1)
check('missing_source_negative', native['real-read-missing']['body'] is None
      and native['real-read-missing']['error']['code'] == -32004)
check('native_24_success_5_expected_errors', sum(r['error'] is None for r in native.values()) == 24
      and sum(r['error'] is not None for r in native.values()) == 5)
check('source_validator_passed', report['summary']['native_validation_failures'] == 0
      and report['summary']['candidate_source_failures'] == 0)
result = {'status': 'passed' if all(c['passed'] for c in checks) else 'failed', 'checks': checks,
          'scope': 'Real source controls complement synthetic ambiguity/Unicode/CRLF cases. Native compatibility is not assumed to prove language-level resolution truth.'}
(ART / 'real-positive-negative-controls.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'status': result['status'], 'checks': len(checks),
                  'failed': [c['name'] for c in checks if not c['passed']]}))
if result['status'] != 'passed':
    raise SystemExit(1)
