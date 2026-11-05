# A independent ROOT review

A is accepted for the existing non-callable `extern_block` representation and graph schema v12. This stage is verification-only: no production source, tests or manifests changed. Physical foreign declaration capture remains B's implementation work.

## Checked evidence

- Canonical issue `bd-3se8` was `in_progress`, ready for review, with no unresolved signal labels. Live Beads comments 3684 (INTENT), 3697 (worker completion) and 3699 (orchestrator completion) were read. INTENT and completion idempotent replays are retained in A.
- Authored tip `9eb2641b9` and normalized tip `f9ceab7578ea08d364905c2bb0a0e8007324f8b6` have identical tree `7ddf0a949ab064ca3d0593afd7fab88b24e5870d`. The authored result has 97 changed files, all under A.
- A's unchanged strict verifier passed with `--require-acceptance --check-identities --check-hashes`: 95 evidence files, four binary identities and three native outputs checked.
- 130 successful case executions comprise 103 graph executions retained from attempt 1 and 27 freshly executed CLI cases: six schema guards, seven build cases and fourteen temporal cases. There are 128 distinct case names. No ignores occurred; invocation filters remain explicit in the raw records.
- The unchanged original five-row PGQ assertion ran, including both `serde::Serialize` rows. It is an executed assertion, not a separately exported SQL result.
- All 185 bound source hashes remain unchanged, fingerprint `08ff58d147fed42e9f10fc286fe31f01a2d66c89e067bb32cc653b6bf675b69e`. The original four PRE receipts were reloaded exactly.
- Actual role exports mark the synthetic container non-callable, non-constructor, non-import and non-fallback. The actual stale-v11 CLI attempt exited 1 without creating a database. Callable, stale-source and stale-reader mutations were rejected.
- ROOT also passed missing and duplicated observed-marker arrays through A's original verifier. Both were rejected; see `independent-row-controls.json`.

The test serialization fixture begins with a synthetic function that is relabeled to `extern_block`. Its successful kind/schema test does not establish foreign syntax coverage. Declaration evidence remains separately versioned v1 and tuple-only.

## Fresh POST

`verify_post.py` passed seven persisted, pinned and exactly reloaded receipts: two SAT and five UNSAT, with 33 independently evaluated catalog predicates. The current catalog and Jev compilations are saved. Exact request, lowering, model and finite evaluation checks are in `proof.json.gz` and `solver_oracle.py`.

Fingerprint: `613cfe3c330cdc8b168a8bf74719e560585b676c82f9be018ce61d356695c923`.

| Case | Status | Receipt |
| --- | --- | --- |
| Actual observations 0 | SAT | sol_56dd0b57efe84cc3 |
| Actual observations 1 | SAT | sol_062fe122694d40d4 |
| Forged observations 0 | UNSAT | sol_25559f8d4a924048 |
| Forged observations 1 | UNSAT | sol_4748332f97664b53 |
| Stale source | UNSAT | sol_222d76fa7a8b4674 |
| Missing observation | UNSAT | sol_4c51a9f930634e42 |
| Duplicate observation | UNSAT | sol_05e95966a4754b46 |

Each positive observation is derived from actual exports or executions and compared against an independent required tuple. Missing and duplicate controls preserve the distinct coverage/uniqueness checks. No expected table replaces the actual exports.

## Preservation and bounded retention

Attempt 1's ENOSPC build failure and zero executed CLI cases remain in the original logs, 68-file archive and protected history. The completed retry supplies the missing acceptance; it does not rewrite that failure. Eight historical direct-native failures remain separate.

The original worker checkout path was restored before running its strict cwd-bound verifier; no raw record or verifier was patched to bypass that binding. Current main's documentation commits were merged into this isolated review branch. Main's five pre-existing dirty files retain their saved hashes.

Before freeing solver retention slots, ROOT independently checked 16 already-closed M6 receipts (57 predicates) and compared complete live envelopes against their archived reloads. Only those 16 were unpinned; all four active G2 PRE receipts remain pinned. The records, comparisons and operations are saved here. No solver-store files or quotas were changed.

## B entry gate

B's fresh PRE is verified in [B-PRE](../B-PRE/README.md): eight receipts, 24 predicates, 199 bound files. It declares future source/owner requirements and rejects a wrong existing block in the same file with the same ABI. It does not claim present foreign capture.

A approval may now release B. B still requires committed test-first assertion RED, minimal implementation, scoped GREEN, actual source-bound exports, independent frozen joins and ROOT POST. C stays gated until B review and fresh C PRE.

Reproduce these finite checks from this checkout:

```sh
python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/ROOT-A-review/verify_post.py
python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/B-PRE/verify.py
```

The stored `verification.json` records the requirement for B PRE at the instant A POST was captured; the separately completed B verification discharges it. Source syntax coverage does not establish cfg activation, expansions, foreign linkage, body execution, compiler dispatch or complete panic detection.
