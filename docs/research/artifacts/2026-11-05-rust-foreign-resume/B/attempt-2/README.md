# B owner-oracle repair — independent join accepted, ROOT review pending

The bounded rework passes its independent gate. Production extraction is unchanged. The legacy inventory and its rejected join remain immutable; the new current-binary export passes the unchanged strict join with ROOT's explicitly versioned owner addendum. B remains `in_progress` with risk open for ROOT disposition. Fresh whole-B ROOT POST and C PRE are still required before approval or C work.

Canonical assignment is `bd-2x4s`, delegation `bd0f81e7112143b2`, plan `c16a0cff-63ff-4abe-83da-07a83b19df64`, task `capture-foreign-declarations`, attempt 2. INTENT audit `3739` persisted and replayed identically before edits/builds. Progress audit `3740` records the intermediate observations. [entry.json.gz](entry.json.gz) retains complete live envelopes for the five amended receipts: three SAT (`sol_3f75112f37d4425a`, `sol_6b6fac45a7bb4466`, `sol_f8e50d641d314a12`) and two UNSAT (`sol_d39a3c0967df4ce0`, `sol_66651c6a9f2f44ed`). All exactly equal ROOT's saved reloads. The entry verifier checked 224 files, fingerprint `213f07c1133728a45c50f81440e9ecc72cfb0e86e94beb750966badc7bfbbd36`, and 15 independent predicates. This PRE is a finite declared contract.

Recovery commit `2f9e55dc6` has the exact tree of ROOT's completed result `d2c12c3810e6b0e449f59c0371001c7e17c204a6`. Test commit `1f0a8e1a4` precedes the observed oracle RED. Original production history remains protected: test `bcb1badc4`, RED `5f524b988`, fix `96bb1933a`, evidence `91a587c92`; authored and normalized trees still match. Both original B and ROOT packet hash manifests verify unchanged.

## Actual RED and correction

`oracle_regression.py --oracle legacy` exited 1 at the method-owner assertion: the old source visitor inferred file ownership for extern `[2865,3363]` physically inside `Api::get` `[2798,4706]`. `--oracle corrected` exited 0 for all 136 supplied sources, 12 foreign rows, six corrected lexical rows and six ROOT visitor controls. These commands exercise the archived independent syn 2.0.118 visitor outputs and approved addendum; they do **not** claim a new parser run. ROOT's executed visitor source/output/readback provenance is bound separately by [owner-addendum-v1.json](owner-addendum-v1.json).

The fresh actual export also reproduced rejection through the unchanged `B/join_exports.py`: `join.py --legacy` exited 1 with `exact preexisting production owner ID`. `join.py` exited 0 with the corrected candidates. Logs and command records preserve both observations. This is an oracle/evidence correction; the original production missing-node RED remains valid and no additional production failure was fabricated.

The version `rust-foreign-owner-addendum-v1` changes exactly one historical candidate, and only its `owner` and `owner_id`. Five foreign functions keep their structural block owner; their lexical owner changes transitively. The copied corrected candidate file is byte-identical to ROOT's approved copy. Original syn rows, candidate inventory, PRE, first handoff, raw failures, logs and hash manifests are unchanged.

## Fresh executions and actual outputs

| New invocation | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| `foreign-green` | 7 | 0 | 1 | 0 |
| `frozen-export` | 1 | 0 | 0 | 7 |
| `owner-export` | 1 | 0 | 0 | 7 |

The focused regression uses the exact frozen `native_registration.rs` bytes (OID `543d7d4432a0199706c961c67d6f022faad978d8`) through real `build_facts`/`artifact_from_facts` and `BytesExtractor`. It checks preexisting method ID `fa62581bd5f960a1`, range `[2798,4706]`, lines `[78,121]`, anchor, method FQN/scope, direct block ownership, and all five full-span declarations with exact IDs and stable-ID inputs. Graph and byte fields agree. The executable verifier additionally compares the actual method graph row to G1 and checks its byte projection. [method-export.json](method-export.json) is actual output.

All three Cargo invocations used `scripts/spur-cargo`, defaults, the authorized local target/TMPDIR, one Cargo writer, offline mode, the existing DuckDB extension path and hash-verified pinned ORT archive. The sole ignored test, `export_frozen_foreign_graph`, was then explicitly executed twice; both export invocations had zero ignores. Their seven filters select that exact export test. Existing dead-code warnings remain visible.

