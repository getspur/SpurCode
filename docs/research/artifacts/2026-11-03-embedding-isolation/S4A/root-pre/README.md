# S4A root PRE: artifact descriptor and strict identity

This is the fresh PRE for the first focused recovery task, based on candidate c6dbb22a1de4308e1070e8742786762d54ea7a39. S4's two attempts produced failing tests and a successful independent Nomic reference; neither implemented worker embeddings.

Two fresh Jev interpretations are OPEN: data_integrity/verify for exact artifact/profile tuples, and resource/verify for actual file bytes <= caller budget. The catalog requires one relation per rule binding. Jev initially grouped two subjects in one binding; the catalog rejected that arity. The raw error is preserved. Root expanded identical facts/policies into separate one-subject bindings and added discovered uniqueness/cardinality bindings. No threshold lowering or new Jev wording was used.

| Positive verification | Receipt |
|---|---|
| Five exact artifact hashes and legacy Nomic profile, unique and complete | sol_a10f258cdf7846fc |
| Measured548026095 artifact bytes fit equal caller budget | sol_f2a426003d4a42a7 |

Six expected negatives reject forged model hash, forged tokenizer hash, missing tokenizer config, duplicate artifact, forged inference profile, and a budget one byte too small. All eight results are persisted and reloaded in pre-proof.json.gz.

grounding.json binds six exact source files at candidate c6dbb22a1 and five independently rehashed cached Nomic files. The source graph is older than these new helper files; exact Git/working-tree bytes were used for those bindings.

This is prospective descriptor/identity/budget policy, not actual descriptor output, product inference, a SHA collision proof, native release proof, or native RSS ceiling. S4A must finish real production metadata/budget/strict-Hello behavior and scoped GREEN; S4B adds model inference, and S4C completes all three model acceptance. Original S4 stays unapproved until the full contract passes.

Verify:

```bash
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S4A/root-pre/verify.py
```

