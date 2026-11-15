# SpurCode Rust snapshot: DuckDB extension quality

The extension successfully extracts every tracked Rust file in the pinned SpurCode snapshot. Ordinary definitions, async/trait methods, selected test functions and real macro evidence passed the checks below. Repository-wide call binding remains incomplete, and macro evidence is not compiler expansion. A separate experiment also exposed a false-negative notebook admission rule in the research harness; it does not affect the Rust-only results.

The source revision is `61ba3d200439a541dce4c1617ca9277be7d438af`. The input inventory contains **4,404 tracked files / 124,274,585 bytes**, including **1,726 `.rs` files / 27,874,179 bytes**. All source comes from hash-verified immutable Git objects. Five pre-existing modified tracked files and untracked work remain untouched. This is one committed snapshot, not temporal history or the dirty working tree.

| Rust-only result | Observed |
|---|---:|
| Rust files extracted | 1,726 / 1,726 |
| Rust extraction exceptions | 0 |
| Non-file Rust symbols | 60,601 |
| Rust nodes/spans, including file roots | 62,327 |
| Recorded Rust call sites | 259,828 |
| Unique existing local call targets | 32,414 (12.48%) |
| Unresolved call targets | 227,414 (87.52%) |
| Rust macro invocation records | 51,634 |
| Rust macro scope records | 19,394 |
| Selected definition controls | 21/21 |
| Raw-string negative controls | 4/4 |
| Local macro source controls | 7/7 |

**Definitions and tests.** Before reading the extracted facts, 21 source controls were frozen across ordinary functions, a trait, a macro definition, inherent and trait methods, async methods, five ordinary test functions and two `#[tokio::test]` functions. Each was found once at the expected location, with its source signature inside the byte span. AST spans omit leading indentation, so only the expected signature's indentation is removed for that membership check; stored source bytes are never normalized. Four fake definitions inside a raw-string fixture correctly produce no nodes. These checks deliberately distinguish lexical source text from actual Rust definitions.

A separate line-anchored SQL marker scan finds 1,107 Rust files with candidate `#[test]`, `#[tokio::test]` or `#[async_std::test]` attributes and 9,653 candidate attribute lines. None of those files is file-only. These are lexical diagnostics, including possible raw-string/comment content, not counts of compiled tests or a recall score. The seven manually checked test functions provide the direct positive evidence.

Six Rust files produce only a file node. Source inspection shows three comment-only placeholders and three re-export facades, rather than the missing ordinary function bodies seen in the DuckDB C++ `TEST_CASE` counterexamples. This does not establish complete alias/re-export resolution.

**Macro evidence.** All 51,634 invocation records have matching file paths and Git source OIDs and valid byte/line bounds. Seven `labels!` invocations point to the local `macro_rules! labels` definition as `local_definition_candidate`, with matching definition source/ranges. The macro records preserve conditional/test context and syntax phase. They do not prove which configuration executes or what generated code a compiler emits.

| Macro resolution evidence kind | Sites |
|---|---:|
| `local_definition_candidate` | 149 |
| `explicit_path_candidate` | 2,485 |
| `unresolved` | 49,000 |

`conditional`, `test_attribute_candidate`, `unspecified`, `function_body_syntax`, `item_syntax` and `const_context_syntax` retain their native meanings. They are not converted into definite runtime test/production classifications. Additional source/owner/reference checks find no invalid source/ranges across 19,394 scopes, no missing nodes for 51,272 non-NULL invocation caller IDs or the scope IDs, and no missing source/OID/range references for 149 definition candidates. All seven selected definition candidates also match the separately frozen macro definition's actual byte/line ranges. A valid reference still does not prove compiler-level binding.

**Integrity and selected native retrieval checks.** All 4,098 primary stored sources match their hashes and exact UTF-8 re-encoding. Every one of 111,554 primary spans is within its source byte and line bounds, including all 62,327 Rust spans. The unchanged reviewed generic audit reports zero affected rows for its 21 structural counters, exact per-path conservation of all five raw/typed collections, no missing/duplicate local IDs or paths, and no repeated stable-key groups. The scope and macro tables are genuinely nonempty in this run; they are not merely mutation-test fixtures.

Three real `code_read_symbol` replies—`source_offset`, `register` and `BytesExtractor::extract_file_facts`—match extension identities, kinds and line ranges. Their returned whole-line source bytes and file OIDs independently match frozen Git input. The MCP graph is older (`b5e1246cc`, graph hash `c4290b43…ced0c8`) and its worktree/freshness fields are null; those fields are preserved. This is a selected source/identity cross-check, not full native API or repository graph parity.

Independent read-only review found no findings in the reviewed Rust evidence. It reproduced all 45 Rust and 43 generic SQL receipts, matched every staged source to pinned Git, and compared every column of all five typed collections against raw facts with `EXCEPT ALL` in both directions: zero differences. It also independently checked Rust span endpoint/line correspondence, macro attribute bytes, scope geometry, and retained controls. Its full audit and hashes are in `independent-review.json`. The empty-input admission limitation is disclosed separately below; the review does not certify general mixed-format admission or compiler completeness.

