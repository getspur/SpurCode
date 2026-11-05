# B implementation handoff — exact-owner gate rejected

Physical foreign capture is committed and the requested scoped runtime checks pass. **B is not accepted:** the strict frozen join rejects six ownership expectations in the unchanged independent inventory/B PRE. ROOT must inspect the supplied source, reconcile that packet, run fresh source/output-bound POST with persisted/reloaded receipts, and supply C PRE before approving B. C remains unimplemented.

Canonical assignment: `bd-2x4s`, delegation `18dfa07b6c864fe9`, plan `c16a0cff-63ff-4abe-83da-07a83b19df64`, task `capture-foreign-declarations`. Entry verified in-progress assignment and approved A at dispatched base `b239fd5b714e6263c7d9ea38b6553c58e177e0da`. INTENT audit `3707` persisted and replayed identically before edits/builds. All 199 PRE-bound files matched; all eight live receipt envelopes matched ROOT's saved reloads exactly (five SAT, three UNSAT, 24 predicates). See [entry.json.gz](entry.json.gz).

## Recoverable history and runtime evidence

- Test-first commit: `bcb1badc4ea9ac4c03c94a6ba12a3b828960e83e`.
- PRE/RED evidence commit: `5f524b988976e0a08b18e259c223425b5d3d5118`.
- Scoped production fix: `96bb1933a03246fb81b7abf7abda92838376d997`.
- [implementation.patch](implementation.patch.gz) is the exact base-to-fix crate diff.

The committed seed executed against unchanged production and exited 101 with **0 passed / 1 failed / 6 filtered**, at the assertion `missing physical symbol m::imported; actual: ["m", "m::defined"]`. [red.log](red.log.gz) and [red.json](red.json) retain the command/output. This was an executed missing-node assertion, not a compile or environment failure.

The first implementation run retained two failures in [green-first.log](green-first.log.gz): Parquet restores canonical row ordering, so the test now compares exact row sets; explicit unsupported Rust paths also fell back to unrelated bare homonyms, so extraction and store rebind now preserve that unresolved result. No assertion about node fields, ranges, IDs or ownership was dropped.

| Recorded invocation | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| `green-extraction` — 22 integration targets | 207 | 0 | 6 | 0 |
| `green-store` — store build unit cases | 36 | 0 | 0 | 544 |
| `green-extract-unit` — extraction unit cases | 95 | 0 | 0 | 485 |
| `green-canonical` | 5 | 0 | 0 | 575 |
| `cli-schema` | 6 | 0 | 0 | 130 |
| `cli-build` | 7 | 0 | 0 | 0 |
| `cli-temporal` | 14 | 0 | 0 | 0 |
| `frozen-export-retry` — explicit opt-in export | 1 | 0 | 0 | 6 |
| `owner-export-retry` — separate exact owner fixture | 1 | 0 | 0 | 6 |
| `small-roundtrip` — actual graph/byte/Parquet output | 1 | 0 | 0 | 6 |

These are scoped executions, not a complete workspace suite or phase approval. Exact arguments, source hashes, exits and unedited logs are adjacent `.json`/`.command.json` and `.log` files. Graph all-target check, local crate formatting and `git diff --check` passed. Existing dead-code/bench warnings remain visible.

All commands used `scripts/spur-cargo`, authorized local target `/private/tmp/rust-gap-g1-acceptance-target`, `TMPDIR=/private/tmp`, default features and one Cargo writer. Network downloads were disabled. The supplied ORT archive was hash-checked against `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`; no dependency/link/lifecycle changes were made. Disk checks reported 13 GiB initially and 12 GiB later; no cache cleanup occurred.

The six integration ignores are `full_and_incremental_emit_byte_identical_edges_for_same_state`, `incremental_matches_full_under_edit_sequence`, `incremental_rebinds_call_edge_after_callee_file_changed`, `export_frozen_declarations`, `export_frozen_graph`, and new `export_frozen_foreign_graph`. The new export was explicitly executed twice against private inputs, with zero ignores in those invocations. The eight historical direct-native failures are named separately in [historical-direct-native.json](historical-direct-native.json); none is counted as fixed, waived or fresh GREEN.

The unchanged five-row PGQ assertion (including both `serde::Serialize` rows) executed in `analyst_build_emits_external_dependency_surface_views` within the 14-case temporal target. This is an executed assertion, not a separately emitted SQL row export. CLI source and current binary identities are bound in [build-identities.json](build-identities.json).

## Source, ranges, ownership and cache

The query captures `foreign_mod_item` plus only direct declaration-list `function_signature_item` children. Full new ranges include owned outer attributes/docs and visibility; ordinary comments only remain as bytes between owned metadata and syntax. The helper stops at preceding items. Tuple range/ordinal behavior is unchanged.

Rust captures are combined before owner-dependent IDs are assigned, preserving G1's two-pass owner assembly. Extern blocks remain structural parents with their own `<extern>` FQN segment. Descendants use the nearest represented non-extern lexical scope in graph bindings, byte extraction, resolver name reconstruction, and persisted artifact names/scopes. The coordinated `tree_sitter.rs` changes are needed because fixing only binding keys would leave resolver/store names inconsistent. Explicit Rust path misses cannot be resurrected by a bare-name singleton in the store.

