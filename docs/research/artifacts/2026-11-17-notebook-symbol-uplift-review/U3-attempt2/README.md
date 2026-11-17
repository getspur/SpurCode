# U3 attempt 2 root approval

Approved the legacy notebook diagnostic compatibility repair. Root independently reran the previously failing native diagnostic test against fresh package ed42b39b: exit 0. Eight malformed JSON/inventory inputs also returned the expected row diagnostic, with the same DuckDB connection usable after every error.

The narrow file_facts.rs change recognizes the exact shared-normalization input-error contexts with complete diagnostic-list delimiters. It preserves original error text, extraction_error/malformed_input and NULL facts. Existing NoTree/setup/internal-error and near-miss checks still propagate errors. No other U3 adapter, core, public schema or ordinary-file behavior changed.

Root verified all 10 attempt-2 execution receipts and their raw logs, exact test predicates, current source bytes, native library/build-script/package hashes and independently recomputed the 325-input extractor identity. All GREEN suites bind to this one fresh artifact: 7 notebook, 8 provenance, 4 Rust and 76 legacy/consumer tests, with no skips. All 17 prior receipts, earlier packages/raw library and historical package 28ddce83 remain unchanged. Original test 61d65f537 precedes fix df39b0d8d. Plan Engine normalization was accounted for explicitly when verifying original ancestry; current source bytes match the original fix.

See review.json for exact root diagnostic command/environment, captured outputs and verifier method. Exact SQL schemas remain unchanged. U4 consumer routing/cache/publication and U5 final native release/docs/notebook checks remain downstream. Validation is authorized HOST recovery after remote infrastructure exit 200, not remote compiled success.
