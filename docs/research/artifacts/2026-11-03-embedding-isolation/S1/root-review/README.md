# S1 root review — request changes

Candidate: `80e6549f95b599a37ccf10ccfd931b6d8fa59a27`.
Authored worker head: `2ab0ebcb8d33887a597debe790437e6a929178dd`.
Task: `bd-339u`; Plan: `6f98d9fa-715e-4aed-9cef-73ad302d937f`.
The graph did not contain the new crate, so review used exact Git candidate source after the graph-first attempt.

## Confirmed defect

`Request::Shutdown` and `Reply::Closed` silently accept additional JSON fields at the variant-object level. Outer envelope and ModelIdentity checks do not cover this. Both contradict the approved strict-schema contract. The root regression file is `crates/spur-embedding/tests/root_wire_review.rs`; its assertions expect both payloads with `extra:0` to be rejected.

The recorded command in red.json uses SPUR_REMOTE=0, TMPDIR=/private/tmp and the shared target through scripts/spur-cargo. It compiled, ran two assertions and exited101 with 0passed/2failed; both raw read_frame results are Ok. See red.log and red.json. No production code was modified during this review.

Worker's original 15 exported observations, six source hashes, saved raw observation log/dependency-tree hashes and saved test-binary hash were independently checked and matched. The original test-first chain (including compile-only REDs and the genuine handshake assertion RED) remains retained; this review does not relabel its successful observations as failures.

## Jev POST and catalog evidence

The original broad Jev request had a blocked completeness gate (0.59), retained in post.json.gz. A direct catalog diagnostic then reported the 64-variable snapshot limit before execution. Neither event is UNSAT or proof. All observations were retained in disjoint bounded case partitions, with separate global identity/coverage and source/native metadata checks.

All three new bounded Jev calls opened after exact request inspection; no threshold was lowered and no case was dropped. Jev compiles one binding; additional global/negative catalog bindings are explicitly direct, not attributed to Jev.

| Actual case scope | Result | Receipt |
|---|---|---|
| Worker recorded cases0–7 | SAT/pass | sol_52933cf7db874bed |
| Worker recorded cases8–14 | SAT/pass | sol_eebdaf6b3be54abd |
| Actual unknown Shutdown/Closed fields | UNSAT/fail | sol_e3ed6256e8b84a01 |
| Global17 identities, count and presence | SAT/pass | sol_0974d946e80142bf |
| Exact source bindings and native-free closure | SAT/pass | sol_3c1cbee2ffd7461f |

Controls reject omission sol_9681ea968d88486b, duplicate sol_b403bfb47bfb4845, stale source digest sol_f6e105aaf53f4785, injected ORT dependency sol_2c9d4344715e4210, inverted valid hello sol_5ed0c08b287945cf, inverted wrong-version outcome sol_da6eb7dfe9b245a1, inverted fingerprint outcome sol_c9d8e59a600e4c12 and inverted nonfinite outcome sol_9d86865363c641b2. All13 POST/diagnostic receipts were persisted/reloaded, with exact original family requests and returned lowered queries retained in post.json.gz.

This rejects the current augmented implementation snapshot under the finite declared policy. It does not prove parser completeness, production subprocess containment or native memory safety.

## Fresh rework PRE

The root recompiled the unchanged required policy with prospective repaired outcomes for the two cases. The Jev gate opened; this is a repair target, not a claim the code is already repaired. Target SAT/pass sol_c552ee9e185a4ece; still accepting Shutdown fails sol_ae996c56d5bd41de; still accepting Closed fails sol_b20d44499efc4dae. All3 persisted/reloaded. Full inspectable input/provenance/requests/receipts are in pre-repair.json.

## Next action

Request a focused S1 rework: preserve all source/evidence/TDD history, reproduce these REDs before modifying production, enforce strict unknown/duplicate-field behavior at every request/reply variant level, retain valid wire shapes and session semantics, and refresh actual-output provenance/POST candidates. Root will run fresh actual-output Jev/SOLVE POST before approval. S2 remains pending.
