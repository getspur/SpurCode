# M05 current-source grounding — prospective only

Base `f607890314afce94c4824b27f04122c2389d31a6`. Canonical issue **bd-kte7**, plan `6313fc38-d63e-4568-ba4d-d734426633f7`, task `M05-accessor-grounding`, new authenticated delegation **b966f0a8ede74530**, assignee `spur-worker-b966f0a8ede74530`. `get_issue` and verified plan projection showed in_progress/dispatched, no supersession and the correct dispatched base. `report_audit` authenticated this delegation and persisted INTENT **4821**, UUID `c06353f7-c6db-45db-8645-8a8b7a00a4e8`. Requested agent/profile/model/effort remains codex/codex/gpt-6.1-sol/xhigh; PM does not separately attest the executing model.

Recovery: `git log f607890314afce94c4824b27f04122c2389d31a6..HEAD` returned no commits; initial status was clean. No prior attempt output was reused. Current projection has only the new dispatch (attempt 2); prior `498d3db0738040ba` is absent. Direct old-process termination is not exposed to this worker, so ROOT must retain its fence. No old issue/dispatch was used as an audit target. Branch uses brain-session `99eaf8bce1ebb355`; successful audit identifies the worker delegation independently of that branch name.

Exactly three documents are the output. No source, tests, Cargo, approved spec/plan or prior evidence changed. No tests, builds, instrumentation, Rust formatter, VM repair, cleanup, solver operations, receipt retention changes, main merge or issue closure occurred. This is neither PRE approval nor M05 acceptance. ROOT must bind fresh PRE before any implementation. A mechanical migration has no fabricated behavioral RED: preserve genuine baseline outcomes, then obtain same-input native equality.

## Source inventory and changes

Approved spec and plan, historical source map and M04 ROOT README were consulted. The old map is grounded at 457b9f70f, so its line numbers are orientation only. Fresh knowledge_context_pack_2 returned zero primary code evidence, low confidence and three unrelated documents, graph `50a577266ca7a91f1c141e67015a18b7d137ed44d008563712451563e8a71ad7`, exact_graph_verified=false. Exact code_read_symbol for schema.rs/rust_evidence_set returned `Mcp error: -32603: internal: code graph tool code_read_symbol exceeded 15000ms deadline`. ROOT previously reported index992f100d against HEADf607. Bounded raw current reads were the explicitly authorized fallback; all 33 bound files were compared byte-for-byte to Git base and have byte length, SHA256, blob OID and line count in source-map.json. No current graph precision certificate is claimed.

There are **15 mechanical field occurrences: 8 reads, 3 writes, 4 literal fields in 2 constructors**, within exactly three production files. There are **31 other named occurrences** classified individually as query methods/calls or module paths and left unchanged. No manifest metadata or typed row-count access/literal is present in M05; no GraphFacts access is present. Five literal legacy evidence basenames in the query_client frozen-I/O test are separately mapped protected path controls and stay unchanged. Corresponding dependencies remain read-only. Source-map ranges also bind enclosing symbols, raw range hashes, byte offsets, proposed operations, field types, validation order and existing reachable tests.

| Site | Current source | Enclosing symbol | Kind |
| --- | --- | --- | --- |
| Q01 | query_client.rs:71–73 | MacroQueryContext::new | field_read |
| Q02 | query_client.rs:75–88 | MacroQueryContext::new | field_read |
| Q03 | query_client.rs:379–393 | impl GraphQueryClient for InMemoryClient::macro_invocations | field_read |
| Q04 | query_client.rs:545 | empty_overlay_delta_artifact | literal_field |
| Q05 | query_client.rs:546 | empty_overlay_delta_artifact | literal_field |
| Q06 | query_client.rs:1461 | ParquetClient::load_macro_context | field_write |
| Q07 | query_client.rs:3038 | tests::artifact | literal_field |
| Q08 | query_client.rs:3039 | tests::artifact | literal_field |
| R01 | rust_evidence.rs:306 | impl RustEvidenceSet::validate_sources | field_write |
| R02 | rust_evidence.rs:307 | impl RustEvidenceSet::validate_sources | field_write |
| R03 | rust_evidence.rs:362–370 | rust_evidence_capability_digest | field_read |
| E01 | rust_evidence/query.rs:159–162 | impl RustEvidenceQueryContext::new | field_read |
| E02 | rust_evidence/query.rs:204–207 | impl RustEvidenceQueryContext::new | field_read |
| E03 | rust_evidence/query.rs:229 | impl RustEvidenceQueryContext::new | field_read |
| E04 | rust_evidence/query.rs:297–305 | impl RustEvidenceQueryContext::project | field_read |

