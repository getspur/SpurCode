"""Typed pair-local temporal matching, using the DuckDB 2.0.0 test runtime."""

import os
from pathlib import Path
import tempfile
import unittest

import duckdb


SYMBOL_TYPE = (
    "STRUCT(entity_name VARCHAR, symbol_kind VARCHAR, enclosing_scope VARCHAR, "
    "byte_start UBIGINT, byte_end UBIGINT, line_start UBIGINT, line_end UBIGINT, "
    "anchor_hash VARCHAR, tokens VARCHAR[])"
)
SYMBOL_LIST = f"{SYMBOL_TYPE}[]"


def symbol(name="item", start=0, anchor="anchor", tokens=None):
    return dict(entity_name=name, symbol_kind="function", enclosing_scope=None,
                byte_start=start, byte_end=start + 10, line_start=1, line_end=2,
                anchor_hash=anchor, tokens=["body", "token"] if tokens is None else tokens)


class TemporalTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, "2.0.0")
        self.connection = duckdb.connect(config={"allow_unsigned_extensions": True})
        extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        if extension:
            path = str(Path(extension).resolve()).replace("'", "''")
            self.connection.execute(f"LOAD '{path}'")

    def tearDown(self):
        self.connection.close()

    def test_registration(self):
        functions = self.connection.execute(
            "SELECT parameter_types, return_type FROM duckdb_functions() "
            "WHERE function_name = 'spur_graph_symbol_changes' AND function_type = 'scalar'"
        ).fetchall()
        self.assertEqual(len(functions), 1, "spur_graph_symbol_changes must be registered")
        parameters, result = functions[0]
        self.assertEqual(parameters, ["VARCHAR"] * 3 + [SYMBOL_LIST] * 2)
        self.assertEqual(result, f"STRUCT(changes STRUCT(symbol {SYMBOL_TYPE}, stable_symbol_id VARCHAR, "
                         "file_path VARCHAR, change_kind VARCHAR, previous_stable_symbol_id VARCHAR)[], "
                         "diagnostics VARCHAR[])")

    def changes(self, kind, old, new, path="input.rs", old_path=None):
        if old_path is None:
            old_path = path
        return self.connection.execute(
            f"SELECT spur_graph_symbol_changes(?::VARCHAR, ?::VARCHAR, ?::VARCHAR, "
            f"?::{SYMBOL_LIST}, ?::{SYMBOL_LIST})", [path, old_path, kind, old, new]
        ).fetchone()[0]

    def extract(self, path, source):
        return self.connection.execute("SELECT spur_graph_symbols(?, ?)", [path, source]).fetchone()[0]

    def test_added_deleted_noop_and_modified(self):
        old = self.extract("input.rs", "pub fn item(x: u32) -> u32 { x + 1 }")
        new = self.extract("input.rs", "pub fn item(x: u32) -> u32 { x + 2 }")
        added = self.changes("A", [], old, old_path="")
        deleted = self.changes("D", old, [])
        self.assertEqual(added["diagnostics"], [])
        self.assertEqual(added["changes"][0]["symbol"], old[0])
        self.assertEqual(added["changes"][0]["change_kind"], "added")
        self.assertEqual(deleted["changes"][0]["change_kind"], "deleted")
        self.assertEqual(deleted["changes"][0]["stable_symbol_id"], added["changes"][0]["stable_symbol_id"])
        self.assertIsNone(added["changes"][0]["previous_stable_symbol_id"])
        self.assertEqual(self.changes("M", old, old), {"changes": [], "diagnostics": []})
        modified = self.changes("M", old, new)["changes"]
        self.assertEqual(len(modified), 1)
        self.assertEqual(modified[0]["change_kind"], "modified")

    def test_equal_anchor_byte_shift_is_modified(self):
        old = symbol()
        new = dict(old, byte_start=20, byte_end=30)
        result = self.changes("M", [old], [new])
        self.assertEqual([c["change_kind"] for c in result["changes"]], ["modified"])
        self.assertEqual(result["changes"][0]["symbol"], new)

    def test_rename_body_and_old_path_identifiers(self):
        old = dict(symbol("before"), enclosing_scope="impl Outer")
        new = dict(symbol("after"), enclosing_scope="impl Outer")
        before_id = self.changes("A", [], [old], path="old.rs")["changes"][0]["stable_symbol_id"]
        renamed = self.changes("R", [old], [new], path="new.rs", old_path="old.rs")["changes"]
        self.assertEqual(len(renamed), 1)
        self.assertEqual(renamed[0]["change_kind"], "renamed_from_symbol")
        self.assertEqual(renamed[0]["previous_stable_symbol_id"], before_id)
        self.assertEqual(renamed[0]["file_path"], "new.rs")
        self.assertEqual(renamed[0]["symbol"], new)
        self.assertNotEqual(renamed[0]["stable_symbol_id"], before_id)
        body_rename = self.changes("M", [old], [new])["changes"]
        self.assertEqual(body_rename[0]["change_kind"], "renamed_from_symbol")

    def test_current_cpp_language_has_no_jaccard_rename(self):
        result = self.changes("R", [symbol("before")], [symbol("after")],
                              path="new.cpp", old_path="old.rs")
        self.assertCountEqual([c["change_kind"] for c in result["changes"]], ["added", "deleted"])
        self.assertTrue(all(c["previous_stable_symbol_id"] is None for c in result["changes"]))

    def test_duplicate_identity_and_rename_order_are_preserved(self):
        first = symbol("overload", start=0, anchor="first")
        last = symbol("overload", start=20, anchor="last")
        self.assertEqual(self.changes("M", [first, last], [last]), {"changes": [], "diagnostics": []})
        old_ids = [c["stable_symbol_id"] for c in self.changes("A", [], [first, last], path="old.rs")["changes"]]
        renamed = self.changes("R", [first, last], [last, first], path="new.rs", old_path="old.rs")["changes"]
        self.assertEqual([c["symbol"]["anchor_hash"] for c in renamed], ["last", "first"])
        self.assertEqual([c["previous_stable_symbol_id"] for c in renamed], old_ids)

    def test_every_ambiguity_diagnostic_is_returned(self):
        result = self.changes("M", [symbol("before_a"), symbol("before_b", start=20)],
                              [symbol("after", start=40)])
        self.assertEqual(len(result["changes"]), 3)
        self.assertEqual(len(result["diagnostics"]), 5)
        self.assertEqual(sum(d.startswith("ambiguous_rename:") for d in result["diagnostics"]), 4)
        self.assertIn("merge_collision: file=input.rs candidate=after", result["diagnostics"])

    def test_empty_and_top_level_null_are_distinct(self):
        self.assertEqual(self.changes("A", [], []), {"changes": [], "diagnostics": []})
        arguments = ["input.rs", "input.rs", "M", [], []]
        for index in range(len(arguments)):
            values = arguments.copy()
            values[index] = None
            result = self.connection.execute(
                f"SELECT spur_graph_symbol_changes(?::VARCHAR, ?::VARCHAR, ?::VARCHAR, "
                f"?::{SYMBOL_LIST}, ?::{SYMBOL_LIST})", values).fetchone()[0]
            self.assertIsNone(result, index)

    def test_null_nested_values_and_invalid_kinds_are_rejected(self):
        invalid = [[None]]
        for field in symbol():
            if field != "enclosing_scope":
                invalid.append([dict(symbol(), **{field: None})])
        invalid.append([symbol(tokens=[None])])
        for symbols in invalid:
            for side in ("left", "right"):
                with self.assertRaisesRegex(duckdb.InvalidInputException, "spur_graph_symbol_changes"):
                    self.changes("M", symbols if side == "left" else [], symbols if side == "right" else [])
        for kind in ("T", "C", "unknown", ""):
            with self.assertRaisesRegex(duckdb.InvalidInputException, "kind"):
                self.changes(kind, [], [])
        with self.assertRaisesRegex(duckdb.InvalidInputException, "old_path"):
            self.changes("M", [], [], old_path="other.rs")

    def test_selected_vectors_constants_and_multiple_chunks(self):
        count = 2 * duckdb.__standard_vector_size__ + 1
        self.connection.execute(
            "CREATE TEMP TABLE pairs AS SELECT i, CASE WHEN i % 3 = 0 THEN '' "
            "WHEN i % 3 = 1 THEN 'pub fn f' || i || '() -> u32 { 1 }' "
            "ELSE 'pub fn f' || i || '() -> u32 { 1 }\npub fn g' || i || '() -> u32 { 2 }' END AS source "
            "FROM range(?) t(i)", [count])
        self.connection.execute(
            "CREATE TEMP TABLE cached AS SELECT i, spur_graph_symbols('input.rs', source) symbols FROM pairs")
        rows = self.connection.execute(
            "SELECT i, spur_graph_symbol_changes('input.rs', '', 'A', spur_graph_symbols('input.rs', ''), "
            "symbols) FROM cached WHERE i % 2 = 0 ORDER BY i"
        ).fetchall()
        self.assertEqual([i for i, _ in rows], list(range(0, count, 2)))
        for i, result in rows:
            self.assertEqual(len(result["changes"]), i % 3)
            self.assertTrue(all(c["change_kind"] == "added" for c in result["changes"]))
            self.assertTrue(all(c["symbol"]["tokens"] for c in result["changes"]))
        deleted = self.connection.execute(
            "SELECT i, spur_graph_symbol_changes('input.rs', 'input.rs', 'D', symbols, "
            "spur_graph_symbols('input.rs', '')) FROM cached WHERE i % 2 = 0 ORDER BY i"
        ).fetchall()
        for (i, added), (other_i, removed) in zip(rows, deleted):
            self.assertEqual(i, other_i)
            self.assertCountEqual([c["symbol"] for c in added["changes"]],
                                  [c["symbol"] for c in removed["changes"]])
            self.assertTrue(all(c["change_kind"] == "deleted" for c in removed["changes"]))
        repeated = self.connection.execute(
            "SELECT spur_graph_symbol_changes('input.rs', '', 'A', spur_graph_symbols('input.rs', ''), "
            "spur_graph_symbols('input.rs', 'pub fn constant() -> u32 { 1 }')) FROM range(3)"
        ).fetchall()
        self.assertEqual(repeated, [repeated[0]] * 3)

    def test_parquet_input_and_output_round_trip(self):
        self.connection.execute(
            "CREATE TEMP TABLE parsed AS SELECT i, spur_graph_symbols('input.rs', source) symbols "
            "FROM (VALUES (0, 'pub fn alpha() -> u32 { 1 }'), (1, '')) t(i, source)")
        with tempfile.TemporaryDirectory() as directory:
            inputs = str(Path(directory) / "inputs.parquet").replace("'", "''")
            output = str(Path(directory) / "changes.parquet").replace("'", "''")
            self.connection.execute(f"COPY parsed TO '{inputs}' (FORMAT PARQUET)")
            self.connection.execute(
                "CREATE TEMP TABLE matched AS SELECT i, spur_graph_symbol_changes('input.rs', '', 'A', "
                f"[]::{SYMBOL_LIST}, symbols) result FROM read_parquet('{inputs}')")
            expected = self.connection.execute("SELECT * FROM matched ORDER BY i").fetchall()
            self.connection.execute(f"COPY matched TO '{output}' (FORMAT PARQUET)")
            with duckdb.connect() as reader:
                self.assertEqual(reader.execute(f"SELECT * FROM read_parquet('{output}') ORDER BY i").fetchall(), expected)
            self.assertEqual(expected[1][1], {"changes": [], "diagnostics": []})


if __name__ == "__main__":
    unittest.main()
