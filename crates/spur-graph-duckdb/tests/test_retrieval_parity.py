"""Fault injections and real-engine retrieval smoke; no production source writes.

    SPUR_GRAPH_EXTENSION=/approved/spur_graph.duckdb_extension \
    SPUR_RETRIEVAL_NATIVE=/path/to/retrieval_native \
    /private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover \
      -s crates/spur-graph-duckdb/tests -p test_retrieval_parity.py -v

ComparatorTests need only the standard library. EngineSmokeTests require the
pinned real extension and authentic native executable; missing configuration is
a failure, never a skip or fake oracle. Temporary complete corpora are prepared
by the native executable. All smoke artifacts stay in a temporary directory;
the standalone compare CLI preserves permanent evidence in a new output dir.
"""

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "examples/retrieval_parity.py"
SPEC_PATH = Path(__file__).resolve().parent / "fixtures/retrieval/cases.json"
loader = importlib.util.spec_from_file_location("retrieval_parity", MODULE_PATH)
parity = importlib.util.module_from_spec(loader)
loader.loader.exec_module(parity)


def symbol(name="leaf", path="src/lib.rs", line=2):
    return dict(selector=f"{path}::{name}", id="engine-local-1", uri="graph://symbol/engine-local-1",
                entity_name=name, qualified_name=name, file_path=path,
                line_range=[line, line], symbol_kind="function", enclosing_scope=None)


def reply(request, body):
    return dict(request, body=body, error=None, elapsed_ns=1, coverage={"supported": True})


