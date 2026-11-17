# Spur Review: GitHub and three-extension POC

The experimental notebook proves the requested data-plane integration before app work: the saved @github-pat connection, DuckTails, Spur Graph and DuckPGQ work in the same DuckDB 2.0 session.

[Open notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-spur-review-extensions-poc.ipynb) · [Evidence](artifacts/2026-11-17-review-extensions-poc/) · [Verification](artifacts/2026-11-17-review-extensions-poc/verification.json) · [Fresh-run receipts](artifacts/2026-11-17-review-extensions-poc/fresh-run.json).

## Installed and executed

| Provider | Actual version | Installation |
|---|---|---|
| DuckDB | 2.0.0 | Managed notebook runtime |
| DuckTails | v1.9.0 | INSTALL from community |
| Spur Graph | 0.1.0 | INSTALL from the previously rebuilt local 2.0 artifact |
| DuckPGQ | 9c11bc192a | INSTALL from community |

All three requested extensions were installed into a task-owned extension directory and loaded together. DuckPGQ grammar was enabled with a separate `SET active_grammar_extensions = ['duckpgq']` statement after loading. Versions, install receipts and binary SHA256 identities are captured.

The notebook was restarted through its scoped visible runtime control. All **19 code cells** then passed in document order on kernel generation 2. The final verification passed **nine assertion groups**. These validate the POC and its evidence consistency, not upstream DuckDB correctness or performance.

## 1. Saved GitHub connection: proven

The existing github-pat connection was ready, attached through the notebook UI and restored after restart. Its managed datasource setup loads the REST bridge; credentials remain behind the saved connection.

Actual SQL queries returned:

- PR #28731, “Preserve aggregate domains for proven singleton groups”, author kevintruong.
- Issue #28615, “Performance regression after #28456: DISTINCT ON loses join-key statistics and changes join order”, reporter ted-wq-x.

A two-query SQL batch materialized these rows into ordinary DuckDB tables. A separate Python cell also materialized the saved connection's table function successfully.

This is a narrowly filtered inbox POC, not a complete repository inbox. The current saved connection has 21 callable tables/actions. Its search schema exposes title, state, author and timestamps; its list-pulls schema exposes branch names. It does not provide the complete immutable-revision, changed-file, review, thread and check schema needed for the future workspace.

A read-only GitHub connector query supplied the full PR revision metadata, with separate provenance. Target base and merge base were `629e395a854d72dcb1f414e1a0451c68ac0e9f1f`; head was `d29b26bb92ac940a0807be0f43cc385b31623839`. Git object reads always use these revisions, independent of local checkout state.

## 2. DuckTails plus Spur Graph: proven

DuckTails' base/head file diff returned five changed files. The experiment analyzed the two C++ implementation files at both revisions; the SQL regression and two configuration files remain explicitly outside symbol analysis.

All four native diagnostic extractions returned `ok`. Exact body comparisons produced six raw function-change candidates and five semantic seeds after collapsing the overlapping changed template/function representation:

- RewriteOrderedFirst
- GetValuePreservingColumn
- HasBaseColumnSource
- HasUniqueGroups
- ExtractAggregationStatsInternal

The POC deliberately avoids treating the known ordinary-symbol matcher as a reliable complete review diff. Raw native symbols, facts, source hashes and Git blob IDs are preserved. Production rename/duplicate matching and normalization still need hardening.

Native numeric node IDs restart inside each file result. The graph therefore uses scoped keys containing the comparison, file and stable key. Joining raw numeric IDs across files would create false relationships.

Across the two head-version files, the adapter yields 118 raw nodes. The explicit same-name/same-kind containment normalization records 24 aliases and displays 94 vertices. This is an experimental presentation policy, with raw data retained, not a general semantic identity guarantee.

## 3. Native property graph and limited blast radius: proven

Actual call facts contain:

| Scope | Resolved call sites | Unresolved call sites |
|---|---:|---:|
| first_last_any.cpp | 1 | 138 |
| relation_statistics_helper.cpp | 16 | 211 |
| Total | 17 | 349 |

The graph's call direction is caller to callee. DuckPGQ asks which callers reach a changed callee within one or two call edges. It returns **13 caller/changed-symbol pairs**, involving five distinct caller vertices. Distinct overload IDs and source lines are retained.

