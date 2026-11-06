# ROOT D4 acceptance

D4 is ready for ROOT approval and integration after independent source review, fresh native replay,
the test-only legacy-fixture correction, and exact actual-output POST verification.

Canonical task: `bd-1qn9`; plan: `527c672b-ee30-4641-94e9-9c9166f36336`.
Reviewed normalized worker tree: `076c4cb2018e5aca54b509b929196263b26315f0`.
Authored history through `74dcae668` is protected at
`spur/review-evidence/g3-d4-attempt3-authored`; attempt-2 TDD history is also protected.
ROOT RED evidence is commit `5b18a7497ed22329623b81a1aea87be951284dff`.
The legacy fixture correction is `652bd154b9a9d23492a946903396981a76586614`.

## Result and scope

Unchanged Rust evidence retains its complete original file object and whole origin inventory.
Changed sources retain the D3 selected snapshot as origin; aggregate composition does not rebind them.
Deletion removes active evidence and prunes unused complete origins. Complete-zero and Unknown
remain distinct. Legacy/incompatible carriers force full rebuild; compatible reused forgery errors.
Gitlinks keep their commit-pointer manifests, graph hashes and original-inventory membership while
remaining excluded from physical Rust source reads and syntax coverage.

The worker implementation touches the five approved graph source/test paths. ROOT additionally
corrected two obsolete D2 fixture setups in `tests/rust_evidence_contract.rs` (17 diff lines):
take and authenticate the current automatic capture, explicitly form the legacy absent fixture,
and require automatic capture to equal canonical recollection. All original byte, checksum,
capability, typed mutation and forgery assertions remain. Production behavior was not changed
by this compatibility correction.

D4 is in-memory composition. Parquet Rust transport, publication/cache identity, durable queries,
overlay transport and notebook acceptance remain later gates. This is not a panic-completeness claim.

## Verification

ROOT independently reran the six measured final AWS executables after checking their exact hashes
and all 1,682 compiled-source bindings. The required suites passed:

| Suite | Passed | Failed | Existing ignores |
|---|---:|---:|---:|
| Rust evidence build | 20 | 0 | 0 |
| Store build units | 38 | 0 | 0 |
| Extractor/core parity | 55 | 0 | 3 |
| Git incremental | 28 | 0 | 1 |
| Macro artifacts | 8 | 0 | 0 |

The supplemental suite reproduced 16 passed / 2 failed before correction. Both failures were the
obsolete legacy-fixture assumptions. After correction, all 18 passed on AWS, and the exact committed
formatted source was recompiled and passed again. Counts describe 167 test cases, four pre-existing
ignores and no remaining failures; conditional replay adapter entry tests are not additional
standalone native API replay claims.

Worker all-target compilation passed. ROOT's changed test target was compiled again; scoped
formatting and complete diff checks passed. Full raw native logs are compressed without altering
their original bytes. Contract command/source records and the final AWS source/executable identity
are in `contract-compat/`. Remote-only Cargo runs used the repository wrapper and existing cache.

The initial scoped format check reported one wrapping change. ROOT applied formatting and reran
the contract against the exact final committed source; both functional and final-source runs are
retained. No local substitute was used for a remote compiler/test failure.

## Actual native observations and POST

ROOT's fresh `actual-current/` contains 16 packets: full/no-op/edit/delete/readd/pruned empty,
private changed/empty/deletion-only deltas, eight compatibility cases, six forgery controls,
reused-source race, mixed gitlink transitions and Rust/Python source-drift controls.
Exact SSM request/results, capture bytes, source bindings and executable identities are retained.

Independent checks derive Git blob and commit OIDs from exact bytes, compare the full six-source
capture with accepted D3 data, check exact unchanged file objects and complete historical origins,
validate selected-vs-aggregate provenance, check pointer-change/deletion/pruning and verify current
stats. Each of the six native forgeries is rejected before and after genuine native rehash.
The omission and Unknown-to-zero controls remain shape-valid but fail exact source validation.

POST fingerprint:
`5f810f730bc5f329391894793890845316e6f5b5e38697cf1db3bf0a0581e7e3`
(1,916 bindings).

| Case | Status | Persisted receipt |
|---|---|---|
| before/no-op/edit epochs | SAT | sol_822e7acf2dd24e92 |
| deletion/readd/pruning epochs | SAT | sol_4d8dadb8f1fd4b6f |
| exact edited whole-file payloads | SAT | sol_9071f8185a534c9d |
| native mutation rejection before/after rehash | SAT | sol_4cc388cab38d4fd4 |
| gitlink and stale-source boundaries | SAT | sol_68d4a728d82644a6 |
| absence/version rebuild policy | SAT | sol_b1d29131860e48ff |
| independent rebinding/false-authentication claims | UNSAT | sol_e6da1e9336af473d |

All seven receipts are pinned, persisted and reloaded. Jev's gate was open; ROOT inspected exact
family/mode/facts/subjects/unknowns, then added catalog unique/cardinality bindings explicitly.
The verifier independently checks 24 finite/lowered predicates, complete requests/models/cores
and both separately failing negative bindings. The proof is for these exact finite observations.
Native BLAKE3 output is retained; the Python verifier does not invent a second BLAKE3 implementation.

The original seven preimplementation D4 PRE receipts had been fully consumed by successful TDD
and native verification. Their complete live envelopes were matched to immutable archives and all
21 old predicates independently checked before unpinning for the seven actual POST replacements.
All eleven D3 POST pins and both supplemental boundary PRE pins remain protected.
No retention quota or solver store implementation was changed.

## Source snapshot and replay

The native replay binds the unchanged production/API harness compiled in the worker tree.
The one intentional compiled-source difference is the separately corrected and recompiled legacy
contract test; its final native source/executable hashes and full green command are checked.
The verification covers the committed isolated D4 snapshot. Existing user changes to the two
ACP adapter files are preserved outside that snapshot: their committed HEAD bytes are explicitly
bound, and no claim is made that those uncommitted edits were built. All other input bindings use
current file bytes. The five protected user files must remain unchanged during integration.

Run:

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/ROOT-acceptance/verify_post.py
```

Review the capability boundary and test limitations above alongside the exact evidence.
