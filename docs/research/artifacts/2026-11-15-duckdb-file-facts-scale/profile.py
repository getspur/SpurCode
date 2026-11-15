"""Sample a dedicated cold run; exclude it from the unprofiled timing matrix."""
import json
from pathlib import Path
import subprocess
import time

OUT = Path(__file__).resolve().parent
folder = OUT / 'profile-control'
folder.mkdir(exist_ok=False)
command = ['/private/tmp/spur-graph-duckdb-v2-20261114/bin/python',
           str(OUT / 'benchmark.py'), 'profile', '--name', 'profile-t4',
           '--threads', '4', '--baseline', 'd-t1-r0']
with (folder / 'child-stdout.log').open('w') as stdout, (folder / 'child-stderr.log').open('w') as stderr:
    proc = subprocess.Popen(command, stdout=stdout, stderr=stderr)
    record = {'command': command, 'target_pid': proc.pid,
              'purpose': 'Dedicated profiler observation, excluded from unprofiled timing medians.'}
    deadline = time.monotonic() + 15
    while proc.poll() is None and time.monotonic() < deadline:
        time.sleep(0.1)
    if proc.poll() is None:
        sample_command = ['/usr/bin/sample', str(proc.pid), '5', '10', '-file', str(folder / 'sample.txt')]
        sampled = subprocess.run(sample_command, capture_output=True, text=True)
        (folder / 'sampler-stdout.log').write_text(sampled.stdout)
        (folder / 'sampler-stderr.log').write_text(sampled.stderr)
        record.update({'sample_command': sample_command, 'sampler_exit': sampled.returncode})
    else:
        record['sampler_skipped'] = 'Target finished before scheduled capture.'
    record['child_exit'] = proc.wait()
record['sample_exists'] = (folder / 'sample.txt').is_file()
(folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record))
raise SystemExit(record['child_exit'])
