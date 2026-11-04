# Standalone native wrapper regressions

This standalone workspace preserves the former graph native and ORT/DuckDB coexistence tests without putting the test-only DuckDB or `owner-test-hooks` dependency into the graph or helper packages. Its runtime loader and build support are the historical graph implementations, retained here to exercise the original wrapper seam. The standalone library is named `spur_graph` solely to preserve the original test paths; test sources and ONNX fixtures are byte-identical to the former graph targets.

Run separately using the repository wrapper and the pinned runtime archive on macOS:

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true SPUR_ORT_ARCHIVE=/path/to/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo test --manifest-path crates/spur-graph/tests/wrapper-regressions/Cargo.toml -- --test-threads=1
```

These are real native tests; failures and process aborts must be reported, not ignored. The inherited shutdown failure signature and the relocation results are recorded under `docs/research/artifacts/2026-11-03-embedding-isolation/S5/worker-partial/`.
