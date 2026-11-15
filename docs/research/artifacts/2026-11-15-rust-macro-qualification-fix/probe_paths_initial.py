"""Validate macro path normalization in a fresh native DuckDB 2 process."""
import argparse
import hashlib
import json
from pathlib import Path
import duckdb

ART = Path(__file__).resolve().parent
PREFIX = """#![allow(dead_code)]
mod outer { pub mod local { pub fn deep() -> u8 { 7 } } }
fn max() -> u8 { 42 }
"""
CASES = {
    "plain": "outer::local::deep()",
    "spaces": "outer :: local :: deep()",
    "internal_block": "outer /* qualifier */ :: local::deep()",
    "internal_line": "outer // qualifier\n :: local::deep()",
    "trailing_block": "outer::local /* qualifier */ ::deep()",
    "trailing_line": "outer::local // qualifier\n ::deep()",
    "all_comments": "outer /* a */ :: /* b */ local /* c */ :: /* d */ deep /* e */ ()",
    "foreign_standard": "std /* foreign */ :: cmp :: max(1u8,2u8)",
}
parser = argparse.ArgumentParser()
parser.add_argument("--extension", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--expect-fixed", action="store_true")
args = parser.parse_args()
assert duckdb.__version__ == "2.0.0"
c = duckdb.connect(config={"allow_unsigned_extensions": "true", "threads": "2"})
c.execute("LOAD '" + str(args.extension.resolve()).replace("'", "''") + "'")
receipts = []
passed = {}
for name, expression in CASES.items():
    source = PREFIX + "fn caller() { let _ = vec![" + expression + "]; }\n"
    facts = c.execute("SELECT spur_graph_file_facts(?,?)",
                      ["src/lib.rs", source]).fetchone()[0]
    nodes = {n["node_id"]: n for n in facts["nodes"]}
    calls = [e for e in facts["edges"] if e["relation"] == "calls"
             and nodes[e["source_node_id"]]["label"] == "caller"]
    selected = [e for e in calls if e["target_label"] == ("max" if name == "foreign_standard" else "deep")]
    parents = {e["target_node_id"]: e["source_node_id"] for e in facts["edges"] if e["relation"] == "contains"}
    if name == "foreign_standard":
        ok = len(selected) == 1 and selected[0]["target_node_id"] is None and selected[0]["scope_text"] == "std::cmp"
    else:
        ok = len(selected) == 1 and selected[0]["target_node_id"] in nodes
        if ok:
            target = nodes[selected[0]["target_node_id"]]
            owner = nodes.get(parents.get(target["node_id"]), {})
            ok = target["label"] == "deep" and owner.get("label") == "local"
            if args.expect_fixed:
                ok = ok and selected[0]["scope_text"] == "outer::local"
    passed[name] = ok
    receipts.append(dict(case=name, sql="SELECT spur_graph_file_facts(?,?)",
                         params=["src/lib.rs", source], calls=calls, facts=facts,
                         expected_binding_passed=ok))
result = dict(duckdb_version=duckdb.__version__,
              extension=str(args.extension.resolve()),
              extension_sha256=hashlib.sha256(args.extension.read_bytes()).hexdigest(),
              validator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              cases=passed, all_checks_passed=all(passed.values()), receipts=receipts)
args.output.write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({k:v for k,v in result.items() if k != "receipts"}))
c.close()
if args.expect_fixed:
    assert result["all_checks_passed"], passed

