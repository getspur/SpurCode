"""Independent native parity contract for DuckDB 2.0.0 file facts.

Build with scripts/spur-cargo --workdir crates/spur-graph-duckdb build
--release --lib --example reference_file_facts --locked (remote normally;
SPUR_REMOTE=0 separately for host-native macOS packaging). Set
SPUR_GRAPH_EXTENSION to the packaged extension and SPUR_FILE_FACTS_ORACLE to
release/examples/reference_file_facts. Run with the pinned Python runtime:
python -m unittest discover -s crates/spur-graph-duckdb/tests -p test_file_facts.py -v

This intentionally fails before the adapter registers spur_graph_file_facts;
there is no skip/expectedFailure or Python replacement for the SQL function.
NativeOracleTests can run separately to establish the oracle's own health.

Normalization is lossless and only applied to native JSON transport:
* Native snake_case unit enum labels remain VARCHAR labels, including nullable
  edge_kind. Macro resolution is STRUCT(kind, definition); inactive definition
  is NULL. ChangeKind becomes STRUCT(kind, renamed_from); RenamePrev becomes
  STRUCT(kind, file, symbol). GitPath becomes STRUCT(path, encoding, bytes),
  preserving either its UTF-8 string or native base64 encoding/bytes payload.
  Inactive enum payloads are NULL, never empty placeholder strings/structs.
* Serde's skipped None edge fields are restored as explicit NULL fields.
* Native confidence_score is f32. Serde emits a shortest f32 decimal; Python's
  JSON reader parses it as f64. Quantize that decimal once to IEEE binary32
  before comparing to DuckDB FLOAT, with no tolerance or rounding of SQL data.
No fields, IDs, ranges, list elements or list order are dropped/sorted. JSON is
only the out-of-engine oracle transport; every SQL fact field is typed below.
"""

import copy
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

import duckdb


def sql_type(shape):
    if isinstance(shape, dict):
        return "STRUCT(" + ", ".join(f"{k} {sql_type(v)}" for k, v in shape.items()) + ")"
    if isinstance(shape, list):
        return sql_type(shape[0]) + "[]"
    return shape


SCOPE = dict(stable_symbol_id="VARCHAR", kind="VARCHAR")
LOCATION = dict(file_path="VARCHAR", source_oid="VARCHAR",
                byte_range=["UBIGINT"], line_range=["UBIGINT"])
ATTRIBUTE = dict(raw_text="VARCHAR", owner=SCOPE,
                 byte_range=["UBIGINT"], line_range=["UBIGINT"])
GIT_PATH = dict(path="VARCHAR", encoding="VARCHAR", bytes="VARCHAR")
SNAPSHOT_KEY = dict(stable_symbol_id="VARCHAR", commit="VARCHAR")
RENAME_PREV = dict(kind="VARCHAR", file=GIT_PATH, symbol=SNAPSHOT_KEY)
CHANGE_KIND = dict(kind="VARCHAR", renamed_from=RENAME_PREV)
NODE = dict(node_id="UBIGINT", stable_key="VARCHAR", label="VARCHAR", kind="VARCHAR",
            file_id="UBIGINT", source_span_id="UBIGINT", first_seen_run_id="UBIGINT")
EDGE = dict(edge_id="UBIGINT", source_node_id="UBIGINT", target_node_id="UBIGINT",
            relation="VARCHAR", target_label="VARCHAR", import_path="VARCHAR",
            receiver_text="VARCHAR", scope_text="VARCHAR", confidence="VARCHAR",
            confidence_score="FLOAT", edge_kind="VARCHAR", bind_method="VARCHAR",
            evidence_id="UBIGINT", directed="BOOLEAN", change_kind=CHANGE_KIND)
SPAN = dict(span_id="UBIGINT", file_id="UBIGINT", start_byte="UINTEGER",
            end_byte="UINTEGER", start_line="UINTEGER", end_line="UINTEGER")
