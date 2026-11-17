"""Exact receipts; host mode is authorized infrastructure recovery, not remote success."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

mode, phase, *filters = sys.argv[1:]
out = Path(__file__).resolve().parent
root = out.parents[3]
command = ["scripts/spur-cargo", "test", "-p", "spur-graph", "--no-default-features", "--lib"] + (filters or ["extract::notebook_symbols"])
overrides = {"SPUR_REMOTE": "0" if mode == "host" else "1", "SPUR_NO_LOCAL_FALLBACK": "1"}
if mode == "host":
    overrides.update(CARGO_TARGET_DIR="/private/tmp/spur-notebook-u1-bd-bi1d-target", CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0", SPUR_SCCACHE_S3="0", RUSTC_WRAPPER="", TMPDIR="/private/tmp/spur-notebook-u1-bd-bi1d-tmp")
sha = lambda b: hashlib.sha256(b).hexdigest()
files = ["Cargo.toml", "Cargo.lock", "scripts/spur-cargo", "crates/spur-graph/Cargo.toml", "crates/spur-graph/src/git_walk.rs"]
files += [f"crates/spur-graph/src/extract/{n}.rs" for n in ["mod", "notebook", "notebook_model", "notebook_symbols", "notebook_symbols_tests", "tree_sitter", "languages"]]
files += ["docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json", "docs/research/artifacts/2026-11-17-notebook-symbol-diagnosis-bd-nb4c/frozen-cell-model-input.json"]
receipt = dict(execution=mode, command=command, environment_overrides=overrides, cwd=str(root), capture_sha256=sha(Path(__file__).read_bytes()), source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), source_sha256={f: sha((root/f).read_bytes()) for f in files}, started_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
env = os.environ.copy(); env.update(overrides)
log = out / f"{phase}.log"
assert not (out/f"{phase}.json").exists(), "receipts are immutable"
receipt["working_diff_sha256"] = sha(subprocess.check_output(["git", "diff", "HEAD", "--", "crates/spur-graph"], cwd=root))
assert not log.exists(), "receipts are immutable"
with log.open("wb") as stream:
    result = subprocess.run(command, env=env, cwd=root, stdout=stream, stderr=subprocess.STDOUT)
receipt.update(exit_code=result.returncode, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), log_sha256=sha(log.read_bytes()))
(out/f"{phase}.json").write_text(json.dumps(receipt, indent=2)+"\n")
print(json.dumps({k:receipt[k] for k in ["execution","command","exit_code","log_sha256"]}))
sys.exit(result.returncode)
