# Independent source review: spur-notebook panic candidates

Rationale: preserve exact source identities and caller guards so root can distinguish locally demonstrable panic conditions from protected operations and unproven application reachability.

This is a bounded static review for **bd-3qk**, plan key NB-AUDIT, current delegation `7cde29739bce4046`, on 2026-11-02. It contains **seven conditional production candidates and three guard-protected controls**, not seven confirmed application bugs. Root owns detector and native experiments; no result from those experiments is claimed as this review's evidence.

## Authorization and scope

- Initial worker worktree was clean; the previous attempt branch has no diff from this attempt's base `848a29f39511cbd27091beb6c171b97e179f353a`.
- `get_issue(bd-3qk)` returned `in_progress`, assignee `spur-worker-7cde29739bce4046`, matching delegation label, and no supersession label.
- Intent audit `bbcc9761-071a-4a20-a6d9-64508558a16e` was accepted as comment `1983`, returning the current delegation ID. Neither lineage issue bd-swa nor plan key NB-AUDIT was used as an audit target.
- Target root, `src/AGENTS.md`, and `jute-notebook/AGENTS.md` were read. Reviewed source is under those Rust areas; gallery instructions were outside the selected scope.
- The graph-first context call failed with “failed to prepare context candidate query” against the SpurCode analyst DB. Task context also records an earlier missing `/Volumes/Projects/spur-notebook` registry path. The explicitly authorized bounded raw-source fallback was used; no graph was published or registered.
- Target project, captures and notebooks were read-only. No application/build/Cargo, detector, exploratory evaluation script, native harness, Notebook MCP, global config, external messaging, or solver-store operation was performed.

## Source binding

All cited source and caller ranges are **inclusive, 1-based lines in the retained capture**, not inferred line numbers from the live checkout. The source HEAD `7fb364638575da96d46a68c5ec6af0215fe97300` and 689 Rust files are root-supplied capture provenance, not an assertion that all files were reviewed.

The retained [source manifest](../source-manifest.json) has SHA-256 `00e517c7f94bbe50632adb3e4b7fee44aa94ebe1119137e224f7fc46c7c4ab39`. [candidates.json](candidates.json) copies manifest byte counts, SHA-256 and Git blob IDs for all 11 cited files; their captured SHA-256 values were independently compared with the manifest. Each candidate's `path` and caller path resolve through that ledger's `sources` map.

## Candidate results

| ID | Exact site | Source fact and guard/caller limit |
| --- | --- | --- |
| C1 | `jute-notebook/src-tauri/src/state.rs::repair_object_store_location`, function 263–284, slices 268/275 | Length-only guards precede string byte slicing. Suggested input `aaaaaaaé` cuts inside é at byte 8 in the first `/https:/` comparison. `is_object_store_location` calls repair **before** classifying the path (286–297), so a non-URI path can reach it. Hydration accepts a deserialized String path for non-ApiTables entries (164–205); persistence also reaches it after ApiTables/credential branches (95–139). Production hydration caller: `commands/hydration.rs` 108–113; save callback: state.rs 563–581. Later suffix slices follow successful ASCII-prefix comparisons and have stronger guards. |
| C2 | `rest-table-gateway/src/adapter/manifest_adapter.rs::apply_header_value_prefix`, 43–56, slice 51 | Empty prefix/value are handled, but byte length does not establish a UTF-8 boundary. Suggested `Some("Basic ")` with `aaaaaé` cuts at byte 6. `resolve_auth` reads an environment String and invokes this helper directly (375–392); scan calls auth at 325. The action dry-run branch returns before auth (1203–1219). No credentials were read or changed. |
| C3 | `jute-notebook/src-tauri/src/backend/wire_protocol/driver_zeromq.rs::from_zmq_payload`, 47–63, indices 51–55 | Delimiter presence is checked, remaining frame count is not. A payload ending at `<IDS|MSG>` reaches the first out-of-bounds header index before JSON can return None. Shell/control/iopub receive loops pass successful receives directly at 178/209/227. Conforming local kernels supply the frame shape; malformed-kernel behavior was not executed. Once content index +5 exists, the +6 trailing slice is in bounds. |
| C4 | `src/dag/ports.rs::block_slice`, 725–728 | Signed footer values are cast to usize, lengths added, then passed to `Buffer::slice_with_length` without a visible local block-range check. `decode_ipc_file` validates trailer/footer/schema but slices each block before decoder.read_* (704–719). `PortStore::get` maps an on-disk non-media path (477–507). Normal `put` validates Arrow IPC (369–399), and files are intended write-once. Malformed footer acceptance and dependency bounds behavior require separate evidence; arithmetic panic depends on overflow checks. |
| C5 | `src/dag/ports.rs::PortStore::bump_version`, 418–471, increment 443; also `put` 344–416, increment 399 | Both add 1 to an existing u64 version. Deserialization assigns the full u64 at 195–238; manifest load adds no tighter bound (562–573). Locking, stale-version equality and port-name checks do not exclude MAX. Overflow panic requires enabled overflow checks; unchecked wrapping is a different outcome. No production release configuration was executed. |
| C6 | `src/dag/events.rs::run_sequencer`, 236–261, expect 249 | `checked_add(1).expect(...)` deliberately panics at MAX. Counter starts at 1 and has no externally selectable seed: it requires 2^64−1 received drafts, with the final failure occurring before broadcast. `spawn` calls this fixed function (195–208); `emit` only enqueues (168–174). This is theoretical exhaustion, not an established practical trigger. |
| C7 | `src/context/symbol_index.rs::facts_for/reindex`, 36–69, expects 39/49/66 | A poisoned private std RwLock causes a secondary panic. Production callers include `notebook_symbol_search.rs` 33–56 and `notebook_symbol_refs.rs` 37–54. A writer must first unwind: extraction/serialization happen before acquiring the writer; the visible write section only inserts into a map. No initiating production panic was found. Reader-only unwinding is insufficient. |