An independent recursive SQL query returned exactly the same pairs and minimum hop counts. This establishes traversal correctness for the bounded supplied graph.

The five changed-symbol rows find 2, 5, 3, 3 and 0 callers respectively for ExtractAggregationStatsInternal, GetValuePreservingColumn, HasBaseColumnSource, HasUniqueGroups and RewriteOrderedFirst.

The final zero is **not zero impact**. The extractor reports single-file resolution. Unresolved member/library/project calls, callbacks and cross-file metadata relationships cannot be supplied by DuckPGQ traversal. Repository-wide linking and clearer unresolved-target classification are the principal missing source-graph capabilities.

The export includes caller/target identities and hop counts. A production review UI should additionally retrieve explicit witness paths and precise edge-source evidence for inspection.

## 4. Broader propagation and architecture: partially grounded

A separate property graph reuses the prior source-inspected map, with every pinned source hash revalidated. It contains 20 source points and 14 structural relationships. It is labeled curated throughout and never merged into automatic native-call coverage.

DuckPGQ finds the ordered-rewrite propagation chain reaching:

| Source-inspected downstream point | Steps in curated graph |
|---|---:|
| Join relation manager | 2 |
| Cardinality estimator | 4 |
| Plan enumeration/cost | 5 |

These steps represent mixed source relationships such as callback dispatch and metadata transfer. They are not native call-graph hop counts or runtime execution proof.

Component aggregation works over explicit POC path mappings: one direct change in aggregate binding/rewrite and four in relational statistics. No approved architecture contract exists here, so alignment remains `not_evaluated`. A future alignment view needs versioned component/rule membership and base-versus-head dependency comparison.

## 5. History and stakeholder composition: proven within scope

DuckTails returned 65 base-pinned history rows and 158 blame lines. The fixed history lower bound is 2026-05-21, with a maximum of 40 commits per file. Blame covers explicit surrounding semantic regions rather than changed lines only.

Previously verified GitHub commit identities map:

- Laurens Kuiper to lnkuiper: 108/116 inspected statistics-region lines; 22 retained helper-file history commits.
- Denis Hirn to kryonix: 38/42 inspected aggregate-rewrite region lines; 7 retained aggregate-file history commits.

The joined people view keeps PR author, issue reporter and historical expertise candidate as distinct roles. Current reviewer assignment and formal ownership remain unverified. History is not a maintainer roster.

## 6. End-to-end SQL join and AI-ready input packet: proven foundation

A final five-row table joins the real GitHub PR with normalized changes, native DuckPGQ caller results and DuckTails expertise evidence. It preserves `partial_single_file` impact coverage and `not_evaluated_no_policy` architecture state.

The exported [review-evidence-packet.json](artifacts/2026-11-17-review-extensions-poc/review-evidence-packet.json) contains comparison identity, inbox provenance, changed sources, bounded impact rows, separate curated propagation, component mappings, historical people evidence and known gaps. Its packet hash includes captured context, while the source snapshot ID identifies the immutable comparison.

AI review is explicitly `not_run`; no findings are fabricated. The remaining AI work is provider execution, source/path retrieval, citation validation and human review decisions.

## Python/Arrow and operational findings

`to_arrow_table()` successfully exports component results. In this current kernel, `fetch_arrow_table()` also succeeds and emits a deprecation warning. The prior app transport errors therefore cannot be generalized as a missing method in every DuckDB 2.0 environment; the actual app seam requires its own environment-specific regression check.

The first two-resource SQL batch returned a failed status without diagnostics. An unchanged retry succeeded, and the fresh-kernel run succeeded. One notebook bridge mutation timed out and recovered using the same idempotency key. These observations are retained without claiming a root cause.

## Next grounded implementation work

1. Extend the saved GitHub collector with immutable PR revisions and detailed file/review/check resources.
2. Harden source change identities and implement revision-correct repository linking, callback/data-flow evidence and unresolved-target classification.
3. Preserve the native-versus-curated distinction and add inspectable witness paths/source citations.
4. Establish architecture mappings and an approved rule baseline.
5. Build the shared notebook frontend over these versioned tables, then connect AI review to the evidence packet.

The integration premise is proven. The largest remaining gap is source-graph completeness and evidence composition, rather than loading the three extensions.
