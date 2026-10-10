"""Reproducible analysis used by the comparison notebook (standard library)."""
from collections import Counter
import csv
import json
import math
from pathlib import Path
import re
import statistics

WAIT = re.compile(r'^(?:__psynch_(?:cvwait|mutexwait|rw_\w+)|semaphore_(?:timed)?wait_trap|'
                  r'kevent(?:64)?|__workq_kernreturn|__ulock_wait\w*|mach_msg\w*(?:trap|internal)|'
                  r'__select|select|poll|swtch_pri)$')

def symbol(frame):
    return re.split(r'\s+\(in\s', frame, maxsplit=1)[0].strip()

def stable_symbol(frame):
    return re.sub(r'::h[0-9a-f]{16}$', '', symbol(frame))

def parse_sample(path):
    text = Path(path).read_text()
    body = text.split('Call graph:\n', 1)[1].split('\nTotal number in stack', 1)[0]
    nodes, stack, roots = [], [], []
    for line in body.splitlines():
        m = re.match(r'^([ +!:|]*)(\d+) (.+)$', line)
        if not m:
            continue
        indent, count, frame = len(m[1]), int(m[2]), m[3]
        while stack and nodes[stack[-1]]['indent'] >= indent:
            stack.pop()
        parent = stack[-1] if stack else None
        node = {'indent': indent, 'count': count, 'frame': frame, 'parent': parent, 'children': []}
        index = len(nodes)
        nodes.append(node)
        if parent is None:
            assert 'Thread_' in frame, frame
            roots.append(index)
        else:
            nodes[parent]['children'].append(index)
        stack.append(index)
    total = sum(nodes[i]['count'] for i in roots)
    leaf_counts, categories, app_frames = Counter(), Counter(), Counter()
    signals = Counter()
    wait_count = exclusive_total = 0
    for node in nodes:
        weight = node['count'] - sum(nodes[i]['count'] for i in node['children'])
        assert weight >= 0, node
        if not weight:
            continue
        exclusive_total += weight
        leaf = symbol(node['frame'])
        leaf_counts[leaf] += weight
        chain = [node['frame']]
        parent = node['parent']
        while parent is not None:
            chain.append(nodes[parent]['frame'])
            parent = nodes[parent]['parent']
        inclusive = '\n'.join(chain)
        for key, token in {
            'snapshot_inclusive': 'load_graph_snapshot',
            'whole_graph_hash_inclusive': 'compute_data_hash',
            'incremental_request_inclusive': 'graph_engine::incremental::',
            'initial_load_inclusive': 'incremental::load_initial',
            'delta_load_inclusive': 'incremental::load_delta',
            'hygiene_inclusive': 'run_index_hygiene_sweep',
        }.items():
            if token in inclusive:
                signals[key] += weight
        if WAIT.match(leaf):
            wait_count += weight
            continue
        if 'load_graph_snapshot' in inclusive or 'compute_data_hash' in inclusive:
            category = 'Legacy snapshot/hash'
        elif 'graph_engine::incremental::' in inclusive:
            category = 'Incremental graph request'
        elif any(x in inclusive for x in ('spur_tui', 'ratatui', 'crossterm')):
            category = 'TUI/session work'
        elif any(x in inclusive for x in ('beads_rust', 'spur_pm::beads_crate', 'sqlite3', 'rusqlite')):
            category = 'Other beads/SQLite work'
        elif 'spur_core::plan::reconciler' in inclusive:
            category = 'Reconciler'
        else:
            category = 'Other/unresolved'
        categories[category] += weight
        caller = next((stable_symbol(f) for f in chain if 'spur_' in f or 'beads_rust' in f), '(no resolved application frame)')
        app_frames[caller] += weight
    assert exclusive_total == total
    assert sum(categories.values()) + wait_count == total
    # Independent check against macOS sample's own collapsed leaf totals.
    collapsed = text.split('Sort by top of stack, same collapsed (when >= 5):\n', 1)[1].split('\nBinary Images:', 1)[0]
    checks = 0
    for line in collapsed.splitlines():
        m = re.match(r'^\s+(.+?)\s{2,}(\d+)\s*$', line)
        if m:
            name, count = symbol(m[1]), int(m[2])
            assert leaf_counts[name] == count, (path, name, leaf_counts[name], count)
            checks += 1
    assert checks > 0
    return {
        'sample': Path(path).stem, 'threads': len(roots),
        'total_thread_observations': total, 'known_wait_observations': wait_count,
        'other_observations': total-wait_count,
        **{k: signals[k] for k in ('snapshot_inclusive', 'whole_graph_hash_inclusive',
            'incremental_request_inclusive', 'initial_load_inclusive', 'delta_load_inclusive', 'hygiene_inclusive')},
        'non_wait_categories': dict(categories),
        'top_application_frames_excluding_known_waits': app_frames.most_common(12),
        'collapsed_leaf_checks_passed': checks,
    }