C1/C2 have direct local Unicode conditions worth root's source-faithful follow-up. C3 depends on malformed transport frames. C4 needs malformed-file/dependency validation. C5 is profile-dependent arithmetic. C6/C7 retain concrete production sites while explicitly recording why boundary-expression or manufactured-state examples would overstate original-path reachability.

## Guard-protected controls and exclusions

| ID | Site | Guard established in the reviewed source |
| --- | --- | --- |
| G1 | `src/mcp/datasource/object_store.rs::repair_object_store_location`, 116–143 | Distinct from C1: `.get(..length)` at 121–133 rejects non-boundary/out-of-range prefixes. Suffix slices only follow a successful checked prefix. |
| G2 | `src/sidebar_chat/manager/prompt.rs::compose_prompt_blocks_with_expander`, 225–323, slices 270/273/317 | `validated_mention_ranges` at 118–176 establishes ordering, bounds, UTF-8 boundaries and non-overlap before the loop. Cursor begins at zero and thereafter takes a validated end. |
| G3 | `src/dag/ports.rs::decode_ipc_file`, 679–723, trailer operations 681–697 | Minimum length 10 makes the trailer slice exactly 10 bytes, protecting its array-conversion expect. Footer length is checked against trailer_start before subtraction. This proves no bound for C4's separate block fields. |

Test-only `state.rs` 1041's kernel unwrap was excluded (test module starts 959). Port round-trip/bump tests at `ports.rs` 1246–1311 were excluded as production callers (test module starts 829). Manifest ASCII prefix tests at 1386–1401 are controls, not evidence of Unicode coverage. The `save.rs` persistence call found at 376 is a test helper and was not used as production caller evidence.

## Harness feasibility and evidentiary limits

The complete unmodified C1/C2 helpers and scheme constant for C1 are std-only. G1 can likewise be extracted without changing its source. C3 needs original ZeroMQ/Bytes/JSON/message types; a Vec indexing analogue changes the function. C4 needs Arrow for a faithful full reproduction; a std cast/add example tests only arithmetic, with explicit pointer width and overflow settings. C5's increment is std-only in isolation but omits persistence/locking. C6 at a manually seeded MAX and C7 with deliberately poisoned state demonstrate constructed preconditions, not the original initialized call path. Full G2/G3 functions require project dependencies.

No harness was run by this worker. Unmodified function extraction, successful compilation, or conditional native panic reproduction would still not establish application call-path reachability. This review does not make detector-performance, exploitability, compiler-coverage, whole-project or arbitrary-workspace safety claims. The caller lists are bounded observed source references, not an exhaustive call graph. No TDD or solver receipts are claimed or needed for these static documentation artifacts.

## Verification and delivery

Artifact verification is limited to JSON parsing/schema-shape checks, retained-manifest identity comparison, captured-file SHA-256 checks, and Git diff/whitespace/scope inspection. No application checks were substituted. Only this report and `candidates.json` are worker outputs. The completion audit records the commit and verification outcomes; bd-3qk stays `in_progress` for root review.