class ComparatorTests(unittest.TestCase):
    def setUp(self):
        self.req = dict(case_id="faults", tool="code_symbol_search", args={"query": "leaf"})
        self.native = reply(self.req, dict(candidates=[symbol(), symbol("other", line=3)],
                                          total_matches=2, truncated=False, file=None))

    def compare(self, candidate, native=None, request=None):
        return parity.compare_replies(request or self.req, native or self.native, candidate)

    def test_current_release_guard_requires_explicit_manifest_trust(self):
        self.assertTrue(callable(getattr(parity, "approved_extension_sha", None)),
                        "retrieval helper must expose verified release-manifest approval")
        with tempfile.TemporaryDirectory(prefix="parity-unapproved-") as directory:
            binary = Path(directory) / "spur_graph.duckdb_extension"
            binary.write_bytes(b"arbitrary runtime bytes")
            with self.assertRaisesRegex(ValueError, "unapproved"):
                parity.approved_extension_sha(binary)

    def test_identical_baseline_and_raw_preservation(self):
        before = copy.deepcopy(self.native)
        result = self.compare(copy.deepcopy(self.native))
        self.assertTrue(result["semantic_equal"])
        self.assertTrue(result["full_api_equal"])
        self.assertEqual(before, self.native)

    def test_missing_extra_and_duplicate_rows_even_if_counts_are_unchanged(self):
        for mutation in ("missing", "extra", "duplicate"):
            with self.subTest(mutation=mutation):
                candidate = copy.deepcopy(self.native)
                rows = candidate["body"]["candidates"]
                if mutation == "missing":
                    rows.pop()
                elif mutation == "extra":
                    rows.append(symbol("unexpected", line=90))
                else:
                    rows.append(copy.deepcopy(rows[0]))
                result = self.compare(candidate)
                self.assertFalse(result["semantic_equal"])
                self.assertFalse(result["rows"]["candidates"]["multiset_equal"])
                self.assertTrue(result["semantic_differences"])

    def test_duplicate_loss_with_adjusted_count_still_fails(self):
        native = copy.deepcopy(self.native)
        native["body"]["candidates"].append(copy.deepcopy(native["body"]["candidates"][0]))
        native["body"]["total_matches"] = 3
        result = self.compare(copy.deepcopy(self.native), native)
        rows = result["rows"]["candidates"]
        self.assertFalse(result["semantic_equal"])
        self.assertEqual(rows["native_duplicates"], 1)
        self.assertEqual(rows["missing"][0]["count"], 1)

    def test_source_path_range_kind_scope_and_unknown_field_are_semantic(self):
        for key, value in (("file_path", "src/wrong.rs"), ("line_range", [2, 4]),
                           ("symbol_kind", "struct"), ("enclosing_scope", "wrong"),
                           ("new_semantic_field", {"evidence": [1, 2]})):
            with self.subTest(key=key):
                candidate = copy.deepcopy(self.native)
                candidate["body"]["candidates"][0][key] = value
                self.assertFalse(self.compare(candidate)["semantic_equal"])

    def test_null_missing_and_scalar_types_are_distinct(self):
        for mutation in ("missing", "empty", "false", "number"):
            candidate = copy.deepcopy(self.native)
            if mutation == "missing":
                del candidate["body"]["file"]
            else:
                candidate["body"]["file"] = {"empty": "", "false": False, "number": 0}[mutation]
            self.assertFalse(self.compare(candidate)["semantic_equal"])
        self.assertTrue(parity.differences(True, 1))

    def test_rank_counts_filter_limit_and_truncation_are_not_dropped(self):
        for field, value in (("total_matches", 9), ("truncated", True), ("file", "src/beta.rs"), ("limit", 1)):
            candidate = copy.deepcopy(self.native)
            candidate["body"][field] = value
            self.assertFalse(self.compare(candidate)["semantic_equal"])
        candidate = copy.deepcopy(self.native)
        candidate["body"]["candidates"].reverse()
        result = self.compare(candidate)
        self.assertFalse(result["semantic_equal"])
        self.assertTrue(result["rows"]["candidates"]["multiset_equal"])
        self.assertFalse(result["rows"]["candidates"]["observed_order_equal"])

    def test_identity_mapping_is_separate_from_raw_ids_uri_and_api(self):
        candidate = copy.deepcopy(self.native)
        candidate["body"]["candidates"][0].update(id="different", uri="graph://symbol/different")
        candidate["body"]["graph_content_hash"] = "sql-independent"
        result = self.compare(candidate)
        self.assertTrue(result["semantic_equal"])
        self.assertFalse(result["full_api_equal"])
        self.assertFalse(result["raw_identity_equal"])
        self.assertTrue(result["metadata_differences"])
        candidate["body"]["candidates"][0]["id"] = None
        self.assertFalse(self.compare(candidate)["semantic_equal"])
        candidate["body"]["candidates"][0]["id"] = 123
        self.assertFalse(self.compare(candidate)["semantic_equal"])

    def test_metadata_names_are_only_excluded_at_documented_locations(self):
        candidate = copy.deepcopy(self.native)
        candidate["body"]["candidates"][0]["graph_content_hash"] = "unexpected row field"
        self.assertFalse(self.compare(candidate)["semantic_equal"])

    def test_metadata_only_error_object_and_null_are_not_collapsed(self):
        native = dict(self.req, body=None, error=dict(code=-32602, message="invalid", data={"graph_content_hash": "native"}))
        candidate = dict(self.req, body=None, error=dict(code=-32602, message="invalid", data=None), coverage={"supported": True})
        result = self.compare(candidate, native)
        self.assertFalse(result["semantic_equal"])
        self.assertEqual(result["semantic_differences"][0]["path"], "/error/data")

    def edge_pair(self):
        request = dict(case_id="edge_faults", tool="code_callees", args={"selector": "entry", "include_unresolved": True})
        edge = {k: v for k, v in symbol().items() if k not in {"selector", "id", "qualified_name"}}
        edge.update(resolved=True, edge_kind="calls", confidence="syntax_exact", bind_method="singleton")
        unresolved = dict(resolved=False, entity_name="outside", target_label="outside", edge_kind="calls")
        native = reply(request, dict(callees=[edge, unresolved], include_unresolved=True,
                                     counts_by_kind={"calls": 2, "unresolved": 1},
                                     counts_by_context={"production": 1}, unresolved_sample=["outside"]))
        return request, native

    def test_changed_edge_kind_confidence_binding_evidence_and_unresolved_omission(self):
        req, native = self.edge_pair()
        for field, value in (("edge_kind", "references_hof"), ("confidence", "heuristic"),
                             ("bind_method", None), ("evidence", {"byte_range": [9, 20]}), ("resolved", False)):
            candidate = copy.deepcopy(native)
            candidate["body"]["callees"][0][field] = value
            self.assertFalse(self.compare(candidate, native, req)["semantic_equal"])
        for mutation in ("row", "count", "sample", "label"):
            candidate = copy.deepcopy(native)
            if mutation == "row":
                candidate["body"]["callees"].pop()
            elif mutation == "count":
                candidate["body"]["counts_by_kind"]["unresolved"] = 0
            elif mutation == "sample":
                candidate["body"]["unresolved_sample"] = []
            else:
                candidate["body"]["callees"][1]["target_label"] = "different"
            self.assertFalse(self.compare(candidate, native, req)["semantic_equal"])

    def test_traversal_multiset_keeps_duplicates_and_observed_order(self):
        req, native = self.edge_pair()
        candidate = copy.deepcopy(native)
        candidate["body"]["callees"].reverse()
        result = self.compare(candidate, native, req)
        self.assertTrue(result["semantic_equal"])
        self.assertFalse(result["full_api_equal"])
        self.assertFalse(result["rows"]["callees"]["observed_order_equal"])
        candidate["body"]["callees"].append(copy.deepcopy(candidate["body"]["callees"][0]))
        self.assertFalse(self.compare(candidate, native, req)["semantic_equal"])

    def test_exact_unicode_crlf_source_and_context_corruption(self):
        source = '// café 🦀\r\npub fn leaf() {}\r\n// end\r\n'
        req = dict(case_id="source", tool="code_read_symbol", args={"path": "src/lib.rs", "name": "leaf", "context_lines": 1})
        native = reply(req, dict(symbol=symbol(), source=source, line_range={"start": 1, "end": 3},
                                 file_oid=parity.blob_oid(source.encode()), context_lines=1))
        spec = dict(files=[dict(path="src/lib.rs", source=source)])
        self.assertTrue(parity.source_checks(native, spec)[0]["passed"])
        for corrupted in (source.replace("\r\n", "\n"), source.replace("é", "e\u0301"), source[:-2], source.replace("🦀", "?")):
            candidate = copy.deepcopy(native)
            candidate["body"]["source"] = corrupted
            self.assertFalse(self.compare(candidate, native, req)["semantic_equal"])
            self.assertFalse(parity.source_checks(candidate, spec)[0]["passed"])
        candidate = copy.deepcopy(native)
        candidate["body"]["line_range"]["end"] = 2
        self.assertFalse(self.compare(candidate, native, req)["semantic_equal"])
        self.assertFalse(parity.source_checks(candidate, spec)[0]["passed"])

    def test_unsupported_is_explicit_and_cannot_be_an_empty_parity_success(self):
        candidate = dict(self.req, body=None, error=dict(code=-32099, message="unsupported", data={"kind": "unsupported"}),
                         coverage={"supported": False})
        result = self.compare(candidate)
        self.assertFalse(result["candidate_supported"])
        self.assertFalse(result["semantic_equal"])
        self.assertFalse(result["matched_semantic_work"])

    def test_request_mismatch_and_invalid_envelopes_fail_closed(self):
        candidate = copy.deepcopy(self.native)
        candidate["args"] = {"query": "another"}
        with self.assertRaises(ValueError):
            self.compare(candidate)
        candidate = copy.deepcopy(self.native)
        candidate["error"] = {"code": 1}
        with self.assertRaises(ValueError):
            self.compare(candidate)


