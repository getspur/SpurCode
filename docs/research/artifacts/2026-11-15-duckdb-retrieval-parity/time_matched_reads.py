"""Warm diagnostics only for positive source reads with exact full API equality."""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import statistics
import sys
import time

ART = Path(__file__).resolve().parent
loader = importlib.util.spec_from_file_location('retrieval_candidate', ART / 'candidate-approved.py')
parity = importlib.util.module_from_spec(loader)
loader.loader.exec_module(parity)


def stats(values):
    ordered = sorted(values)
    return {'n': len(values), 'p50_ms': statistics.median(values) / 1e6,
            'p95_ms': ordered[math.ceil(.95 * len(values)) - 1] / 1e6,
            'min_ms': ordered[0] / 1e6, 'max_ms': ordered[-1] / 1e6}


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--corpus', required=True)
parser.add_argument('--comparison', required=True)
parser.add_argument('--name', required=True)
parser.add_argument('--repetitions', type=int, default=30,
                    help='Experimental observations per request/backend; not a production quota')
args = parser.parse_args()
assert args.repetitions > 0
out = ART / args.name
out.mkdir(exist_ok=False)
corpus = Path(args.corpus).resolve()
report = json.loads(Path(args.comparison).read_text())
spec, inputs = parity.shared_inputs(corpus)
assert report['inputs'] == inputs
assert report['summary']['native_validation_failures'] == 0
assert report['summary']['candidate_source_failures'] == 0
if parity.sha256((ART / 'candidate-approved.py').read_bytes()) != '3d7f35655c60636fe101f1c147a6d834cbb95fc13bfc1ba0952c93e92c884fc7':
    raise ValueError('Candidate differs from reviewed version')
native_path = ART / 'native-release/retrieval_native'
assert parity.sha256(native_path.read_bytes()) == 'f897e4d526038e3df5203d741e7f0513af8900f8de6908cce4c86e800e8bcfaa'
extension = ART.parent / '2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension'
native_rows = {r['case_id']: r for r in map(json.loads, (Path(args.comparison).parent / 'native.jsonl').read_text().splitlines())}
eligible = {r['case_id'] for r in report['cases'] if r['full_api_equal'] and r['matched_semantic_work']
            and r['tool'] == 'code_read_symbol' and native_rows[r['case_id']]['error'] is None
            and 'source' in native_rows[r['case_id']]['body']}
requests = [r for r in spec['requests'] if r['case_id'] in eligible]
result = {'status': 'running', 'inputs': inputs, 'eligible_cases': [r['case_id'] for r in requests],
          'excluded_cases': [r['case_id'] for r in spec['requests'] if r['case_id'] not in eligible],
          'repetitions': args.repetitions,
          'eligibility': 'Positive source reads with exact body/error equality; no semantic-only, negative, graph-mismatch or unsupported cases timed.',
          'timing': 'Both persistent local endpoints warmed the full matrix. Client encode/pipe/decode and server dispatch measured separately. Candidate has extra diagnostic envelope/query logging; native performs freshness/metadata work. Not a pure engine comparison.',
          'percentile_method': 'p50 median; p95 nearest rank ceil(0.95*n); raw observations retained.',
          'host': {'platform': platform.platform(), 'logical_cpus': os.cpu_count(), 'load_start': os.getloadavg()},
          'observations': [], 'cases': {}}
endpoints = {}
started = time.time()
try:
    if requests:
        endpoints['candidate'] = parity.JsonlEndpoint(
            [sys.executable, str(ART / 'candidate-approved.py'), 'serve-sql', '--corpus', str(corpus),
             '--extension', str(extension)], out / 'candidate.stderr.jsonl')
        endpoints['native'] = parity.JsonlEndpoint([str(native_path), 'serve', str(corpus)], out / 'native.stderr.jsonl')
        result['startup_wall_ns'] = {k: v.startup_wall_ns for k, v in endpoints.items()}
        with (out / 'raw-pairs.jsonl').open('w') as stream:
            for repetition in range(args.repetitions):
                for index, request in enumerate(requests):
                    replies = {}
                    order = ['native', 'candidate'] if (repetition + index) % 2 == 0 else ['candidate', 'native']
                    for name in order:
                        reply, raw, sample = endpoints[name].request(request)
                        replies[name] = reply
                        row = {'repetition': repetition, 'case_id': request['case_id'], 'endpoint': name,
                               'order': order, 'load': os.getloadavg(), **sample}
                        result['observations'].append(row)
                        stream.write(json.dumps({'observation': row, 'wire_utf8': raw.decode()}, ensure_ascii=False) + '\n')
                    assert parity.payload(replies['native']) == parity.payload(replies['candidate']), request['case_id']
                    assert parity.payload(replies['native']) == parity.payload(native_rows[request['case_id']]), request['case_id']
        for request in requests:
            case = {}
            for name in endpoints:
                rows = [r for r in result['observations'] if r['case_id'] == request['case_id'] and r['endpoint'] == name]
                case[name] = {key: stats([r[key] for r in rows]) for key in ['client_wall_ns', 'dispatch_ns']}
                case[name]['response_bytes'] = sorted({r['response_bytes'] for r in rows})
            result['cases'][request['case_id']] = case
    result['status'] = 'passed' if requests else 'no_eligible_cases'
except BaseException as exc:
    result['status'], result['error'] = 'failed', repr(exc)
    raise
finally:
    close_errors = []
    for name, endpoint in endpoints.items():
        try:
            endpoint.close()
        except BaseException as exc:
            close_errors.append({'endpoint': name, 'error': repr(exc)})
    if close_errors:
        result['status'], result['close_errors'] = 'failed', close_errors
    result['wall_seconds'] = time.time() - started
    result['host']['load_end'] = os.getloadavg()
    parity.write_json(out / 'result.json', result)
    print(json.dumps({k: result[k] for k in ['status', 'eligible_cases', 'repetitions', 'cases']}))
    if close_errors:
        raise RuntimeError(close_errors)
