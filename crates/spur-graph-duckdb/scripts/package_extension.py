"""Package a native library for the pinned DuckDB 2.0.0 V2 contract.

The footer layout follows DuckDB extension-ci-tools at
2e5d22fa1b1c42e6fc7aa84a8e66a15e6b492b30, scripts/append_extension_metadata.py.
The 32-byte fields, 256-byte unsigned signature, and custom-section prefix are
wire-format constants from that upstream contract.
"""

import argparse
from pathlib import Path
import shutil
import tomllib

import duckdb


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if duckdb.__version__ != "2.0.0":
        parser.error("this adapter requires the DuckDB 2.0.0 packaging/test runtime")
    if args.library.resolve() == args.out.resolve():
        parser.error("the output must differ from the raw library")
    manifest = Path(__file__).resolve().parents[1] / "Cargo.toml"
    version = tomllib.loads(manifest.read_text())["package"]["version"]
    with duckdb.connect() as connection:
        platform = connection.execute("PRAGMA platform").fetchone()[0]
    fields = ["", "", "", "C_STRUCT_UNSTABLE", version, "v2.0.0", platform, "4"]
    encoded = [field.encode("ascii") for field in fields]
    if any(len(field) > 32 for field in encoded):
        parser.error("an extension metadata field exceeds the upstream 32-byte slot")
    footer = b"\x00\x93\x04\x10duckdb_signature\x80\x04"
    footer += b"".join(field.ljust(32, b"\x00") for field in encoded)
    footer += bytes(256)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.out.with_name(args.out.name + ".tmp")
    shutil.copyfile(args.library, temporary)
    with temporary.open("ab") as output:
        output.write(footer)
    temporary.replace(args.out)
    print(f"Packaged {args.out}: DuckDB v2.0.0, {platform}, C_STRUCT_UNSTABLE")


if __name__ == "__main__":
    main()
