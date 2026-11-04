# Executed root PRE for S4C

Actual assigned canonical issue: `bd-30z6`; task `embedding-models-acceptance`; source epic `bd-2tan`. Use this exact canonical ID for audits/signals, verify it is in_progress and assigned to your current delegation, and never infer an assignment from branch UUIDs or cancelled source issues.

Root executed two fresh OPEN Jev interpretations: finite three-model compatibility and per-identity cache capacity. Both positive catalog requests passed; all eight negative controls were UNSAT. All ten results were persisted, reloaded and pinned for this handoff. Full intents, prospective facts, routes, family requests, exact saved lowered solver envelopes and receipts are in `proof.json.gz`.

| Case | Receipt | Result |
|---|---|---|
| compatibility | sol_52434636cc134d91 | SAT |
| cache | sol_b34dbb87ea1749c7 | SAT |
| wrong_nomic_dimensions | sol_29398e0f7d52487a | UNSAT |
| wrong_coderank_recipe | sol_52e275bef2ef47df | UNSAT |
| jina_double_prefix | sol_b98e92f13d8c48ef | UNSAT |
| wrong_quantization | sol_d705559145174b78 | UNSAT |
| wrong_output_key | sol_82a4e4fa7d9e4ded | UNSAT |
| missing_model | sol_53d87f4f4af5468f | UNSAT |
| duplicate_model | sol_f6595197153443b6 | UNSAT |
| two_cache_entries | sol_1aac17db605747ff | UNSAT |

Run `verify.py`, then reload these exact IDs with `get_solve_result` and compare their saved `request`, `status` and `model` with the archived reload envelopes. Worker-local Jev absence is bridged by root and is not a blocker. Root will execute a new POST from your actual outputs before approval. Keep this archive and all earlier evidence intact.

The initial four-row duplicate control exceeded the catalog's64-variable ceiling and produced no receipt. Its exact tool error is preserved, not counted as a rejection. The executed corrected control replaces one of the three existing rows with another valid supported model tuple; it fits57 variables and is rejected by uniqueness while retaining cardinality3. No product requirement was weakened.

## Concrete implementation handoff

S4B candidate `f98148ac0b7786908c3b974f9526e0feae242765` has real Nomic inference. Root independently verified48 tests, both reference/client cycles and a normal production helper, with measured maximum vector difference0 and raw normal waits0. Instrumented EOF exits1/raw256 after cleanup, not abort. Read `S4B/final-review/README.md` for exact POST results and source/runtime hashes.

The current `ResolvedArtifacts::verified_bytes` reads the already-open handles, enforces the checked total, recomputes fingerprints and freezes the consumed model/tokenizer/config bytes. `ModelCache` owns one lazily initialized `Option<TextEmbedding>`. It loads at first Embed, reuses thereafter and passes caller strings unchanged. `RuntimeOwner::close_after_join` closes admission, joins jobs, invokes cache cleanup and then calls checked environment release; the existing pre-join hook semantics remain intact. Preserve both normal Shutdown and error/EOF cleanup.

Extend that actual pipeline to all three supported models through test-first production changes. Nomic and Jina must retain the registered FastEmbed5.8.0 recipes; CodeRank must retain the graph's custom repository `jamie8johnson/CodeRankEmbed-onnx`, revision `151669b173750250e611e1be00c812f112ea6020` and Mean user-defined recipe. Every model has dimensions768, max_length8192, Mean pooling, existing unit normalization, no quantization and default output selection. Retain the legacy acquisition semantics (CodeRank hf-hub from_env/progress/retries3) without introducing host graph dependencies. Parent callers already add their model-specific query/document formatting; do not add prefixes again in the worker. Jina query/doc and CodeRank doc inputs remain raw.

Acquire missing CodeRank/Jina assets only into authorized isolated /private/tmp caches. Measure/hash all consumed artifacts and record resolved snapshot/revision, including any actual external weights. The new models' bytes and fingerprints are not invented in PRE. Checked sums and direct catalog request<=explicit measured-budget controls use the existing approved interpretation; a new observation does not require stopping for another policy interpretation. New policy semantics do require a root signal while unrelated authorized implementation continues. Do not use Nomic's548026095 total as a budget for a different model.

Add a real independent reference and client comparisons for all three models, same-worker load/reuse observations, repeated independent raw exit0 cycles, prefix/identity/artifact tampering and premature-close controls. Separate processes are required where a managed environment has already been closed. Preserve existing native/client/descriptor tests and test-support feature isolation. Product inference, exact consumed identities and normal exits must all pass; metadata, reference-only runs or skipped missing models are incomplete. No host migration or broad O1 wrapper audit belongs to this task.
