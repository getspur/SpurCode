"""Collapse macOS sample trees and verify conservation of all thread observations."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parent
validation = []
for raw in sorted(root.glob("spur-*.sample.txt")):
    pid = int(raw.name.split("-")[1].split(".")[0])
    collapsed = subprocess.check_output([
        "awk", "-f", str(root / "stackcollapse-sample.awk"),
        "-f", str(root / "include-waits.awk"), str(raw),
    ], text=True)
    rows = [(stack, int(count)) for stack, count in
            (line.rsplit(" ", 1) for line in collapsed.splitlines())]
    assert all(count >= 0 for _, count in rows), "negative exclusive sample count"
    roots = re.findall(r"^    (\d+) Thread_", raw.read_text(), re.M)
    assert sum(map(int, roots)) == sum(n for _, n in rows), "collapse lost observations"
    folded = root / f"spur-{pid}.folded"
    folded.write_text("".join(f"{stack} {count}\n" for stack, count in rows if count))
    validation.append({
        "pid": pid, "thread_roots": len(roots),
        "root_observations": sum(map(int, roots)),
        "folded_observations": sum(n for _, n in rows),
        "nonzero_stacks": sum(n > 0 for _, n in rows),
        "zero_parent_rows_omitted": sum(n == 0 for _, n in rows),
        "raw_sha256": hashlib.sha256(raw.read_bytes()).hexdigest(),
        "folded_sha256": hashlib.sha256(folded.read_bytes()).hexdigest(),
    })
(root / "collapse-validation.json").write_text(json.dumps(validation, indent=2) + "\n")
print(json.dumps(validation, indent=2))