**Call graph limits.** Only 12.48% of recorded Rust calls have a unique target in the same extracted file. The remaining 87.52% keep unresolved labels and receiver/scope evidence. Many are standard-library or external calls; the percentage is not a measured domain-edge recall failure. It does show that file-local extraction cannot supply a complete workspace dependency graph. A non-NULL target and `syntax_exact` confidence also do not certify correct receiver/trait dispatch. Crate/import/receiver-aware binding needs separate validation.

**Initial mixed-format run and the admission finding.** The primary database contains 4,098 extracted files, 305 failures of the exact-path empty-source admission probe, and one unparsed symlink. It stores 111,554 nodes, 466,565 edges, 111,554 spans, 51,634 macro invocations and 19,394 scopes across its admitted formats. It is retained unchanged so every original receipt remains reproducible.

Of the 305 admission failures, **79 are notebooks falsely rejected because the probe sends empty text to a parser requiring valid notebook JSON**. A separate batch recheck on their actual immutable serialized source accepts all 79 and produces 5,202 additional nodes, with no source-extraction failures. Those diagnostic rows are not silently merged into the primary database or Rust counts. They do not establish notebook cell-level fidelity. The notebook recheck parses repository data; it does not open, edit or execute live notebooks. All 1,726 Rust files were already admitted, so the Rust-only quality conclusions are unaffected. A general-purpose ingestion workflow should validate capability using real input or distinguish unsupported paths from invalid empty probe content.

**Timing and implementation.** The primary mixed-format snapshot process took **112.86 seconds wall**, 105.28 user and 2.87 system seconds, with 1,119,698,944 bytes maximum RSS. Stage wall observations were 4.550 seconds for inventory/admission, 4.074 for batch Git reads/hash checks/staging, 100.162 for extension extraction and 3.550 for typed materialization. These are whole mixed-batch timings, not isolated Rust parsing timings. Notebook admission rechecks and later analysis are separate.

The run reused the DuckDB-project protocol: native DuckDB 2.0.0, approved extension SHA `e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`, two threads, 1GB managed memory and disk spill on the same host. The only ingestion-script adaptation is grouping `crates/<name>/…` by crate. Native Rust allocations can exceed DuckDB's managed-memory limit. Extraction is one set-shaped SQL operation over staged files, but the extension still parses each file; this does not turn Tree-sitter into a columnar AST parser.

The previous DuckDB snapshot took 131.84 seconds, with 6,166 admitted files and different languages, sizes, metadata and macros. Its 9.46% overall local call-target rate also has a different denominator. Those observations are context, not an apples-to-apples language speed or accuracy ranking. The useful contrast is explicit: this Rust run retains rich macro/test-context evidence and selected Rust test functions, while the C++ run exposed real `TEST_CASE` body misses. Neither provides a complete bound graph.

**SQL exploration.** The Rust notebook includes editable native2 SQL for coverage, definitions by kind, per-crate call binding, macro context/resolution, macro source samples and selected API symbols. For example:

```sql
SELECT n.kind, count(*) AS nodes
FROM nodes n JOIN inventory i ON i.path = n.logical_path
WHERE i.suffix = '.rs'
GROUP BY n.kind ORDER BY nodes DESC;

SELECT m.test_context, m.execution_phase,
       m.resolution.kind AS resolution_kind, count(*) AS sites
FROM macro_invocations m
JOIN inventory i ON i.path = m.logical_path
WHERE i.suffix = '.rs'
GROUP BY ALL ORDER BY sites DESC;
```

All numeric-ID joins retain `logical_path`; no unresolved labels are promoted into cross-file targets. The primary database is `docs/research/artifacts/2026-11-15-spurcode-project-quality/snapshot.duckdb`. The generic reader is the unchanged approved `94b50b05a9cffb3d0c53d1e11ae2199adc526f7f23e89028bf6c9c5f15b1fce3` implementation, whose 12 real-engine fault tests and the 82-test integrated suite passed in the preceding phase. No production code or runtime changed here, so those suites were not unnecessarily rerun; this phase validates real Rust data and new experiment scripts directly.

Notebook: `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`. Protocol/ingest cells begin `459e1a65`/`5c3ef637` (ingestion execution 143); Rust checks/generic audit `7de674c4` (execution 144); editable SQL `9bcb7f9d` (execution 145); admission recheck `179dd3d3` (execution 146); macro-reference diagnostics `814981d8`. Full queries, controls, raw results, native replies, source evidence and independent review are retained alongside the database. Small evidence is committed; the database and complete tracked inventory remain local with hashes and can be recreated from the pinned Git revision.

The extension is useful now for Rust symbol/source analysis and macro evidence inspection. Before replacing graph retrieval, address repository/type-aware binding and validate proc-macro/generated-code behavior. Before broad mixed-format ingestion, correct the empty-input admission rule. None of the selected checks measures exhaustive Rust semantic accuracy or full temporal-history ingestion.
