# DuckDB file facts implementation plan

Parent: `bd-ns7b`. Spec: `../specs/2026-11-15-duckdb-file-facts.md`.

The plan engine owns isolated task branches and dependency order. Parent owns
architecture, review gates, integration and Rust-notebook evaluation.

1. **Core source seam.** Test a pure `BytesExtractor::extract_file_facts` against
   native one-file extraction, then add the minimal implementation. Own only
   `crates/spur-graph/src/extract/tree_sitter.rs` and new
   `crates/spur-graph/tests/extract_file_facts.rs`.
2. **SQL contract and oracle (depends on 1).** Add independent native file oracle
   example and test the intended typed SQL function. Build the current adapter
   and record a missing-function RED failure before adapter implementation.
   Own new `crates/spur-graph-duckdb/examples/reference_file_facts.rs` and
   `crates/spur-graph-duckdb/tests/test_file_facts.py`.
3. **Typed V2 adapter (depends on 2).** Add `src/file_facts.rs`, wire registration
   in `src/lib.rs`, adjust standalone dependency metadata only if needed. Reuse
   native extraction and worker parser state, reserve all nested children and
   pass new and existing real-engine SQL tests. Own adapter source/Cargo files;
   coordinate necessary test-contract corrections explicitly with parent.
4. **SQL data pipeline and docs (depends on 3).** Add executable staging/cache/
   UNNEST/Parquet example and API documentation. Validate outputs and incremental
   anti-join behavior against the built function. Own README and new
   `examples/file_facts.sql` (plus a focused SQL-workflow test if needed).
5. **Parent integration and measured demonstration.** Review each task's actual
   diff/audit/evidence, integrate approved branches, build a host-native frozen
   artifact from an isolated committed worktree and use notebook MCP Rust cells
   for the bounded source experiment. Preserve existing managed SQL engine and
   all previous history artifacts.
6. **Independent final review and record.** Verify exact oracle parity, return
   schema, memory/lifecycle, scope/cache identity, SQL workflow and experiment
   claims. Fix findings before final commit/approval. Record all outcomes in
   beads; do not claim production replacement of global resolution.

Worker routing: use narrow codex tasks. Claude-code transport previously failed
authentication in this session, so its broader refactor suitability does not
justify retrying unavailable authentication. Parent retains architectural
decisions; the explicit small ownership sets bound codex coordination.
