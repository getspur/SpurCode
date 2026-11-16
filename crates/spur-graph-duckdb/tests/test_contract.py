"""Native metadata and source provenance contract (DuckDB exactly 2.0.0).

Set SPUR_GRAPH_EXTENSION to this build's packaged artifact and
SPUR_GRAPH_BUILD_SCRIPT to its Cargo build-script executable. Provenance probes
run that same executable on isolated source copies, never the user's checkout.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
import unittest

import duckdb


CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
INFO_TYPE = ("STRUCT(extension_version VARCHAR, facts_schema_version UINTEGER, "
             "duckdb_api_version VARCHAR, extractor_id VARCHAR, "
             "supported_languages VARCHAR[], resolution_scope VARCHAR)")


def connect():
    if duckdb.__version__ != "2.0.0":
        raise AssertionError("metadata acceptance requires DuckDB exactly 2.0.0")
    extension = Path(os.environ["SPUR_GRAPH_EXTENSION"]).resolve(strict=True)
    con = duckdb.connect(config={"allow_unsigned_extensions": True})
    con.execute("LOAD '" + str(extension).replace("'", "''") + "'")
    return con


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.con = connect()
        self.addCleanup(self.con.close)
        found = self.con.execute(
            "SELECT count(*) FROM duckdb_functions() WHERE "
            "function_name='spur_graph_info' AND function_type='scalar'"
        ).fetchone()[0]
        self.assertEqual(found, 1, "spur_graph_info must be registered by this artifact")

    def info(self):
        return self.con.execute("SELECT spur_graph_info()").fetchone()[0]

    def test_typed_versioned_single_file_contract(self):
        actual = self.con.execute("DESCRIBE SELECT spur_graph_info()").fetchone()[1]
        self.assertEqual(actual, INFO_TYPE)
        info = self.info()
        package = tomllib.loads((CRATE / "Cargo.toml").read_text())["package"]
        self.assertEqual(info["extension_version"], package["version"])
        self.assertEqual(info["facts_schema_version"], 1)
        self.assertEqual(info["duckdb_api_version"], "2.0.0")
        self.assertEqual(info["resolution_scope"], "single_file")
        self.assertRegex(info["extractor_id"], r"^source-sha256-v1:[0-9a-f]{64}$")
        self.assertNotEqual(info["extractor_id"].split(":")[1], hashlib.sha256(
            Path(os.environ["SPUR_GRAPH_EXTENSION"]).read_bytes()).hexdigest())

    def test_repeated_fresh_connections_and_vector_rows_agree(self):
        expected = self.info()
        self.assertEqual(self.info(), expected)
        with connect() as fresh:
            self.assertEqual(fresh.execute("SELECT spur_graph_info()").fetchone()[0], expected)
        count = 2 * duckdb.__standard_vector_size__ + 1
        rows = self.con.execute(
            "SELECT spur_graph_info() FROM range(?) WHERE range % 2 = 0", [count]
        ).fetchall()
        self.assertEqual(len(rows), len(range(0, count, 2)))
        self.assertTrue(all(row[0] == expected for row in rows))

    def test_languages_match_native_registry(self):
        # The native registry is private; read its declarations as an independent
        # oracle. Production uses public all_supported_extensions + from_path.
        registry = (ROOT / "crates/spur-graph/src/extract/languages.rs").read_text()
        registry = registry.split("fn language_registry()", 1)[1]
        expected = sorted(set(re.findall(r"language: Language::(\w+)", registry)))
        self.assertTrue(expected)
        self.assertEqual(self.info()["supported_languages"], expected)

    def test_existing_signatures_and_types_remain_usable(self):
        rows = self.con.execute(
            "SELECT function_name, parameter_types FROM duckdb_functions() WHERE "
            "function_name IN ('spur_graph_symbols','spur_graph_file_facts',"
            "'spur_graph_symbol_changes') ORDER BY function_name"
        ).fetchall()
        self.assertEqual([r[0] for r in rows], ["spur_graph_file_facts",
                         "spur_graph_symbol_changes", "spur_graph_symbols"])
        self.assertEqual(rows[0][1], ["VARCHAR", "VARCHAR"])
        self.assertEqual(rows[2][1], ["VARCHAR", "VARCHAR"])
        self.assertEqual(rows[1][1][:3], ["VARCHAR"] * 3)
        self.assertEqual(len(rows[1][1]), 5)
        self.assertEqual(self.con.execute("SELECT spur_graph_symbols('x.rs','')").fetchone()[0], [])
        facts = self.con.execute("SELECT spur_graph_file_facts('x.rs','')").fetchone()[0]
        self.assertEqual(list(facts), ["nodes", "edges", "spans", "macro_invocations", "macro_scopes"])


class SourceIdentityTests(unittest.TestCase):
    def setUp(self):
        self.script = Path(os.environ["SPUR_GRAPH_BUILD_SCRIPT"]).resolve(strict=True)
        self.temp = tempfile.TemporaryDirectory(prefix="spur-metadata-contract-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def probe(self, root):
        out = self.root / "probe-output"
        out.mkdir(exist_ok=True)
        env = dict(os.environ, CARGO_MANIFEST_DIR=str(root / "crates/spur-graph-duckdb"),
                   OUT_DIR=str(out))
        result = subprocess.run([str(self.script)], cwd=root, env=env,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        receipt = json.loads((out / "spur_graph_source_identity.json").read_text())
        self.assertIn("cargo:rustc-env=SPUR_GRAPH_EXTRACTOR_ID=" + receipt["extractor_id"], result.stdout)
        return receipt

    def source_copy(self, name, receipt):
        root = self.root / name
        for entry in receipt["inputs"]:
            path = entry["path"].split("#", 1)[0]
            source = ROOT / path
            destination = root / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
        return root

    def test_receipt_matches_loaded_source_identity_and_independent_hash(self):
        receipt = self.probe(ROOT)
        with connect() as con:
            self.assertEqual(receipt["extractor_id"], con.execute(
                "SELECT spur_graph_info().extractor_id").fetchone()[0])
        digest = hashlib.sha256(b"spur-graph-extractor-source-v1\0")
        paths = [entry["path"] for entry in receipt["inputs"]]
        self.assertEqual(paths, sorted(set(paths)))
        for entry in receipt["inputs"]:
            label = entry["path"].encode()
            data = (entry["content"].encode() if "content" in entry
                    else (ROOT / entry["path"]).read_bytes())
            self.assertEqual(entry["sha256"], hashlib.sha256(data).hexdigest())
            digest.update(len(label).to_bytes(8, "little"))
            digest.update(label)
            digest.update(len(data).to_bytes(8, "little"))
            digest.update(data)
        self.assertEqual(receipt["extractor_id"], "source-sha256-v1:" + digest.hexdigest())

    def test_relocation_no_git_dirty_git_and_unrelated_edits(self):
        receipt = self.probe(ROOT)
        first = self.source_copy("first", receipt)
        second = self.source_copy("different-absolute-path", receipt)
        self.assertEqual(self.probe(first), self.probe(second))
        unrelated = second / "crates/unrelated/src/lib.rs"
        unrelated.parent.mkdir(parents=True)
        unrelated.write_text("unrelated source edit")
        (second / "README.md").write_text("unrelated docs")
        self.assertEqual(self.probe(second), receipt)
        for args in (("init", "-q"), ("add", "."),
                     ("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                      "commit", "-qm", "source fixture")):
            subprocess.run(["git", "-C", str(second), *args], check=True, capture_output=True)
        relevant = second / "crates/spur-graph/src/extract/languages.rs"
        relevant.write_text(relevant.read_text() + "\n// dirty extraction source\n")
        self.assertNotEqual(self.probe(second)["extractor_id"], receipt["extractor_id"])
        self.assertNotIn("HEAD", self.probe(second)["extractor_id"])

    def test_adapter_native_query_grammar_and_locked_inputs_affect_identity(self):
        receipt = self.probe(ROOT)
        fixture = self.source_copy("inputs", receipt)
        paths = ["crates/spur-graph-duckdb/src/lib.rs",
                 "crates/spur-graph-duckdb/src/metadata.rs",
                 "crates/spur-graph-duckdb/build.rs",
                 "crates/spur-graph/src/extract/tree_sitter.rs",
                 "crates/spur-graph/queries/rust/tags.scm",
                 "crates/tree-sitter-mermaid/src/parser.c",
                 "crates/spur-graph-duckdb/Cargo.lock", "rust-toolchain.toml"]
        for path in paths:
            with self.subTest(path=path):
                target = fixture / path
                original = target.read_bytes()
                suffix = b"\n# changed locked input\n" if target.suffix in (".lock", ".toml") else b"\n// changed source\n"
                target.write_bytes(original + suffix)
                self.assertNotEqual(self.probe(fixture)["extractor_id"], receipt["extractor_id"])
                target.write_bytes(original)
        added = fixture / "crates/spur-graph/src/extract/new_source.rs"
        added.write_text("// newly added native source")
        self.assertNotEqual(self.probe(fixture)["extractor_id"], receipt["extractor_id"])


if __name__ == "__main__":
    unittest.main()
