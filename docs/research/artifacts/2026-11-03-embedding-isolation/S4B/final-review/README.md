# S4B root review: real Nomic inference

Canonical issue `bd-1ydx`, delegation `007cf1071167479e`, candidate `f98148ac0b7786908c3b974f9526e0feae242765`. This record verifies the scoped Nomic implementation; all-model acceptance and migration of host callers remain subsequent tasks.

The worker's test commit `67755a63911e9d851a08b4d7eb205636cca341ea` precedes fix `1024ba6a4e764dda340cf2f03e6d29c6e902f33d`; export-test commit `81a9f4d36299c732f786c6da474204302d39bbaa` and evidence commit `da3a07072722e079f7c0786949c88d2f5df018f4` follow. The authored chain is protected by `spur/review-evidence/embedding-s4b-attempt1-authored`. The recorded runtime RED was the real product test receiving Unavailable before implementation. Root checked the 54-file result: seven implementation/test paths, imported prior proof and scoped worker evidence. Beads completion audits 2819/2822 and the current assignment were verified; no unresolved signal labels were present.

## Independent verification

Root ran `run_checks.py` with `SPUR_REMOTE=0 TMPDIR=/private/tmp`, the one authorized local target and pinned official ORT archive. Exact wrapper commands, raw compressed logs and digests are in `verification.json`. No production files changed during review.

| Gate | Actual result |
|---|---|
| Real client/reference product test | 1 passed; two independent worker cycles; two batches per cycle |
| Worker units / metadata / native lifecycle | 2 / 6 / 7 passed |
| Client/protocol/supervisor | 32 passed |
| All-target check, format, normal helper build | Exit 0 |
| Numerical comparison | Every batch: two finite 768-dimensional rows; maximum reference and repeated-batch difference 0 |
| Independent direct normal-build process | Real vectors/reference match; raw wait 0; Closed reply |
| Instrumented process | One load, one reuse; join then model drop then environment release; raw wait 0 |
| EOF after real inference | Same cleanup order; normal exit1, raw Unix wait256, I/O error; no Closed |

`capture_processes.py` records real wire bytes, generation/request correlation, actual `os.waitpid` status and binary hashes. Normal executables are copied outside the repository; their measured hashes are archived rather than relying on the mutable shared target. Instrumentation runs only in test-support. The client dependency tree contains no native inference/database packages; the helper contains FastEmbed/ORT and no graph/analyst/DuckDB/Lance.

The model loads lazily at first Embed. Ready establishes identity/native-environment negotiation; it does not claim a warmed model. POST preserves this distinction. The source revalidates the bytes read from the retained file handles, binds the effective user-defined loader/quantization/output-key profile, and consumes those immutable bytes. The registered FastEmbed5.8.0 reference remains independent of that memory-loader path. Both use Mean pooling, max_length8192, no quantization, default output selection and unchanged already-formatted inputs.

## Root Jev POST

All five fresh Jev interpretations were OPEN. Exact inputs, routes, family requests, lowered solver envelopes, outcomes and reloaded receipts are in `proof.json.gz`. One equivalent resource result was a cache hit; it is not claimed as a new Z3 execution. Catalog unique/cardinality checks supplement the three finite tuple models.

| Model | Receipt | Result |
|---|---|---|
| resources | sol_27e186b9c2294ab9 | SAT (cached) |
| lifecycle | sol_8e9bf19e56754cc4 | SAT |
| vectors | sol_563f9fe0f56c4680 | SAT |
| completion | sol_4211b773e5f343ed | SAT |
| bindings | sol_a9548fd763844bd2 | SAT |

All 16 persisted/reloaded controls were UNSAT: second cache entry, byte budget one below the measured total, release/drop reversal, early Closed, reload, native abort, wrong vector shape, nonfinite values, bad norm/reference classifications, EOF falsely marked clean/zero, normal status replaced by abort, and changed source/helper/runtime digests. Independently replayed numerical corruption checks reject wrong row count/dimension, NaN, changed values and zero-norm output.

These are bounded measurements and consistency checks. Numeric classifications come from independent Python recomputation of actual floats; the finite solver does not prove ONNX or IEEE floating-point semantics. Artifact_bytes548026095 is the measured five-file total, not RSS or a shipping resource default. One cache entry is not a claim about global concurrency or a fabricated native-owner count. Closed-contract cleanup in uninstrumented processes is distinguished from directly observed phase traces. EOF exit1 is not successful operation completion and is not a native abort.

## Recheck

Run `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S4B/final-review/verify.py` for the offline archive/source audit. Add `--live` to rehash the currently retained copied binaries and runtime. `build_post.py` reproduces the fact projection while those local resources exist. Both verifiers and the SHA256 manifest passed before the review decision. A transient external-source approval-review timeout is preserved in the proof; the instructed single retry succeeded and left no blocked action.

The next task's executed prospective PRE is in `S4C/root-pre/`. It does not assert that CodeRank/Jina have run yet.
