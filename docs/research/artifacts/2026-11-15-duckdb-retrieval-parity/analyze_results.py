"""Recompute parity and produce the bounded notebook comparison summary."""
import collections
import importlib.util
import json
from pathlib import Path

ART = Path(__file__).resolve().parent
loader = importlib.util.spec_from_file_location('candidate', ART / 'candidate-approved.py')
parity = importlib.util.module_from_spec(loader)
loader.loader.exec_module(parity)
result = {'cohorts': {}, 'by_tool': {}, 'mismatch_categories': {}, 'source_reads': [],
          'scope': '58 synthetic requests plus 29 requests on six complete real source files; this is a bounded compatibility matrix, not measured repository-wide recall.'}
categories = collections.Counter()
for label in ['synthetic', 'real']:
    directory = ART / f'comparison-{label}-r0'
    report = json.loads((directory / 'report.json').read_text())
    native = {r['case_id']: r for r in map(json.loads, (directory / 'native.jsonl').read_text().splitlines())}
    candidate = {r['case_id']: r for r in map(json.loads, (directory / 'candidate.jsonl').read_text().splitlines())}
    result['cohorts'][label] = {'summary': report['summary'], 'by_tool': report['by_tool']}
    for tool, stats in report['by_tool'].items():
        target = result['by_tool'].setdefault(tool, {k: 0 for k in stats})
        for key, value in stats.items():
            target[key] += value
    for case in report['cases']:
        case_id = case['case_id']
        # Recompute comparisons from the retained raw replies, never trust a cached score.
        request = {k: case[k] for k in ['case_id', 'tool', 'args']}
        recomputed = parity.compare_replies(request, native[case_id], candidate[case_id])
        assert recomputed == case, (label, case_id)
        if not case['semantic_equal']:
            if native[case_id]['error'] is not None:
                category = 'error_shape_or_message'
            elif case['tool'] == 'code_read_symbol':
                assert all(d['path'] == '/body/context_lines' for d in case['semantic_differences'])
                category = 'source_context_field'
            else:
                assert case['tool'] in ['code_callers', 'code_callees']
                category = 'graph_binding_counts_or_unresolved'
            categories[category] += 1
        if case['tool'] == 'code_read_symbol' and 'source' in (native[case_id]['body'] or {}):
            left, right = native[case_id]['body'], candidate[case_id]['body']
            assert left['source'].encode() == right['source'].encode(), case_id
            result['source_reads'].append({'cohort': label, 'case_id': case_id,
                'exact_source_bytes_equal': True,
                'range_equal': left.get('range', left.get('line_range')) == right.get('range', right.get('line_range')),
                'file_oid_equal': left['file_oid'] == right['file_oid'],
                'source_sha256': parity.sha256(left['source'].encode())})
    timing = json.loads((ART / f'timing-{label}-r0/result.json').read_text())
    assert timing['status'] in ['passed', 'no_eligible_cases']
    result['cohorts'][label]['timing'] = {k: timing[k] for k in ['status', 'eligible_cases', 'repetitions', 'cases']}
result['mismatch_categories'] = dict(categories)
result['totals'] = {key: sum(c['summary'][key] for c in result['cohorts'].values())
                    for key in ['requests', 'semantic_matches', 'full_api_matches', 'raw_identity_cases',
                                'raw_identity_matches', 'unsupported', 'native_validation_failures', 'candidate_source_failures']}
assert sum(categories.values()) == result['totals']['requests'] - result['totals']['semantic_matches']
assert all(r['range_equal'] and r['file_oid_equal'] for r in result['source_reads'])
parity.write_json(ART / 'summary.json', result)
print(json.dumps({k: result[k] for k in ['totals', 'by_tool', 'mismatch_categories']}, indent=2))
print('Exact successful source reads:', len(result['source_reads']))
