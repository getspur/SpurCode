#!/usr/bin/env python3
"""Reproduce bounded summaries from the committed synthetic benchmark samples."""
import json
import math
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def distribution(values):
    values = sorted(values)
    return {
        'n': len(values),
        'median': statistics.median(values),
        'p95_nearest_rank': values[math.ceil(0.95 * len(values)) - 1],
        'minimum': values[0],
        'maximum': values[-1],
    }


def summarize():
    result = {'request_units': 'microseconds', 'write_units': 'milliseconds',
              'requests': [], 'writes': [], 'allocations': [], 'work': []}
    for stage, file in [('baseline', 'baseline-samples.json'),
                        ('first_fix', 'first-fix-samples.json'),
                        ('final', 'final-samples.json')]:
        rows = json.loads((ROOT / file).read_text())
        assert sum(r['kind'] == 'request' for r in rows) == 248
        if stage == 'final':
            result['environment'] = next(r for r in rows if r['kind'] == 'environment')
        for size in [12, 2331]:
            for shape in ['open_unpaginated', 'all_non_template']:
                for reuse in [False, True]:
                    samples = [r['ns'] / 1000 for r in rows if r['kind'] == 'request'
                               and (r['size'], r['shape'], r['reuse']) == (size, shape, reuse)]
                    assert len(samples) == 31
                    result['requests'].append(dict(stage=stage, size=size, shape=shape,
                                                   reuse=reuse, **distribution(samples)))
        result['work'].extend(dict(stage=stage, **r) for r in rows if r['kind'] == 'work')
        if stage == 'baseline':
            for mode in [0, 1, 2]:
                for kind in ['write', 'insert']:
                    samples = [r['ns'] / 1_000_000 for r in rows
                               if r['kind'] == kind and r['mode'] == mode]
                    result['writes'].append(dict(mode=mode, operation=kind, batch_rows=200,
                                                 **distribution(samples)))
            result['allocations'] = [r for r in rows if r['kind'] == 'allocation']
    return result


if __name__ == '__main__':
    print(json.dumps(summarize(), indent=2))
