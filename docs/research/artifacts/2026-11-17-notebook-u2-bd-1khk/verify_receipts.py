"""Verify immutable U2 receipts, tested bytes, schema and source patch."""
import hashlib
import json
from pathlib import Path
import subprocess

out = Path(__file__).resolve().parent
root = out.parents[3]
base = "26ce5a162ba844f5b13433c069265fb56f7f55ba"
test_commit = "6eb1c4cf6"
sha = lambda data: hashlib.sha256(data).hexdigest()
run = lambda args: subprocess.check_output(args, cwd=root)
receipts = {}
for name in ["remote-red", "host-red", "host-green-initial", "host-extract-green", "host-matcher-green", "host-real-evidence", "host-canonical-mutation-red", "host-canonical-green"]:
    r = json.loads((out/f"{name}.json").read_text())
    assert r["log_sha256"] == sha((out/f"{name}.log").read_bytes()), name
    receipts[name] = r
assert receipts["remote-red"]["exit_code"] == 200
assert receipts["host-red"]["exit_code"] == 101
assert receipts["host-canonical-mutation-red"]["exit_code"] == 101
for name in ["host-green-initial", "host-extract-green", "host-matcher-green", "host-real-evidence", "host-canonical-green"]:
    r = receipts[name]
    assert r["exit_code"] == 0
    for f, digest in r["source_sha256"].items():
        assert digest == sha((root/f).read_bytes()), (name, f)
assert receipts["host-red"]["command"] == receipts["host-green-initial"]["command"]
assert receipts["host-red"]["environment_overrides"] == receipts["host-green-initial"]["environment_overrides"]
assert receipts["host-canonical-mutation-red"]["command"] == receipts["host-canonical-green"]["command"]
assert receipts["host-canonical-mutation-red"]["environment_overrides"] == receipts["host-canonical-green"]["environment_overrides"]
tests = "crates/spur-graph/src/extract/notebook_symbols_tests.rs"
assert receipts["host-red"]["source_sha256"][tests] == sha((root/tests).read_bytes())
for f, digest in receipts["host-red"]["source_sha256"].items():
    assert sha(run(["git", "show", f"{test_commit}:{f}"])) == digest, f
source = "crates/spur-graph/src/extract/notebook_symbols.rs"
mutation = (root/source).read_text().replace('format!("{}#cell:{}", escape(path), escape(id))', 'format!("{path}#cell:{id}")')
assert sha(mutation.encode()) == receipts["host-canonical-mutation-red"]["source_sha256"][source]
for file in ["notebook.rs", "notebook_model.rs", "tree_sitter.rs", "languages.rs"]:
    f = f"crates/spur-graph/src/extract/{file}"
    assert run(["git", "show", f"{base}:{f}"]) == (root/f).read_bytes(), f
schema = (out/"public-interface.rs.txt").read_text()
s = (root/source).read_text()
assert s[s.index("#[derive"):s.index("/// Extract code-cell")] in schema
assert "50 symbol changes; 10 cell events" in (out/"host-real-evidence.log").read_text()
assert 'left: 0\n right: 46' in (out/"host-red.log").read_text()
assert 'left != right' in (out/"host-canonical-mutation-red.log").read_text()
for phase in ["pre", "post"]:
    for c in json.loads((out/f"solver-{phase}.json").read_text())["checks"]:
        assert c["request"]["persist"] is False
        result = c["result"] if "result" in c else c["response"]
        result = json.loads(result["content"][0]["text"])
        expected = c["key"] in ["cell_helpers", "structured_id_witness", "exact_before_source_ref"]
        assert result["status"] == ("sat" if expected else "unsat")
        assert result["outcome"] == ("pass" if expected else "fail")
offset = json.loads((out/"offset-only-real-changes.json").read_text())
assert len(offset["offset_only"]) == 5
for r in offset["offset_only"]:
    assert sha(r["original_slice"].encode()) == r["identical_original_slice_sha256"]
files = ["crates/spur-graph/src/extract/mod.rs", source, tests, "crates/spur-graph/src/git_walk.rs"]
patch = run(["git", "diff", base, "--", *files])
assert patch == (out/"source.patch").read_bytes()
checks = []
for command in [["git", "apply", "--check", "--reverse", str(out/"source.patch")], ["scripts/spur-cargo", "fmt", "-p", "spur-graph", "--", "--check"], ["git", "diff", "--check", base, "--", *files]]:
    result = subprocess.run(command, cwd=root, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    checks.append(dict(command=command, exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr))
summary = dict(base=base, test_commit=test_commit, source_sha256={f:sha((root/f).read_bytes()) for f in files}, patch_sha256=sha(patch), public_interface_sha256=sha((out/"public-interface.rs.txt").read_bytes()), receipt_exits={k:r["exit_code"] for k,r in receipts.items()}, unchanged_test_predicates=True, unchanged_U1_and_ordinary_extraction=True, checks=checks)
(out/"verification.json").write_text(json.dumps(summary, indent=2)+"\n")
print(json.dumps(summary, indent=2))
