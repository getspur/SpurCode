# Identical-input code_* / DuckDB retrieval validation

User requests an apples-to-apples comparison of code_* results with retrieval
through the spur_graph DuckDB extension. Design issue: bd-3nph. This is validation
work, not authorization to replace production retrieval or upgrade its runtime.

## Reference and candidate

The reference must invoke the real `spur_graph::mcp::GraphMcpModule` handlers via
`with_worktree_root_for_request`, with no rewritten native search/edge oracle.
Use an isolated, immutable Git corpus and a persistent JSONL process. The corpus
and query specification are shared inputs; persist full raw native responses.

The candidate must independently call `spur_graph_file_facts` in the pinned native
DuckDB 2.0.0 runtime, materialize typed facts, and query those facts with SQL.
It may share source bytes, paths and request definitions with the reference. It
must not copy reference nodes, edges, resolutions or answer rows into candidate
tables. The existing native extractor artifact is unchanged. No new global
resolver, production adapter, index policy or runtime migration is in this task.

Primary corpus is a small complete Rust repository containing same-file calls,
cross-file imports/calls, duplicate names in distinct scopes, structs/methods,
macros, unresolved external calls and Unicode/CRLF source. Add positive, negative
and ambiguity cases with known fixtures. A second corpus uses a deterministic
frozen cohort of real source files from SpurCode b5e1246cc, retaining original
logical paths and byte hashes. It is a bounded standalone source cohort, not the
whole original repository. Both pipelines receive the entire same corpus in
each comparison. Capture implementation, input, native graph and extension hashes.

## Request matrix and equality

Cover `code_symbol_search` (exact/prefix/substring; path/kind filters and limit),
`code_read_symbol` (exact source, ranges/context), `code_callers` and `code_callees`
(resolved-only and unresolved-enabled). Include stable named selectors and
ambiguity/empty-result cases. Native IDs may be captured for API compatibility,
but candidate answer construction must not depend on oracle answer values.

Keep raw responses intact. Semantic comparison may map engine-local identities
to independently derived `(logical_path, qualified_name/kind, source ranges)`
keys, with the exact mapping recorded. Preserve nullability, duplicates, edge
kinds/confidence/evidence, counts, filters, ranking/ties and truncation semantics.
Compare multisets where order is not contractual and ordered lists where it is.
Source text is an exact byte comparison against the frozen source, including
Unicode, CRLF and context boundaries. Report native stable-ID/URI and metadata
compatibility separately from semantic equality; normalization cannot establish
full API compatibility. No blanket removal of mismatching semantic fields.

Unsupported candidate behavior, one-file versus repository resolution differences,
and reference anomalies are explicit results. They must not become empty answers,
silent query exclusions or successful parity assertions. This experiment can
complete with measured discrepancies; it is not required to make the extension
a drop-in replacement. Clearly identify which queries are directly comparable.

## Timing

Correctness precedes timing. Construction, extraction, database materialization,
first load and warm lookup are separate costs. Use persistent local processes
with equivalent request/result serialization boundaries for native and SQL paths;
never label a local SQL query versus remote MCP round-trip as engine latency.
Use interleaved paired repetitions on immutable data, retain raw samples and host
load, and report p50/p95 only with sample counts and method. Any chosen repetitions
are experimental protocol, not a production quota or claimed confidence bound.
Report speed ratios only for cases with matched semantic work, with API coverage
limitations visible. If equality fails, report the failure without a speed claim.

## Verification and scope

Verify the native oracle calls actual production handlers, positive fixtures have
nonempty results, negative cases are meaningful, and candidate SQL uses extension
facts only. Comparator tests must detect deliberately missing/extra rows, changed
ranges/source bytes, duplicate loss and unresolved-edge omission. Keep all prior
extension tests unchanged. Build Rust with scripts/spur-cargo; native macOS oracle
needs an explicit host-native build, independently of any remote check outcome.

Changes are confined to research examples/tests in crates/spur-graph-duckdb,
minimal example-only Cargo dependencies if needed, and parent-owned docs/artifacts
and notebook cells. Preserve user edits and live project registration/indexes.
Parent runs the comparison through Rust notebook cells after independent review,
then commits the exact match/mismatch report. Full temporal ingest, semantic/vector
search, production cutover and broad performance optimization are outside scope.
