# Notebook cell symbols for the Spur Graph DuckDB extension

Date: 2026-11-17. User approved implementation with TDD after executed PoC bd-26sp.
Design evidence: docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/README.md and executed spur-notebook/2026-11-17-spur-graph-notebook-rust-poc.ipynb.
Open bugs: bd-3ct2 (notebook symbols), bd-3ofb (Rust directives).

## Goal and scope

Support notebook code exploration and review with original-source, cell-aware symbols and changes through the native DuckDB 2.0 extension and its headless consumer. Keep ordinary-file SQL schemas/behavior and the existing graph stable identities compatible. This implements the data contract; no frontend app rewrite, cross-cell execution analysis, deployment or unsigned-package trust upgrade.

The PoC recovered 46/24 symbols and 17/11 functions, 55 native matcher deltas, 10 structural cell events, 70 symbol refs, 55 delta refs and 28 independently verified function anchors. Its 30 concrete checks are acceptance evidence, not universal parser correctness. Preserve original retained binary 28ddce83; build a separate new candidate.

## Shared normalization and Rust preparation

Create a public normalized notebook model in spur-graph::extract::notebook_model, consumed by both existing graph extraction and new symbol extraction. Preserve existing explicit-ID trimming, BLAKE3 source-derived anonymous IDs/occurrence suffixes, cell identity path <notebook>#cell:<id>, and core language-token precedence/aliases. Preserve all cells, decoded original source, source SHA256, metadata, ordinal/type, identity quality and diagnostics.

Malformed JSON, missing/invalid cells/source/type shapes and duplicate effective IDs must not silently become valid empty extraction. Unsupported language keeps a cell inventory row with diagnostics. Anonymous identities explicitly report source-derived limitations. Preserve root and cell metadata; do not interpret saved profile names as verified runtime compatibility. Python magics are not implemented: recognize a leading IPython magic/shell prefix (%/%%/!) as unsupported kernel syntax rather than claiming complete plain-Python extraction; retain original bytes and metadata.

Rust preparation is notebook-context-only, shared by graph and symbol routes. Support leading single-line :dep headers by replacing their non-CR/LF bytes with ASCII spaces. Preserve every UTF8 byte position and CRLF boundary. Retain original source/hash, analysis hash, directive text and version evcxr-leading-dep-mask-v1, with explicit identity coordinate mapping. Never execute directives or read :load targets. Unsupported commands/multiline/trailing forms must be explicit partial/unsupported coverage rather than successful empty code extraction. Avoid rewriting directive-looking content inside Rust strings/comments: use a defensible lexical boundary; conservative diagnostics are preferable to unsound transformation. Keep ordinary .rs extraction unchanged.

Graph extraction must reuse normalized IDs/languages/preparation; known :dep cells recover functions. Legacy GraphFacts remains schema-compatible. Surface notebook coverage diagnostics through the new typed notebook result; document that legacy graph status alone is not complete notebook coverage.

## Cell-aware core API

Create public spur_graph::extract::notebook_symbols functions extract_notebook_symbols(path, bytes) and compare_notebook_sources(path, old_path, change_kind, left_source, right_source), with strongly typed document/cell/symbol/source-reference/change results. Exact Rust parameter wrapper types may follow existing crate conventions, but export these entry points and report the final signatures to downstream tasks. Public result content is fixed by this spec.

Each cell result contains ID/identity quality, ordinal/type, resolved language, original decoded source, source hash, analysis hash/preparation version/directives, metadata JSON, status/diagnostics and symbols. Each symbol retains native semantic fields and original decoded-cell byte/line ranges. Canonical IDs must include notebook and cell context and must not expose fabricated .py/.rs transport paths. For arbitrary accepted IDs, use an unambiguous structured/escaped identity preimage: raw path#cell:id concatenation can collide across notebooks. Preserve original typed path/id fields and existing legacy graph stable keys; do not mistake legacy graph-key compatibility for new public-ID safety.

Each SourceRef carries notebook path/hash, cell ID/source hash/language, coordinate_space=decoded_code_cell_utf8, and byte/line ranges. Snapshot/repository labels remain caller assertions: the consumer attaches its independently validated fixture identity; native source hashes are computed, never trusted caller labels.

Compare paired cells first. Use an explicit-language inner ordinary matcher with canonical notebook/cell identity paths; retain ordinary wrapper behavior. Never concatenate cells into one identity map. Same-cell duplicate matching keys must produce explicit ambiguous/partial diagnostics, not silent HashMap loss. Keep original-source before/after references on deltas, including renames; do not derive references by guessing a different source after matching.

