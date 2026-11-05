# M1 independent root review

The focused metadata implementation is accepted by the independent checks below. The full embedding-default migration remains in progress. Source issue: `bd-1l17`; plan: `dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`.

The production change is limited to an early `--compatibility` return and making JSON a normal worker dependency. The worker's raw test commit is `2fb09db4220ce0bb3d1162f3c91e97849bcd5a07`, fix is `2c3c13dd191b14ebfe86d0e0218637b5408d3642`, and normalized submission is `9a20c8bb3254c0a91695893181ba0a70e998a93d`. The authored and normalized source trees agree; protected review refs retain the original commits.

Root independently built baseline `c4253c5b704d8fc302d8ca4452b5fe4a26079fbe` and the candidate with ordinary default features, in sequence. Both compiler artifact inventories show `features=[]` and `profile.test=false` for the worker and neutral crate. The baseline rejects metadata with the old seven-limits error; the candidate returns clean JSON with both model and native cache paths unusable:

```json
{"arch":"aarch64","os":"macos","protocol":2,"version":"1.24.0"}
```

The source version/protocol constants and native build target independently match those fields. Candidate binary SHA256 is `614ad71d173741d7938cb46344a9edaa18416a2a4b0e80f066ba0604e3297a59`; the baseline is `db6a2d485363148687a1909006a6895877ab5e8b63ad2c928e4ac67f2a720579`. The original worker binary was frozen before shared-target reuse and retains SHA256 `dc4628e3639df6eb7d73676865ed6afa70c0483678310746414938a278e42122`; its historical live-target verifier must not be mistaken for a check of a later rebuilt binary.

| Fresh root verification | Result |
|---|---|
| Scoped worker tests | 10 passed, 0 failed; 6 named model metadata cases filtered |
| All-target, all-feature check | Exit 0 |
| Crate formatting | Exit 0 |
| Ordinary argument comparisons | All 5 match the fresh baseline |
| Invalid metadata argument combinations | All 6 reject with empty stdout |
| macOS loader trace | 853 lines, 0 ONNXRuntime matches |
| Jev POST | Open, weakest confidence 0.97; exact supplied facts retained |
| Catalog POST | 1 SAT, 9 UNSAT; 10 persisted and reloaded receipts |
| Independent finite oracle | All 30 rule predicates and fixed assignments match |

POST selects `data_integrity.mutually_consistent`, `data_integrity.unique` and `data_integrity.cardinality` over eight actual observed tuples bound to the 40-file source fingerprint `baafa46271a6b66399c914df92d3b557add71cd93cb2312dfb16d743a5d49ae3` and the fresh binary hash. Positive receipt: `sol_f2d6b647cead48b8`. Rejections are synthetic tuple mutations, including wrong protocol, missing target, duplicate case and unbound source; they are not fabricated runtime observations.

The artifact-count limit of 512 interrupted a control before persistence. All eight completed M1 requirement PRE receipts were reloaded and checked exactly equal to the already committed archive at `c4253c5b7:docs/research/artifacts/2026-11-05-embedding-sol-continuation/proof.json.gz`, then unpinned. They had no running consumer. The last two controls subsequently persisted and reloaded. New POST receipts and the older captured-fixture controls remain pinned. The quota error was not interpreted as a solver verdict.

Exact commands, environment selections, raw compressed logs, artifact inventories and source identities are adjacent. Verification:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M1-review/verify.py
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M2-pre/verify_pre.py
```

These results apply to the recorded macOS ARM64 development builds. The unusable caches, early return and loader observation corroborate no ORT initialization on this command; they do not prove absence of every filesystem read or establish product inference, ownership safety or other platforms. Root prepared the concrete [M2 PRE](../M2-pre/README.md) before releasing the successor.

The root extension contains review evidence and the successor entry gate only. The existing specification context receives an entry link so no plan-spec rewrite can accidentally discard the user's explicit `codex / gpt-6.1-sol / xhigh` settings. The plan's single-parent branch inheritance is implemented in `crates/spur-core/src/plan/reconciler/base_spec.rs`; the completed worker branch is fast-forwarded with this documentation extension before approval.
