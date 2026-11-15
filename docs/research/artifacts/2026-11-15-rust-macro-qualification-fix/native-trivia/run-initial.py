import datetime, hashlib, json, os, pathlib, subprocess, sys
root = pathlib.Path(__file__).resolve().parent
label = sys.argv[1]
argv = sys.argv[2:]
env = os.environ.copy()
env.update(SPUR_REMOTE="1", SPUR_NO_LOCAL_FALLBACK="1")
head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
started = datetime.datetime.now(datetime.timezone.utc).isoformat()
log = root / (label + ".log")
with log.open("wb") as out:
    p = subprocess.Popen(argv, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    while True:
        data = p.stdout.read1(65536)
        if not data:
            break
        out.write(data)
        out.flush()
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()
    code = p.wait()
result = {"argv": argv, "environment": {k: env[k] for k in ["SPUR_REMOTE", "SPUR_NO_LOCAL_FALLBACK"]}, "cwd": os.getcwd(), "head": head, "started_utc": started, "finished_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "exit_code": code, "log": str(log), "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest()}
(root / (label + ".json")).write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result), flush=True)
sys.exit(code)