Report cell add/delete/move/type/language/source/metadata changes separately, and root metadata changes separately. Imports-only/empty cells remain visible. Output-only changes do not manufacture symbol edits in the tested contract. Language changes use conservative removal/addition; anonymous edits use conservative source-derived add/delete and identity-limit diagnostics. Dependency-only Rust changes remain configuration events even when functions are unchanged. Paired same-cell/same-language/same-symbol-identity with byte-for-byte identical original symbol slices must not yield Modified solely because an offset moved. Test variable-length dependency versions and header insertion/removal; keep ordinary-file matcher semantics unchanged. Retain full before symbols keyed by pre-match identity before rename detection, which otherwise preserves only the previous SnapshotKey. These do not imply execution equivalence.

Support M/A/D and ordinary supported rename semantics where safe; unhandled kinds must return explicit unsupported/partial diagnostics, never a clean empty comparison. Null/absent sides for A/D are absence, not malformed empty notebooks. Invalid required side inputs are typed invalid-input results. Infrastructure/internal errors propagate as errors, not row-level coverage diagnostics.

## Additive DuckDB API

Implement:
- spur_graph_notebook_symbols(path VARCHAR, source VARCHAR)
- spur_graph_notebook_changes(path VARCHAR, old_path VARCHAR, change_kind VARCHAR, left_source VARCHAR, right_source VARCHAR)

Return typed nested STRUCT/LIST results, not JSON blobs containing all facts. Metadata may use metadata_json VARCHAR for opaque notebook facets; symbols/refs/statuses/diagnostics/changes remain typed. Both results include contract_version=1 and explicit status/coverage diagnostics. Cells preserve original source and typed symbol lists; changes preserve before/after SourceRefs, cell events and document events. Scalar registration must use existing DuckDB2 bindings and documentation conventions. Declare the exact DESCRIBE-tested schema in release-contract.md.

Null required path/source inputs to symbols yield NULL. Changes: NULL path or kind yields NULL; A permits absent left_source, D permits absent right_source; M requires both sides. old_path may default to path except a rename requiring the old path. Kind/input failures must be explicit and documented. Check large multi-row variable-size lists/nested children and NULL rows.

Existing spur_graph_symbols and spur_graph_symbol_changes nested schemas stay unchanged. For .ipynb, reject with actionable instructions to the new API rather than misleading silent empty output or cell flattening. Make the existing consumer route notebooks to new APIs before enabling this guard; plan integration order ensures the final release is coherent. Existing ordinary-file functions retain prior behavior.

Publish per-API notebook capability/discovery documentation without silently extending the exact spur_graph_info v1 STRUCT. Native duckdb_functions() must discover both new functions. Source provenance fingerprint must include new source files/preparation policy; cache invalidation follows the changed extractor identity.

## Consumer, publication and documentation

Extend app_consumer to route notebook sources and pairs to the new typed APIs. Preserve parent coverage, cell inventory/symbols, original-source refs, structural/config changes and diagnostics through typed cache/Parquet export and immutable publication/readback. Attach caller-frozen snapshot identity only after source-hash validation. Existing ordinary-file tables/results remain readable; version new consumer schemas explicitly and assess old-result compatibility. Publication failure guarantees and historical runs remain valid.

Document runnable SQL, exact schemas, partial/anonymous/unsupported semantics, coordinate frames and Rust preparation bounds. Do not claim complete Evcxr syntax or runtime dependency resolution.

## Verification gates

Every production task must commit meaningful failing regression tests before the fix/feature commit, retain exact RED/GREEN commands/exits, and use scripts/spur-cargo for Rust builds/tests. Remote red is real; no local rerun to conceal failures. Root verified an infrastructure-only exception: both routes returned200 before compilation, with the shared builder EC2healthy but SSMConnectionLost. The wrapper-supported host fallback is authorized in that case, explicitly recorded as host execution; it is never a retry of a failed remote compile/assertion. Preserve remote error receipts. Local fmt only. No invented quotas/timeouts, no broad unrelated refactor.

Reuse catalog-first uniqueness and SourceRef contract verification; store exact requests/responses locally if persistence quota remains full. Tests and finite solver results have distinct claims.

Required concrete regressions: mixed Python/JS/Rust metadata; source string/list; UTF8/CRLF original ranges; valid empty/import-only cells; malformed/NULL/unsupported inputs; duplicate explicit/anonymous IDs; duplicate helper across cells and ambiguous identities within a cell; same-document no-op; cell add/delete/reorder/type/language/output-only/profile/root metadata; Rust :dep execution-compatible extraction, unsupported commands and directive-only changes; old ordinary-file schema/regression suite; nested vector batches; typed export/readback and publication failures.

Build and package a new exact native candidate with verified source/artifact identities. Run the existing release driver, focused remote Rust tests and complete native SQL suite with no hidden skips. Replay the frozen real inputs with explicit assertions for symbol names, cell IDs/hashes/ranges, 17/11 function anchors, no-op and safe non-empty real changes. Exact 55 delta count is a PoC matcher observation, not a requirement to preserve a bug; explain any intentional difference with source-grounded evidence. Re-run in notebook DuckDB2 only after verifying runtime and candidate ABI/hash. Preserve all pre-existing user edits and old evidence.
