# Real DuckDB snapshot: extension ingestion and SQL quality

The extension successfully ingested its accepted portion of the real DuckDB project, and SQL can explore the resulting source facts quickly. The experiment also exposes an incomplete call graph and unsupported SQLLogic test containers. The results support source analysis over explicit facts; they do not justify treating the current output as a complete repository graph.

The snapshot is pinned to local DuckDB Git revision `b678afa97b529d5856731e115e31542a72d2d978`. Every one of **18,193 tracked entries** remains in the coverage denominator. Sources come from verified Git blobs through one persistent `git cat-file --batch` process, not the mutable checkout. This experiment leaves the DuckDB checkout, production retrieval and other temporal experiments untouched. It measures one snapshot, not history ingestion.

| Result | Observed |
|---|---:|
| Tracked inventory | 18,193 entries / 283,376,911 bytes |
| Extracted files | 6,166 |
| Rejected by exact-path admission | 12,025 |
| Symlinks retained but not parsed | 2 |
| Extraction exceptions among admitted sources | 0 |
| Nodes, including file roots | 187,413 |
| Non-file symbols | 181,247 |
| Edges, all relations | 593,690 |
| Source spans | 187,413 |
| Macro invocation / scope rows | 0 / 0 |
| Database size | 69,480,448 bytes |

Admission uses `TRY(spur_graph_file_facts(path,''))` for each exact tracked path, including special basenames. Rejected inputs stay in `inventory`; representative exact errors are retained per suffix. They are not all failed source parses: many are data/config files or unsupported containers. All 6,166 admitted blobs were valid UTF-8 without NUL bytes, so no admitted source was excluded by text checks. Actual extraction uses one SQL statement:

```sql
CREATE TABLE facts AS
SELECT logical_path,
       TRY(spur_graph_file_facts(logical_path, source)) AS value
FROM sources;

CREATE TABLE nodes AS
SELECT logical_path, item.*
FROM facts, UNNEST(value.nodes) AS u(item);
```

Edges, spans and both macro collections expand separately in the same way. A NULL extraction result would retain its file and an exact error retry; none occurred. SQL batches and schedules extraction, but the extension still parses individual source files with Tree-sitter. This is not evidence of a SIMD/columnar AST parser.

The cold snapshot run took **131.84 seconds wall**, 83.79 user and 6.11 system seconds, with 900,907,008 bytes maximum RSS. Stage wall times were 29.525 seconds for inventory/path admission, 12.514 for batch Git reads/hash checks/staging, 82.068 for extension SQL extraction, and 6.085 for typed materialization. The memory-pressured 24GB host used two DuckDB threads, a 1GB managed-memory limit and enabled disk spill. These are recorded experimental settings, not production quotas. Native parser allocations are not guaranteed to obey DuckDB's memory limit.

**Source coverage and symbol yield.** All tracked files of the following listed suffixes were admitted; file-only output is reported separately from extraction exceptions.

| Suffix | Extracted files | Non-file symbols | File-only results |
|---|---:|---:|---:|
| `.cpp` | 2,527 | 62,422 | 91 |
| `.hpp` | 1,903 | 68,027 | 27 |
| `.h` | 457 | 18,780 | 26 |
| `.cc` | 27 | 1,186 | 0 |
| `.c` | 68 | 1,575 | 3 |
| `.py` | 202 | 3,355 | 6 |
| `.sql` | 609 | 1,509 | 476 |

Across all admitted suffixes, **721 files produce only a file node**. A targeted query finds none of those `.cpp` file-only results under core `src/`. File-only is a diagnostic, not automatic evidence of a miss: declarations, include-only units, query-only SQL and metadata have different expected graph shapes. Rejected containers include **6,515 `.test`**, **863 `.test_slow`** and **1,279 `.benchmark`** files. The run therefore does not capture most SQLLogic test bodies as source facts. Zero rows in the macro invocation/scope collections also does not mean C++ has no macros; macro definition nodes are present.

**A confirmed test-body extraction gap.** Among 377 admitted C++ files under `test/`, a line-anchored SQL regex finds 340 files with `TEST_CASE(` markers. Of these, **82 yield only a file node**, containing 260 candidate marker lines. Those lexical counts are diagnostics, not a parser-derived test count or a recall estimate. Manual inspection confirms real `TEST_CASE` bodies in `test/api/test_relation_api.cpp`, `test/api/capi/v1/test_capi_data_chunk.cpp` and `test/api/capi/v1/test_capi_values.cpp`; their bodies have no extracted symbol nodes. This negative evidence is preserved separately from the 17 positive ordinary-definition controls. The largest file-only sample also includes generated UTF-8 and Brotli data tables, illustrating why a file-only counter needs source inspection.

**Call binding is the largest measured graph gap.** Of 247,049 recorded call sites, **23,376 have a unique existing target in the same file (9.46%)** and **223,673 remain unresolved (90.54%)**. In core, 10,130 of 145,183 calls have local targets (**6.98%**). These are file-local binding rates, not semantic precision/recall estimates. Unresolved labels and receiver text remain evidence; they are not promoted into cross-file target IDs.

| Area | Recorded calls | Unique local targets | Unresolved calls |
|---|---:|---:|---:|
| Core (`src/`) | 145,183 | 10,130 | 135,053 |
| Extensions | 22,910 | 1,868 | 21,042 |
| Tests | 26,902 | 1,882 | 25,020 |
| Third party | 31,959 | 7,184 | 24,775 |
| Tools | 10,820 | 728 | 10,092 |
| Benchmarks | 1,663 | 50 | 1,613 |
| Other | 7,612 | 1,534 | 6,078 |

