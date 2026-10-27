"""Observe existing macOS processes; never send signals or mutate their data."""
import argparse
import ctypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time
import uuid


class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_ubyte * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups",
            "pageins", "wired_size", "resident_size", "phys_footprint",
            "proc_start_abstime", "proc_exit_abstime", "child_user_time",
            "child_system_time", "child_pkg_idle_wkups", "child_interrupt_wkups",
            "child_pageins", "child_elapsed_abstime", "diskio_bytesread",
            "diskio_byteswritten",
        )
    ]


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pids", nargs="+", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=35)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    lib.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    lib.proc_pid_rusage.restype = ctypes.c_int
    clock = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
    clock.mach_timebase_info.argtypes = [ctypes.POINTER(Timebase)]
    clock.mach_timebase_info.restype = ctypes.c_int
    timebase = Timebase()
    if clock.mach_timebase_info(ctypes.byref(timebase)) != 0 or not timebase.denom:
        raise RuntimeError("mach timebase unavailable")
    seconds_per_tick = timebase.numer / timebase.denom / 1e9

    def snapshot():
        values = {}
        for pid in args.pids:
            usage = Usage()
            if lib.proc_pid_rusage(pid, 2, ctypes.byref(usage)) != 0:
                raise OSError(ctypes.get_errno(), f"proc_pid_rusage({pid})")
            values[pid] = {name: getattr(usage, name) for name, _ in Usage._fields_ if name != "uuid"}
            values[pid]["image_uuid"] = str(uuid.UUID(bytes=bytes(usage.uuid)))
        return values

    metadata = {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(), "machine": platform.machine(),
        "logical_cpus": os.cpu_count(), "sample_interval_ms": 1,
        "sample_seconds_per_process": args.seconds,
        "processes": subprocess.check_output(
            ["ps", "-p", ",".join(map(str, args.pids)), "-o", "pid=,lstart=,time=,rss=,args="], text=True
        ),
        "initial_counters": snapshot(),
        "mach_timebase": {"numer": timebase.numer, "denom": timebase.denom},
        "notes": "Live observational windows; no workload replay, no cache purge, no maintenance disable/enable. CPU percentages convert raw rusage Mach ticks via mach_timebase_info; stack counts are wall-thread observations, including waits.",
    }
    binary = Path("/Users/kevintruong/.cargo/bin/spur")
    image = binary.read_bytes()
    metadata["binary"] = {
        "path": str(binary), "sha256": hashlib.sha256(image).hexdigest(),
        "mtime_epoch": binary.stat().st_mtime,
        "maintenance_marker": b"history directory must be a real directory" in image,
        "stamp_marker": b".spur-maintenance.json" in image,
    }
    del image
    rows = []

    def observe(label, action):
        before = snapshot()
        started = time.monotonic()
        action()
        elapsed = time.monotonic() - started
        after = snapshot()
        for pid in args.pids:
            old, new = before[pid], after[pid]
            assert old["proc_start_abstime"] == new["proc_start_abstime"], "PID was reused"
            raw_user = new["user_time"] - old["user_time"]
            raw_system = new["system_time"] - old["system_time"]
            user = raw_user * seconds_per_tick
            system = raw_system * seconds_per_tick
            row = {
                "phase": label, "pid": pid, "wall_seconds": elapsed,
                "raw_user_ticks": raw_user, "raw_system_ticks": raw_system,
                "user_seconds": user, "system_seconds": system,
                "cpu_core_percent": 100 * (user + system) / elapsed,
                "resident_bytes": new["resident_size"], "footprint_bytes": new["phys_footprint"],
                "resident_delta_bytes": new["resident_size"] - old["resident_size"],
                "disk_read_bytes": new["diskio_bytesread"] - old["diskio_bytesread"],
                "disk_write_bytes": new["diskio_byteswritten"] - old["diskio_byteswritten"],
                "pageins": new["pageins"] - old["pageins"],
                "idle_wakeups": new["pkg_idle_wkups"] - old["pkg_idle_wkups"],
                "image_uuid": new["image_uuid"],
            }
            rows.append(row)
            print(json.dumps(row), flush=True)
        (args.output / "counters.json").write_text(json.dumps({"metadata": metadata, "observations": rows}, indent=2) + "\n")

    observe("before_sampling", lambda: time.sleep(15))
    for pid in args.pids:
        def capture(pid=pid):
            with (args.output / f"spur-{pid}.sample.log").open("w") as log:
                subprocess.run(["/usr/bin/sample", str(pid), str(args.seconds), "1", "-mayDie", "-file", str(args.output / f"spur-{pid}.sample.txt")], stdout=log, stderr=subprocess.STDOUT, check=True)
        observe(f"sample_{pid}", capture)
    observe("after_sampling", lambda: time.sleep(15))


if __name__ == "__main__":
    main()
