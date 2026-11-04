#!/usr/bin/env python3
"""Root review: signal the actual CLI during startup and active work.
All children are protocol fixtures; failed cases are cleaned after recording.
"""
import gzip
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = next(p for p in HERE.parents if (p / "crates/spur-analyst").is_dir())
BINARY = Path(os.environ["SPUR_SIGNAL_TEST_BINARY"])
EXPECTED = hashlib.sha256(BINARY.read_bytes()).hexdigest()
FIXTURE = ROOT / "crates/spur-analyst/tests/embedding/fixtures/owned_fault_worker.py"
OUT = Path(os.environ.get("SPUR_SIGNAL_TEST_OUTPUT") or tempfile.mkdtemp(prefix="spur-signal-results-"))
OUT.mkdir(parents=True, exist_ok=True)

def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True

def wait_until(predicate, proc):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if predicate():
            return
        if proc.poll() is not None:
            raise AssertionError(("CLI exited before case setup", proc.returncode))
        time.sleep(0.01)
    raise TimeoutError("case setup")

def main():
    observations = []
    cases = [
        ("analyst_warm_term", "analyst", "warm", signal.SIGTERM),
        ("bundled_warm_term", "bundled", "warm", signal.SIGTERM),
        ("analyst_warm_int", "analyst", "warm", signal.SIGINT),
        ("analyst_query_term", "analyst", "query", signal.SIGTERM),
        ("analyst_idle_term", "analyst", "idle", signal.SIGTERM),
        ("bundled_query_term", "bundled", "query", signal.SIGTERM),
        ("bundled_idle_int", "bundled", "idle", signal.SIGINT),
        ("analyst_idle_cleanup_error", "analyst", "idle", signal.SIGTERM),
    ]
    for name, kind, phase, sig in cases:
        with tempfile.TemporaryDirectory(prefix="spur-s6b-signal-") as tmp:
            tmp = Path(tmp)
            (tmp / ".spur").mkdir()
            (tmp / ".spur/analyst.duckdb").touch()
            log = tmp / "events"
            stdout_path, stderr_path = tmp / "stdout", tmp / "stderr"
            config = {
                "executable": sys.executable, "args": [str(FIXTURE)], "artifact_budget": 10000,
                "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
                           "max_total_text_bytes": 16384, "max_dimensions": 768,
                           "max_identity_bytes": 256, "max_pending": 1},
                "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
                "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000,
                "shutdown_poll_interval_ms": 10,
            }
            fault = "descriptor_hang" if phase == "warm" else ("query_hang_after_warm" if phase == "query" else "")
            if name == "analyst_idle_cleanup_error":
                fault = "shutdown_abort"
            env = dict(os.environ, SPUR_TELEMETRY="0", SPUR_PROJECT_CATALOG=str(tmp / "projects.toml"),
                       SPUR_ANALYST_EMBED_MODE="inprocess", SPUR_EMBED_SOCKET=str(tmp / "missing.sock"),
                       SPUR_GRAPH_TEST_WORKER_LOG=str(log), SPUR_GRAPH_TEST_WORKER_FAULT=fault,
                       SPUR_EMBEDDING_HELPER_CONFIG=json.dumps(config))
            command = [str(BINARY), *(["analyst", "mcp"] if kind == "analyst" else ["mcp"]), "--root", str(tmp)]
            errors = []
            status, pids, remaining, events, setup = None, [], [], [], False
            with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
                proc = subprocess.Popen(command, cwd=tmp, env=env, stdin=subprocess.PIPE, stdout=stdout, stderr=stderr, start_new_session=True)
                def event_lines():
                    return log.read_text().splitlines() if log.exists() else []
                try:
                    if phase == "warm":
                        wait_until(lambda: any(x.startswith("descriptor_pid ") for x in event_lines()), proc)
                    else:
                        init = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                                "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                                           "clientInfo": {"name": "root-signal-review", "version": "1"}}}
                        proc.stdin.write(json.dumps(init).encode() + b"\n")
                        proc.stdin.flush()
                        wait_until(lambda: any(json.loads(x).get("id") == 1 for x in stdout_path.read_text().splitlines()), proc)
                        if phase == "query":
                            proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}).encode() + b"\n")
                            proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                                       "params": {"name": "knowledge_context_pack_2", "arguments": {"query": "root blocked query"}}}).encode() + b"\n")
                            proc.stdin.flush()
                            wait_until(lambda: sum(x.startswith("embed ") for x in event_lines()) == 2, proc)
                    setup = True
                    proc.send_signal(sig)
                    status = proc.wait(timeout=15)
                    # Observe before experiment cleanup. A live recorded PID,
                    # including an unreaped child, contradicts joined shutdown.
                    deadline = time.monotonic() + 1
                    pids = [int(x.split()[1]) for x in event_lines() if x.startswith(("service_pid ", "descriptor_pid "))]
                    while time.monotonic() < deadline and any(alive(pid) for pid in pids):
                        time.sleep(0.01)
                    remaining = [pid for pid in pids if alive(pid)]
                    events = event_lines()
                    if remaining:
                        errors.append("recorded fixture still exists after CLI return")
                    if status == 0:
                        errors.append("signal termination returned a successful status")
                    if name == "analyst_idle_cleanup_error" and "command embedding shutdown failed" not in stderr_path.read_text():
                        errors.append("cleanup failure was not reported")
                except Exception as error:
                    errors.append(type(error).__name__ + ": " + str(error))
                    events = event_lines()
                    pids = [int(x.split()[1]) for x in events if x.startswith(("service_pid ", "descriptor_pid "))]
                    remaining = [pid for pid in pids if alive(pid)]
                finally:
                    # Only this newly-created process group, never unrelated PIDs.
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    if proc.poll() is None:
                        proc.wait(timeout=10)
                    if proc.stdin:
                        proc.stdin.close()
            stdout_bytes, stderr_bytes = stdout_path.read_bytes(), stderr_path.read_bytes()
            (OUT / (name + ".stdout.gz")).write_bytes(gzip.compress(stdout_bytes, mtime=0))
            (OUT / (name + ".stderr.gz")).write_bytes(gzip.compress(stderr_bytes, mtime=0))
            (OUT / (name + ".events.json")).write_text(json.dumps(events, indent=2) + "\n")
            observations.append({"case": name, "command": command, "phase": phase, "signal": sig.name,
                                 "setup_complete": setup, "exit": status, "recorded_fixture_pids": pids,
                                 "alive_before_observer_cleanup": remaining, "errors": errors,
                                 "observer_cleanup": "SIGKILL to experiment process group after observation",
                                 "binary_sha256": EXPECTED, "fixture_sha256": hashlib.sha256(FIXTURE.read_bytes()).hexdigest(),
                                 "config": config})
    (OUT / "observations.json").write_text(json.dumps(observations, indent=2) + "\n")
    failures = [x["case"] for x in observations if x["errors"]]
    print(json.dumps({"cases": len(observations), "failures": failures, "output": str(OUT), "observations": observations}))
    raise SystemExit(1 if failures else 0)

if __name__ == "__main__":
    main()
