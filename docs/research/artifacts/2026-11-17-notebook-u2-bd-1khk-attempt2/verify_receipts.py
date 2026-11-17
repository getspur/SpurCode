"""Verify immutable RED/GREEN identity, source bytes, and bounded handoff claims."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

out = Path(__file__).resolve().parent
root = out.parents[3]
sha = lambda b: hashlib.sha256(b).hexdigest()
git = lambda *args: subprocess.check_output(["git", *args], cwd=root)
source = "crates/spur-graph/src/extract/notebook_symbols.rs"
tests = "crates/spur-graph/src/extract/notebook_symbols_tests.rs"
base = "033b9edad"
test_commit = "6fa59d9f2"
initial_phases = ["remote-red", "host-red", "host-green"]
phases = initial_phases + ["host-red-final", "host-green-final", "host-extract-green", "host-matcher-green"]
receipts = {phase: json.loads((out / f"{phase}.json").read_text()) for phase in phases}
logs = {phase: (out / f"{phase}.log").read_text() for phase in phases}
for phase, receipt in receipts.items():
    assert receipt["log_sha256"] == sha((out / f"{phase}.log").read_bytes()), phase
    assert receipt["capture_sha256"] == sha((out / "capture.py").read_bytes()), phase
    assert receipt["cwd"] == str(root), phase
    predicate_commit = "bce9f671a" if phase in initial_phases else test_commit
    assert receipt["source_sha256"][tests] == sha(git("show", f"{predicate_commit}:{tests}")), phase

remote = receipts["remote-red"]
assert remote["exit_code"] == 200 and remote["execution"] == "remote"
assert "all remote clouds unavailable" in logs["remote-red"]
assert "Compiling " not in logs["remote-red"]
red, green = receipts["host-red-final"], receipts["host-green-final"]
assert red["exit_code"] == 101 and green["exit_code"] == 0
assert red["command"] == green["command"] == remote["command"]
assert red["environment_overrides"] == green["environment_overrides"]
assert red["environment_overrides"]["SPUR_REMOTE"] == "0"
assert red["environment_overrides"]["RUSTC_WRAPPER"] == ""
assert red["source_sha256"][source] == sha(git("show", f"{base}:{source}"))
assert red["working_diff_sha256"] == sha(git("diff", "bce9f671a", test_commit, "--", "crates/spur-graph"))
assert remote["source_sha256"] == receipts["host-red"]["source_sha256"]
assert receipts["host-red"]["exit_code"] == 101
assert receipts["host-green"]["exit_code"] == 101
assert "1 passed; 6 failed; 0 ignored" in logs["host-red"]
assert "6 passed; 1 failed; 0 ignored" in logs["host-green"]
assert "0 passed; 7 failed; 0 ignored" in logs["host-red-final"]
assert "7 passed; 0 failed; 0 ignored" in logs["host-green-final"]
test_names = lambda s: set(re.findall(r"^test (\S+) \.\.\. (?:ok|FAILED)$", s, re.M))
assert len(test_names(logs["host-red-final"])) == 7
assert test_names(logs["host-red-final"]) == test_names(logs["host-green-final"])
assert set(test_names(logs["host-red-final"])) <= test_names(logs["host-extract-green"])

for phase in ["host-green-final", "host-extract-green", "host-matcher-green"]:
    receipt = receipts[phase]
    assert receipt["execution"] == "host" and receipt["exit_code"] == 0
    assert receipt["environment_overrides"] == red["environment_overrides"]
    assert receipt["working_diff_sha256"] == sha(git("diff", test_commit, "--", "crates/spur-graph"))
    for name, expected in receipt["source_sha256"].items():
        assert sha((root / name).read_bytes()) == expected, (phase, name)
assert "143 passed; 0 failed; 0 ignored" in logs["host-extract-green"]
assert "48 passed; 0 failed; 0 ignored" in logs["host-matcher-green"]
assert "frozen real comparison: 50 symbol changes; 10 cell events" in logs["host-extract-green"]

for phase in ["pre", "post-final"]:
    checks = json.loads((out / f"solver-{phase}.json").read_text())["checks"]
    assert len(checks) == 6
    for check in checks:
        assert check["request"]["persist"] is False
        result = json.loads(check["result"]["content"][0]["text"])
        assert result["outcome"] == check["expected"]
        assert result["status"] == ("sat" if check["expected"] == "pass" else "unsat")
pre = json.loads((out / "solver-pre.json").read_text())["checks"]
post = json.loads((out / "solver-post-final.json").read_text())["checks"]
assert [c["request"] for c in pre] == [c["request"] for c in post]

assert (out / "rework-final.patch").read_bytes() == git("diff", base, "--", "crates/spur-graph")
prior_evidence = "docs/research/artifacts/2026-11-17-notebook-u2-bd-1khk"
assert not git("diff", base, "--", prior_evidence)
for commit in ["6eb1c4cf6", base, test_commit]:
    subprocess.run(["git", "merge-base", "--is-ancestor", commit, "HEAD"], cwd=root, check=True)
# All public definitions and both entry point signatures precede this marker.
marker = b"    let before = left_source"
assert (root / source).read_bytes().split(marker)[0] == git("show", f"{base}:{source}").split(marker)[0]
assert not git("diff", base, "--", "crates/spur-graph/src/git_walk.rs", "crates/spur-graph/src/extract/notebook_model.rs", "crates/spur-graph/src/extract/mod.rs", "crates/spur-graph-duckdb")
summary = {
    "red": "host: final seven-test predicate failed meaningfully against unchanged production; exit 101",
    "green": "host: 7 regressions, 143 extraction tests, 48 matcher tests; exit 0",
    "remote": "both routes unavailable before compilation; exit 200; not a remote test result",
    "frozen": "46/24 symbols, 17/11 functions, 28 anchors asserted; 50 symbol changes / 10 cell events retained",
    "solver": "six finite snapshots replayed pre/post; expected pass/fail, no unbounded proof",
    "rework_patch_sha256": sha((out / "rework-final.patch").read_bytes()),
    "prior_evidence": "unchanged; original test/implementation commits retained in ancestry",
    "public_interface": "unchanged from 033b9edad",
}
print(json.dumps(summary, indent=2))
