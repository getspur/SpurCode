# code_* / DuckDB Retrieval Validation Plan

Source spec: `docs/superpowers/specs/2026-11-15-duckdb-retrieval-parity.md`.
Design issue bd-3nph. User authorized the comparison; no additional plan approval
is required. Worker preference: codex / gpt-6.1-sol / xhigh.

## 1. Native oracle and shared corpus

Own `crates/spur-graph-duckdb/examples/retrieval_native.rs`,
`crates/spur-graph-duckdb/tests/fixtures/retrieval/cases.json`, and only necessary
example/test dependency edits in that standalone crate's Cargo.toml/Cargo.lock.
Build a persistent JSONL driver around actual GraphMcpModule dispatch in an
isolated frozen Git corpus. Prepare the shared fixture, queries and source-hash
manifest, capture full actual responses and graph provenance. Document CLI and
positive/negative smoke evidence. Do not implement a replacement native oracle.
Preserve production source and existing examples/tests. Commit and await review.

## 2. Independent SQL retrieval and comparator

Depends on approved task 1. Own
`crates/spur-graph-duckdb/examples/retrieval_parity.py` and
`crates/spur-graph-duckdb/tests/test_retrieval_parity.py` only.
Implement extension-derived typed materialization, SQL retrieval for the shared
request matrix, canonical semantic comparison and separate full API compatibility
results. Preserve raw native/candidate answers and mismatch details. No oracle
facts/resolutions may feed candidate tables. Add meaningful comparator fault
injection tests, a small true-engine smoke run, and persistent query endpoints
with equivalent timing boundaries. Negative parity is a valid experiment result.
Keep timed repeats separate for the parent notebook; no source optimizations.

## 3. Independent fairness and correctness review

Depends on approved task 2. No planned file writes. Review source and exact data
flow, immutable source/config hashes, native-handler authenticity, candidate
independence, canonicalization/API distinctions, full source-byte comparison,
cross-file/unresolved/count/ranking coverage and timing equivalence. Inspect
meaningful corruption tests and native/engine smoke evidence. Report concrete
findings or explicit none; no production edits or timed duplicate benchmark.

## Parent execution

Freeze the real-source cohort from b5e1246cc and preserve all user file hashes.
After review, integrate only verified task bytes. Run the synthetic and frozen
real-source comparisons in Rust notebook cells using native 2.0.0 and the approved
unchanged extension. Persist complete raw answers, discrepancies, hashes and
eligible latency observations. Capture exact coverage of each code_* operation;
do not infer full-project or full-history equivalence from bounded fixtures.
Commit the research report, notebook receipts and selected small evidence, then
record completion in beads. Any found production bugs are reported separately
rather than silently changing the reference while measuring it.
