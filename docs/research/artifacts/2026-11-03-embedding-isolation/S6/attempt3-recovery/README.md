# S6 attempt 3 recovery

S6 attempt 2 expired, with no approval or root POST. The complete committed checkpoint is `9179bb700ed146f76f1aac30732a94b9f6db1c4e`, protected on `spur/review-evidence/embedding-s6-attempt2`. Its full eight-commit authored history after dispatch base `38f6838c07770dd5f1631a3a363896dff94ef7e0` is recorded in `checkpoint.json`. Root made no production changes.

The partial adaptation removes analyst's direct native dependency and changes cache types to the graph helper handle. It remains an unapproved intermediate state: strong global handles, detached jobs and descriptor-only warm-up remain. Four captured normal/build dependency trees are evidence of package removal, not lifecycle or feature acceptance.

Root read the ownership risk and inspected both call paths. Core constructs `AnalystMcpModule` inside `handle_analyst_tool` per request; the worker server calls the public analyst handler directly. A runtime stored only in the temporary module would lose reuse and miss worker calls. The recovery therefore explicitly authorizes the necessary server/worker/CLI owner seams as part of the already-approved lifecycle goal. Implementation remains ordered: owned runtime API, host lifetime integration, then verification.

The expired checkout was removed during review. Its committed code is preserved. Two uncommitted evidence paths were observed before cleanup but could not be recovered; their names are recorded. There is no completed result for the cold analyst build, and no related build process remains active. Do not infer GREEN or interrupt another build to reproduce the loss.

The same pinned runtime archive is present outside Cargo build output at `/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz`, SHA-256 `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`. Its contained dylib checksum remains `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`. Do not use the obsolete archive path under an old Cargo build hash.

The original S6 PRE remains archived in `../root-pre/` (four OPEN gates, four SAT and twelve expected UNSAT). The supplemental ownership PRE has one OPEN Jev interpretation, one SAT and six expected UNSAT. All seven new receipts are persisted/reloaded/pinned; `ownership-pre.json.gz` retains exact inputs, provenance, current catalog and requests/results. This is a prospective finite contract, not a runtime ownership or native memory-safety proof.

| Case | Status | Receipt |
|---|---|---|
| ownership_contract | sat | sol_fbc2643b84004e4a |
| per_request_owner | unsat | sol_47e1d83f1f184b8e |
| worker_implicit_global | unsat | sol_5b80c9a14ae9416b |
| socket_without_owner | unsat | sol_e847801fc25f46c9 |
| global_strong_handle | unsat | sol_d2d7d1fc45ab4edc |
| detached_shutdown | unsat | sol_eeed5a41d5434395 |
| close_before_join | unsat | sol_4e47b63d551645cd |

Replay the full recorded authored chain into the new clean worker base, then import only this recovery directory from root's recovery commit. The new source hashes bind the partial checkpoint; the older S5 source hashes are historical and should not be used to undo S6's current source changes. Run `verify.py` before implementing.

Use short-yield commands and regular progress reports during cold compilation. Commit each test/fix/evidence checkpoint and log updates before returning or approaching a lease boundary. Runtime/source changes still require test-first RED, real GREEN and root POST. Parent S6 remains the review gate; packaging and final application/notebook acceptance remain downstream.