The 136 OID-checked frozen inputs and separate owner fixture were materialized only in private `/private/tmp/spur-g2b-*` directories. No live project was rebuilt. [frozen-export.json.gz](frozen-export.json.gz) is the losslessly retained fresh 34,928,662-byte actual graph/byte export. Its entire decompressed output equals the immutable attempt-1 export, SHA-256 `7af6fb298be43a1cdb5ac5dfdfb6624c766e8bb74a900c2b5e1dbd66b44e17bd`; retained gzip SHA-256 is `9ff4d1261901e6f0016a150f14c7a9e1b79370f616ebf96c488f47d7341758df`. Expected rows never construct observed nodes.

[frozen-join.json](frozen-join.json) accepts all nine functions and three blocks by exact source OID/kind/full span/structural and lexical identity, with unique direct owners, body/ABI source syntax and byte projections. It independently preserves all 446 tuple records/ordinals/IDs/spans/parents, all 136 source bindings, and 133 zero-row sources. The recovered `tests/extension_credential_bridge.rs` stays Unknown; 11 unsupplied held-out rows remain unvalidated.

All nine fresh independent mutations were rejected, including a different **existing** block in the same file with the same ABI. The mutation rewires second function `abcc79bd432b6254` from block `21ac02cc3dae4c3e` to existing block `dc40b2c82b0a582c`, changing both Contains and Defines while preserving lexical owner. The remaining mutations cover wrong lexical scope, leaked extern namespace, full span, forged ID, missing/duplicate physical nodes, stale source OID, and missing zero-row source. [independent-controls.json](independent-controls.json) and the actual mutated export retain the results. The control checker is unchanged except for explicit attempt-2 input/output locations.

## Reused verification and current identities

The original **373** scoped Rust executions and graph all-target check are reused and their raw logs/source hashes are freshly revalidated. They are distinct from the **nine** new Rust executions above. The original graph-byte/store/lexical/receiver/macro/ownership/tuple/extractor/canonical and CLI schema/build/temporal coverage, named six integration ignores, and eight historical direct-native failures remain documented in the immutable [first handoff](../README.md). No broad suite or historical native fix is newly claimed.

Only the authorized test differs among the 224 amended PRE-bound files; all production/default-feature/declaration-API sources match. The unchanged CLI temporal test file hash is verified against its original whole-file identity; its original five-row PGQ assertion is preserved. Original CLI binaries still match their saved hashes. The current foreign-test binary SHA-256 is `881963320bf8b1c81aa74ef03cd1c374e19dab12cddf7fe84b0d5660e7d12705`; all paths, compiler/wrapper identities and command environments are in [build-identities.json](build-identities.json).

The schema remains v12 and extractor identity remains `2026-11-03-rust-foreign-declarations-v1`. Current source-derived manifest `bf1ad64f405c4cd5222120dad7b0d5461d0b7d2f52f9aeb0bbe79c07ae61d524` equals the fresh actual export and original B manifest. The verifier rechecks all seven permitted parsed golden/snapshot diffs as exactly `/macro_invocations/extractor_version`. Local crate fmt and scoped diff checks pass. The five protected dirty-main file hashes still match; no notebook, global runtime/model, installed binary, live notebook graph/CURRENT, main merge, restart, push or publication changed.

An evidence-builder first compared the CLI whole-file SHA against the original assertion-region hash field and failed; its dependent verifier then correctly refused the missing binding. Both failures are retained in `post-first-failure` and `verifier-first-failure` logs. The builder now verifies the original whole-file hash field. Environment observations separately retain the initial Git sandbox write error, invalid fmt flag and unavailable process listing. None is counted as RED/GREEN runtime evidence or hidden as a passing check.

## ROOT handoff and replay

Run from this checkout:

```sh
python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/B/attempt-2/verify.py --require-hashes
```

The verifier composes the immutable first attempt with the exact approved addendum and fresh actual outputs. [ROOT-POST-inputs.json](ROOT-POST-inputs.json) supplies current source/build/output hashes, actual twelve-row joins, all 446 tuple joins, 136 source/zero-row bindings, controls and original packet provenance for ROOT's fresh whole-B POST. [source-binding.json](source-binding.json) binds 232 current files to fingerprint `296bba690cb14b2092c9d218ba87c9ba6c4c6fdbfdfd0dccf025e05b71e6c136`.

Cargo logs are losslessly gzip-retained with original hashes in [log-retention.json](log-retention.json). The initial diff check flagged only their raw terminal blank lines; compression preserves those exact observations and permits the final scoped diff check to pass.

This accepted independent join is not B approval or a ROOT POST receipt. ROOT must run fresh persisted/reloaded POST and independent review, dispose of the open risk, and supply C PRE before approval. C remains pending. Source syntax and finite supplied-file coverage do not establish Cargo cfg, macro expansion, compiler-grade dispatch, resolved linkage, local foreign bodies or complete panic detection.
