# Graph candidate RED

Execution issue bd-1t5m, delegation eddc419e936b468d, dispatched base 095abc05d1e541e9b1e2f7e93eaf4e5fddbeb173. The existing public-path regression was present in the base.

Before edits, `PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-04-analyst-recovery/verify_recovery.py` exited 0 and verified checkpoint 38051998648e0aa6e7ff622b46c4ce654c208588, six source files and the PRE (1 pass / 4 expected rejects). It explicitly did not approve the whole prior C task.

`SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb scripts/spur-cargo test -p spur-analyst --test search context_candidates::graph_candidates_return_primary_and_neighbor_rows -- --exact --nocapture` exited 101. One test executed and failed in 0.60s: the public graph query failed to prepare because `stem` exists in FTS but was not loaded. 23 tests were filtered out. Full unmodified output is in red.log.gz (compressed after the RED commit to preserve DuckDB whitespace without diff-check noise). The build took 3m 06s. A separate optional process-list probe encountered sandbox EPERM and was not used as verification.
