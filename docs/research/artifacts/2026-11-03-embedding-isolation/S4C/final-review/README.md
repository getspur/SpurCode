# S4C root review: three real embedding models

Canonical issue `bd-30z6`; delegation `2a1b85bac2e24bcc`, attempt2. Reviewed candidate `d2956a1fb604a1467829ace828d26238ab29e4d9` is based on approved S4B `f98148ac0b7786908c3b974f9526e0feae242765`. Root changed evidence and review scripts only.

## Decision evidence

All55 current scoped tests are covered with none ignored. The original root all-target suite passed47 tests and hit one review-harness failure: the older Nomic test tried writing into an export directory that root had not created. After creating that directory, the exact failed Nomic test passed1/1 and the previously unrun native lifecycle target passed7/7. No production or test source changed between runs. The original exit101 and its complete log remain in `suite.log.gz`; it is not relabeled as a passing command. Exact commands, hashes, stage statuses and summaries are in `verification.json`. All-target compilation, scoped formatting and the normal helper build exited0.

The final-source three-model reference/client test ran two independent worker lifecycles per model and two batches per lifecycle. Root independently recomputed all12 actual batch outputs: each has two finite768-dimensional rows, norms within the existing1e-4 tolerance, and maximum legacy-reference/repeated-batch difference0. All six client workers recorded raw Unix wait0. Reference processes use the existing registered Nomic/Jina loaders or the pinned custom CodeRank recipe; they are separate from the helper and each other.

Root then ran the copied final instrumented and default binaries for all three models. All six real processes emitted validated vectors and Closed and had actual waitpid raw status0. Instrumented phases show one load, one reuse, admission close, job join, model drop and checked environment release in order. Jina stdin EOF after inference followed cleanup with normal exit1/raw256 and no Closed. Six cross-model hash/model swaps returned correlated InvalidRequest before native startup; all3 exact artifact budgets passed and all3 one-byte-under descriptors returned Unavailable. A typed error is operation failure even when its process exits0.

| Model | Snapshot / revision | Verified artifact bytes |
|---|---|---:|
| NomicEmbedTextV15 | e9b6763023c676ca8431644204f50c2b100d9aab | 548026095 |
| CodeRankEmbed | 151669b173750250e611e1be00c812f112ea6020 | 548431020 |
| JinaEmbeddingsV2BaseCode | 516f4baf13dec4ddddda8631e019b5737c8bc250 | 644080771 |

`verified-artifacts.json` rehashes all15 files and independently recomputes the profile/role fingerprints using the versioned big-endian encoding. These match actual descriptors. Profiles retain768 dimensions, max_length8192, Mean pooling, existing unit normalization, no quantization/default output selection and unchanged caller-preformatted strings. Nomic/Jina are evaluated against registered FastEmbed5.8.0 references; the helper consumes verified owned bytes. CodeRank uses its fixed repository/revision and the legacy Mean user-defined recipe. The hf-hub0.4.3 synchronous get/cache/download seam and FastEmbed5.8.0 pooling selection were read from exact indexed dependency revisions; lock checksums are archived. Normal client remains native/database-free; normal helper is database-free. No host migration is claimed.

## TDD, recovered evidence and authorization

The original descriptor/model test commits `cf084fe3b`/`20457f8d4` precede implementation `bc8834e62`. Cold-cache test `70e4060b8` precedes acquisition fix `164dd5808`; late adversary `a9fdac521` is now executed by root. The chain is protected at `spur/review-evidence/embedding-s4c-attempt2-authored`. Recovered genuine runtime RED logs show CodeRank Unavailable and the pinned cold-cache assertion failing before their fixes. The previous 54-test run and pre-acquisition instrumented trace remain historical evidence, not substitutes for the final-source checks.

Normalization retained the authorized root imports and formerly untracked worker evidence before removing the worker checkout. The inline summary lists seven source/test files; the actual normalized candidate has57 changed paths including those allowed evidence files. `diff-review.json` records the discrepancy and exact paths. No unexpected source scope was added.

Beads assignee/dispatch and orchestrator completion audit2854 identify the current delegation. Worker evidence/signal calls nevertheless reported an authorization denial. Root inspected the guard: it requires exactly one matching delegation label and the durable current dispatch, beyond the assignee. The completed worker no longer has an active delegation label. The earlier failing call's complete label snapshot is unavailable, so its precise active-time cause remains unproven. Root completed review and records through the authorized brain issue channel, without granting a completed worker renewed access. The next dispatch must verify its current label and audit as well as assignee; evidence must be committed before completion.

## Fresh Jev POST

Ten interpretations were OPEN, with unchanged facts and no solver unknowns. All10 positive catalog models passed and25 independent invalid controls were UNSAT; complete requests, outcomes and reloaded lowered envelopes are in `proof.json.gz`.

| Group | Receipt | Outcome |
|---|---|---|
| configuration | sol_a6181cbf93954714 | SAT |
| resources_NomicEmbedTextV15 | sol_53489ca1fa4e4eae | SAT |
| resources_CodeRankEmbed | sol_f5d186ba06614a4b | SAT |
| resources_JinaEmbeddingsV2BaseCode | sol_4da31cdaf71e4626 | SAT |
| vectors | sol_5b3f58a27f584efa | SAT |
| lifecycle_NomicEmbedTextV15 | sol_dede61b75ba74555 | SAT (cached) |
| lifecycle_CodeRankEmbed | sol_c678426a7b6f4d2d | SAT (cached) |
| lifecycle_JinaEmbeddingsV2BaseCode | sol_e4905582a914497e | SAT (cached) |
| completion | sol_06ea3ef4dc614262 | SAT |
| bindings | sol_5177d59f223b4517 | SAT |

The25 controls cover model/configuration/prefix/quantization/output-key/coverage uniqueness; one-byte-under budgets for every model; a second cache entry; wrong shape, nonfinite/norm/reference labels; reload/drop/release/Closed/exit ordering; false EOF completion; and altered source/helper/runtime/manifest hashes. The offline verifier independently injects15 raw numerical corruptions, separate from the finite labels sent to the solver.

These verify observed finite traces, measured artifacts and consistency. They do not encode full ONNX/floating-point semantics, all inputs/interleavings, complete Rust/FFI memory safety, native owner counts, global concurrency or RSS. Ready remains identity/native-environment negotiation followed by lazy model load on first Embed. Default and instrumented evidence are kept distinct. The helper currently acquires pinned CodeRank on cold cache; the tested Nomic/Jina snapshots are prepared caches. S5 must preserve the old graph's default model-materialization behavior while moving its ownership into the helper.

## Recheck and continuation

Run `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S4C/final-review/verify.py` for the offline archive audit, or add `--live` to rehash currently retained binaries/assets. Raw frame bytes, float outputs, wait statuses, source/build/runtime/model hashes, failure history and recursive checksums are archived. Collection scripts write observations and should be run in a fresh review directory; the verifier reads the committed archive.

The executed S5 PRE is in `S5/root-pre/`. Graph/analyst migration, companion packaging, final host inspection, unchanged CLI DuckPGQ regression and fresh Notebook Rust-cell demonstration remain later gates.
