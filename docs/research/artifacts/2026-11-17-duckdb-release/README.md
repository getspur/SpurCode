# Spur Graph DuckDB release acceptance — 2026-11-17

The approved extension-side release gates are implemented with separate test and implementation commits. This is a local, unsigned release candidate for **DuckDB exactly 2.0.0**, `C_STRUCT_UNSTABLE`, macOS ARM64. The extension package version is **0.1.0**, with facts schema **1**.

Start with [root-final.json](root-final.json), the [final independent-review receipt](R7/worker/verification.json), and the [complete release-driver receipt](R7/full-release/receipt.json). The runtime contract and runnable quickstarts are in [release-contract.md](../../../../crates/spur-graph-duckdb/docs/release-contract.md).

| Check | Observed result |
| --- | --- |
| Fresh native release suite | 206 tests, zero failures, errors or skips |
| Remote Rust library tests | 2 passed |
| Native source identity | 321 inputs, identical before/after the suite and in the final integration checkout |
| Current macro controls/native facts parity | Passed against the current package |
| Preserved corpus | Read-only sample parity and controls; published files unchanged |
| Copied package | Checksums, metadata and fresh native LOAD verified after relocation |
| Headless Explore/Review | Typed output, diagnostics, full-key joins and conservative change pairing verified |
| Notebook consumption | Saved SQL cells on DuckDB 1.5.5 read typed Parquet produced by native 2.0.0 |

Current package SHA-256: `28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122`.

Current sidecar SHA-256: `db50a83c8c652d468b560deb25769a8826250d34659198e90213d6be415cf6f8`.

Extractor identity: `source-sha256-v1:ea849cf9a54b442671ac3e2b4972291c24a64c03457bcffcee4e9a50ef22de35`.

The build HEAD is `66663c00a4b4b7c898b1d1f5b66aecf99fc348f5`. The reviewed documentation commit is `09be38b1f6155b0f090d3e19915add39182d380a`. The final branch preserves original test/fix commit history; its native inputs match the build exactly. The root plan continuation record is its only tracked-tree difference from the Plan Engine integration branch.

Acceptance covers the committed source built in an isolated clean checkout. The five pre-existing user edits remain uncommitted and unchanged. Two are ACP source inputs, so rebuilding directly from the dirty main checkout produces a different extractor identity; those edits were deliberately excluded from this release candidate.

The 17 MB binary is retained locally in `native-osx-arm64/spur_graph.duckdb_extension`, beside its bound JSON sidecar. The binary is intentionally excluded from Git. Follow the crate README to reproduce packaging; copy both dereferenced files and run `verify_package.py` in the target native environment.

`R1` through `R6` retain meaningful RED/GREEN evidence and independent-review corrections. `R5/attempt-1` and `R6/attempt-1` are superseded, withheld attempts, not final passes. The baseline records an older known-corrected binary and is not current release evidence. Original receipts preserve exact temporary command paths; the small receipts, raw logs, source manifests and acceptance scripts are copied here without rewriting their historical identities.

`R6/approved-demo` is the small immutable synthetic dataset used by the saved notebook. It was produced with approved predecessor package `2968d20ef8d4f07df100e50f6aaccb0b2bece7df048c1a92884fc12c90e2ae89`, which has the same extractor identity. Its ten Parquet tables and manifests remain bound to that package; it is not relabelled as the later R7 binary. The final R7 full suite separately exercises the headless consumer against the current package.

The acceptance notebook is `getspur/spur-notebook/docs/research/2026-11-17-spur-graph-release-acceptance.ipynb`. Its saved SQL cells preserve unavailable results as NULL, show row diagnostics and keep identical source bytes at different logical paths distinct.

Hosted macOS ARM64 CI is configured but was not run in this session. Native Linux/Windows acceptance, production notebook UI/native host wiring, compiler-complete resolution, signed distribution and public release are outside these results. The old corpus was not re-extracted, and local-target fractions are not precision measurements. No public tag, upload, push or deployment was performed.