INVOCATION = dict(site_id="VARCHAR", file_path="VARCHAR", source_oid="VARCHAR",
                  caller_stable_symbol_id="VARCHAR", enclosing=SCOPE,
                  raw_macro_path="VARCHAR", normalized_macro_path="VARCHAR",
                  byte_range=["UBIGINT"], line_range=["UBIGINT"], attributes=[ATTRIBUTE],
                  test_context="VARCHAR", debug_assertions_candidate="BOOLEAN",
                  execution_phase="VARCHAR", resolution=dict(kind="VARCHAR", definition=LOCATION),
                  sink_family_candidate="VARCHAR")
MACRO_SCOPE = dict(scope=SCOPE, **LOCATION)
FACTS = dict(nodes=[NODE], edges=[EDGE], spans=[SPAN],
             macro_invocations=[INVOCATION], macro_scopes=[MACRO_SCOPE])
FACTS_TYPE = sql_type(FACTS)

RUST = '''#![allow(dead_code)]
// 雪 🦀: exercise UTF-8 byte coordinates, not character offsets.
use crate::elsewhere::Remote;
macro_rules! local { () => { helper() }; }
fn helper() {}
trait Worker { fn run(&self); }
fn by_ref(value: &dyn Worker) { value.run(); }
#[cfg(test)]
mod tests {
    #[cfg_attr(feature = "extra", test)]
    fn outer() {
        local!();
        wrapper!(panic!("opaque"));
        core::panic!("candidate");
        debug_assert!(true);
        fn inner() { assert!(true); }
        inner(); helper(); crate::helper(); remote_call();
    }
}
const CHECK: () = { assert!(true); };
'''
CPP = '''#include "other.hpp"
namespace demo {
struct Worker { int run() { return 1; } };
int helper() { return 2; }
int entry(Worker &worker) { return worker.run() + helper() + external(); }
}
'''
PYTHON = '''from remote import remote_call
def helper():
    return 1
class Worker:
    def run(self):
        return helper() + remote_call()
'''
NOTEBOOK = json.dumps(dict(nbformat=4, nbformat_minor=5,
    metadata=dict(kernelspec=dict(name="python3")), cells=[
        dict(id="intro", cell_type="markdown", metadata={},
             source=["# Notebook\n", "[link](missing.py)\n"]),
        dict(id="python", cell_type="code", metadata={},
             source=["def helper():\n", "    return 1\n", "helper()\n"]),
        dict(id="rust", cell_type="code", metadata=dict(spur=dict(code_type="rust")),
             source=['fn rust_cell() { panic!("cell"); }\n']),
    ]))
FIXTURES = [
    ("virtual/雪/lib.rs", RUST),
    ("virtual/source.cpp", CPP),
    ("virtual/source.py", PYTHON),
    ("virtual/readme.md", "# Overview\n\n[helper](missing.rs)\n\n## Details\n"),
    ("virtual/settings.json", '{"service":{"enabled":true}}'),
    ("virtual/settings.yaml", "service:\n  enabled: true\n"),
    ("virtual/settings.toml", "[service]\nenabled = true\n"),
    ("virtual/openapi.json", '{"openapi":"3.0.0","info":{"title":"Demo","version":"1"},'
     '"paths":{"/pets":{"get":{"operationId":"listPets","responses":{"200":{"description":"ok"}}}}}}'),
    ("virtual/openapi.yaml", "openapi: 3.0.0\ninfo:\n  title: Demo\n  version: '1'\npaths:\n"
     "  /pets:\n    get:\n      operationId: listPets\n      responses:\n        '200':\n          description: ok\n"),
    ("virtual/analysis.ipynb", NOTEBOOK),
    ("virtual/empty.ipynb", '{"cells":[],"metadata":{}}'),
    ("virtual/remote.py", "def remote_call():\n    return 1\n"),
    ("virtual/caller.py", "def caller():\n    return remote_call()\n"),
    ("virtual/icu.cpp", '// This file is generated by scripts/inline-data.py, do not edit it manually //\n'
     '#include "unicode/utypes.h"\nextern "C" const unsigned char data[] = {0, 1, 255};\n'),
] + [(f"virtual/empty.{ext}", "") for ext in ("rs", "cpp", "py", "md", "json", "yaml", "toml")]