Q01/Q02 decode the macro Option once into a local and borrow it for validation and snapshot derivation. Q03 acquires its owned macro set only inside the existing Complete branch after context/result processing. Q06 consumes the same Option via set_macro_evidence after table validation and owner hydration. R01 then R02 clear the cloned original view using set_macro_evidence(None)? followed by set_rust_evidence(None)?, before resetting origin hash/version/manifests. R03 must retain an owned Rust Option plus an Option<&RustEvidenceSet> view so the existing checksum/map_or tuple stays unchanged. E01/E02/E03 share one owned Rust Option in new; E04 gets its own owned set only after self.result(filter)?. Borrowed matches/file references must not outlive those locals; decode per manifest, borrowed getters or new cached carriers are unnecessary and outside the contract.

At both literals remove only the two named None fields and append `..GraphIndexArtifact::empty()`. Retain every other explicit field. In particular overlay-empty manifest/hash and test/test/test header/manifest/hash must survive; these are unchecked synthetic fixtures, not complete graphs. There are no Some literals requiring setters after construction in this batch. Neither whole-literal replacement nor validation of synthetic hashes is mechanical conservation.

## Current G03 semantics

Actual types are `Option<crate::RustEvidenceSet>` and `Option<crate::MacroInvocationEvidenceSet>` (schema.rs:95–97). The full exact signatures/ranges are in source-map.json#/apis. Artifact getters return `anyhow::Result<Option<owned set>>` through `Ok(field.clone())`; setters take `&mut self` plus the owned Option and assign exactly before `Ok(())`. They do not validate, normalize, remove Some(empty), or have an ordinary error branch at this epoch. A getter result can be mutated independently; only a setter changes the artifact. Propagate Result with `?` at the original logical access positions; do not introduce unwraps, contexts or changed errors. Later generic decode errors are a cutover concern: a nonmechanical ordering/error difference returns to ROOT.

`GraphIndexArtifact::empty() -> Self` (schema.rs:112–136) is an unchecked draft with temporal header, current manifest, hash of empty source inventory, empty vectors and both lanes None. Explicit literal fields override its defaults. Manifest methods are `rust_evidence_metadata() -> anyhow::Result<Option<RustEvidenceManifest>>`, `macro_evidence_metadata() -> anyhow::Result<Option<MacroEvidenceManifest>>` and matching setters accepting those Options; they clone/assign without validation (parquet.rs:413–442). Manifest empty (387–410) is incomplete, metadata None; count empty (470–487) has scalar zero and optional macro/edge counts None. M05 uses none of those manifest/count methods. Existing Self construction inside these compatibility APIs remains untouched until cutover. Legacy export guard remains a no-op (schema.rs:140–142).

Keep absence wire semantics: artifact None fields are omitted; manifest macro None serializes null while Rust None is omitted. Keep canonical field order/omission and full ordered-envelope capability tuple. rust_evidence_capability_digest deliberately does **not** validate stored/source semantics, and must continue hashing unchecked versions and nested payloads. No schema/cache/query/extractor changes are permitted.

## Existing reachable native routes

The following **21 lanes** are source-grounded test entry points. Detailed fixture locations, expected source assertions, exports, direct/bypass reachability and unresolved values are in the JSON documents. Every lane is **NOT RUN**. A test name or old receipt does not establish present native success.

| Lane | Existing entry point | Sites / attribution |
| --- | --- | --- |
| macro-positive | macro_invocation_queries.rs:80 / filters_forwarding_and_scope_locations_match_parquet | Q01, Q02, Q03, Q06 |
| macro-absence | macro_invocation_queries.rs:155 / complete_empty_legacy_unavailable_and_unsupported_source_parity | Q01, Q02, Q03, Q04, Q05, Q06 |
| macro-malformed | macro_invocation_queries.rs:200 / stale_source_and_missing_scope_fail_even_when_filtered_out | Q01, Q02, Q03, Q06 |
| macro-frozen | macro_invocation_queries.rs:244 / corrupt_core_tables_fail_before_any_filter_and_freeze_after_validation | Q01, Q02, Q04, Q05, Q06 |
| macro-manifest | macro_invocation_queries.rs:723 / parquet_macro_queries_reject_missing_source_manifest_rows | Q04, Q05, Q06 |
| overlay-draft | macro_invocation_queries.rs:632 / source_oid_without_file_is_rejected_by_overlay_as_by_base | Q04, Q05 |
| macro-zero | macro_invocation_queries.rs:608 / file_scope_and_whole_artifact_complete_empty_round_trip | Q01, Q02, Q03, Q06 |
| rust-positive | rust_evidence_queries.rs:101 / ordered_whole_files_origins_unknowns_and_forwarding_match | E01, E02, E03, E04 |
| rust-absence | rust_evidence_queries.rs:215 / absent_carriers_keep_known_domains_and_validate_original_filters | E01, E02, E03, E04 |
| rust-empty | rust_evidence_queries.rs:277 / empty_present_and_absent_are_distinct | E01, E02, E03, E04 |
| rust-malformed | rust_evidence_queries.rs:356 / full_validation_rejects_filtered_out_corruption_and_actual_owner_changes | E01 |
| rust-frozen | rust_evidence_queries.rs:417 / frozen_tables_survive_replacement_deletion_and_source_edits_but_cold_reads_fail | Q04, Q05, E01, E02, E03 |
| rust-orphan | rust_evidence_queries.rs:442 / absent_metadata_rejects_orphan_evidence_tables | guard only |
| rust-io | query_client.rs:2900 / rust_evidence_frozen_context_performs_no_additional_evidence_file_reads | Q04, Q05, E01, E02, E03, E04 |
| test-draft | query_client.rs:3267 / in_memory_client_search_symbols_delegates_to_search_symbols | Q07, Q08 |
| source-recollection | rust_evidence_contract.rs:218 / canonical_mutations_distinguish_stored_from_unseen_syntax | R01, R02 |
| source-origins | rust_evidence_contract.rs:375 / original_snapshot_survives_current_aggregate_composition | R01, R02 |
| source-auth-order | rust_evidence_contract.rs:342 / exact_domain_authentication_precedes_uncertainty | guard only |
| capability-empty | rust_evidence_contract.rs:797 / d2_absence_empty_legacy_bytes_and_capability_encoding | R03 |
| accessors | generic_evidence_providers.rs:1112 / g03_artifact_accessors_preserve_presence_owned_values_and_native_bytes | R03 |
| native-accessors | generic_evidence_providers.rs:1199 / g03_native_accessor_writer_export_and_metadata_controls | R01, R02, R03 |

