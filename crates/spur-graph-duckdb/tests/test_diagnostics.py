"""Real DuckDB 2.0 diagnostic contract; requires the packaged native extension.

Use the same SPUR_GRAPH_EXTENSION and SPUR_FILE_FACTS_ORACLE as test_file_facts.
No skips or Python extraction substitute: missing registration must fail RED.
"""

import os
from pathlib import Path
import unittest

import duckdb

from test_file_facts import FACTS, FACTS_TYPE, FIXTURES, RUST, native_facts


class DiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        self.connection = self.connect()

    def connect(self):
        connection = duckdb.connect(config={"allow_unsigned_extensions": True})
        self.addCleanup(connection.close)
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        self.assertTrue(extension, "set SPUR_GRAPH_EXTENSION to this task's package")
        quoted = str(Path(extension).resolve()).replace("'", "''")
        connection.execute(f"LOAD '{quoted}'")
        return connection

    def result(self, path, source):
        return self.connection.execute(
            "SELECT spur_graph_file_facts_result(?::VARCHAR, ?::VARCHAR)",
            [path, source]).fetchone()[0]

    def assert_result(self, result, status, facts=None, code=None):
        self.assertIsNotNone(result, "even a NULL input needs a diagnostic parent")
        self.assertEqual(list(result), ["status", "error_code", "error_message", "facts"])
        self.assertEqual(result["status"], status)
        self.assertEqual(result["facts"], facts)
        self.assertEqual(result["error_code"], code)
        if code:
            self.assertTrue(result["error_message"])
        else:
            self.assertIsNone(result["error_message"])

    def test_unsupported_language_has_row_diagnostic(self):
        registered = self.connection.execute(
            "SELECT count(*) FROM duckdb_functions() "
            "WHERE function_name = 'spur_graph_file_facts_result'").fetchone()[0]
        self.assertEqual(registered, 1, "diagnostic scalar function must be registered")
        r = self.connection.execute(
            "SELECT spur_graph_file_facts_result('bad.unknown','text')").fetchone()[0]
        self.assert_result(r, "extraction_error", code="unsupported_language")
        self.assertIn("bad.unknown", r["error_message"])

    def test_exact_typed_schema_and_runnable_documented_example(self):
        parameters, result_type, examples = self.connection.execute(
            "SELECT parameter_types, return_type, examples FROM duckdb_functions() "
            "WHERE function_name = 'spur_graph_file_facts_result'").fetchone()
        self.assertEqual(parameters, ["VARCHAR", "VARCHAR"])
        expected = self.connection.execute(
            "SELECT typeof(NULL::STRUCT(status VARCHAR, error_code VARCHAR, "
            f"error_message VARCHAR, facts {FACTS_TYPE}))").fetchone()[0]
        self.assertEqual(result_type, expected)
        self.assertTrue(examples)
        for example in examples:
            self.assertIsNotNone(self.connection.execute(example).fetchone()[0])

    def test_all_native_fields_and_list_order_including_empty_source(self):
        expected = native_facts(FIXTURES)
        for (path, source), facts in zip(FIXTURES, expected):
            with self.subTest(path=path):
                status = "empty_source" if source == "" else (
                    "empty_facts" if all(not facts[key] for key in FACTS) else "ok")
                self.assert_result(self.result(path, source), status, facts)
        # Partial syntax trees are native successes, not invented syntax errors.
        fixture = ("partial.rs", "fn broken( {")
        self.assert_result(self.result(*fixture), "ok", native_facts([fixture])[0])

    def test_null_inputs_precede_path_language_and_notebook_validation(self):
        for path, source in ((None, RUST), ("input.rs", None), (None, None),
                             ("../bad.rs", None), ("bad.unknown", None),
                             (None, "not JSON"), ("bad.ipynb", None)):
            with self.subTest(path=path, source=source):
                self.assert_result(self.result(path, source), "null_input")

    def test_invalid_paths_and_malformed_notebooks_preserve_native_diagnostics(self):
        invalid = ["", ".", "./", "/absolute.rs", "../escape.rs", "src/../../escape.rs",
                   "..\\escape.rs", "C:\\virtual\\file.rs", "C:relative.rs",
                   "./C:\\virtual\\file.rs", "\\\\server\\share\\file.rs"]
        for path in invalid:
            with self.subTest(path=path):
                self.assert_result(self.result(path, RUST), "extraction_error", code="invalid_path")
        for source in ("", "not JSON", '{"cells":['):
            with self.subTest(source=source):
                r = self.result("bad.ipynb", source)
                self.assert_result(r, "extraction_error", code="malformed_input")
                self.assertIn("parse .ipynb JSON", r["error_message"])
                with self.assertRaises(duckdb.InvalidInputException) as old:
                    self.connection.execute("SELECT spur_graph_file_facts('bad.ipynb', ?)", [source])
                self.assertIn(r["error_message"], str(old.exception))
        expected = native_facts([FIXTURES[0]])[0]
        self.assert_result(self.result("./virtual/雪/./lib.rs", RUST), "ok", expected)

    def test_mixed_selected_chunks_and_constant_arguments(self):
        good = FIXTURES[0]
        empty = ("virtual/empty.rs", "")
        expected_good, expected_empty = native_facts([good, empty])
        cases = [(good, "ok", expected_good, None),
                 (("../bad.rs", RUST), "extraction_error", None, "invalid_path"),
                 (empty, "empty_source", expected_empty, None),
                 ((None, RUST), "null_input", None, None),
                 (("bad.unknown", "text"), "extraction_error", None, "unsupported_language"),
                 (("bad.ipynb", "not JSON"), "extraction_error", None, "malformed_input"),
                 (("input.py", None), "null_input", None, None)]
        vector_size = duckdb.__standard_vector_size__
        count = 4 * vector_size + 3
        self.connection.execute("CREATE TEMP TABLE inputs(i BIGINT, path VARCHAR, source VARCHAR)")
        self.connection.executemany("INSERT INTO inputs VALUES (?, ?, ?)",
                                   [(i, *cases[i % len(cases)][0]) for i in range(count)])
        rows = self.connection.execute(
            "SELECT i, spur_graph_file_facts_result(path, source) FROM inputs "
            "WHERE i % 2 = 0 ORDER BY i").fetchall()
        self.assertGreater(len(rows), 2 * vector_size)
        self.assertEqual([i for i, _ in rows], list(range(0, count, 2)))
        for i, r in rows:
            _, status, facts, code = cases[i % len(cases)]
            self.assert_result(r, status, facts, code)
        for path, source in (good, (None, RUST), ("bad.unknown", "text")):
            rows = self.connection.execute(
                "SELECT spur_graph_file_facts_result(?::VARCHAR, ?::VARCHAR) FROM range(?)",
                [path, source, vector_size + 1]).fetchall()
            self.assertEqual(rows, [(self.result(path, source),)] * (vector_size + 1))
        for arguments, parameters in (("'virtual/雪/lib.rs', source", []),
                                      ("path, ?::VARCHAR", [RUST])):
            rows = self.connection.execute(
                f"SELECT spur_graph_file_facts_result({arguments}) FROM inputs "
                "WHERE path = 'virtual/雪/lib.rs' ORDER BY i", parameters).fetchall()
            self.assertTrue(rows)
            for (r,) in rows:
                self.assert_result(r, "ok", expected_good)

    def test_large_nested_lists_mixed_with_null_and_error_parents(self):
        count = duckdb.__standard_vector_size__ + 1
        fixture = ("many.rs", '#![allow(dead_code)]\nmacro_rules! local { () => {} }\n'
                   '#[cfg(test)]\nfn many() {\n' + 'local!(); assert!(true);\n' * count + '}\n')
        facts = native_facts([fixture])[0]
        self.assertGreater(len(facts["macro_invocations"]), duckdb.__standard_vector_size__)
        self.connection.execute("CREATE TEMP TABLE large(i INTEGER, path VARCHAR, source VARCHAR)")
        self.connection.executemany("INSERT INTO large VALUES (?, ?, ?)",
            [(0, None, None), (1, *fixture), (2, "bad.ipynb", "invalid"), (3, *fixture), (4, "ok.rs", "")])
        rows = self.connection.execute(
            "SELECT spur_graph_file_facts_result(path, source) FROM large ORDER BY i").fetchall()
        self.assert_result(rows[0][0], "null_input")
        self.assert_result(rows[1][0], "ok", facts)
        self.assert_result(rows[2][0], "extraction_error", code="malformed_input")
        self.assert_result(rows[3][0], "ok", facts)
        self.assert_result(rows[4][0], "empty_source", native_facts([("ok.rs", "")])[0])

    def test_repeated_connections_and_old_exception_semantics(self):
        expected = self.result(*FIXTURES[0])
        for connection in (self.connect(), self.connect()):
            self.assertEqual(connection.execute(
                "SELECT spur_graph_file_facts_result(?, ?)", FIXTURES[0]).fetchone()[0], expected)
            self.assertIsNone(connection.execute(
                "SELECT spur_graph_file_facts(NULL::VARCHAR, 'text')").fetchone()[0])
            with self.assertRaises(duckdb.InvalidInputException):
                connection.execute("SELECT spur_graph_file_facts('bad.unknown', 'text')")
            self.assertIsNone(connection.execute(
                "SELECT TRY(spur_graph_file_facts('bad.unknown', 'text'))").fetchone()[0])


if __name__ == "__main__":
    unittest.main()
