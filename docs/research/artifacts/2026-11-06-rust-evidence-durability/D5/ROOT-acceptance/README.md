# D5 ROOT acceptance

D5's scoped implementation is ready for approval and local-main integration. ROOT independently reviewed the correction, confirmed durable completion audit 4077/idempotent replay, matched 1,876 current source/config bindings to the measured build, and freshly executed the exact hash-verified native binaries on AWS. No production or test code changed during ROOT final review.

Fresh graph results: **128 passed, four existing ignores, zero failures** across extractor and all seven original scoped targets. The worker's same-source graph all-targets check and formatting passed. The source-bound ROOT replay also ran analyst overlay checks: 23 passed and the single pre-existing bd-16eq failure reproduced. That failure was already demonstrated on exact pre-D5 production. D5 acceptance is scoped to transport; overall Rust durability remains in progress and bd-16eq remains open for the overlay stage. No ignored/filtered test was substituted for the failure.

The exact canonical legacy identity fix rejects all 11 malformed names while 13 recognized canonical absence controls load. The Rust golden's only parsed change is /rust_evidence/origins/0/manifest_version; all five non-Rust goldens remain byte-identical. The original implementation, both committed runtime REDs, earlier failing outputs and original authored commits remain available.

ROOT-native evidence:
- [Replay driver](replay_native.py) checks every current source/config and fixture binding plus exact executable hashes before native execution.
- [Whole native archive](root-native-outputs.tar.gz) contains 192 current profiles and ten fresh independently mutated disk probes, including actual schemas, metadata, row values, manifests, Parquet bytes, full/slim envelopes, source bytes and reader/source-authentication verdicts. SHA-256: a853cefc0e2327a28a3482ae234a63985ffeab46cc2ca4e552c0c5a270ff71ae.
- All 155 original corruptions and 11 alias controls reject in both readers. Both rehashed existing-body/owner forgeries pass transport integrity but fail saved-source authentication, as required.
- Ten fresh ROOT probes cover baseline, missing zero-row source table, a valid-Parquet same-count call cell mutation, checksum/schema mismatch, canonical v12 and signed/zero-padded aliases. Their raw requests and whole resulting disks remain in the archive.
- None, Some(empty), a complete zero-row source, Unknown lanes, retained snapshots, composed original inventories and changed/zero/Unknown/no-change/deletion deltas retain their distinctions. No slim/history full-artifact hash or complete panic-detection claim is made.

Run from checkout root:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D5/ROOT-acceptance/verify_post.py
```

Final fingerprint: **a95f13abcd56b7c44c71f58168a229a4cdddc647be49be97ebdb9a5c859ff7e6**.
The verifier binds **1,903** source/evidence inputs and independently evaluates all finite decisions and compiler-lowered predicates, variables, fixed values, complete models and receipt identities. It rederives source OIDs/lengths and call/body joins from frozen raw bytes and predecessor collector witnesses. Typed transport rows are compared with pre-write collector rows, while source authentication remains a separate native check. Complete ordered tuple commitments use SHA256 to keep the finite models bounded; their full observation tuples and derivation are retained in post-inputs.json and the archive.

| Persisted and fully reloaded POST | Status |
|---|---|
| sol_a5778192d8c74012 — physical table contract | SAT |
| sol_59ba134448a245b8 — independent raw source identities/lanes | SAT |
| sol_f420e05dae8846f2 — typed rows and raw call-owner geometry | SAT |
| sol_78f370a027f14431 — roundtrip profiles 0 | SAT |
| sol_1376bdf3d49e4587 — roundtrip profiles 1 | SAT |
| sol_69f4eeede7c448ce — all corruptions/aliases/canonical absence | SAT |
| sol_11fe4c8a1f4f45f1 — fresh ROOT disk mutations | SAT |
| sol_100946d0d8cd4ae7 — independent false acceptance/zero/owner claims | UNSAT |

All **38 independent predicates** match expectations. The final model has three separately failing exact-tuple predicates; its wrong existing-body FK still passes, demonstrating why membership alone is insufficient. Jev returned an open data_integrity/verify candidate for tables_exact with identical supplied facts; ROOT explicitly added every other required rule before solving. Interpretation confidence and Z3 verification are separate records.

Retention was full. Before releasing pins, ROOT saved and exactly compared the complete live envelopes for the three consumed original D5 PRE receipts, two consumed rework PRE receipts and three superseded closed-D3 domain partitions. Their **46** independent predicates replayed successfully. retired-archives.json.gz, retired-live.json.gz and retirement-operations.json preserve these records. Only explicit pin=false calls were used; no solver store/quota files changed. All seven current D4 POST receipts remain pinned, alongside these eight D5 POST receipts.

Source policy: the two pre-existing user-edited ACP files are bound to committed HEAD bytes, matching the isolated tested snapshot. Every other input uses current bytes. The five protected user files on main remain untouched. D6 remains gated on D5 approval and integration; publication/cache behavior is not claimed by this transport acceptance.