Macro positive/None/zero/malformed cases instantiate memory and disk clients. Full macro validation in MacroQueryContext::new precedes filtering, caller inference and row/scope projection. Parquet load_macro_context freezes two tables, hydrates file manifests only for Some, enforces declared row count, reads files/caller symbols, sets the exact Option and then validates context. Keep that ordering, optional hydration and warm OnceLock behavior. `macro-zero` proves captured physical zero-site coverage; it does not independently prove an entirely empty macro artifact. G03 accessors/native export supplies whole-artifact captured-empty and None values.

Rust query new validates the whole present set before stale aggregate hash and manifest/domain checks. Per-source availability derives from the same Option; Some(empty) remains captured. project validates original filter/source_oid through result before fetching rows/origins. Existing positive/None/empty tests cover Complete/Partial/Unavailable/UnsupportedSource, exact original inventories and trait forwarding. Malformed off-filter cases test validation before selection. Disk captured queries call RustEvidenceQueryTables::open -> rust_evidence_validation_projection -> new/result; disk rows come from frozen tables, not project. The absence path uses the constructor and domain validation; an orphan-table guard can reject before them and is marked a bypass control.

Source-bound recollection retains authenticate_sources -> validate_stored -> per-file byte length/origin -> clone -> clear macro -> clear Rust -> restore original hash/version/manifests -> declaration validate -> canonical call recollect -> nested call validate. Real composition and rehashed mutation tests reach the clears; early collector authentication controls are not misrepresented as direct clear-site execution. Stored validation can accept recount-consistent omissions that exact bytes reject; retain that distinction. Current authoritative graph owners, original complete inventories, IDs and nested versions/hashes remain intact.

Existing stdout exports are D7_NATIVE (Rust artifact/query/errors/lifecycle), D7B_IO (five initial evidence files, actual compressed byte sum, zero additional warm evidence IO, cold error), M5_FILTER_FACTS and M5_REWORK_* (macro selected rows/source-count/shape controls), and G03_NATIVE_JSON (actual empty/nonempty typed values, serialized bytes, canonical/capability, source/provider results and metadata negatives). They can be reused without adding tests. G03 embedded compiled bindings are partial and historical metadata labels remain G03: additionally bind current M05 command/source/executable/compiler, never relabel its internal base as M05.

**Native gaps for ROOT:** private constructors Q04/Q05 and Q07/Q08 are reachable but currently do not export complete artifact/canonical/capability/skipped-ID values. Source-preserving tests alone cannot prove native fixture equality. ROOT must choose a real native route, such as separately authorizing a reversible observer at exact owned return sites with its source/patch/executable in fresh PRE, and remove it from the final implementation diff. No observer is written or authorized by this grounding. Several malformed/source tests are assertion-only; D1_EXPORT_DIR/D2_EXPORT_DIR can export their existing packets, but remote environment forwarding and output sync must first be verified. Use known stdout routes where sufficient; do not pretend an unavailable packet exists. These gaps block a complete native acceptance claim until resolved; they do not authorize broader files or new behavior.

## Proposed commands, never executed here

