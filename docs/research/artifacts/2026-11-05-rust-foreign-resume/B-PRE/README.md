# B fresh implementation PRE

This is the entry packet for canonical task `bd-2x4s`, plan `c16a0cff-63ff-4abe-83da-07a83b19df64`, task `capture-foreign-declarations`. It was prepared and verified by ROOT before A approval. B may start only after the orchestrator approves A and assigns B.

## Entry verification

Run `python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/B-PRE/verify.py` before changing tests or source. Reload all eight receipt IDs below with the worker solve tool; compare each complete request and expected status to `proof.json.gz`. Record the reloads, current source hashes, canonical task assignment and an identical successful replay of the INTENT audit. Use the fresh dispatched issue/delegation, never source planning IDs.

The verifier checks 199 files, fingerprint `5311a6fbe9dd927013086e31ffeb77df72505c6fff664cbfc3e9b8028a69d7f6`, exact solver lowerings/models, independent finite truth values, two Jev compilations, and cross-batch site uniqueness. Eight persisted/reloaded receipts passed: five SAT and three UNSAT, 24 compound predicates.

| Case | Status | Receipt |
| --- | --- | --- |
| Declared policy | SAT | sol_ae204988f6054907 |
| Frozen source sites 0 | SAT | sol_81a54b3363974754 |
| Frozen source sites 1 | SAT | sol_753945a9df944fd7 |
| Frozen source sites 2 | SAT | sol_2878f2e1a21a47cc |
| Same-file, same-ABI fixture | SAT | sol_f73be61b84c24e53 |
| Wrong existing same-file owner | UNSAT | sol_3f8cd89ebc254afb |
| Wrong existing corpus owner | UNSAT | sol_7876e32a46ba49a1 |
| Stale extractor | UNSAT | sol_3435acfd9c6147b7 |

These are declared preimplementation constraints, not assertions that current graph exports already satisfy them. Preserve the supplied packet unchanged; implementation evidence belongs under B.

## Grounding and scope

The supplied frozen archive contains 136 source files. ROOT recomputed every raw UTF-8 Git blob OID. All 458 candidates (446 tuple fields, nine foreign functions, three extern blocks) exactly match independent syn rows by path, OID, kind, full span and name, including common owner/ABI fields. The independent inventory has 469 rows: eleven additional rows belong to `__fixture__/held-out.rs`, whose source is absent from those 136 inputs. Do not count them as validated supplied-source rows or as recovered-source rows.

Recovered `tests/extension_credential_bridge.rs` is a separate uncertainty case in the historical G1c join. Preserve its Unknown status; neither syntax recovery nor cfg activation is proved here.

The twelve selected foreign sites are declared with exact source locators `path#git_oid:kind:start:end`, structural owner, lexical owner and ABI. These locators are not production graph stable IDs. Candidate experimental IDs must not replace IDs computed by the production extractor.

`owner_fixture.rs` supplies two distinct `unsafe extern "C"` blocks in one module/file. The negative fixture assigns the second function to the first existing block while retaining the correct module lexical owner. Existence, ABI or foreign-key checks alone would permit it; the exact required source/owner tuple rejects it. Use these exact bytes as an adversarial test fixture.

## Implementation gate

The canonical Beads task body is authoritative for allowed files and commands. The coordinated seam permits Rust queries, languages, store/build, necessary transparent-ancestor handling in tree_sitter, and the new foreign-declaration tests. Six unrelated corpus goldens and the canonical snapshot may change only extractor metadata, with parsed path proofs. Do not modify the separate declaration-evidence API yet.

Commit tests and observe runtime assertion RED for missing foreign nodes or wrong namespaces before the minimal fix. Capture only direct foreign signatures as Function; capture foreign containers as non-callable ExternBlock. Build owners before IDs. Keep the block as structural owner while descendants skip it in lexical qualified names and enclosing scopes, including artifact conversion. Test graph/byte extraction, full metadata spans, repeated blocks, Unicode/raw names, cfg duplicates and nested module/function owners. Preserve G1 tuple identities.

Use the prior approved extractor identity `2026-11-03-rust-foreign-declarations-v1`; graph schema stays v12. Bodyful extern functions, trait signatures and extern crate are exclusion controls. Uncertain dispatch remains unresolved.

Run the scoped foreign, byte/store, lexical/receiver, macro/ownership/tuple, extractor/canonical tests, graph all-target check, unchanged CLI schema/build/temporal gates and formatting/diff checks through `scripts/spur-cargo` as specified. There must be one Cargo writer. Rebuild frozen inputs only in a private temporary project, never the live notebook project.

Join actual exported production nodes to the frozen source and independent inventory, retaining zero-row source coverage, exact full ranges, stable identities and both owners. Require the selected nine functions and three containers while preserving the explicit recovered/held-out distinctions above. Save actual exports and negative controls for ROOT's fresh POST.

Leave B in progress for ROOT review. Before B approval, ROOT must prepare C PRE; C remains pending. No source-syntax result here establishes Cargo cfg, expansions, linkage, foreign bodies, compiler dispatch or complete panic detection.