The edge relation counts are 247,049 calls, 181,247 containment, 153,846 defines, 5,548 imports, 3,076 references, 2,475 extends, 350 constructs and 99 links. A call marked `syntax_exact` can describe an observed call expression without identifying its correct receiver type or target. Neither that confidence value nor a non-NULL target proves semantic correctness.

**Independent checks.** All 6,166 stored sources match their byte hashes and exact UTF-8 re-encoding. All 187,413 spans have valid byte and line bounds. Seventeen source controls were manually frozen before inspecting full extraction results: four `Connection` constructors, three `Query` overloads, two `Prepare` overloads, transaction methods, three `DuckDB` constructors, `Parser::ParseQuery`, and the `Connection` class. All 17 were found with matching source signatures inside their spans. Five inspected `context->Query`/`context->Prepare` calls were recorded without incorrectly binding them to the local `Connection` wrappers. Those calls remain unresolved; the check does not supply a global receiver resolver.

The exact revision was also indexed with `external_index` (job `2b588772-30ff-4e77-9e6f-bd8096199b18`, snapshot 10009, catalog generation 1519). Three `Connection::Query` overloads match external search IDs, kinds and line ranges. `Parser::ParseQuery` also matches external source bytes and byte/line ranges. These four selected definitions are a cross-check, not full native/SQL parity. The external service's build source/version is not established by its query responses, so its overall graph counts are not used as an apples-to-apples accuracy baseline.

The reusable read-only audit counts path-local ID health, orphans, spans, per-path raw/typed collection conservation, coverage and call resolution. Its 12 real-native2 tests passed, including injected orphan IDs that exist only in another file, duplicate/missing IDs, malformed byte spans, null facts, offsetting per-path losses, unsupported inventory, exact SQL receipt replay and database immutability. The integrated suite passed **82/82 tests**, without skips, in 64.568 seconds. On this snapshot its 21 structural counters are zero. Numeric-ID/path health counters and repeated stable-key group counts are also zero. These are structural checks, not a language-completeness certificate. Independent review found no critical/important issues and replayed all 43 SQL receipts. Its minor EOF-line-bound caveat is covered by the parent's independent check. The final reviewed-reader notebook run, execution 142, reproduced the structural counters and call rates. Reader/test implementation is committed as `cef30b875`.

**Explore the persisted snapshot with SQL.** The notebook contains editable SQL inside a Rust cell that runs native DuckDB 2.0.0. Schema discovery precedes queries. The seven exploration queries took 0.002–0.074 seconds each on the materialized snapshot; those are individual observations, not a latency benchmark.

```sql
-- Preserve full coverage denominators, including rejected inputs.
SELECT area, status, count(*) AS files, sum(bytes) AS tracked_bytes
FROM inventory GROUP BY area, status ORDER BY area, status;

-- Missing targets stay unresolved; IDs join with logical_path.
SELECT i.area, count(*) AS calls,
       count(*) FILTER (WHERE e.target_node_id IS NOT NULL) AS local_targets,
       count(*) FILTER (WHERE e.target_node_id IS NULL) AS unresolved
FROM edges e JOIN inventory i ON i.path = e.logical_path
WHERE e.relation = 'calls'
GROUP BY i.area ORDER BY calls DESC;

-- Distinguish overloads using identity and source location.
SELECT n.logical_path, n.node_id, n.stable_key, n.label,
       p.start_line, p.end_line
FROM nodes n JOIN spans p
  ON n.logical_path = p.logical_path AND n.source_span_id = p.span_id
WHERE n.logical_path = 'src/main/connection.cpp'
  AND n.label = 'Connection::Query'
ORDER BY p.start_line;
```

Data lives locally in `docs/research/artifacts/2026-11-15-duckdb-project-quality/snapshot.duckdb`. Use the pinned native2 Python environment; neither the managed notebook engine nor the application's DuckDB was upgraded. The new `examples/project_quality.py --database ... --out NEW.json` reader emits exact count/display SQL and parameters, full row counts and explicit truncation flags. It opens a read-only transaction and does not load the extension or perform extraction. The ingestion receipt authenticates the approved extension separately.

Notebook: `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`, cells `c1d8aac1-…` (protocol), `5aa27ea2-…` (ingest, execution 137), `17adc4b1-…` (semantic checks, execution 138), `5a6f3f1b-…` (editable SQL, execution 139), `aa087f28-…` / `85d338b8-…` (file-only source inspection, executions 140/141), and `e9a97443-…` (reviewed audit and macro diagnostics, execution 142). Research scripts, raw SQL/results, controls, runtime/extension hashes, review/test receipts and the evidence manifest are retained next to the database. Small evidence is committed; the database, spill files and full tracked inventory remain local with hashes. `freeze_snapshot.py` regenerates and verifies the exact inventory from the pinned Git objects.

The approved extension SHA-256 is `e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`; native DuckDB 2.0.0 engine SHA-256 is `1e0c41d7d52834b43bf66c1e8b25d6aeee7911609515c9563383b3ffb28919b3`. Parent receipts preserve the same extension/runtime identities as the previous retrieval experiment.

The next useful improvements are C++ test-macro body extraction, SQLLogic/benchmark container extraction, repository binding and typed receiver resolution while preserving overload identity, and explicit parser error/recovery diagnostics. Passing a bounds check or 17 curated definitions does not measure exhaustive recall, templates/macros completeness, or repository-wide target accuracy. The current evidence supports using SQL for source-fact analysis while retaining the known graph limitations.