Run the baseline at this source epoch and the same commands/config/fixture inputs after later authorized migration, retaining actual exits, failure names and raw native records. No local build fallback after remote failure. All-target check and local fmt are required later; no broad workspace GREEN is implied.

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --lib
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --test rust_evidence_queries --test macro_invocation_queries -- --nocapture
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --test rust_evidence_contract -- --nocapture
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --test generic_evidence_providers g03_ -- --nocapture
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --lib query_client::tests::rust_evidence_frozen_context_performs_no_additional_evidence_file_reads -- --exact --nocapture
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --test macro_invocation_queries source_oid_without_file_is_rejected_by_overlay_as_by_base -- --exact --nocapture
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo check -p spur-graph --all-targets
scripts/spur-cargo fmt -- --check
```

The full library command covers both private test helper and frozen-I/O test; focused reruns with nocapture are for actual observation attribution. The macro shape focused command explicitly reaches the empty-overlay constructor. Integration source fixtures and relevant included legacy JSON are fingerprinted, not copied. ROOT must extend executable/compiler/transitive source binding as needed at acquisition; this grounding fingerprint set is a bounded minimal dependency map, not a claim to cover every compiler input or the external cloud-build script. No unverified D1/D2 environment/output command is prescribed as working.

## Hazards, unchanged defects and ROOT handoff

1. Future fallible decodes could change error precedence; current G03 clone/assignment has no such ordinary failure. Preserve logical placement and raise differences rather than repair silently.
2. New owned values require local lifetime bindings, especially repeated snapshot/domain accesses and R03 Option<&set>. No cache or second mutable carrier is allowed.
3. Preserve full-set validation before filters, original filter shape before inference, capability hashing without validation, and the order of the two clone-view clears.
4. Preserve separate physical-source predicates: Rust eligible excludes any #cell spelling; query physical_rust and macro path logic differ. M05 must not reconcile them or infer a new source policy.
5. Synthetic overlay-empty/test hashes must survive default tails. Existing frozen warm queries must not reopen files, recollect mutable source or add owner IO.
6. Native before/after hashes, error chains, executable/config/toolchain IDs, compressed sizes and current named outcomes are unknown. pre-inputs.json contains null observations and 18 individual false controls for ROOT; no source-expected value is presented as measurement.
7. Disk previously exhausted and prevented initial ROOT issue creation. ROOT reported recovery to about4.2 GiB after deleting only its fully merged clean disposable review worktree and preserving1,085 dirty main paths. This worktree measured about24 GiB available, filesystem95% used. Residual disk risk remains: compact documents only, no deletion/build here; ROOT rechecks before native acquisition.

Live get_issue reads confirmed these inherited bugs still **open**; none was rerun or repaired: bd-3dkq (three real-tool graph tuple-owner fixture failures), bd-1izf (two unrelated/non-source edit freshness failures, fresh versus not_needed), bd-3umt (worker tools34 versus expected31), bd-16eq (analyst overlay gamma->helper edge0 versus expected1). They are separate from mechanical M05; retain genuine failures if encountered. No ignored assertion, threshold/quota change or expected-output blessing.

Accepted M04 fingerprint `47d3fa5fff4c17ab94325bc9f98704e4c3db44f947ec5cc04c4b99ab9c76475c` and pinned receipts sol_20d1c51504e24bf7 / sol_3a83a9eb74604722 remain untouched. M03 sol_8f83ae0c197f4e2e / sol_9e5493d4cc3a4371 are reserved for ROOT consumption during fresh M05 PRE. M04 Python verification was not rerun; its accepted README is bound as inherited evidence, not new proof.

Review the three-document diff, verify every file/range SHA against base, reconcile 15 field sites and all excluded query/module spellings, check site-to-test reachability (including explicit bypasses), and resolve native observation gaps before authorizing implementation. ROOT owns Jev/PRE, native execution/POST and independent acceptance. Worker does not self-approve, merge or close. Final ACTION/RECORD hashes and commit identity are persisted via report_audit to avoid self-referential file hashes.

ACTION checkpoint 4823 retained the draft hashes before lease expiry. Runtime risk `76f7a570-b159-4e21-89b3-14f21c3c9594` was successfully recorded (late=false) after the first report_signal call with only kind was rejected: `Mcp error: -32602: invalid params: invalid args: missing field signal_id`. Reissue supplied the exact required UUID/severity/reason/estimated_subtasks on the same canonical issue. Progress requested a same-delegation lease extension; no alternate assignment or PM write was used. Final review corrected native-accessor reachability: provider replay reaches source validation and capability helpers, not typed query contexts; malformed/orphan guard lanes are explicitly upstream/bypass controls. Final RECORD binds the corrected hashes.

Git staging/commit initially failed because the filesystem sandbox denied creating the existing worktree index.lock outside the writable checkout. Narrow escalation is limited to staging/committing these three authorized document paths; no source or main changes are included.
