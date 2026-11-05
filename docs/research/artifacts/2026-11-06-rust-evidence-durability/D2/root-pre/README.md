# D2 fresh ROOT PRE

D1 is ROOT-approved and integrated at `b98f28a4375e916703d1629fda9a6827b7ca2f7d`. This prospective D2 contract binds **68** current source/plan/producer/verifier files, the actual D1 graph/envelope and complete-empty capture, the unchanged canonical-byte snapshot, and all **64 constructors in 32 files**. Fingerprint: `fa6d0778fc9d69cb0d44c2004949d2e42a435a7efae77d1de039c15625a9f03e`.

No D2 field/helper has been implemented or executed. The models declare required presence, canonical hash and source-authentication behavior; they do not pretend that a future D2 helper already works. The populated envelope checksum is actual D1 Rust output. The empty set SHA-256 is a finite input binding, not a claimed production BLAKE3 checksum.

The contract keeps `None` unavailable and omitted from Serde/canonical bytes; `Some(empty)` is captured and distinct. Every present envelope field affects canonical body hash, while the self-referential header content hash remains excluded. Recomputing a hash cannot authenticate a forged source/owner/body record. The capability helper belongs in D2, after the artifact field exists. Clearing that new optional field in canonical origin views is permitted; collectors and IDs remain unchanged.

Jev opened at **0.99**. ROOT inspected exact facts, verify mode, family, tuple subject and no unknowns, then added the separately discovered uniqueness/cardinality rules. Six receipts were persisted, pinned and exactly reloaded; complete lowerings/models and **18 independent predicates** passed:

| Case | Result | Receipt |
| --- | --- | --- |
| capabilities | SAT | sol_4197a1cac30c4d6d |
| hash_policy | SAT | sol_3faaf049e3374afc |
| absent_as_complete_zero | UNSAT | sol_fdbd70d708244d3e |
| evidence_omitted_from_hash | UNSAT | sol_65a016a8fc9b4a56 |
| self_header_in_body | UNSAT | sol_96a9672550d24632 |
| rehash_authenticates_forgery | UNSAT | sol_e95f03b740a04413 |

Six already-closed D1 PRE receipts were matched against complete live/archive envelopes and independently verified before releasing their pins. D1 POST remains pinned. Full retirement records and operations are saved; no quota or solver-storage edits occurred.

Before test or implementation edits on the dispatched base:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/root-pre/verify.py
```

Reload all six current solve IDs with `get_solve_result` and compare the complete saved request/status. Preserve pre-edit evidence. Changed implementation needs a fresh actual-output POST; do not rewrite or rerun this predecessor fingerprint as if it certified new source.

Follow D2 in the approved plan. The 64 cross-crate additions are mechanical `rust_evidence: None` only. Builders still return unavailable until D3; persistence, schema v13, queries and caches remain later tasks. Do not bless the unchanged canonical layout or claim complete panic detection. ROOT owns fresh POST/Jev and approval.