def evaluate_capture(directory):
    directory = Path(directory)
    meta = json.loads((directory / 'capture.json').read_text())
    rows = list(csv.DictReader((directory / 'telemetry.csv').open()))
    factor = meta['timebase']['numer'] / meta['timebase']['denom'] / 1e9
    intervals = []
    phases = {}
    for a, b in zip(rows, rows[1:]):
        wall = float(b['elapsed_s']) - float(a['elapsed_s'])
        user = (int(b['user_time']) - int(a['user_time'])) * factor
        system = (int(b['system_time']) - int(a['system_time'])) * factor
        assert wall > 0 and user >= 0 and system >= 0
        phase = b['phase']
        intervals.append({'elapsed_s': float(b['elapsed_s']), 'phase': phase,
                          'wall_s': wall, 'user_s': user, 'system_s': system,
                          'cpu_pct': 100*(user+system)/wall})
        entry = phases.setdefault(phase, {'wall_s': 0, 'cpu_s': 0})
        entry['wall_s'] += wall
        entry['cpu_s'] += user + system
    for entry in phases.values():
        entry['avg_cpu_pct'] = 100*entry['cpu_s']/entry['wall_s']
    def changes(column):
        values = [r[column] for r in rows if r.get(column, '') != '']
        return {'values': sorted(set(map(int, values))),
                'transitions': sum(a != b for a, b in zip(values, values[1:]))}
    wall = float(rows[-1]['elapsed_s']) - float(rows[0]['elapsed_s'])
    cpu = sum(i['user_s'] + i['system_s'] for i in intervals)
    summary = {
        'pid': meta['pid'], 'wall_s': wall, 'cpu_s': cpu,
        'avg_cpu_pct': 100*cpu/wall,
        'peak_one_second_interval_cpu_pct': max(i['cpu_pct'] for i in intervals if i['wall_s'] >= 0.5),
        'db_data_version': changes('db_data_version'),
        'graph_revision': changes('graph_revision'),
        'schema_version': changes('schema_version'),
        'db_probe_errors': [r['db_probe_error'] for r in rows if r['db_probe_error']],
        'rss_start_mib': int(rows[0]['resident_size']) / 2**20,
        'rss_end_mib': int(rows[-1]['resident_size']) / 2**20,
        'footprint_start_mib': int(rows[0]['phys_footprint']) / 2**20,
        'footprint_end_mib': int(rows[-1]['phys_footprint']) / 2**20,
        'disk_read_mib': (int(rows[-1]['diskio_bytesread'])-int(rows[0]['diskio_bytesread'])) / 2**20,
        'disk_written_mib': (int(rows[-1]['diskio_byteswritten'])-int(rows[0]['diskio_byteswritten'])) / 2**20,
        'phase_metrics': phases,
        'samples': [parse_sample(p) for p in sorted(directory.glob('sample_*.txt'))],
    }
    assert not summary['db_probe_errors']
    return {'metadata': meta, 'rows': rows, 'intervals': intervals, 'summary': summary}

def evaluate_requests(path):
    payload = json.loads(Path(path).read_text())
    responses = []
    for row in payload['requests']:
        assert not row['response'].get('isError', False)
        response = json.loads(next(c['text'] for c in row['response']['content'] if c['type'] == 'text'))
        assert response['data_hash'].startswith('g2:')
        responses.append(response)
    values = sorted(r['elapsed_ms'] for r in payload['requests'])
    def percentile(p):
        x = (len(values)-1)*p
        lo, hi = math.floor(x), math.ceil(x)
        return values[lo] + (values[hi]-values[lo])*(x-lo)
    return {
        'count': len(values), 'p50_roundtrip_ms': statistics.median(values),
        'p95_roundtrip_ms': percentile(0.95), 'max_roundtrip_ms': max(values),
        'min_roundtrip_ms': min(values), 'unique_hashes': sorted({r['data_hash'] for r in responses}),
        'identical_responses': all(r == responses[0] for r in responses),
        'nodes_returned': responses[0]['nodes'], 'edges_returned': responses[0]['edges'],
        'timing_scope': payload['timing_scope'],
    }
