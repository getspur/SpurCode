"""Observe an existing macOS process without restarting it or changing caches."""
import argparse
import csv
import ctypes
import datetime as dt
import hashlib
import json
from pathlib import Path
import resource
import sqlite3
import subprocess
import time
import uuid

parser = argparse.ArgumentParser()
parser.add_argument('--pid', type=int, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--mode', choices=['ambient', 'controlled'], default='ambient')
parser.add_argument('--samples', type=int, default=3)
parser.add_argument('--baseline', type=int, default=10)
parser.add_argument('--recovery', type=int, default=10)
args = parser.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
if (out / 'telemetry.csv').exists():
    raise SystemExit('Refusing to overwrite an existing capture.')
fields = ('user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size '
          'resident_size phys_footprint proc_start_abstime proc_exit_abstime '
          'child_user_time child_system_time child_pkg_idle_wkups child_interrupt_wkups '
          'child_pageins child_elapsed_abstime diskio_bytesread diskio_byteswritten').split()

class Usage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_ubyte * 16)] + [(x, ctypes.c_uint64) for x in fields]

class Timebase(ctypes.Structure):
    _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]

libproc = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
libproc.proc_pid_rusage.restype = ctypes.c_int
timebase = Timebase()
assert ctypes.CDLL('/usr/lib/libSystem.B.dylib').mach_timebase_info(ctypes.byref(timebase)) == 0

def usage(pid):
    value = Usage()
    if libproc.proc_pid_rusage(pid, 2, ctypes.byref(value)):
        raise OSError(ctypes.get_errno(), 'proc_pid_rusage failed')
    return {'image_uuid': str(uuid.UUID(bytes=bytes(value.uuid))),
            **{key: getattr(value, key) for key in fields}}

def command(*words):
    return subprocess.check_output(words, text=True).strip()

initial = usage(args.pid)
binary = Path('/Users/kevintruong/.cargo/bin/spur')
meta = {
    'pid': args.pid, 'mode': args.mode,
    'started_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
    'process_identity': command('ps', '-p', str(args.pid), '-o', 'pid=,ppid=,lstart=,comm='),
    'process_image_uuid': initial['image_uuid'],
    'installed_binary_uuid': command('dwarfdump', '--uuid', str(binary)),
    'installed_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'installed_binary_mtime_utc': dt.datetime.fromtimestamp(binary.stat().st_mtime, dt.timezone.utc).isoformat(),
    'version': command(str(binary), '--version'),
    'workspace_head': command('git', 'rev-parse', 'HEAD'),
    'exact_running_git_commit': None,
    'build_evidence': 'Loaded UUID matches installed image. Binary strings include graph_engine/incremental.rs and indexed label-query SQL. Exact embedded git revision not exposed by --version.',
    'os': command('sw_vers'),
    'logical_cpus': int(command('sysctl', '-n', 'hw.logicalcpu')),
    'physical_memory_bytes': int(command('sysctl', '-n', 'hw.memsize')),
    'method': 'proc_pid_rusage RUSAGE_INFO_V2 once per second; CPU counters converted from Mach ticks; preflight conversion agrees with ps cumulative CPU time',
    'timebase': {'numer': timebase.numer, 'denom': timebase.denom},
    'scope': 'One existing warm process; child CPU excluded; process and caches preserved',
    'sampling_interval_ms_requested': 5,
    'sample_duration_s_requested': 10,
    'db_probe': 'One persistent read-only SQLite connection: data_version, graph revision and schema_version; no issue/dependency scans',
    'workload': ('Ambient session/background work only; no graph retrieval, issue mutation or notebook execution initiated during the window'
                 if args.mode == 'ambient' else 'Explicit read-only graph requests recorded separately in requests.json; ambient work may continue'),
    'windows': [],
}
assert initial['image_uuid'].upper() in meta['installed_binary_uuid']
db = sqlite3.connect('file:/Volumes/Projects/spur/.beads/beads.db?mode=ro', uri=True, isolation_level=None, timeout=0.25)
db.execute('PRAGMA query_only=ON')
tracking = db.execute("SELECT 1 FROM sqlite_schema WHERE name='spur_graph_clock'").fetchone() is not None
meta['graph_tracking_present'] = tracking
meta['ps_cpu_before'] = command('ps', '-p', str(args.pid), '-o', 'time=')
start = time.monotonic()
with (out / 'telemetry.csv').open('w', newline='') as stream:
    writer = csv.DictWriter(stream, fieldnames=['utc', 'elapsed_s', 'phase', 'image_uuid',
        'db_data_version', 'graph_revision', 'schema_version', 'db_probe_error'] + fields)
    writer.writeheader()

    def record(phase):
        row = usage(args.pid)
        if row['proc_start_abstime'] != initial['proc_start_abstime'] or row['image_uuid'] != initial['image_uuid']:
            raise RuntimeError('Process identity changed during capture')
        try:
            # One coherent tiny read view; this does not write or scan graph data.
            db.execute('BEGIN')
            row['db_data_version'] = db.execute('PRAGMA data_version').fetchone()[0]
            row['schema_version'] = db.execute('PRAGMA schema_version').fetchone()[0]
            row['graph_revision'] = db.execute('SELECT revision FROM spur_graph_clock WHERE id=1').fetchone()[0] if tracking else ''
            db.execute('COMMIT')
            row['db_probe_error'] = ''
        except sqlite3.Error as exc:
            if db.in_transaction:
                db.execute('ROLLBACK')
            row['db_probe_error'] = str(exc)
        row.update(utc=dt.datetime.now(dt.timezone.utc).isoformat(), elapsed_s=time.monotonic()-start, phase=phase)
        writer.writerow(row)
        stream.flush()

    def observe(phase, duration):
        end = time.monotonic() + duration
        record(phase)
        while time.monotonic() < end:
            time.sleep(min(1, max(0, end-time.monotonic())))
            record(phase)

    observe('baseline', args.baseline)
    for index in range(1, args.samples + 1):
        phase = f'sample_{index}'
        words = ['/usr/bin/sample', str(args.pid), '10', '5', '-file', str(out / f'{phase}.txt')]
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        window = {'phase': phase, 'command': words, 'command_start_s': time.monotonic()-start}
        (out / 'progress.json').write_text(json.dumps({'phase': phase, 'utc': dt.datetime.now(dt.timezone.utc).isoformat()}))
        with (out / f'{phase}.log').open('w') as log:
            child = subprocess.Popen(words, stdout=log, stderr=subprocess.STDOUT)
            record(phase)
            while child.poll() is None:
                time.sleep(1)
                record(phase)
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        window.update(command_end_s=time.monotonic()-start, returncode=child.returncode,
                      sampler_user_s=after.ru_utime-before.ru_utime,
                      sampler_system_s=after.ru_stime-before.ru_stime)
        meta['windows'].append(window)
        if child.returncode:
            raise RuntimeError(f'Native sample failed: {phase}')
    observe('recovery', args.recovery)
db.close()
meta['elapsed_s'] = time.monotonic()-start
meta['ended_utc'] = dt.datetime.now(dt.timezone.utc).isoformat()
meta['ps_cpu_after'] = command('ps', '-p', str(args.pid), '-o', 'time=')
(out / 'capture.json').write_text(json.dumps(meta, indent=2) + '\n')
print(json.dumps({'pid': args.pid, 'mode': args.mode, 'elapsed_s': meta['elapsed_s'], 'samples': len(meta['windows']), 'output': str(out)}))