def normalize_change(value):
    if value is None:
        return None
    if isinstance(value, str):
        return dict(kind=value, renamed_from=None)
    # Externally tagged native enums: {renamed_from: {file: GitPath}} or
    # {renamed_from: {symbol: SnapshotKey}}. Preserve every payload member.
    if set(value) != {"renamed_from"}:
        raise AssertionError(f"unknown native ChangeKind: {value!r}")
    previous = value["renamed_from"]
    if set(previous) == {"file"}:
        path = previous["file"]
        if isinstance(path, str):
            path = dict(path=path, encoding=None, bytes=None)
        elif set(path) == {"encoding", "bytes"}:
            path = dict(path=None, **path)
        else:
            raise AssertionError(f"unknown native GitPath: {path!r}")
        payload = dict(kind="file", file=path, symbol=None)
    elif set(previous) == {"symbol"}:
        payload = dict(kind="symbol", file=None, symbol=previous["symbol"])
    else:
        raise AssertionError(f"unknown native RenamePrev: {previous!r}")
    return dict(kind="renamed_from", renamed_from=payload)


def normalize_native(facts):
    result = copy.deepcopy(facts)
    for edge in result["edges"]:
        for field in ("import_path", "receiver_text", "scope_text", "edge_kind", "bind_method", "change_kind"):
            edge.setdefault(field, None)
        edge["confidence_score"] = struct.unpack("!f", struct.pack("!f", edge["confidence_score"]))[0]
        edge["change_kind"] = normalize_change(edge["change_kind"])
    for invocation in result["macro_invocations"]:
        invocation["resolution"].setdefault("definition", None)
    return result


def native_facts(fixtures):
    executable = os.environ.get("SPUR_FILE_FACTS_ORACLE")
    if not executable:
        raise AssertionError("set SPUR_FILE_FACTS_ORACLE to the built reference_file_facts example")
    with tempfile.TemporaryDirectory(prefix="spur-file-facts-input-") as directory:
        inputs = Path(directory) / "input.jsonl"
        outputs = Path(directory) / "output.jsonl"
        inputs.write_text("".join(json.dumps(dict(path=p, source=s), ensure_ascii=False) + "\n"
                                  for p, s in fixtures), encoding="utf-8")
        process = subprocess.run([executable, str(inputs), str(outputs)], capture_output=True, text=True)
        if process.returncode:
            raise AssertionError(f"native oracle failed ({process.returncode}): {process.stderr}")
        result = [normalize_native(json.loads(line)) for line in outputs.read_text(encoding="utf-8").splitlines()]
    if len(result) != len(fixtures):
        raise AssertionError("oracle must return exactly one result per input, including empty files")
    return result


def assert_shape(test, value, shape):
    if value is None:
        return
    if isinstance(shape, dict):
        test.assertIsInstance(value, dict)
        test.assertEqual(set(value), set(shape))
        for key, child in shape.items():
            assert_shape(test, value[key], child)
    elif isinstance(shape, list):
        test.assertIsInstance(value, list)
        for child in value:
            assert_shape(test, child, shape[0])
    elif shape in ("UBIGINT", "UINTEGER"):
        test.assertIs(type(value), int)
        test.assertGreaterEqual(value, 0)
    elif shape == "VARCHAR":
        test.assertIsInstance(value, str)
    elif shape == "FLOAT":
        test.assertIs(type(value), float)
    elif shape == "BOOLEAN":
        test.assertIs(type(value), bool)


