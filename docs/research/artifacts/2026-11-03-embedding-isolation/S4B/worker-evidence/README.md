# S4B worker evidence — bd-1ydx

Status: scoped GREEN; root Jev POST and review pending. This is not approval or issue closure. Source candidate `81a9f4d36299c732f786c6da474204302d39bbaa` (tree `f617e9c77ae2557687cc592f95c92e8a4008b7f4`). Test commit `67755a639`, implementation `1024ba6a4`, vector export `81a9f4d36`. `worker.patch` is the unified worker diff from approved S4A candidate `ae0da00331472c6a7b715197dd8c87d14bcaa719` through the candidate.

Rationale: Bind the actual bytes consumed by one reusable FastEmbed model to the descriptor and drop its native session after joining work and before checked environment release.

All cargo commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true SPUR_ORT_ARCHIVE=/private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz` and `scripts/spur-cargo`.

## RED

Before implementation, `test -p spur-embedding-worker --test embedding_models product_nomic_matches_registered_reference_and_closes -- --exact --nocapture` exited 101 after 30.92s: the registered reference completed and the first client embed failed with `Remote { code: Unavailable, message: "product model loading is unavailable" }`. This is the actual observed terminal excerpt, not a fabricated passing result.

## Scoped checks

- `test -p spur-embedding-worker --test embedding_models product_nomic_matches_registered_reference_and_closes -- --exact --nocapture`, with `SPUR_S4B_VECTOR_EXPORT` set to this directory: exit 0, 1 passed; `product-test.log` records client generations, raw waits, and row measurements. Files `vectors-cycle-{0,1}.json` contain the actual first, second and reference vectors, no input text.
- `test -p spur-embedding-worker --features test-support --test native_lifecycle --test embedding_metadata --bin spur-embedding-worker -- --test-threads=1 --nocapture`: exit 0; 2 unit, 6 metadata and 7 native tests passed. `worker-tests.log` includes actual model load/reuse and close events, raw wait 0, and EOF release order. Real native probes are not mocks.
- `test -p spur-embedding --features test-support -- --test-threads=1`: sandbox run failed 2 close-review cases because spawning `ps` returned EPERM (`client-sandbox-failure.log`); the same complete suite was rerun with process inspection enabled and passed 32 tests (`client-tests.log`). This is recorded separately rather than concealing the initial failure.
- `test -p spur-embedding-worker --test embedding_models descriptor_returns_actual_nomic_identity_with_bounded_execution -- --exact` and the equivalent `hello_rejects_unverified_artifact_identity`: each exit 0, 1 passed (`s4b-descriptor.log`, `s4b-forged.log`).
- `check -p spur-embedding -p spur-embedding-worker --all-targets --features spur-embedding-worker/test-support`, `build -p spur-embedding-worker`, and `fmt --package spur-embedding-worker -- --check`: each exit 0. See logs; existing vendored ORT and worker dead-code warnings remain.
- `tree -p spur-embedding-worker -e normal,build` contains no graph, analyst, DuckDB, PGQ, test-support or owner-test-hooks; `tree -p spur-embedding -e normal,build` contains no FastEmbed, ORT, tokenizer or ONNX dependency. Tree logs included.

`python3 verify_vectors.py` independently reads the six exported matrices, verifies 2 finite rows of 768 dimensions and <1e-4 reference/reuse error, then rejects altered row count, dimensions, NaN and reference difference. Its actual output is `vector-observations.json`. Both cycles measured zero maximum reference and reuse difference; the two row SHA-256 values are `20e7b37a25d564eca88aac5b07a29f4c5dc7ab498f756387683403294f65c5c1` and `8505f0e0ecefaf77c7f00ceb092a26f12da5f9222a2ad91e964f0754d92561f4` (little-endian f32). Raw client waits are `unix_wait_status(0)` for both cycles; direct instrumented product worker raw wait is 0. The trace's one load and one reuse are separate test-support observations in one generation. No native-owner count or RSS is claimed.

`hashes.json` binds source, normal binary, product test binary, ORT archive/dylib, five model files, model revision and descriptor identity. Artifact budget is the measured sum 548026095 bytes; one cache entry is an explicit policy witness, not a shipped resource default.

## Root POST candidates

`post-candidates.json` contains exact catalog requests and independent expectations: resource SAT at one cache entry and 548026095 bytes; lifecycle SAT in the order Ready → first batch → reuse → admission close → join → cache/output drop → checked release → Closed → normal exit. The six supplied adversaries must be UNSAT: two entries, one byte under budget, release before drop, Closed before release, reload instead of reuse, and abnormal exit. These are requests for fresh root execution, not POST results. Root must independently cross-check actual `product-test.log`, `worker-tests.log`, vector exports, hashes and verifier output before mapping observations to those facts. The PRE is prospective and does not prove measured results; no POST receipts have been produced here.