def current_release():
    if not any(os.environ.get(key) for key in (
            "SPUR_RELEASE_MANIFEST", "SPUR_RELEASE_MANIFEST_SHA256", "SPUR_RELEASE_SOURCE_IDENTITY")):
        return {}
    scripts = str(MODULE_PATH.parents[1] / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    from check_release import approved_extension_sha, release_trust
    trust = release_trust()
    return dict(expected_sha=approved_extension_sha(os.environ["SPUR_GRAPH_EXTENSION"], **trust),
                release_manifest=trust["manifest"], release_manifest_sha256=trust["manifest_sha256"],
                release_source_identity=trust["source_identity"])


class EngineSmokeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.extension = os.environ.get("SPUR_GRAPH_EXTENSION")
        cls.native = os.environ.get("SPUR_RETRIEVAL_NATIVE")
        if not cls.extension or not cls.native:
            raise AssertionError("set SPUR_GRAPH_EXTENSION and SPUR_RETRIEVAL_NATIVE; real-engine smoke is mandatory")
        cls.temp = tempfile.TemporaryDirectory(prefix="spur-retrieval-parity-tests-")
        cls.root = Path(cls.temp.name)
        cls.corpus = cls.root / "corpus"
        result = subprocess.run([cls.native, "prepare", str(SPEC_PATH), str(cls.corpus)], capture_output=True)
        if result.returncode:
            cls.temp.cleanup()
            raise AssertionError(result.stderr.decode())
        cls.prepare = json.loads(result.stdout)
        cls.release = current_release()
        cls.candidate = parity.Candidate(cls.corpus, cls.extension, cls.root / "independent-sql", **cls.release)
        cls.spec = cls.candidate.spec
        cls.requests = {r["case_id"]: r for r in cls.spec["requests"]}
        cls.sql = {k: cls.candidate.dispatch(v) for k, v in cls.requests.items()}

    @classmethod
    def tearDownClass(cls):
        cls.candidate.finish()
        cls.temp.cleanup()

    def test_extension_facts_typed_materialization_and_file_local_id_joins(self):
        db = self.candidate.db
        self.assertEqual(self.candidate.provenance["duckdb"], "2.0.0")
        self.assertEqual(self.candidate.provenance["extension_sha256"], self.release.get("expected_sha", parity.APPROVED_EXTENSION_SHA256))
        self.assertEqual(db.execute("SELECT count(*) FROM sources").fetchone()[0], len(self.spec["files"]))
        for collection in ("nodes", "edges", "spans", "macro_invocations", "macro_scopes"):
            source = db.execute(f"SELECT sum(len(value.{collection})) FROM facts").fetchone()[0]
            self.assertEqual(db.execute(f"SELECT count(*) FROM {collection}").fetchone()[0], source)
        # Every file starts IDs at 1. A path-less join would multiply these rows.
        self.assertEqual(db.execute("SELECT count(*) FROM nodes WHERE node_id=1").fetchone()[0], 6)
        self.assertEqual(len(self.sql["code_callees_samefile_all"]["body"]["callees"]), 1)
        self.assertEqual(self.sql["code_callees_crossfile_all"]["body"]["counts_by_kind"]["unresolved"], 2)
        self.assertEqual(self.sql["code_callees_import_resolved"]["body"]["callees"], [])
        self.assertTrue(db.execute("SELECT count(*) FROM macro_invocations").fetchone()[0])
        schema = json.loads((self.root / "independent-sql/schema.json").read_bytes())
        self.assertTrue(any(row[0] == "facts" and row[1] == "value" and "STRUCT" in row[2] for row in schema))

    def test_sql_search_modes_filters_ranking_limit_and_parameter_binding(self):
        self.assertEqual(self.sql["search_duplicates"]["body"]["total_matches"], 4)
        self.assertEqual(len(self.sql["search_glob"]["body"]["candidates"]), 2)
        self.assertEqual(self.sql["search_file"]["body"]["candidates"][0]["file_path"], "src/alpha.rs")
        self.assertEqual(self.sql["search_method"]["body"]["candidates"][0]["qualified_name"], "impl Widget::run")
        self.assertEqual(self.sql["search_limit"]["body"]["total_matches"], 4)
        self.assertTrue(self.sql["search_limit"]["body"]["truncated"])
        self.assertEqual(len(self.sql["search_limit"]["body"]["candidates"]), 1)
        for case in ("search_empty", "search_wrong_kind", "search_wrong_file"):
            self.assertEqual(self.sql[case]["body"]["candidates"], [])
        injected = dict(case_id="quote", tool="code_symbol_search", args={"query": "' OR true --", "mode": "exact"})
        self.assertEqual(self.candidate.dispatch(injected)["body"]["total_matches"], 0)
        request = dict(case_id="clamped", tool="code_symbol_search", args={"query": "repeated", "limit": 0})
        body = self.candidate.dispatch(request)["body"]
        self.assertEqual((body["limit"], body["requested_limit"]), (1, 0))
        self.assertTrue(self.candidate.query_log[-1]["params"])
        scoped = self.candidate.dispatch(dict(case_id="qualified", tool="code_callees", args={"selector": "left::invoke"}))
        self.assertEqual(scoped["body"]["callees"][0]["enclosing_scope"], "left")

    def test_exact_sql_source_bytes_context_and_ranges(self):
        checked = 0
        for result in self.sql.values():
            for check in parity.source_checks(result, self.spec):
                self.assertTrue(check["passed"], check)
                self.assertTrue(check["file_oid_matches"])
                checked += 1
        self.assertGreaterEqual(checked, 9)
        text = self.sql["read_crlf"]["body"]["source"]
        self.assertIn("naïve café 🦀", text)
        self.assertEqual(text.count("\r\n"), 3)
        entire = next(f["source"] for f in self.spec["files"] if f["path"] == "src/crlf.rs")
        self.assertEqual(self.sql["read_context_boundary"]["body"]["source"].encode(), entire.encode())
        self.assertEqual(self.sql["read_context_boundary"]["body"]["line_range"], dict(start=1, end=6))

    def test_negative_ambiguity_and_unsupported_requests(self):
        self.assertEqual(self.sql["read_missing"]["error"]["code"], -32004)
        self.assertEqual(self.sql["read_path_rejected"]["error"]["code"], -32602)
        self.assertGreaterEqual(len(self.sql["read_ambiguous"]["body"]["candidates"]), 2)
        self.assertEqual(self.sql["code_callees_ambiguity_error"]["error"]["code"], -32602)
        for args in ({"query": "leaf", "file_glob": "src/**/*.rs"}, {"query": "leaf", "response_format": "table"},
                     {"query": "leaf", "as_of": "HEAD"}):
            result = self.candidate.dispatch(dict(case_id="unsupported", tool="code_symbol_search", args=args))
            self.assertIsNone(result["body"])
            self.assertEqual(result["error"]["data"]["kind"], "unsupported")
            self.assertFalse(result["coverage"]["supported"])

    def test_candidate_cannot_read_oracle_files_and_detects_changed_sources(self):
        original = Path.read_bytes
        touched = []
        def guarded(path):
            touched.append(str(path))
            self.assertNotIn(".retrieval/native", str(path))
            self.assertNotIn(".spur/graph", str(path))
            return original(path)
        with patch.object(Path, "read_bytes", guarded):
            independent = parity.Candidate(self.corpus, self.extension, **self.release)
            self.assertEqual(independent.dispatch(self.requests["search_exact"])["body"]["total_matches"], 1)
            independent.finish()
        self.assertTrue(touched)
        with self.assertRaisesRegex(ValueError, "unapproved extension hash"):
            parity.Candidate(self.corpus, self.extension, expected_sha="0"*64)
        source = self.corpus / "src/crlf.rs"
        saved = source.read_bytes()
        try:
            source.write_bytes(saved.replace(b"\r\n", b"\n"))
            with self.assertRaisesRegex(ValueError, "source differs"):
                parity.shared_inputs(self.corpus)
        finally:
            source.write_bytes(saved)

    def test_true_native_and_sql_persistent_cli_and_raw_mismatch_report(self):
        output = self.root / "comparison"
        release_args = []
        if self.release:
            release_args = ["--extension-sha256", self.release["expected_sha"],
                            "--release-manifest", self.release["release_manifest"],
                            "--release-manifest-sha256", self.release["release_manifest_sha256"],
                            "--release-source-identity", self.release["release_source_identity"]]
        process = subprocess.run([sys.executable, str(MODULE_PATH), "compare", "--native", self.native,
                                  "--corpus", str(self.corpus), "--spec", str(SPEC_PATH),
                                  "--extension", self.extension, "--out", str(output), *release_args], capture_output=True)
        self.assertEqual(process.returncode, 0, process.stderr.decode())
        report = json.loads((output / "report.json").read_bytes())
        self.assertEqual(report["summary"]["requests"], 58)
        self.assertEqual(report["summary"]["native_validation_failures"], 0)
        self.assertEqual(report["summary"]["candidate_source_failures"], 0)
        self.assertGreater(report["summary"]["semantic_matches"], 0)
        self.assertLess(report["summary"]["semantic_matches"], 58)  # global resolution is a real coverage result
        self.assertEqual(report["speed_claims"], [])
        native = [json.loads(r) for r in (output / "native.jsonl").read_bytes().splitlines()]
        sql = [json.loads(r) for r in (output / "candidate.jsonl").read_bytes().splitlines()]
        self.assertEqual(len(native), 58)
        self.assertEqual(len(sql), 58)
        cases = {r["case_id"]: r for r in report["cases"]}
        self.assertTrue(cases["search_exact"]["semantic_equal"])
        self.assertTrue(cases["read_crlf_context"]["semantic_equal"])
        self.assertFalse(cases["search_exact"]["full_api_equal"])
        self.assertFalse(cases["code_callers_import_all"]["semantic_equal"])
        self.assertFalse(cases["code_callees_import_all"]["semantic_equal"])
        self.assertTrue(cases["code_callers_import_all"]["rows"]["callers"]["missing"])
        # Coordinator preserves complete bodies, including metadata and extension evidence.
        self.assertIn("graph_content_hash", native[0]["body"])
        self.assertTrue(any(r.get("evidence", {}).get("extension_edge_records") for r in sql if r.get("evidence")))
        samples = json.loads((output / "smoke-samples.json").read_bytes())
        self.assertEqual(len(samples), 116)
        self.assertEqual({s["endpoint"] for s in samples}, {"native", "candidate"})
        self.assertTrue(all(s["client_wall_ns"] > 0 and s["request_bytes"] > 0 and s["response_bytes"] > 0 for s in samples))
        # Output evidence cannot be silently overwritten.
        repeated = subprocess.run([sys.executable, str(MODULE_PATH), "compare", "--native", self.native,
                                   "--corpus", str(self.corpus), "--extension", self.extension, "--out", str(output), *release_args], capture_output=True)
        self.assertNotEqual(repeated.returncode, 0)
        manifest = json.loads((output / "manifest.json").read_bytes())
        for entry in manifest:
            self.assertEqual(parity.sha256((output / entry["path"]).read_bytes()), entry["sha256"])


if __name__ == "__main__":
    unittest.main()
