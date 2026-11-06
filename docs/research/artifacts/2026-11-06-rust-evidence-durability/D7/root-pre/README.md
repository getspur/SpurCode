# D7 ROOT PRE

Predecessor: integrated local main `992f100d619b045f7ad1643352da6e4e093c858b`; D6 canonical `bd-21oj` is approved and closed. D7 source issue is `bd-2o7z`. No D7 production implementation or test execution is claimed by this packet.

`python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7/root-pre/verify.py` passed. It rederives the accepted D6 native POST exactly, hashes the D5 native archive, independently checks raw Git source OIDs and byte lengths, body spans and innermost call ownership, per-file row counts and both availability lanes, then checks all finite and lowered solver predicates and complete receipt envelopes.

The model contains 20 **prospective** selections/lifecycle requirements, including seven physical sources, healthy zero, cfg/recovery/invalid Partial, exact and wrong-existing OIDs, invalid shape with absent evidence, missing/unsupported sources, legacy known domain, present-empty versus absent, custom unknown domain, immutable hot data and corrupt cold data. Full nested expected payloads are retained in inputs.json; SHA-256 compact tuples reduce solver variables without claiming that hashing authenticates new API execution. FFI declaration counts include foreign_functions and extern_blocks as well as tuple rows. Source-filter pruning must never hide malformed metadata or owners.

Jev gate: open, data_integrity / verify / mutually_consistent, unchanged facts and no unknowns. The complete execution additionally binds the freshly discovered unique, cardinality and conditional_required rules. Three persisted and pinned receipts cover 11 predicates:

| Case | Raw status | Receipt |
| --- | --- | --- |
| Prospective queries 0–9 and native known-domain requirement | SAT / pass | sol_5aee4f82ef334308 |
| Prospective queries 10–19 | SAT / pass | sol_68958d620d3947c6 |
| False zero, stale-OID, absence, corruption, freeze and domain claims | UNSAT / fail | sol_e23bc8fbc3b64cb2 |

Fingerprint: `0e1e0171c6ece0c400a7ee97fc10774d8cb4fa734a34cca02856bbfdeda807ce`; 1,925 source/evidence bindings. Solver results prove only the finite encoded contract. A real assertion RED and native query output remain implementation and POST obligations.

Retention: three closed D4 POST partitions are archived here with their complete live envelopes, compared exactly and checked against nine independent finite/lowered predicates before unpinning. They remain historical evidence; accepted D5 and D6 POST receipts stay pinned. No solver quota or cache files were edited.

Dispatch follows worker-spec.md: codex/profile codex, gpt-6.1-sol, xhigh; one coupled crate-local task. Commit compile RED accurately, then a compiling assertion RED before implementation. Preserve actual native memory/disk responses and IO/cold-read outcomes for ROOT's fresh Jev POST and independent mutation replay. D8–D10 and existing analyst overlay issue bd-16eq remain outside this task.
