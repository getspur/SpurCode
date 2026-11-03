# S4 supplemental artifact-byte PRE

This resolves worker signal75632e39-060d-42d0-a330-573e45b4c524 for active issue bd-2dhg, delegation10a0cc37d8a64bc5. The first descriptor test commit5190d7279 already produced an assertion RED; no production implementation of this new bound preceded this PRE.

Root independently measured the same five Nomic files in ../root-pre/grounding.json: total **548026095 bytes**. The caller-declared equal budget is a concrete test witness. It is neither a shipped default nor a native RSS bound.

Fresh Jev gate OPEN. Inspected resource / verify / resource.request_within_limit / nomic_artifacts, resource artifact_bytes; complete facts, no unknowns or weakened limits.

- Equal budget548026095: SAT/pass **sol_715942d8810a453a**.
- Budget548026094, one byte too small: UNSAT/reject **sol_24a448fc11d14c37**.
- Both full requests/results persisted and reloaded; exact raw evidence in pre-proof.json.gz.

Worker can now implement this caller-declared bound test-first. Verify checked addition and actual used artifact bytes before reads/materialization; do not infer resident memory from artifact sizes or allow model files to change without identity revalidation. Keep default policy deferred to S7/S8. Continue all real inference, cache reuse, normal native exit and fresh root POST gates from the original handoff.

Verify with:

```bash
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S4/root-pre-artifact-budget/verify.py
```

