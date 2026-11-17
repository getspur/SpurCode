# U3 attempt 1 root review

Request changes for a legacy diagnostic compatibility regression before approval. Root independently ran all **7 new native notebook SQL tests** against the exact candidate 0ea46655 with DuckDB 2.0.0: exit 0. Source references, frozen 46/24 symbols, 17/11 functions, all 28 anchors, typed schemas, NULL/absence handling and large nested vector/Parquet cases pass.

The unchanged legacy malformed-notebook diagnostic test fails independently: exit 1 with four errors. Core now emits `parse .ipynb JSON: [...]` and `invalid .ipynb notebook inventory: [...]`; the old SQL classifier recognizes only the former exact text. Malformed input becomes InternalException and invalidates the DuckDB connection, causing subsequent queries to fail. The worker also reproduces this on its pre-U3 baseline, tying it to the earlier U1 integration change.

Independent review bd-1drm found no other Important/Critical issue in the adapter. The prepared classifier patch is a suitable narrow direction. Root expanded U3's explicit write scope to file_facts.rs via mutation 1cc24264-5159-4eab-95dc-e172a3f254a5 and resolved the scope signal. This compatibility work is already authorized by U3's acceptance requirement and needs no additional user approval.

Root verified 17 phase receipts, native package/sidecar hashes, retained historical package 28ddce83, final build/package/native/Rust/interface source hashes and original TDD ancestry. The existing eight-test provenance GREEN belongs to the earlier candidate 790a1922; all suites must be rerun on a fresh post-fix artifact before approval. Core, metadata and build.rs remain unchanged.

See review.json for exact commands, environment, outputs, scope decision and limitations. The first diagnostic invocation lacked the native oracle; it is recorded separately. The final reproducer supplies the oracle and contains no setup failure. Host validation follows authorized precompilation remote-infrastructure recovery. App consumer integration and final release/notebook acceptance remain downstream.
