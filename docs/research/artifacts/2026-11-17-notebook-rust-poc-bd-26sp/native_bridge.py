"""Native-only DuckDB bridge for the Rust notebook PoC; no notebook matching policy."""
import hashlib
import json
import sys
from pathlib import Path
import duckdb

ROOT = Path(__file__).resolve().parents[4]
EXT = ROOT / "docs/research/artifacts/2026-11-17-duckdb-release/native-osx-arm64/spur_graph.duckdb_extension"
EXPECTED = "28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122"

def main():
    request = json.load(sys.stdin)
    assert duckdb.__version__ == "2.0.0", duckdb.__version__
    assert hashlib.sha256(EXT.read_bytes()).hexdigest() == EXPECTED
    with duckdb.connect(config={"allow_unsigned_extensions": True}) as con:
        assert con.execute("SELECT version()").fetchone()[0] == "v2.0.0"
        con.execute("LOAD '" + str(EXT).replace("'", "''") + "'")
        op = request["op"]
        if op == "extract":
            rows = []
            for item in request["sources"]:
                symbols, result = con.execute(
                    "SELECT spur_graph_symbols(?, ?), spur_graph_file_facts_result(?, ?)",
                    [item["path"], item["source"], item["path"], item["source"]]
                ).fetchone()
                facts = result.get("facts")
                rows.append({"symbols": symbols, "graph_result": result,
                             "source_sha256": hashlib.sha256(item["source"].encode()).hexdigest()})
            output = rows
        elif op == "compare":
            output = []
            # JSON preserves the exact v1 nested types at the process boundary.
            symbol_type = "STRUCT(entity_name VARCHAR,symbol_kind VARCHAR,enclosing_scope VARCHAR,byte_start UBIGINT,byte_end UBIGINT,line_start UBIGINT,line_end UBIGINT,anchor_hash VARCHAR,tokens VARCHAR[])[]"
            for item in request["pairs"]:
                value = con.execute(
                    "SELECT spur_graph_symbol_changes(?, ?, ?, CAST(?::JSON AS " + symbol_type + "), CAST(?::JSON AS " + symbol_type + "))",
                    [item["path"], item.get("old_path", item["path"]), item.get("kind", "M"),
                     json.dumps(item["left"]), json.dumps(item["right"])]
                ).fetchone()[0]
                output.append(value)
        else:
            raise ValueError("unsupported bridge operation: " + str(op))
    json.dump({"duckdb_version": duckdb.__version__, "extension_sha256": EXPECTED, "result": output}, sys.stdout)

if __name__ == "__main__":
    main()
