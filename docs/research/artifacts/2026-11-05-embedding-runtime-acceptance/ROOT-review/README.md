# ROOT runtime and Rust notebook review

The scoped macOS ARM64 debug acceptance gates passed. This review supports the supervised inference boundary and measured behavior; it does not approve broad ORT ownership safety, complete panic detection, remaining G2/G3–G5 graph gaps, other platforms or optimized release execution. Beads records the authoritative review decision.

ROOT independently replayed the unchanged worker verifier (exit 0), checked all 297 authored evidence files against Git blobs, confirmed authored/normalized tree equality, and retained the original 64-file production fingerprint. The only new source file is the opt-in 74-line graph measurement test. All 106 source, native runtime, model, tokenizer and executable identities still match after ROOT's real notebook rerun.

The live Notebook MCP document initially matched the saved worker document. ROOT reran its existing Rust cell 36c97aa6-606a-4ca0-a2e7-d03e8d215bb1, without source changes. It passed the unchanged default CLI graph assertion before and after a real helper SIGABRT. A real Nomic request first returned 768 values; the next request failed in 30.040 ms after the abort. The host remained responsive and the prior vector stayed valid. Children disappeared and the socket was removed. The failed lifecycle closed with host raw wait 256 / exit code 1, not clean success. The notebook SQL kernel lacks DuckPGQ; the Rust cell ran the real built CLI PGQ test in separate processes. No native SIGABRT was caught by Rust catch_unwind.

Notebook: /Users/kevintruong/project/spur-notebook/embedding-runtime-acceptance-bd-1pvg-563da938.ipynb (nb-08896ffe4a694c270f8311ef). Live exports and the durable rerun response are preserved here; no existing notebook or spur-notebook source was edited.

The ordinary product evidence includes six cycles over Nomic, CodeRank and Jina, finite unit vectors within the existing 1e-4 reference tolerance, clean normal host exits, and separately observed clean direct-helper exits. Product v1 requests are sequential single-text calls; separate graph-owned runs exercise distinct two-row document batches. Host live maps exclude ORT and helper maps exclude DuckDB. Fault publication preserves validated vectors and leaves pending output NULL.

Fresh Cargo metadata accounts for all 112 applicable graph runners among 113 configured targets (the build script is excluded). The original all-target invocation exited 2 when Criterion rejected libtest arguments; corrected benchmark/example commands are separately preserved. Small private correctness fixtures used for missing benchmark fixtures do not establish equivalent benchmark performance. Analyst/CLI results, named ignores/filters and doctests remain in the original command ledger. The separate legacy direct-native workspace still has exactly eight historical failures and exit 101; none was waived or called fixed.

Observed product cold requests were approximately 47–58 s and warm queries 10–20 ms under the recorded debug/cache/background-load conditions. Graph-owned warm two-row batches were 25–63 ms. Sampled combined host/helper RSS was approximately 1.77–2.06 GiB. The complete-cache test-only in-process comparison has different startup/cache methodology and is not a reused-session comparison. These measurements establish neither a performance improvement nor optimal policy.

Two bounded follow-ups remain open and are accepted as explicit risks for this runtime review:

- bd-1rpe: the initial test-only CodeRank reference suffered an offline-cache lookup failure and native teardown abort (raw 6 / Python -6). The separately labeled complete-private-cache retry exited 0. This new failure is not one of the eight historical baselines and is not a repaired production path.
- bd-31du: ROOT reproduced PyArrow 21.0.0 ArrowInvalid when reading nullable fixed-size-list vectors, while DuckDB reads exactly two committed vectors and one NULL. Responsibility between writer and reader is unproven; NULL was not replaced with a fake vector.

The worker's initial INTENT and identical replay persisted, and orchestrator completion audit 3589 exists. The final worker audit UUID was not found as an audit in the frozen Beads snapshot; its timeout remains unconfirmed. ROOT did not retry under the worker's identity. Risk signal 8cdcf291-0a24-4fde-a64e-0d1ef18bdd8f was reviewed.

Fresh Jev compilations preserved supplied facts, verify mode, unknowns and rule selection. Explicit subject bindings and additional unique/cardinality/range rules come from the inspected current catalog. Thirteen fresh pinned/reloaded POST receipts yielded four SAT and nine UNSAT results; an independent finite oracle checked 52 compound predicates, including 22 independently bound tuple forgeries. Separate runtime-data mutations reject stale bytes, semantic vector forgery, missing observations and duplicates. Numeric limits use the actual declared request/shutdown/dimension policy with inclusive bounds and one-above controls. The final-proof requirement is checked after receipt verification, outside its own input, rather than assuming its own completion.

Acceptance fingerprint: a13b2d26918d07bf1eb1120c0cbfa772d91fcd6ceb2fabee3293182f0136101e. It covers production source, the new measurement test, the complete authored evidence tree, proof helpers, exact native identities and ROOT's captured observations. The old 64-file production fingerprint alone does not bind this new test and evidence.

Reproduce from this retained exact review worktree, using the saved native/private paths:

    /Library/Frameworks/Python.framework/Versions/3.12/bin/python3 -B docs/research/artifacts/2026-11-05-embedding-runtime-acceptance/verify.py
    /Library/Frameworks/Python.framework/Versions/3.12/bin/python3 -B docs/research/artifacts/2026-11-05-embedding-runtime-acceptance/ROOT-review/verify_targets.py
    /Library/Frameworks/Python.framework/Versions/3.12/bin/python3 -B docs/research/artifacts/2026-11-05-embedding-runtime-acceptance/ROOT-review/verify_post.py

No production repair, global cache/runtime change, installed binary overwrite, publication or push was performed during this review. The optional cargo-dist plan remains unverified; this review does not convert the earlier rejected bypass attempt into a passing check. Next source work is G2, followed by G3–G5, with fresh grounding and PRE/TDD/POST on merged source.

The default staged whitespace check reports only trailing blank EOF lines in the new evidence (two frozen proof helpers and the initial README). Proof helper bytes remain unchanged after their hashes were bound to receipts. The staged check with only blank-at-eof excluded passes; this is an explicit evidence-format exception, not a passing default-check claim.