class NativeOracleTests(unittest.TestCase):
    """These must pass even while SQL is RED, without loading any extension."""

    def test_native_collections_nested_evidence_and_one_file_isolation(self):
        expected = native_facts(FIXTURES)
        for (path, _), facts in zip(FIXTURES, expected):
            with self.subTest(path=path):
                assert_shape(self, facts, FACTS)
                self.assertTrue(facts["nodes"])
                self.assertTrue(facts["spans"])
        rust = expected[0]
        local = next(m for m in rust["macro_invocations"] if m["normalized_macro_path"] == "local")
        self.assertEqual(local["resolution"]["kind"], "local_definition_candidate")
        self.assertIsNotNone(local["resolution"]["definition"])
        for raw in ('#![allow(dead_code)]', '#[cfg(test)]', '#[cfg_attr(feature = "extra", test)]'):
            self.assertIn(raw, [a["raw_text"] for a in local["attributes"]])
        self.assertTrue(any(m["resolution"]["kind"] == "explicit_path_candidate" for m in rust["macro_invocations"]))
        self.assertTrue(any(m["resolution"]["kind"] == "unresolved" for m in rust["macro_invocations"]))
        self.assertTrue(any(m["sink_family_candidate"] is not None for m in rust["macro_invocations"]))
        edges = [e for facts in expected for e in facts["edges"]]
        for field in ("target_node_id", "target_label", "import_path", "receiver_text", "scope_text", "edge_kind", "bind_method"):
            self.assertTrue(any(e[field] is not None for e in edges), field)
            self.assertTrue(any(e[field] is None for e in edges), field)
        caller = expected[12]
        self.assertTrue(any(e["relation"] == "calls" and e["target_label"] == "remote_call"
                            and e["target_node_id"] is None for e in caller["edges"]))
        # Preceding definitions must not leak across isolated roots/records.
        self.assertEqual(native_facts([FIXTURES[12]])[0], caller)
        self.assertEqual(native_facts([FIXTURES[0], FIXTURES[0]]), [rust, rust])
        for invocation in rust["macro_invocations"]:
            start, end = invocation["byte_range"]
            self.assertIn(invocation["raw_macro_path"], RUST.encode()[start:end].decode())

    def test_enum_transport_normalization_preserves_all_payloads(self):
        self.assertIsNone(normalize_change(None))
        for kind in ("added", "modified", "deleted"):
            self.assertEqual(normalize_change(kind), dict(kind=kind, renamed_from=None))
        for path in ("old/雪.rs", dict(encoding="base64", bytes="/w==")):
            result = normalize_change(dict(renamed_from=dict(file=path)))
            self.assertEqual(result["renamed_from"]["kind"], "file")
            self.assertEqual(result["renamed_from"]["file"],
                             dict(path=path, encoding=None, bytes=None) if isinstance(path, str)
                             else dict(path=None, **path))
        key = dict(stable_symbol_id="fedcba9876543210", commit="0123456789abcdef")
        self.assertEqual(normalize_change(dict(renamed_from=dict(symbol=key))),
                         dict(kind="renamed_from", renamed_from=dict(kind="symbol", file=None, symbol=key)))


class FileFactsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.expected = native_facts(FIXTURES)

    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        self.connection = duckdb.connect(config={"allow_unsigned_extensions": True})
        self.addCleanup(self.connection.close)
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        self.assertTrue(extension, "set SPUR_GRAPH_EXTENSION to the packaged native extension")
        quoted = str(Path(extension).resolve()).replace("'", "''")
        self.connection.execute(f"LOAD '{quoted}'")

    def facts(self, path, source):
        return self.connection.execute(
            "SELECT spur_graph_file_facts(?::VARCHAR, ?::VARCHAR)", [path, source]).fetchone()[0]

    def test_registration_and_complete_typed_schema(self):
        functions = self.connection.execute(
            "SELECT parameter_types, return_type FROM duckdb_functions() "
            "WHERE function_name = 'spur_graph_file_facts' AND function_type = 'scalar'").fetchall()
        self.assertEqual(len(functions), 1, "spur_graph_file_facts must be registered")
        parameters, result_type = functions[0]
        self.assertEqual(parameters, ["VARCHAR", "VARCHAR"])
        canonical = self.connection.execute(f"SELECT typeof(NULL::{FACTS_TYPE})").fetchone()[0]
        self.assertEqual(result_type, canonical)

    def test_every_native_field_and_native_list_order(self):
        for (path, source), expected in zip(FIXTURES, self.expected):
            with self.subTest(path=path):
                actual = self.facts(path, source)
                assert_shape(self, actual, FACTS)
                self.assertEqual(actual, expected)
                self.assertIsNotNone(actual)
                self.assertTrue(all(isinstance(actual[k], list) for k in FACTS))

    def test_null_short_circuits_and_empty_source_has_native_facts(self):
        for path, source in ((None, RUST), (FIXTURES[0][0], None), (None, None),
                             ("../bad.rs", None), (None, "not a notebook")):
            self.assertIsNone(self.facts(path, source))
        expected = self.expected[FIXTURES.index(("virtual/empty.rs", ""))]
        self.assertEqual(self.facts("virtual/empty.rs", ""), expected)
        self.assertTrue(expected["nodes"])

    def test_invalid_paths_unsupported_language_and_malformed_notebook_are_sql_errors(self):
        invalid = ["", ".", "./", "/absolute.rs", "../escape.rs", "src/../../escape.rs",
                   "..\\escape.rs", "C:\\virtual\\file.rs", "C:relative.rs",
                   "./C:\\virtual\\file.rs", "\\\\server\\share\\file.rs",
                   "input.unknown"]
        for path, source in [(p, RUST) for p in invalid] + [("bad.ipynb", ""), ("bad.ipynb", "not JSON")]:
            with self.subTest(path=path, source=source):
                with self.assertRaises(duckdb.InvalidInputException):
                    self.facts(path, source)
        self.assertEqual(self.facts(*FIXTURES[0]), self.expected[0])

    def test_current_directory_components_normalize_without_changing_identity(self):
        self.assertEqual(self.facts("./virtual/雪/./lib.rs", RUST), self.expected[0])

    def test_try_mixed_rows_preserves_good_empty_null_and_error_rows(self):
        self.connection.execute("CREATE TEMP TABLE mixed(i INTEGER, path VARCHAR, source VARCHAR)")
        rows = [(0, *FIXTURES[0]), (1, "../bad.rs", RUST), (2, "virtual/empty.rs", ""),
                (3, None, RUST), (4, "bad.unknown", ""), (5, *FIXTURES[1]),
                (6, "bad.ipynb", "invalid"), (7, "input.py", None), (8, *FIXTURES[0])]
        self.connection.executemany("INSERT INTO mixed VALUES (?, ?, ?)", rows)
        actual = self.connection.execute(
            "SELECT i, TRY(spur_graph_file_facts(path, source)) FROM mixed ORDER BY i").fetchall()
        empty = self.expected[FIXTURES.index(("virtual/empty.rs", ""))]
        self.assertEqual(actual, list(enumerate([
            self.expected[0], None, empty, None, None, self.expected[1], None, None, self.expected[0]])))

    def test_constants_selected_vectors_multiple_chunks_and_repeated_local_ids(self):
        # Runtime size, not a hardcoded DuckDB chunk-size assumption. The selected
        # output itself exceeds two standard vectors, including interleaved NULLs.
        vector_size = duckdb.__standard_vector_size__
        count = 4 * vector_size + 3
        self.connection.execute("CREATE TEMP TABLE sources(i BIGINT, path VARCHAR, source VARCHAR)")
        rows = []
        for i in range(count):
            path, source = FIXTURES[i % len(FIXTURES)]
            rows.append((i, path, None if i % 5 == 0 else source))
        self.connection.executemany("INSERT INTO sources VALUES (?, ?, ?)", rows)
        actual = self.connection.execute(
            "SELECT i, spur_graph_file_facts(path, source) FROM sources WHERE i % 2 = 0 ORDER BY i").fetchall()
        self.assertGreater(len(actual), 2 * vector_size)
        self.assertEqual([i for i, _ in actual], list(range(0, count, 2)))
        for i, facts in actual:
            self.assertEqual(facts, None if i % 5 == 0 else self.expected[i % len(FIXTURES)], i)
        constant = self.connection.execute(
            "SELECT spur_graph_file_facts(?::VARCHAR, ?::VARCHAR) FROM range(?)",
            [*FIXTURES[0], vector_size + 1]).fetchall()
        self.assertEqual(constant, [(self.expected[0],)] * (vector_size + 1))
        # Constant path / selected source, and selected path / constant source.
        for expression, values in (("'virtual/雪/lib.rs', source", []),
                                   ("path, ?::VARCHAR", [RUST])):
            query = ("SELECT spur_graph_file_facts(" + expression + ") FROM sources "
                     "WHERE path = 'virtual/雪/lib.rs' AND source IS NOT NULL ORDER BY i")
            actual = self.connection.execute(query, values).fetchall()
            self.assertTrue(actual)
            self.assertEqual(actual, [(self.expected[0],)] * len(actual))

    def test_one_file_resolution_does_not_leak_between_calls_or_rows(self):
        for index in (11, 12, 0, 12, 11, 0):
            self.assertEqual(self.facts(*FIXTURES[index]), self.expected[index])
        self.connection.execute("CREATE TEMP TABLE isolated(i INTEGER, path VARCHAR, source VARCHAR)")
        self.connection.executemany("INSERT INTO isolated VALUES (?, ?, ?)",
                                    [(i, *FIXTURES[index]) for i, index in enumerate((11, 12, 11, 12))])
        rows = self.connection.execute(
            "SELECT spur_graph_file_facts(path, source) FROM isolated ORDER BY i").fetchall()
        self.assertEqual(rows, [(self.expected[index],) for index in (11, 12, 11, 12)])

    def test_single_row_child_lists_exceed_runtime_vector_capacity(self):
        count = duckdb.__standard_vector_size__ + 1
        fixture = ("virtual/many.rs", "#![allow(dead_code)]\nfn many() {\n" +
                   'assert!(true);\n' * count + "}\n")
        expected = native_facts([fixture])[0]
        self.assertEqual(len(expected["macro_invocations"]), count)
        self.assertEqual(self.facts(*fixture), expected)

    def test_typed_parquet_input_and_output_round_trip_without_extension_on_reader(self):
        self.connection.execute("CREATE TEMP TABLE staged(i INTEGER, path VARCHAR, source VARCHAR)")
        self.connection.executemany("INSERT INTO staged VALUES (?, ?, ?)",
                                    [(i, *fixture) for i, fixture in enumerate(FIXTURES)] +
                                    [(len(FIXTURES), "null.rs", None)])
        with tempfile.TemporaryDirectory(prefix="spur-file-facts-parquet-") as directory:
            inputs = str(Path(directory) / "sources.parquet").replace("'", "''")
            output = str(Path(directory) / "facts.parquet").replace("'", "''")
            self.connection.execute(f"COPY staged TO '{inputs}' (FORMAT PARQUET)")
            self.connection.execute(
                "CREATE TEMP TABLE parsed AS SELECT i, path, spur_graph_file_facts(path, source) facts "
                f"FROM read_parquet('{inputs}')")
            expected = [(i, path, facts) for i, ((path, _), facts) in
                        enumerate(zip(FIXTURES, self.expected))] + [(len(FIXTURES), "null.rs", None)]
            self.assertEqual(self.connection.execute("SELECT * FROM parsed ORDER BY i").fetchall(), expected)
            self.connection.execute(f"COPY parsed TO '{output}' (FORMAT PARQUET)")
            with duckdb.connect() as reader:
                self.assertEqual(reader.execute(
                    f"SELECT * FROM read_parquet('{output}') ORDER BY i").fetchall(), expected)
                actual_type = reader.execute(
                    f"SELECT typeof(facts) FROM read_parquet('{output}') LIMIT 1").fetchone()[0]
                canonical = reader.execute(f"SELECT typeof(NULL::{FACTS_TYPE})").fetchone()[0]
                self.assertEqual(actual_type, canonical)


if __name__ == "__main__":
    unittest.main()
