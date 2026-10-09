"""Observe one existing macOS process; no restart or synthetic workload."""
import csv
import ctypes
import datetime as dt
import json
from pathlib import Path
import resource
import sqlite3
import subprocess
import time

PID = 71291
ROOT = Path(__file__).resolve().parent
FIELDS = ('user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size '
          'resident_size phys_footprint proc_start_abstime proc_exit_abstime '
          'child_user_time child_system_time child_pkg_idle_wkups child_interrupt_wkups '
          'child_pageins child_elapsed_abstime diskio_bytesread diskio_byteswritten').split()

class Usage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_ubyte * 16)] + [(x, ctypes.c_uint64) for x in FIELDS]

libproc = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
libproc.proc_pid_rusage.restype = ctypes.c_int

class Timebase(ctypes.Structure):
    _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]

timebase = Timebase()
assert ctypes.CDLL('/usr/lib/libSystem.B.dylib').mach_timebase_info(ctypes.byref(timebase)) == 0

def usage():
    value = Usage()
    if libproc.proc_pid_rusage(PID, 2, ctypes.byref(value)):
        raise OSError(ctypes.get_errno(), 'proc_pid_rusage failed')
    return {key: getattr(value, key) for key in FIELDS}

def command(*args):
    return subprocess.check_output(args, text=True).strip()

if (ROOT / 'telemetry.csv').exists():
    raise SystemExit('Capture already exists; use a fresh directory for another capture.')
meta = {
    'pid': PID,
    'started_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
    'process_identity': command('ps', '-p', str(PID), '-o', 'pid=,lstart=,comm='),
    'os': command('sw_vers'),
    'logical_cpus': int(command('sysctl', '-n', 'hw.logicalcpu')),
    'physical_memory_bytes': int(command('sysctl', '-n', 'hw.memsize')),
    'method': 'proc_pid_rusage RUSAGE_INFO_V2 once per second; CPU counters in Mach absolute-time ticks',
    'timebase': {'numer': timebase.numer, 'denom': timebase.denom},
    'scope': 'Single warm existing process; child CPU excluded; no injected workload',
    'sampling_interval_ms_requested': 5,
    'sample_duration_s_requested': 10,
    'windows': [],
}
db = sqlite3.connect('file:/Volumes/Projects/spur/.beads/beads.db?mode=ro', uri=True, isolation_level=None, timeout=0.25)
db.execute('PRAGMA query_only=ON')
meta['db_path'] = '/Volumes/Projects/spur/.beads/beads.db'
meta['db_probe'] = 'PRAGMA data_version on one persistent read-only connection; changes indicate commits from other connections'
meta['workload'] = 'No graph retrieval, issue mutations, or notebook execution initiated by investigator during capture; ordinary session/background work may continue.'
start = time.monotonic()
identity = usage()['proc_start_abstime']
with (ROOT / 'telemetry.csv').open('w', newline='') as stream:
    writer = csv.DictWriter(stream, fieldnames=['utc', 'elapsed_s', 'phase', 'db_data_version', 'db_probe_error'] + FIELDS)
    writer.writeheader()

    def record(phase):
        row = usage()
        try:
            row['db_data_version'] = db.execute('PRAGMA data_version').fetchone()[0]
            row['db_probe_error'] = ''
        except sqlite3.Error as exc:
            row['db_data_version'] = ''
            row['db_probe_error'] = str(exc)
        if row['proc_start_abstime'] != identity:
            raise RuntimeError('PID identity changed')
        row.update(utc=dt.datetime.now(dt.timezone.utc).isoformat(),
                   elapsed_s=time.monotonic() - start, phase=phase)
        writer.writerow(row)
        stream.flush()

    def observe(phase, duration):
        end = time.monotonic() + duration
        record(phase)
        while time.monotonic() < end:
            time.sleep(min(1, max(0, end-time.monotonic())))
            record(phase)

    observe('baseline', 10)
    for index in range(1, 4):
        phase = f'sample_{index}'
        args = ['/usr/bin/sample', str(PID), '10', '5', '-file', str(ROOT / f'{phase}.txt')]
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        window = {'phase': phase, 'command': args, 'command_start_s': time.monotonic()-start}
        with (ROOT / f'{phase}.log').open('w') as log:
            child = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
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
            raise RuntimeError(f'sample failed: {phase}; see log')
    observe('recovery', 10)
db.close()
meta['ended_utc'] = dt.datetime.now(dt.timezone.utc).isoformat()
meta['elapsed_s'] = time.monotonic()-start
(ROOT / 'capture.json').write_text(json.dumps(meta, indent=2))
print(json.dumps(meta, indent=2))
