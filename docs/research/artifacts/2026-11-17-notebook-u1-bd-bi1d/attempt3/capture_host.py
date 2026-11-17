"""Retain attempt-3 host receipts without overwriting prior-attempt evidence."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

receipt_dir = Path(__file__).resolve().parent
root = receipt_dir.parents[4]
phase = sys.argv[1]
command = ["scripts/spur-cargo", "test", "-p", "spur-graph", "--no-default-features", "--lib"]
command += sys.argv[2:] or ["extract::notebook"]
overrides = {
    "SPUR_REMOTE": "0",
    "CARGO_TARGET_DIR": "/private/tmp/spur-notebook-u1-bd-bi1d-target",
    "CARGO_INCREMENTAL": "0",
    "CARGO_PROFILE_DEV_DEBUG": "0",
    "SPUR_SCCACHE_S3": "0",
    "TMPDIR": "/private/tmp/spur-notebook-u1-bd-bi1d-tmp",
}
Path(overrides["TMPDIR"]).mkdir(parents=True, exist_ok=True)
sha = lambda data: hashlib.sha256(data).hexdigest()
files = ["Cargo.lock", "Cargo.toml", "crates/spur-graph/Cargo.toml", "scripts/spur-cargo"]
files += [f"crates/spur-graph/src/extract/{f}.rs" for f in ["mod", "notebook", "notebook_model"]]
receipt = {
    "execution": "host (authorized infrastructure-recovery fallback, not remote)",
    "command": command,
    "environment_overrides": overrides,
    "cwd": str(root),
    "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "source_sha256": {f: sha((root / f).read_bytes()) for f in files},
    "test_predicate_sha256": {
        f: sha((root / f).read_bytes().split(b"\n#[cfg(test)]\nmod tests {", 1)[1])
        for f in files[-2:]
    },
    "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
env = os.environ.copy()
env.update(overrides)
with (receipt_dir / f"{phase}.log").open("wb") as log:
    result = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
receipt.update(exit_code=result.returncode, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
receipt["log_sha256"] = sha((receipt_dir / f"{phase}.log").read_bytes())
(receipt_dir / f"{phase}.json").write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt, indent=2))
sys.exit(result.returncode)
