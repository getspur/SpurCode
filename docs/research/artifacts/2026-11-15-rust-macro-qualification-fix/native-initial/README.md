Preserve each macro callee’s qualified path before native resolution so unknown paths stay unresolved and valid local calls retain their heuristic metadata.

{
  "test_commits": [
    "08fed24010248119a0c883ed7cd65244a56b386a",
    "3427630baa2ffd3d1237cee9309758248e8497c4"
  ],
  "fix_commit": "9a05457df4d565979024d45296b76a8603cef99d",
  "modified_files": [
    "crates/spur-graph/queries/rust/spur-edges.scm",
    "crates/spur-graph/src/extract/languages.rs",
    "crates/spur-graph/tests/rust_macro_qualified_calls.rs"
  ],
  "green_passed": 62,
  "clean": true
}

Full diff: changes.patch. Production diff: production.patch. Exact commands, results and log SHA-256: commands.json. Initial sandbox network failure is red.log (exit 200); assertion RED is red-remote.log and red-strengthened.log (exit 101). Focused GREEN is green-focused.log; related GREEN is green-related.log; macro library GREEN is green-lib-macro.log. Formatting is fmt.log.
