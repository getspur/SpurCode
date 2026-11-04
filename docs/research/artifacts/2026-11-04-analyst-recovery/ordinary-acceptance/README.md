# Ordinary analyst acceptance — bd-2jqb

This is a verification-only result for plan d155a8d0-540b-4a8c-873a-a31e30a4b663, delegation dbe593caaa1f49c9. It is ready for root review; no approval or POST solver run is asserted. The approved prerequisite is bd-1t5m. Source tracking bd-2rzo is not the execution issue. The source checkout started at 8d3fb7c9462529150701b459efe163c9e23473c8, tree 9ba0f4f106cbc3cfd84168790d14feb844d4844a, equivalent to authored 36d466ac88e872c6d42f12c657c97dd4a4deab57. `ordinary-production.diff` is empty. `approved-graph-source.diff` records the predecessor's one-file diff from 095abc05d1e541e9b1e2f7e93eaf4e5fddbeb173.

The sole outside-directory import was `root-graph-review/` from immutable d063722291a72c7e8239b0ee036736b7cc939a4c, no source replay. Its verifier passed before any acceptance checks: 24 tests, 12 receipts, 36 predicates, 16 source files. Its source fingerprint is f1cbef28ee20baca309a7617d01975fcae565a8e39169f4345688c7ea1e1815d, SHA-256 of sorted compact JSON of `source_hashes`. All seven supplied PRE receipts were reloaded through get_solve_result: sol_60ee99a3908c4b01 SAT and sol_a2d289821f9b4d88, sol_1948a751e087448a, sol_e3bb199667fc4a0a, sol_ed2bf69907844c53, sol_0e2aee4d9fdc4dfc, sol_2464076473c04930 UNSAT. They specify requirements, not prior ordinary execution.

## Executed checks

Both suites used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz`, with authorized process and Unix socket access, one Cargo writer. Raw combined stdout/stderr and exit files are retained unchanged.

* `scripts/spur-cargo test -p spur-analyst -- --test-threads=1 --skip knowledge_context_pack_reports_confidence_from_real_hybrid_fusion`: exit 0; 197 passed, 5 ignored, 1 filtered. `tests/search.rs` executed all 24 with 0 failed/ignored/filtered; the public `graph_candidates_return_primary_and_neighbor_rows` passed. Analyst build 54.34s; search 19.20s. Library 107 passed, 3 ignored, 1 filtered; integration owned-real target 2 ignored.
* `scripts/spur-cargo test -p spur-embedding --features test-support -- --test-threads=1`: exit 0; 40 passed, no failures, ignores, or filters. The `owner_can_join_cancelled_descriptor_process` asserts a positive PID, a successful live `/bin/kill -0`, and after cleanup status 1 plus `No such process`; `descriptor_pid_file_requires_a_positive_numeric_pid` separately rejects empty, zero, negative and non-numeric input. Both passed; logs contain no illegal PID diagnostic.
* For each of spur-graph, spur-analyst, spur-core, spur-cli, `scripts/spur-cargo tree -p <package> -e normal,build` and `-e normal,build,features` completed with exit 0. The stored eight trees are parsed as complete package tokens; exact fastembed, ort, ort-sys and any `test-support` feature are absent. Parser explicitly handles build-dependency headings and checks every line. `gix-transport` is not an `ort` token.
* `SPUR_REMOTE=0 scripts/spur-cargo fmt --all -- --check`: exit 0. `git diff --check` on the tracked working tree passed. Raw log whitespace is retained. Staged `git diff --check` exits 2 solely for final blank lines in analyst.log and embedding.log and context whitespace/new final blank line in the verbatim approved-graph-source.diff; these are preserved evidence, not normalized.
* Focused core host filters were not repeated: this final graph fix is confined to the analyst graph connection initialization; predecessor independently verified the full search target, and no new core-impact evidence appeared. This is not a full core suite.

Environment: macOS Darwin 25.6.0 arm64, rustc 1.94.1 (e408947bf 2026-03-25), cargo 1.94.1 (29ea6fb6a 2026-03-24). `extension-hashes.txt` records files actually present in the isolated extension tree. The pinned ORT archive and its dylib member matched the requested SHA-256 values in `runtime-hashes.txt`; the ordinary native-free product closure does not establish real runtime/model execution. No neural model was loaded by this task and no model identity or neural measurement is claimed. Cached models and global caches were not modified. Windows and real final acceptance remain unverified.

## Deferred handoff to bd-eha0

Explicit one-case filter:
* `pack::service::tests::knowledge_context_pack_reports_confidence_from_real_hybrid_fusion` — real hybrid fusion confidence, `crates/spur-analyst/tests/pack/service_unit.rs`.

Five explicit analyst ignores, all owned by real final acceptance bd-eha0:
* `embedding::sidecar_service::real_acceptance::warm_query_and_v1_multi_text_share_owned_runtime` — explicitly configured real helper/model cache.
* `pack::service::tests::real_owned_helper_returns_indexed_context_pack` — real owned helper and DuckDB extensions.
* `pack::service::tests::real_cli_host_serves_indexed_context_pack` — real CLI host, helper, and DuckDB extensions.
* `graph_real_batch_reuses_and_closes` — real graph batch owner and model cache, `tests/real_owned_acceptance.rs`.
* `owned_real_query_reuses_and_closes` — real query owner and model cache, `tests/real_owned_acceptance.rs`.

The separate CLI `second_concurrent_session_attach_emits_rejected` is marked ignored in `crates/spur-cli/tests/session_attach_collision.rs` because its headless TUI event-dump and session-metadata fixture do not exist. It is not an analyst-suite result or a neural acceptance case; its ownership lies with the CLI/TUI fixture work, not bd-eha0.

## Root POST inputs

`verify_ordinary.py` independently validates the source binding, hashed raw files, suite result counts, graph case, PID evidence, trees and formatting exit, and exercises local negative controls. Run with `PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-04-analyst-recovery/ordinary-acceptance/verify_ordinary.py`. Controls are local verification, not executed solver receipts. Root must re-encode actual evidence and execute fresh POST. The source-bound rows corresponding to the supplied PRE contract are:

| check | scope | status | coverage | source |
|---|---|---|---|---|
| analyst | ordinary_analyst | pass | graph_included_real_deferred | f1cbef28ee20baca309a7617d01975fcae565a8e39169f4345688c7ea1e1815d |
| client | embedding_test_support | pass | valid_positive_pid_and_absence | same |
| graph_case | public_graph_query | pass | executed_unfiltered | same |
| dependencies | four_product_closures | pass | native_and_test_hooks_absent | same |

Evidence: analyst and graph rows derive from `analyst.log` and `analyst.exit`; client from `embedding.log`, `embedding.exit`, and the source assertions in `model_discovery.rs`; dependency row from the eight captured trees.

The fresh POST must reject missing graph coverage, a nonzero suite exit, an invalid PID probe, native leakage, and test-hook leakage; it must also retain the source-binding and row-cardinality constraints in the original PRE. These controls have not been submitted to Jev by this worker. Immutable graph PRE/POST and ordinary PRE packets remain unchanged.
