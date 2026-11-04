# S4 root Jev PRE

Issue bd-2dhg. Source candidate65189c90c146bc6bc3d972f5b88b076b729ef19c. This is a prospective contract for product-model compatibility, finite cache entry accounting and an eleven-step model lifecycle. It is **not** evidence that S4 already returns embeddings.

Three fresh Jev gates are open. Root inspected complete facts, rule families, verify mode, subjects and parameters, then used catalog SOLVE with persistence and full reloads.

| Positive | Receipt |
|---|---|
| Three exact legacy model/profile tuples, unique and complete | sol_777fae22d5bc4680 |
| One pinned cache entry fits one declared slot | sol_e5aef14c26ab4396 |
| Reuse, admission close, join, model/output drop, checked release and normal exit | sol_6fba2bf794eb423c |

Ten negatives rejected: wrong profile for each model, missing model, duplicate model, two entries in one slot, environment release before model drop, Closed before release, reload instead of reuse, and abnormal exit. Exact requests, SMT, all13 solve IDs/statuses/reloads and Jev provenance are preserved in pre-proof.json.gz. An initial malformed catalog query used two selectors; those errors and corrected single-selector discovery are retained and are not solver evidence.

grounding.json binds five exact source files and all five available Nomic model/tokenizer/config files. Cache discovery followed symlinks; ONNX is a real547310275-byte file, not a missing-file inference from an extension-only search. CodeRank/Jina were absent only from this particular cache root.

Read HANDOFF.md for the concrete metadata bootstrap, no double prefix requirement, native cleanup ownership, expanded test scope and actual inference acceptance. Prefixes/max_length/dimensions are inherited source requirements. One cache slot is an explicit single-identity witness. Artifact sizes/approximate model MB do not prove a hard native RSS ceiling.

Recheck local evidence:

```bash
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S4/root-pre/verify.py
```

Worker may reload these receipts and implement test-first now after dispatch. Root will compile and execute fresh POST from actual S4 observations before approval.

