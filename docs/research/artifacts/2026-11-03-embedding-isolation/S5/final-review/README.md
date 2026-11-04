# S5 root review

Candidate: `c06d12b2489c8f49af1c0be1f3d28a9e1428b64a`. All 283 changed paths are in the authorized source/evidence scope. Production and Cargo bytes match protected checkpoint `2814a3f5935b34a119efbe0ec760b51f7a8b49ab`. Root reviewed the actual diff, including owned helper shutdown, identity discovery, cold artifact acquisition, preserved batching/storage identities, and native test relocation.

Root independently reran 34 client tests and five graph tests: all passed, none ignored. The fresh/frozen artifact verifier passed and checked actual Parquet vectors, input ordering, child waits, three model references, fifteen artifact hashes and dependency/feature trees. Actual-model outputs are the frozen source-matched real runs; root did not claim new inference from this read-only verifier. Nomic's maximum reference difference was 1.0058283805847168e-7; CodeRank and Jina were 0, under the inherited 1e-4 acceptance tolerance.

## Baseline comparison and risk decision

Root ran the unchanged lifecycle and runtime-cache test targets on pre-migration base `d2956a1fb604a1467829ace828d26238ab29e4d9`, using the pinned runtime and isolated caches. Both commands exited 101. Seven lifecycle cases and the eighth relocated-runtime case reproduced the saved abort failures after correct inference; combined cases also returned the expected PGQ row before aborting. Four parent tests passed, eight failed, none ignored. Exact commands are recorded below and full logs are retained.

Seven moved test sources, their runtime loader and build support are byte-identical to the approved base. This establishes that the eight failures predate the S5 migration. It does not establish general ORT memory safety or prove every shutdown error has one cause.

The risk can be accepted for the staged S5 graph-isolation task: graph no longer links or initializes native inference, the new supervised client has passing failure/teardown tests, and the direct legacy seam remains as a separate explicit regression target. The eight failures remain real and recorded; they have not been fixed, ignored, deleted or counted as passes. Analyst migration, packaging and final CLI/PGQ/notebook acceptance remain separate downstream gates.

## Fresh root POST

Three OPEN Jev interpretations were inspected against observed facts, then augmented with current catalog uniqueness and cardinality bindings. Three verified models passed and all eighteen corruptions were rejected. All 21 receipts were persisted and reloaded. The proof is finite metadata verification, not a native memory or unbounded execution proof. `post-proof.json.gz` retains inputs, Jev provenance, catalog, exact family/lowered requests and results. `actual-facts.json` records grounding. Run `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S5/final-review/verify.py`.

| Case | Result | Receipt |
|---|---|---|
| dependency_boundary_post | sat | sol_38faaea43310454d |
| publication_policy_post | sat | sol_602918aba5a6475b |
| real_models_post | sat | sol_686986f4b98e4b33 |
| graph_native_dependency | unsat | sol_97b4b7ab442b4903 |
| graph_inprocess_route | unsat | sol_5c0fe544cf044ffb |
| worker_database_dependency | unsat | sol_126bfd2f546a4d8e |
| disabled_spawned | unsat | sol_6ee2fe867d6d4013 |
| error_published | unsat | sol_fb2a0213b71d41a2 |
| abort_pending_published | unsat | sol_89cedb74be614973 |
| abort_prior_erased | unsat | sol_10e45d16d55e4781 |
| wrong_identity_published | unsat | sol_99c4c9c332364abd |
| nonfinite_published | unsat | sol_9365e1e7dcb143df |
| truncated_exit0_published | unsat | sol_0935d0a7d6054422 |
| wrong_dimensions | unsat | sol_a6d4fafd5bc44e77 |
| nonfinite_real | unsat | sol_abb46f5f329b4306 |
| identity_changed | unsat | sol_f87a47b6096f4f76 |
| reference_outside_tolerance | unsat | sol_b81400e2dddd4d05 |
| failed_real_wait | unsat | sol_a933778aec6340cb |
| missing_close | unsat | sol_1a41ca6f6efe41d0 |
| model_reload | unsat | sol_9cd159f50cbc4095 |
| missing_batch | unsat | sol_90570ac401e141ec |

## Root commands

All cargo commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true`; native commands also used the pinned `SPUR_ORT_ARCHIVE` already recorded in the worker packet.

- Baseline: `scripts/spur-cargo test -p spur-graph --test ort_lifecycle --test ort_runtime_cache -- --test-threads=1` — exit 101 at lifecycle (2 passed, 7 failed); Cargo did not run the second target.
- Baseline second target: `scripts/spur-cargo test -p spur-graph --test ort_runtime_cache -- --test-threads=1` — exit 101 (2 passed, 1 failed).
- Candidate: `scripts/spur-cargo test -p spur-embedding --features test-support -- --test-threads=1` — exit 0, 34 passed.
- Candidate: `SPUR_S5_TEST_EXPORT=/private/tmp/spur-s5-root-fake scripts/spur-cargo test -p spur-graph --lib store::lance_sections::tests::graph_ -- --test-threads=1 --nocapture` — exit 0, 5 passed.
- `python3 docs/research/artifacts/2026-11-03-embedding-isolation/S5/attempt3/verify_fresh.py` — exit 0.
- Production source diff against checkpoint — empty. Root source formatting/diff and proof checks are recorded with the final commit. Historical unscoped diff whitespace failures remain in the worker packet.

Fresh prospective S6 PRE is prepared in `../../S6/root-pre/` before upstream approval releases S6. Neither Jev confidence nor a worker success status constitutes approval; the engine review audit records the actual decision.
