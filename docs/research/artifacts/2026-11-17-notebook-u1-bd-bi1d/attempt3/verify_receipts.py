"""Pin tested/committed bytes, retained prior artifacts and exact unified diffs."""
import hashlib
import json
from pathlib import Path
import subprocess

here = Path(__file__).resolve().parent
root = here.parents[4]
sha = lambda data: hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(["git", *args], cwd=root)


prior = git("rev-parse", "e0c32e6b6").decode().strip()
base = "52e14c9cde58f0650bacb72daa04b705fd3c17da"
fixed = git("rev-parse", "0bac16063").decode().strip()
source_paths = [f"crates/spur-graph/src/extract/{f}.rs" for f in ["mod", "notebook", "notebook_model"]]
receipts = {
    phase: json.loads((here / f"{phase}.json").read_text())
    for phase in ["host-red", "host-green", "host-green-final", "host-extract-green"]
}
red = receipts["host-red"]
green = receipts["host-green-final"]
extract = receipts["host-extract-green"]
assert red["exit_code"] == 101
assert all(receipts[p]["exit_code"] == 0 for p in receipts if p != "host-red")
assert red["command"] == green["command"] == receipts["host-green"]["command"]
assert extract["command"] == green["command"][:-1] + ["extract::"]
assert all(r["environment_overrides"] == red["environment_overrides"] for r in receipts.values())
assert all(r["test_predicate_sha256"] == red["test_predicate_sha256"] for r in receipts.values())
assert all(r["cwd"] == str(root) for r in receipts.values())

for phase, receipt in receipts.items():
    assert sha((here / f"{phase}.log").read_bytes()) == receipt["log_sha256"]
for path, expected in red["source_sha256"].items():
    assert sha(git("show", f"{red['source_commit']}:{path}")) == expected
for path, expected in green["source_sha256"].items():
    assert sha(git("show", f"{fixed}:{path}")) == expected
    assert sha((root / path).read_bytes()) == expected
assert green["source_sha256"] == extract["source_sha256"]
for path in red["test_predicate_sha256"]:
    marker = b"\n#[cfg(test)]\nmod tests {"
    assert sha(git("show", f"{fixed}:{path}").split(marker, 1)[1]) == red["test_predicate_sha256"][path]

prior_artifacts = {}
for path in git("ls-tree", "-r", "--name-only", prior, "--", str(here.parent.relative_to(root))).decode().splitlines():
    data = git("show", f"{prior}:{path}")
    assert data == (root / path).read_bytes(), path
    prior_artifacts[path] = sha(data)
for path in source_paths[:2]:
    assert git("show", f"{prior}:{path}") == git("show", f"{fixed}:{path}")

patches = {}
for filename, from_commit in [("rework.patch", prior), ("source.patch", base)]:
    patch = git("diff", "--binary", from_commit, fixed, "--", *source_paths)
    (here / filename).write_bytes(patch)
    result = subprocess.run(
        ["git", "apply", "--reverse", "--check", str(here / filename)],
        cwd=root, capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    patches[filename] = {
        "from_commit": from_commit, "to_commit": fixed, "sha256": sha(patch),
        "reverse_apply_check_exit": result.returncode, "check_output": result.stdout + result.stderr,
    }
check_command = ["git", "diff", "--check", prior, fixed, "--", *source_paths]
check = subprocess.run(check_command, cwd=root, capture_output=True, text=True)
assert check.returncode == 0, check.stdout + check.stderr
changed = git("diff", "--name-only", prior, fixed).decode().splitlines()
assert all(p == source_paths[-1] or p.startswith(str(here.relative_to(root)) + "/") for p in changed)

verification = {
    "base_commit": base, "prior_attempt_commit": prior, "fixed_source_commit": fixed,
    "red_commit": red["source_commit"],
    "host_execution_only": True,
    "red_green_identical_command_environment_predicates": True,
    "green_matches_committed_and_worktree_source_sha256": green["source_sha256"],
    "all_log_hashes_verified": {p: r["log_sha256"] for p, r in receipts.items()},
    "prior_artifacts_verified_unchanged": prior_artifacts,
    "notebook_graph_adapter_and_exports_unchanged": source_paths[:2],
    "source_whitespace_check": {
        "command": check_command, "exit": check.returncode,
        "output": check.stdout + check.stderr,
    },
    "patches": patches,
    "limitations": "Finite fixture evidence. No remote compiled success, Evcxr runtime or full syntax/semantic proof. identity_path is legacy graph compatibility, not a collision-safe symbol preimage.",
}
(here / "verification.json").write_text(json.dumps(verification, indent=2) + "\n")
print(json.dumps({
    "fixed_source_commit": fixed,
    "red_exit": red["exit_code"], "green_exit": green["exit_code"],
    "extract_exit": extract["exit_code"],
    "identical_command_environment_predicates": True,
    "retained_prior_artifact_count": len(prior_artifacts), "patches": patches,
}, indent=2))
