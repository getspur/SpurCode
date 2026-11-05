# D3 ROOT PRE: exact full and selected Rust source domains

The preceding D2 carrier/hash/validator implementation was independently accepted and integrated at b2d257c1c0bd3ad43f39b369181172c44c4c6d23. D3 automatic capture has not been implemented or executed. This packet is a prospective finite contract, grounded in the six accepted D1 fixture byte strings and the independently replayed D2 manual collector output.

Fingerprint: 9ea37e195982bf582303ffdb83eb1fbeaba44dd69f4ed8483b4bfe39ca8d6ae8. The 68 source/data/verifier bindings are rehashed by prepare.py. It independently recomputes fixture Git blob OIDs and SHA-256, joins the accepted availability and count facts, and derives exact full/selected domains. Full capture has six physical Rust sources; selected capture has empty.rs and main.rs. A separate metadata row states the empty-workspace and missing-requested-path contract, not a fabricated production source. No selected graph hash or source OID is invented.

Fresh catalog receipts, persisted and immediately reloaded/pinned:

| Case | Expected/actual | Receipt |
|---|---|---|
| Full first half | SAT | sol_78cac1b1ed2a4d2e |
| Full second half | SAT | sol_4699de8991b149c6 |
| Selected and empty contract | SAT | sol_f7673775fb414371 |
| Healthy zero removed | UNSAT | sol_7458af22f3a74dd3 |
| Unknown fabricated as zero | UNSAT | sol_12af1d0ec4f14332 |
| Unselected source leaked | UNSAT | sol_5f0b754dbf69468e |
| Selected bound to full inventory | UNSAT | sol_b3b450a1a7524fcc |
| Read bytes/OID mismatch | UNSAT | sol_287db40e0e754a3c |

Catalog rules: data_integrity.mutually_consistent, data_integrity.unique, data_integrity.cardinality, verify mode, no unknowns. The models have at most four rows and six fields; completeness, tuple membership, exact cardinality and key uniqueness are explicit. Jev's gate opened and its exact family/mode/facts/subjects/no-unknown bindings were inspected. Jev proposed mutually_consistent only; ROOT explicitly added the two other catalog rules. Jev interpretation is not a solver result.

verify.py checks all 24 independent finite predicates, the complete persisted lowering/model envelopes, and exact case requests. It also compares eight archived complete receipts from already closed D1/D2 stages with their live envelopes before their pins were released to make retention room. Current D2 POST receipts were retained. No solver quota/configuration/state files were read or modified.

Run from the repository root:

    python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/root-pre/verify.py

Before any D3 test or production edit the worker must run this verifier and reload every current receipt through get_solve_result, comparing the complete envelopes with proof.json.gz. This pre-edit source fingerprint will intentionally become stale after source edits; retain it unchanged. ROOT's POST must bind the actual resulting full/selected artifacts, exact source bytes and native execution.

D3 requires an existing-API behavioral assertion RED before implementation, then focused runtime GREEN. This packet makes no persistence, query/cache, original incremental provenance, Cargo cfg evaluation, call-target correctness or complete panic-detection claim.

