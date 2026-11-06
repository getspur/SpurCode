# D6 ROOT acceptance

D6 is ready for scoped approval and local-main integration. ROOT reviewed the single-module implementation and committed TDD history, confirmed durable completion audit 4100 and replay, and independently checked the worker's nine command records, four complete native archives, PRE envelopes and source/code hashes. No unrelated production file or policy constant changed.

Fresh ROOT native replay, after matching every source/config binding and exact executable hash, reported **31 cache tests and 46 integration tests passed**, zero failures, and one existing cross-device ignore. The same-source worker all-targets check and local formatting passed. ROOT then ran ten independently authored active native replay requests in separate temporary fixtures.

The changes compare full Rust envelopes before direct publication and canonical dedup/pointer updates. Cache keys include canonical artifact path, Rust capability and every parsed manifest field; cold loads check decoded capability/counts and reread the manifest before caching. The existing generic later pointer-I/O failure ordering remains intact.

Evidence:
- [Replay driver](replay_native.py), exact script, launch metadata and lossless logs reproduce ROOT's calls to measured native binaries without compiling or editing production.
- [Whole native archive](root-native-outputs.tar.gz) contains 37 native observations, actual canonical paths/keys, loaded/retained envelopes, raw/parsed manifests, pointer bytes and ten fresh request/replay fixtures. All **917 captured files** are hash-bound. SHA-256: da81735bc23a4e14b4fa130f5c5687f32fd9896bb816fd08c1ac852ac1bf5511.
- Six different-envelope direct/dedup publications reject while both pointers and canonical manifests remain unchanged; four matching controls publish.
- Seventeen malformed declared headers return no seed. Changed capability, path or valid header identity does not alias the immediately retained snapshot. Existing immutable Arcs retain their original meaning.
- An unchanged declared header may keep a validated hot Arc after table mutation; cold decoding at a fresh path rejects the missing table. This explicitly does not claim continuous monitoring of table bytes.
- Fresh ROOT requests cover present/absent/zero transitions, replacement paths, core and Rust incompleteness/count changes, a valid declared Unknown checksum transition, missing-table hot/cold behavior and a malformed legacy alias. The Unknown fixture is an explicit stored mutation, not a parser availability claim.

Run from the checkout root:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D6/ROOT-acceptance/verify_post.py
```

Fingerprint: **22643d53948d9cc9016a97594357aabb77028977c07222bfde1a462c9379323f**.
The verifier rederives **1,909** current source/evidence bindings, full native joins and complete tuples, then independently evaluates all finite decisions, compiler-lowered variables/fixed values/predicates/models and receipt identities. SHA256 tuple commitments keep the finite models bounded; raw observations and their complete derivation remain in the archive and post-inputs.json.

| Persisted and fully reloaded POST | Expected |
|---|---|
| sol_72a3e1c9d07449e3 — loaded capability/path/manifest bindings | SAT |
| sol_c98ef8fe3b814830 — changed headers, immutable hot and cold rejection | SAT |
| sol_fd5d44977d904579 — direct/dedup publication and pointer safety | SAT |
| sol_1eb9390f83c946a9 — independently authored ROOT native requests | SAT |
| sol_4ff19667b89e4d3b — false capability, aliased path key and pointer claims | UNSAT |

All **21 independent predicates** match expectations, including three separately failing negative bindings. Jev returned an open data_integrity/verify candidate for loads_exact with identical supplied facts; ROOT explicitly added the remaining catalog bindings. Interpretation and solver verification remain separate.

Retention: before releasing the two consumed D6 PRE pins and three superseded closed-D3 POST pins, ROOT archived complete live envelopes, compared them exactly and independently replayed their 21 predicates. The retirement files preserve these records; only explicit pin=false calls were used, with no solver/quota file edits. All eight current D5 POST and seven current D4 POST receipts remain pinned alongside these five D6 POST receipts.

The two pre-existing user-edited ACP sources are bound to committed HEAD bytes, matching the isolated tested build; all other inputs bind current bytes. Five protected main files remain untouched. The wider durability phase remains in progress, including separate pre-existing analyst overlay bug bd-16eq. D7 is gated on D6 approval and integration.
