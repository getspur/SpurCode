"""Run with the DuckDB 2.0.0 test environment and SPUR_GRAPH_EXTENSION set."""

import os
from pathlib import Path
import tempfile
import unittest

import duckdb


class ExtensionTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        self.connection = duckdb.connect(config={"allow_unsigned_extensions": True})
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        if extension:
            quoted = str(Path(extension).resolve()).replace("'", "''")
            self.connection.execute(f"LOAD '{quoted}'")

    def tearDown(self):
        self.connection.close()

    def require_function(self):
        found = self.connection.execute(
            "SELECT count(*) FROM duckdb_functions() "
            "WHERE function_name = 'spur_graph_symbols' AND function_type = 'scalar'"
        ).fetchone()[0]
        self.assertEqual(found, 1, "spur_graph_symbols must be registered by the extension")

    def symbols(self, path, source):
        self.require_function()
        return self.connection.execute(
            "SELECT spur_graph_symbols(?::VARCHAR, ?::VARCHAR)", [path, source]
        ).fetchone()[0]

    def test_registration(self):
        self.require_function()

    def test_rust_symbols_have_typed_fields(self):
        source = "pub fn alpha() -> i32 { 1 }\n"
        symbols = self.symbols("input.rs", source)
        alpha = next(symbol for symbol in symbols if symbol["entity_name"] == "alpha")
        self.assertEqual(alpha["symbol_kind"], "function")
        self.assertIsNone(alpha["enclosing_scope"])
        self.assertIn("fn alpha", source.encode()[alpha["byte_start"]:alpha["byte_end"]].decode())
        self.assertLessEqual(alpha["line_start"], alpha["line_end"])
        self.assertTrue(alpha["anchor_hash"])
        self.assertIsInstance(alpha["tokens"], list)

    def test_cpp_uses_the_existing_language_registry(self):
        symbols = self.symbols("input.cpp", "int total(int x) { return x + 1; }\n")
        self.assertIn("total", [symbol["entity_name"] for symbol in symbols])

    def test_nested_scope_and_nonempty_tokens_are_preserved(self):
        symbols = self.symbols(
            "input.rs", "pub struct Outer; impl Outer { pub fn nested(&self) -> i32 { 1 } }"
        )
        nested = next(symbol for symbol in symbols if symbol["entity_name"] == "nested")
        self.assertEqual(nested["enclosing_scope"], "impl Outer")
        self.assertEqual(nested["tokens"], ["1", "i32"])

    def test_null_and_empty_results_remain_distinct(self):
        self.assertIsNone(self.symbols(None, "fn alpha() {}"))
        self.assertIsNone(self.symbols("input.rs", None))
        self.assertEqual(self.symbols("input.rs", ""), [])

    def test_unsupported_path_is_a_sql_error(self):
        self.require_function()
        with self.assertRaisesRegex(duckdb.InvalidInputException, "unsupported language for path"):
            self.connection.execute("SELECT spur_graph_symbols('input.unknown', 'text')").fetchall()

    def test_variable_lists_survive_selection_and_multiple_chunks(self):
        self.require_function()
        count = 2 * duckdb.__standard_vector_size__ + 1
        self.connection.execute(
            "CREATE TEMP TABLE sources AS SELECT i, CASE WHEN i % 3 = 0 THEN '' "
            "WHEN i % 3 = 1 THEN 'pub fn f' || i || '() {}\npub fn g' || i || '() {}' "
            "ELSE 'pub fn f' || i || '() {}' END AS source FROM range(?) t(i)", [count]
        )
        rows = self.connection.execute(
            "SELECT i, spur_graph_symbols('input.rs', source) "
            "FROM sources WHERE i % 2 = 0 ORDER BY i"
        ).fetchall()
        self.assertEqual([row[0] for row in rows], list(range(0, count, 2)))
        for index, symbols in rows:
            expected = set() if index % 3 == 0 else {f"f{index}"}
            if index % 3 == 1:
                expected.add(f"g{index}")
            self.assertEqual({symbol["entity_name"] for symbol in symbols}, expected)

    def test_struct_lists_round_trip_through_parquet(self):
        self.require_function()
        self.connection.execute(
            "CREATE TEMP TABLE parsed AS SELECT i AS blob_id, "
            "spur_graph_symbols('input.rs', source) AS symbols "
            "FROM (VALUES (0, 'pub fn alpha() {}'), (1, '')) t(i, source)"
        )
        expected = self.connection.execute("SELECT * FROM parsed ORDER BY blob_id").fetchall()
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "symbols.parquet").replace("'", "''")
            self.connection.execute(f"COPY parsed TO '{path}' (FORMAT PARQUET)")
            with duckdb.connect() as reader:
                actual = reader.execute(f"SELECT * FROM read_parquet('{path}') ORDER BY blob_id").fetchall()
            self.assertEqual(actual, expected)
        self.assertEqual(expected[1][1], [])


if __name__ == "__main__":
    unittest.main()