The extractor is the prior approved `2026-11-03-rust-foreign-declarations-v1`; schema remains v12. [source-binding.json](source-binding.json) recomputes the manifest from exact query bytes and binds it to the actual runtime export: old `36baeb745d1f8286600eaed67aec48f819546d68fe9f2a6646478db07c5e3179`, current `bf1ad64f405c4cd5222120dad7b0d5461d0b7d2f52f9aeb0bbe79c07ae61d524`. The stale-extractor runtime test selected `BuildMode::Full` for unchanged source. Only four original bound production sources and the permitted canonical snapshot changed among the 199 original bindings. Seven parsed fixture diffs change only `/macro_invocations/extractor_version`; see [metadata-diff.json](metadata-diff.json). No blanket blessing or declaration-evidence API change occurred.

## Actual frozen export and rejected gate

The 136 supplied UTF-8 files were OID-checked and materialized only under a private `/private/tmp/spur-g2b-frozen-*` root. [frozen-export.json.gz](frozen-export.json.gz) is the actual current-code graph/byte output, retained losslessly with both hashes/sizes in [export-retention.json](export-retention.json). It contains all 136 source bindings, including 133 sources with zero foreign rows. Nine functions and three blocks match independent source OID/kind/full span/name exactly. All 446 tuple records, spans, physical ordinals, IDs and direct parents are unchanged from historical production output; [tuple-preservation.json](tuple-preservation.json) records the exact joins.

**The ownership gate still fails.** In frozen `rest-table-gateway-ext/src/native_registration.rs`, OID `543d7d4432a0199706c961c67d6f022faad978d8`, extern `[2865,3363]` is inside existing represented method `impl Api::get` `[2798,4706]`, ID `fa62581bd5f960a1`. Production block ID `4c131c387c0e1c0e` has FQN `impl Api::get::<extern>` and lexical scope `impl Api::get`. The independent syn/candidate inventory and B PRE instead require file owner `6e84a632f695be3b`. The block and five child declarations therefore conflict with those expected ownership/lexical tuples. Changing production to file ownership would violate the task's nearest represented non-extern owner requirement.

The worker has not changed the inventory or B PRE and has not relaxed the check. [frozen-join-rejected.log](frozen-join-rejected.log.gz), [owner-discrepancy.json](owner-discrepancy.json), and [frozen-observations.json](frozen-observations.json) preserve the rejection and exact actual/required rows. Risk audit `3720` and signal `8ffbf4ef-fb2f-41a7-ab69-d64291882852` report the conflict to ROOT. Existing owner graph core spans and independent syn full spans remain distinct; the accepted G1 function-owner doc-span convention is retained. The recovered `tests/extension_credential_bridge.rs` stays Unknown. Eleven independent rows for unsupplied `__fixture__/held-out.rs` are not claimed validated.

The separate ROOT-supplied `owner_fixture.rs` has two existing blocks in one file/module with the same C ABI. Actual output is accepted, and nine independent mutations are rejected. The critical mutation assigns second function `abcc79bd432b6254` to existing first block `dc40b2c82b0a582c`, replacing correct block `21ac02cc3dae4c3e` while retaining lexical owner `a29cfa121c0950a5`. Both owners exist in the same file with the same ABI; FK/ABI checks alone are insufficient. [independent-controls.json](independent-controls.json) and [wrong-existing-owner-export.json](wrong-existing-owner-export.json) retain the actual controls.

## Evidence corrections and preservation

The first real frozen/owner exports passed, but the evidence runner reused their JSON filenames for command records and overwrote the data. Both first execution records/logs are retained; the corrected runner uses separate `.command.json` files and collision checks. Real exports were rerun, and only regenerated data supports observations. [evidence-correction.json](evidence-correction.json) records the error. An initial verifier also used display FQNs directly for impl IDs; the exact existing `impl_identity_fqn` convention was then applied and all physical production IDs checked. All 527 initial impl diagnostics matched unchanged G1 rows. The canonical snapshot's permitted metadata change was included in the source-diff allowlist. These verifier corrections do not resolve or hide the separate rejected owner gate.

[main-preservation.json](main-preservation.json) confirms the five protected dirty main files retain ROOT's saved hashes. No main merge, push, restart, publication, installed runtime/model change, existing notebook edit or live `../spur-notebook` rebuild occurred. The implementation and evidence remain isolated. No syntax or finite supplied-file result establishes Cargo cfg activation, macro expansion, resolved linkage, local foreign bodies, compiler-grade dispatch or complete panic detection.

ROOT can reproduce `bind_sources.py`, `check_controls.py`, and `observe_frozen.py` from this directory; `join_exports.py` must continue to exit nonzero against the unchanged conflicting inventory. ROOT POST inputs are in `ROOT-POST-inputs.json`. B remains in progress with an unresolved risk; no self-approval, epic closure or downstream dispatch occurred.

Raw command logs and the unified patch are losslessly gzip-retained; `raw-artifact-retention.json` binds original and retained hashes. The verifier hashes decompressed bytes against original command records.
