# Generic Graph Evidence Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/superpowers/specs/2026-11-07-generic-graph-evidence.md`.
**Formal @spec cells (if notebook):** none.
**Design epic:** `bd-2rg4` (in_progress; ROOT owns review, no fabricated closed status).
**Goal:** Make graph evidence persistence/query/cache identity generic while preserving accepted Rust records, source validation and old-artifact usability.
**Architecture:** One deterministic extension inventory and shared relational table transport, with static Rust and legacy macro codecs/validators. Stage mechanical caller migration before replacing typed artifact/manifest fields, then integrate storage, query/generation forwarding and aggregate cache/runtime identity.
**Tech Stack:** Existing Rust/Serde/Arrow/Parquet/DuckDB, native wrapper builds and ROOT source-bound catalog/Jev review; no new dependency/parser/plugin loader.
**Grounded base:** `457b9f70fde8a9c26ce4ac4e2dec5c56cd9a1c3a`; [report](../../research/artifacts/2026-11-07-generic-graph-evidence/grounding/REPORT.md), [exact file/range/constructor map](../../research/artifacts/2026-11-07-generic-graph-evidence/grounding/source-map.json), [prospective PRE inputs](../../research/artifacts/2026-11-07-generic-graph-evidence/grounding/pre-inputs.json).

This is a proposed, unsubmitted DAG for ROOT review inside already approved scope. This grounding does not create tasks, submit a plan, run Rust/new solves, or request another user approval. Every future task uses **codex / gpt-6.1-sol / xhigh**, one worker, no subdelegation. ROOT supplies fresh generic Jev PRE before production edits, actual-output POST and independent review after each boundary. Approved predecessor state is required before the next task starts. Leave canonical tasks in_progress for review; do not self-approve/close/merge/push.

## Files and serial execution

| Unit | Files / responsibility |
| --- | --- |
| Generic contract | New `src/evidence/{mod,contract,query}.rs`; deterministic shared schemas, integrity, inventory/digest and selection types |
| Provider boundary | New `src/evidence/providers/{mod,rust,legacy_macros,legacy_wire}.rs`; typed encode/decode/validation, built-in composition, old wire/hash adapters |
| Typed semantics | Existing `rust_evidence.rs`, `rust_evidence/query.rs`, `rust_declarations.rs`, `panic_evidence/call_sites.rs`, `macro_invocations.rs`; retain collectors/records, narrowly adapt original manifest compatibility |
| Generic storage | New `src/store/evidence.rs`; existing `store/{parquet,mod,rust_evidence,macro_invocations}.rs`; generic IO plus provider codecs, full/delta/slim/history routes |
| Artifact/API migration | `schema.rs`, `store/canonical_hash.rs` plus the exact mechanical batches below; one mutable truth throughout |
| Build and publication | `store/{build,cache}.rs`; generic bucket groups, current source guards and full inventory publication/dedup |
| Queries and generations | `query_client.rs`, `overlay_generation.rs`, `mcp/{mod,request_replay}.rs`; frozen generic contexts and every trait wrapper |
| Identity/runtime | `mcp/{overlay_snapshot,request_cache,overlay_runtime,mod}.rs`; visible/exact/seed/inflight/publication/current-base/late-load identity |

Paths above are relative to `crates/spur-graph/`. Each task below lists its exact ownership; only seven cross-crate files in mechanical batches may be touched. No production/test edits are made by this documentation task. Existing source anchors are current-base anchors; successors must rebind hashes after predecessors move lines. No parallel ownership of any file.

The serial chain is **G01 -> G02 -> G03 -> M01 ... M19 -> G04 -> G05 -> G05a -> G06 -> G07 -> G08 -> G09 -> G10 -> G11 -> G11b -> G12 -> G13 -> G14 -> ROOT-final**. G04 is a codec/compatibility prerequisite, G05 the artifact cutover; G08 transports generic tables independently before G09 switches the disk manifest. Mechanical steps are deliberately split to avoid an unreviewable 55-file atomic API change. Exact M batch count and files are recorded below; use that chain rather than guessing ownership from this overview.

## Common TDD and evidence procedure

For every behavior task: bind fresh current source/fixture/oracle PRE; get canonical issue/assignment/plan/task/delegation; persist/replay UUID INTENT. Write the listed regression before implementation and commit `test(spur-graph): <task-id> ...`; run it and retain full command/output/exit status. A missing new signature is **compile RED**, not behavioral RED. Establish the specified runnable assertion failure before fixing behavior; retain setup/infrastructure failures separately. Implement minimally, run focused GREEN and required checks, commit intent-focused `feat|fix|refactor` messages. ROOT independently mutates actual output, persists/reloads full POST envelopes, reviews and only then advances.

All builds/tests/checks use **scripts/spur-cargo**, remote by default and **no local fallback after remote failure**. Use `SPUR_REMOTE=1` for sandbox clippy if later needed. `scripts/spur-cargo fmt -- --check` is local. Do not introduce capacities/retries or run broad suites repeatedly without a changed reason. Each task also runs `scripts/spur-cargo check -p spur-graph --all-targets` and local fmt after its focused GREEN, except cross-crate batches additionally check their actual package. A task's public temporary surface must compile on its own predecessor; do not silently rely on a later task's fields.

Evidence folder per task is prospective `docs/research/artifacts/2026-11-07-generic-graph-evidence/<task-id>/`, owned only after ROOT dispatch. Retain `source-bindings.json`, actual fixture bytes/inventory, `RED.txt`, `GREEN.txt`, `actual-export.json`, exact native executable/compiler/config identities, PRE/POST requests/raw results/reloaded complete envelopes, mutation requests/errors and Jev. Each native export includes artifact/manifest/capability and selected provenance as relevant, not a prewritten expected JSON. Python may authenticate/join archives; it does not implement BLAKE3 capability, collection or expected cache events.

Common test fixture in `tests/generic_evidence_contract.rs` (all names below are defined here or in the spec). Copy these small test-only helpers verbatim into each listed test module that uses them; no production helper or undefined fixture API is assumed:

```rust
use std::collections::BTreeMap;
use spur_graph::evidence::*;
fn counter_extension(value: i64) -> EvidenceExtension {
    EvidenceExtension {
        descriptor: EvidenceDescriptor {
            provider_id: "test.counter".into(), schema_version: "counter-v1".into(),
            extractor_version: "fixture-v1".into(), component_versions: BTreeMap::new(),
            presence: EvidencePresence::Captured, metadata: BTreeMap::new(),
        },
        origins: vec![], domain: EvidenceDomain::Known { sources: vec![] },
        tables: vec![EvidenceTable {
            name: "values".into(), storage_path: "evidence/test.counter/values.parquet".into(),
            scope: EvidenceTableScope::Snapshot,
            columns: vec![
                EvidenceColumn { name: "key".into(), data_type: EvidenceColumnType::Utf8,
                    nullable: false },
                EvidenceColumn { name: "value".into(), data_type: EvidenceColumnType::Int64,
                    nullable: false },
            ],
            primary_key: vec!["key".into()], order_by: vec!["key".into()],
            metadata: BTreeMap::new(),
            rows: vec![vec![EvidenceValue::Utf8("fixture".into()), EvidenceValue::Int64(value)]],
        }],
    }
}
```

This deliberately simple snapshot annotation has no parser/language claim. Later source-bound counter cases change scope to File with columns `file_path/source_oid/origin_graph_content_hash/key/value`, build one row per actual physical fixture manifest and bind EvidenceOrigin/SourceCoverage to that graph's exact saved bytes. TestCounterProvider implements the spec's trait: supports normal physical paths, domain from actual manifests, validates exactly that fixed schema/nonnegative fixture values and source bindings; validate_sources checks saved bytes/OIDs. It has no production registration. Preserve an unregistered ID variant to exercise opaque status independently of the validated fixture provider.

## G01 — Generic contract, registry and deterministic inventory identity

**Task ID:** G01. **Depends on:** none beyond approved base. **Files:** Create `src/evidence/{mod,contract}.rs`, `tests/generic_evidence_contract.rs`; modify `src/lib.rs` exports and `src/query_client.rs` generic default only (five files). **Scope:** implement every contract/manifest/query transport type, registry and inventory-only helper from the spec. Add optional GraphQueryClient::evidence with shape-checked empty Unavailable default now, so G02 adapter query functions compile; built-in implementations wait for G11. Artifact/manifest digest wrappers wait for G05a/G09, so no nonexistent field is referenced here.

First compile RED adds the fixture and this test; after the surface compiles, a stub ignoring payload produces assertion RED:

```rust
#[test]
fn counter_only_change_changes_inventory_capability() {
    let mut a = EvidenceInventory::empty();
    a.replace(counter_extension(1)).unwrap();
    let mut b = a.clone(); b.replace(counter_extension(2)).unwrap();
    assert_ne!(evidence_inventory_capability_digest("fixture-manifest", &a).unwrap(),
               evidence_inventory_capability_digest("fixture-manifest", &b).unwrap());
}
```

Implement exact tuple/determinism/typed-cell checks, no languages in core. Add provider-order invariance, add/remove/version/presence/metadata changes, duplicate IDs/rows/paths, required-null/type/order errors and captured-empty versus absent cases. Unknown registry entries are integrity checked and semantically UnknownProvider. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_contract`. **Native export/negative:** real serialized tuples/digests/schema, duplicate key and changed counter payload. ROOT PRE/POST checks unique/cardinality/exact capability correspondence; do not call these values artifact helper observations yet.

## G02 — Rust and legacy macro codecs, source validation and fixture provider

**Task ID:** G02. **Depends on:** G01. **Files:** Create `src/evidence/providers/{mod,rust,legacy_macros}.rs`, `tests/generic_evidence_providers.rs`; modify `src/evidence/mod.rs` module declaration (five files). **Scope:** static registry and pure typed codecs against current graph; leave production artifact/storage/build wiring unchanged. Public collector APIs remain untouched.

Runnable RED after new signatures exist: with a real empty root built by current `artifact_from_facts`, encode/decode must preserve the actual typed carrier:

```rust
#[test]
fn rust_codec_keeps_actual_captured_empty() {
    let root = tempfile::tempdir().unwrap();
    let graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let set = graph.rust_evidence.as_ref().unwrap(); // current field; migrate in M batches
    let ext = spur_graph::evidence::providers::rust::encode_rust_evidence(&graph, set).unwrap();
    assert_eq!(spur_graph::evidence::providers::rust::decode_rust_evidence(&graph, &ext).unwrap(),
               *set);
}
```

Move/extract table **row codecs**, without changing existing disk IO yet; if extraction needs existing `store/rust_evidence.rs` or `store/macro_invocations.rs`, use G04 rather than exceeding five files here. Implement pure codecs in provider files first, then G04 removes old duplicate codec functions by delegation. Typed rows/JSON/order and all duplicated scalars must agree exactly. Implement TestCounterProvider and unknown/unsupported status tests. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_providers --test rust_evidence_contract --test macro_invocation_artifacts`. **Native export/negative:** original typed set, extension tables, decoded set, original byte-bound validation; mutate existing body/owner and independently rehash, showing semantic/source rejection. No generic disk claim.

## G03 — Stable accessor and construction preparation surface

**Task ID:** G03. **Depends on:** G02. **Files:** Modify `src/schema.rs`, `src/store/parquet.rs`; extend `tests/generic_evidence_providers.rs` (three files). **Scope:** add exact getter/setter/empty signatures in spec, still backed only by existing typed fields. Add `GraphArtifactRowCounts::empty() -> Self` for count literal migration. Add the temporary validate_legacy_evidence_export no-op and prepare full/delta writers to invoke it. No generic carrier or wire format switch yet.

```rust
#[test]
fn typed_accessor_distinguishes_absence_and_empty() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let captured = graph.rust_evidence_set().unwrap().unwrap();
    graph.set_rust_evidence(None).unwrap();
    assert!(graph.rust_evidence_set().unwrap().is_none());
    graph.set_rust_evidence(Some(captured.clone())).unwrap();
    assert_eq!(graph.rust_evidence_set().unwrap(), Some(captured));
}
```

Compile RED is API introduction; runnable RED must expose a setter/getter that drops Some(empty). Empty construction is explicitly an unchecked draft, not a Complete result. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_providers`; existing canonical-hash/parquet tests. **Export/negative:** old serialized bytes before/after accessor roundtrip, explicit None versus Some(empty), null metadata controls. This surface permits the following serial migration without introducing two mutable carriers.

## Mechanical migration contract (M01–M19)

Each batch owns only its listed files and depends on its immediately preceding batch (M01 depends on G03). Change artifact field reads to owned typed getters; change mutation to decode -> mutate -> setter; None removal uses setter. Manifest accesses use matching metadata methods. Remove named typed fields from literals, preserve Some values by setter after construction, and use `..GraphIndexArtifact::empty()`, `..GraphArtifactManifest::empty()` or `..GraphArtifactRowCounts::empty()` so the later field change compiles. Current `Self` construction inside the G03 compatibility methods remains until cutover. Do not change GraphFacts fields or legacy wire JSON/path mutation test inputs; old-schema negative controls must still mean old-schema inputs.

These are reversible mechanical preparation tasks with no intended behavior change. They do not fabricate a failing behavioral test: G03 owns the accessor regression. A compile failure identifies an incomplete migration, not a new cache RED. Each batch runs existing affected tests and exports byte/value equality of a preexisting fixture before/after migration; ROOT compares it to predecessor output. No mirror-the-implementation tests are added. Any semantic difference blocks that batch and returns to ROOT. All tasks still require fresh PRE and POST/independent review; no concurrent worker edits.

### M01 — Mechanical accessor/constructor batch

**Task ID:** M01. **Depends on:** G03. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-analyst/src/overlay/session_tests.rs` (literal lines 308, 350, 317)
- `crates/spur-cli/tests/analyst_temporal_views.rs` (literal lines 1272)
- `crates/spur-core/src/server/handlers/rebuild_singleflight.rs` (literal lines 211)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-analyst overlay`; `scripts/spur-cargo test -p spur-cli --test analyst_temporal_views`; `scripts/spur-cargo check -p spur-core --all-targets`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M02 — Mechanical accessor/constructor batch

**Task ID:** M02. **Depends on:** M01. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-core/tests/code_graph_e2e.rs` (literal lines 249, 392, 491, 525, 2042, 2438)
- `crates/spur-core/tests/worker_server_dispatch.rs` (literal lines 993)
- `crates/spur-graph/benches/incremental.rs` (literal lines 988, 1084)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo check -p spur-core --all-targets`; `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M03 — Mechanical accessor/constructor batch

**Task ID:** M03. **Depends on:** M02. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/benches/parquet.rs` (literal lines 572, 620, 787, 808)
- `crates/spur-graph/src/git_walk.rs` (literal lines 1347)
- `crates/spur-graph/src/mcp/mod.rs` (literal lines 9078)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M04 — Mechanical accessor/constructor batch

**Task ID:** M04. **Depends on:** M03. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/mcp/overlay_runtime.rs` (literal lines 1316)
- `crates/spur-graph/src/mcp/rebuild_singleflight.rs` (literal lines 296)
- `crates/spur-graph/src/mcp/request_cache.rs` (literal lines 622, 660)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M05 — Mechanical accessor/constructor batch

**Task ID:** M05. **Depends on:** M04. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/query_client.rs` (literal lines 528, 3021)
- `crates/spur-graph/src/rust_evidence.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/src/rust_evidence/query.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M06 — Mechanical accessor/constructor batch

**Task ID:** M06. **Depends on:** M05. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/schema.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/src/search.rs` (literal lines 223)
- `crates/spur-graph/src/selector.rs` (literal lines 320)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M07 — Mechanical accessor/constructor batch

**Task ID:** M07. **Depends on:** M06. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/store/build.rs` (literal lines 2915, 3671)
- `crates/spur-graph/src/store/cache.rs` (literal lines 2597)
- `crates/spur-graph/src/store/canonical_hash.rs` (literal lines 61, 209)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M08 — Mechanical accessor/constructor batch

**Task ID:** M08. **Depends on:** M07. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/store/lance_sections.rs` (literal lines 3553, 3971, 5303, 5430)
- `crates/spur-graph/src/store/macro_invocations.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/src/store/parquet.rs` (literal lines 636, 821, 985, 1064, 1159, 1294, 1377, 1601, 5094, 646, 831)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M09 — Mechanical accessor/constructor batch

**Task ID:** M09. **Depends on:** M08. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/src/store/rust_evidence.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/src/temporal.rs` (literal lines 1109)
- `crates/spur-graph/src/traversal.rs` (literal lines 614, 657, 718, 860, 978, 1016, 1057, 1107, 1139, 1170, 1236)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --lib`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M10 — Mechanical accessor/constructor batch

**Task ID:** M10. **Depends on:** M09. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/diagnostics_persist.rs` (literal lines 80)
- `crates/spur-graph/tests/extractor.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/integration_git_incremental.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test diagnostics_persist --test extractor --test integration_git_incremental`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M11 — Mechanical accessor/constructor batch

**Task ID:** M11. **Depends on:** M10. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/lance_sections.rs` (literal lines 383)
- `crates/spur-graph/tests/load_artifact_dispatch.rs` (literal lines 44)
- `crates/spur-graph/tests/macro_invocation_artifacts.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test lance_sections --test load_artifact_dispatch --test macro_invocation_artifacts`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M12 — Mechanical accessor/constructor batch

**Task ID:** M12. **Depends on:** M11. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/macro_invocation_end_to_end.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/macro_invocation_parquet.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/macro_invocation_queries.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test macro_invocation_end_to_end --test macro_invocation_parquet --test macro_invocation_queries`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M13 — Mechanical accessor/constructor batch

**Task ID:** M13. **Depends on:** M12. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/overlay_client.rs` (literal lines 1820)
- `crates/spur-graph/tests/parquet_roundtrip.rs` (literal lines 463)
- `crates/spur-graph/tests/parquet_schema_invariants.rs` (literal lines 279)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test overlay_client --test parquet_roundtrip --test parquet_schema_invariants`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M14 — Mechanical accessor/constructor batch

**Task ID:** M14. **Depends on:** M13. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/query_client_parity.rs` (literal lines 92)
- `crates/spur-graph/tests/rust_evidence_build.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_build_root_review.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test query_client_parity --test rust_evidence_build --test rust_evidence_build_root_review`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M15 — Mechanical accessor/constructor batch

**Task ID:** M15. **Depends on:** M14. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/rust_evidence_contract.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_d7b.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_empty_domain_root_review.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_contract --test rust_evidence_d7b --test rust_evidence_empty_domain_root_review`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M16 — Mechanical accessor/constructor batch

**Task ID:** M16. **Depends on:** M15. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/rust_evidence_overlays.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_parquet.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_pruning_d7b.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_overlays --test rust_evidence_parquet --test rust_evidence_pruning_d7b`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M17 — Mechanical accessor/constructor batch

**Task ID:** M17. **Depends on:** M16. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/rust_evidence_pruning_root_review.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_evidence_queries.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/rust_foreign_declarations.rs` (bound field/API sites in source-map.json)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_pruning_root_review --test rust_evidence_queries --test rust_foreign_declarations`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M18 — Mechanical accessor/constructor batch

**Task ID:** M18. **Depends on:** M17. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-graph/tests/rust_selected_path_semantics_root_review.rs` (bound field/API sites in source-map.json)
- `crates/spur-graph/tests/temporal_resolution.rs` (literal lines 314)
- `crates/spur-tui/src/mentions/registry.rs` (literal lines 4008)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_selected_path_semantics_root_review --test temporal_resolution`; `scripts/spur-cargo check -p spur-tui --all-targets`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.

### M19 — Mechanical accessor/constructor batch

**Task ID:** M19. **Depends on:** M18. **Suggested Worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- `crates/spur-utilities/mentions/src/code/source.rs` (literal lines 797)

**Implementation/acceptance:** Apply the mechanical migration contract only to these files; retain old wire negative controls and GraphFacts fields. G03 getter/setter/empty signatures are the complete required new API. Export native existing fixture bytes/value equality before/after; no semantic RED or new cache measurement is claimed. Incomplete compile migration must be repaired before commit. **GREEN:** `scripts/spur-cargo check -p spur-mentions --all-targets`; package all-target check and local fmt as above. **Native negative:** rerun existing None/Some(empty), malformed fixture and source-bound negative controls in the owning suites; do not modify their expected semantic results. ROOT verifies the migration-only diff and exact predecessor/native fixture equality. OUT: generic carrier/schema/extractor/cache behavior.


## G04 — Consolidate provider codecs and explicit legacy compatibility

**Task ID:** G04. **Depends on:** M19. **Files:** Modify `src/store/rust_evidence.rs`, `src/store/macro_invocations.rs`, `src/evidence/providers/rust.rs`, `src/evidence/providers/legacy_macros.rs`; extend provider tests (five files). **Scope:** old readers/writers call the single provider row codec instead of a second implementation. Move nine Rust metadata/version checks and v12 macro condition into explicit compatibility functions; preserve current output/absence semantics. No default schema bump yet.

This is a behavior-preserving codec consolidation: extend G02's test with a real v13 disk roundtrip and assert encoded table column/value equality, then corrupt a duplicated declaration scalar while retaining valid JSON; decode must error. Run this conservation test before and after extraction; it may already be GREEN because G02 owns the codec behavior. Do not invent a new assertion RED for a refactor. Keep all five legacy Rust tables and two macro tables/nullable fields. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_providers --test rust_evidence_parquet --test macro_invocation_parquet`. **Export/negative:** exact old table rows/schemas/metadata and unknown/null/scalar disagreement errors. Old archived inputs remain untouched; only private fixture copies may be mutated.

## G05 — Single generic artifact carrier and old-wire/hash adapter

**Task ID:** G05. **Depends on:** G04. **Files:** Modify `src/schema.rs`, `src/store/canonical_hash.rs`; create `src/evidence/providers/legacy_wire.rs`; modify `src/evidence/providers/mod.rs` declaration and contract tests (five files). **Scope:** replace both artifact typed fields with `evidence`, virtual wire tag and generic body hash; accessor bodies now encode/decode the sole inventory. Implement `evidence_capability_digest` wrapper in the already declared `evidence/contract.rs` in a separate G05a sub-boundary below, avoiding a sixth file.

```rust
#[test]
fn artifact_accepts_generic_counter_and_rejects_dual_layout() {
    let root = tempfile::tempdir().unwrap();
    let graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let mut wire = serde_json::to_value(&graph).unwrap();
    wire.as_object_mut().unwrap().remove("rust_evidence");
    wire.as_object_mut().unwrap().remove("macro_invocations");
    wire["evidence_format"] = "spur-evidence-v1".into();
    let mut inventory = EvidenceInventory::empty();
    inventory.replace(counter_extension(1)).unwrap();
    wire["evidence"] = serde_json::to_value(inventory).unwrap();
    assert!(serde_json::from_value::<spur_graph::GraphIndexArtifact>(wire.clone()).is_ok());
    wire["rust_evidence"] = serde_json::json!(null);
    assert!(serde_json::from_value::<spur_graph::GraphIndexArtifact>(wire).is_err());
}
```

This test compiles against old strict Serde and gives assertion RED before the format exists. Keep old DTO strictness/None omission and original payload values; add new v14 golden rather than blessing old bytes. Implement original v13 origin-version acceptance in the provider by **G05a**, before attempting v14 persisted migration. Until generic disk integration, old writers must reject unrepresentable counter/opaque inventories rather than silently discard them. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_contract --test generic_evidence_providers`; canonical hash and artifact macro/Rust suites. **Export/negative:** native old omission hash, new canonical bytes/hash, same source hash, both-layout rejection and payload-rehash forgery rejection.

**G05a (separate task, depends on G05):** owns `src/evidence/contract.rs`, `src/evidence/providers/rust.rs`, `src/rust_evidence.rs`, provider tests (four files). Add exact artifact helper; adapt strict origin equality to the explicit accepted old/current origin policy without relabeling values. Add a compiling helper equality assertion against `evidence_inventory_capability_digest(&graph.manifest_version, &graph.evidence)` and mutate only test.counter; retain runnable RED for any source-only wrapper. GREEN: contract/provider/Rust contract suites. Export actual helper parity and original-view source validation; unsupported old origin errors. G06 depends on G05a.

## G06 — Full and selected capture through provider composition

**Task ID:** G06. **Depends on:** G05a. **Files:** Modify `src/evidence/providers/mod.rs`, `src/store/build.rs`, `src/query_client.rs` extraction call only; create `tests/generic_evidence_build.rs` (four files). **Scope:** exact `capture_default_evidence` signature; move Rust eligibility/source authentication and macro wrapping into adapters/composition. Core full/selected builder calls composition, never matches provider versions/language for evidence.

```rust
#[test]
fn empty_full_build_has_captured_provider_entries() {
    let root = tempfile::tempdir().unwrap();
    let graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    assert_eq!(graph.evidence.get("spur.rust.physical").unwrap().descriptor.presence,
               EvidencePresence::Captured);
    assert!(graph.rust_evidence_set().unwrap().unwrap().files.is_empty());
}
```

Introduce capture_default_evidence via compile RED, then require parity with current full/selected outputs, including two actual files but one selected manifest. Existing capture already works through G05 accessors, so this refactor may have no new behavioral RED; do not disable correct production behavior to manufacture one. Static review must confirm core evidence management no longer contains Rust eligibility/version branches. Add source-read OID-race rejection and actual cfg/recovery/FFI preservation. **GREEN:** generic/Rust build tests, macro artifacts and selected-path root review tests. **Export/negative:** actual full/selected inventories, fixed source OIDs, unselected-source omission and zero source; mismatch read OID errors.

## G07 — Generic per-file reuse, delta and original-origin retention

**Task ID:** G07. **Depends on:** G06. **Files:** Modify `src/store/build.rs`, `src/evidence/contract.rs`; extend `tests/generic_evidence_build.rs` (three files). **Scope:** replace Rust-only FileBucket payload/origin fields with generic provider file groups; implement `replace_evidence_files` and original origin union/pruning. Existing macro facts/extractors stay intact.

First regression uses two real files, injects counter File-scoped rows, changes one file and runs current `artifact_from_facts_incremental`. Assert the untouched file's counter row/source/origin is identical and no changed file retains the old row. The test can use existing build APIs; the fixture extension is constructed exactly as specified above. Runnable RED is currently missing provider groups, not a fabricated parser failure. Add delete/readd, no-op/captured-empty delta, Unknown/absent live replacement and unsupported provider Snapshot-scoped preservation rules. **GREEN:** `scripts/spur-cargo test -p spur-graph --test generic_evidence_build --test rust_evidence_build --test integration_git_incremental`; store::build unit tests. **Export/negative:** before/after group rows/origins/tombstones and provenance validation; reinsert deleted row or rewrite old origin, expect rejection. No disk delta claim until G09.

## G08 — Generic table IO independent of disk-manifest cutover

**Task ID:** G08. **Depends on:** G07. **Files:** Create `src/store/evidence.rs`, `tests/generic_evidence_parquet.rs`; modify `src/store/mod.rs` (three files). **Scope:** EvidenceManifest and generic `write_evidence`/`read_evidence_tables`, logical table checksums and safe columnar paths. The old GraphArtifactManifest is still current; defer the wrapper `read_evidence` and frozen query wrapper until G09/G11. No language dispatch in shared IO.

In a unit test inside store/evidence.rs, use a current built graph, add counter_extension(7), call write_evidence and then read_evidence_tables with the returned generic manifest. Assert full inventory equality and a readable Int64 `value` column, then duplicate a primary key and assert write rejection. New API compile RED must become runnable row-preservation RED before fixing transport. Add opaque provider and unsupported semantic schema retention; unsupported generic column types reject. For every declared table test missing/truncation/orphan/schema/null/count/checksum/order/malformed JSON where the provider owns JSON, including zero rows and off-filter corruption. **GREEN:** `scripts/spur-cargo test -p spur-graph store::evidence::tests`; `scripts/spur-cargo test -p spur-graph --test generic_evidence_parquet`. **Export/negative:** real Parquet schemas/metadata/rows, generic manifest/logical checksums and all cold errors, no invented compressor byte digest.

## G09 — v14 manifest, full/delta/slim/history persistence migration

**Task ID:** G09. **Depends on:** G08. **Files:** Modify `src/store/parquet.rs`, `src/store/build.rs` schema constant only, `src/store/evidence.rs`, `src/store/mod.rs` helper re-export; extend generic Parquet tests (five files). **Scope:** replace manifest typed metadata/counts with EvidenceManifest, implement wrapper and old manifest DTO route, bump default disk v14. Existing metadata accessors delegate compatibility codec; no new unchecked checksum setter. Preparation M batches made all named old-field literals disappear already.

```rust
#[test]
fn full_writer_preserves_second_provider() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    graph.evidence.replace(counter_extension(7)).unwrap();
    let out = tempfile::tempdir().unwrap();
    spur_graph::write_artifact_parquet(&graph, out.path(),
        spur_graph::WriteOptions::default(), vec![]).unwrap();
    assert_eq!(spur_graph::read_artifact_parquet(out.path()).unwrap().evidence, graph.evidence);
}
```

Before integration the old writer must refuse the provider; that error is runnable RED, not acceptable silent loss. Add actual artifact/manifest helper parity; manifest aggregate wrapper implementation belongs in store/evidence.rs and is re-exported (the spec signature names the API, not a mandatory source file). For v12/v13 immutable fixtures, authenticate legacy metadata/hash before normalization; test original saved source validation and unchanged table basenames/columns. Full/slim validates inventory before graph clearing; history projections carry empty/unavailable and never claim a full hash. Full/delta writes publish manifest only after all tables sync. **GREEN:** generic/Rust/macro Parquet, parquet_roundtrip, parquet_schema_invariants, load_artifact_dispatch, diagnostics_persist and `scripts/spur-cargo test -p spur-analyst overlay`. **Export/negative:** actual v14 full/slim/delta, zero/deletion delta, native helper parity/absence bytes, legacy source/typed-record equality and version-only golden delta inventory. Unchanged old archives are inputs, not regenerated evidence.

## G10 — Generic publication, dedup and base cache

**Task ID:** G10. **Depends on:** G09. **Files:** Modify `src/store/cache.rs` and its unit tests only. **Scope:** BaseArtifactCacheKey generic capability, complete inventory publication/dedup equality and cold validation; preserve current pointer staging and parsed-header identity.

Inside current cache tests, build graph, clone/add counter_extension(2), write it to a private output and assert `validate_publication(&original, out.path()).is_err()`. This is a runnable regression using the existing private function; insert the generic fixture helper in that test module. Test both only-counter and opaque-provider differences, captured/absent/Unknown and pointer unchanged on failed publication. **GREEN:** `scripts/spur-cargo test -p spur-graph store::cache::tests`; generic Parquet and incremental suites. **Export/negative:** complete cache key, old/new artifact/manifest helper outputs, actual Arc reuse/pointer outcomes; same-source/different-provider alias must reject/reload. Hot immutable Arc behavior is not disk monitoring.

## G11 — Generic memory/disk/overlay query and frozen validation

**Task ID:** G11. **Depends on:** G10. **Files:** Create `src/evidence/query.rs`; modify `src/query_client.rs`, `src/store/evidence.rs`, `src/evidence/mod.rs`; create `tests/generic_evidence_queries.rs` (five files). **Scope:** generic optional trait method/default, &T/memory/Parquet/OverlayClient implementations, single frozen contexts, typed default shims call adapter functions from G02. Remove built-in duplicated typed context routing once shims work. No generation/runtime yet.

```rust
#[test]
fn generic_counter_query_is_not_default_unavailable() {
    use spur_graph::GraphQueryClient;
    let root = tempfile::tempdir().unwrap();
    let mut graph = spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    graph.evidence.replace(counter_extension(1)).unwrap();
    let memory = spur_graph::InMemoryClient::new(std::sync::Arc::new(graph));
    let result = memory.evidence(&EvidenceFilter { provider_id: "test.counter".into(),
        file_path: None, source_oid: None }).unwrap();
    assert_eq!(result.tables.len(), 1); // opaque fixture remains semantically unvalidated here
    assert_eq!(result.semantic_status, EvidenceSemanticStatus::UnknownProvider);
}
```

Compile RED becomes assertion RED against the Unavailable empty default. Register TestCounterProvider only in explicit test contexts for Complete tests; default production registry preserves it as opaque. Add typed Rust parity/default custom backend cases, source shape/stale-OID/UnsupportedSource, selected whole-file zero/Unknown/delete/readd, known legacy and unknown custom domains. Full frozen validation precedes filters; after first query corrupt/delete tables and assert prior context unchanged with zero further evidence IO, fresh client error. Provider needs actual graph owner projection as currently used; no unrelated history/Lance hydration. **GREEN:** generic/Rust queries/overlays, macro_invocation_queries, query_client_parity and query_client unit suites. **Export/negative:** ordered selected rows/status/origins, IO counts, cold/frozen errors; a wrong off-filter body/scalar must still fail.

## G11b — Generation ownership and all request forwarding wrappers

**Task ID:** G11b. **Depends on:** G11. **Files:** Modify `src/overlay_generation.rs`, `src/mcp/mod.rs`, `src/mcp/request_replay.rs`; extend generic query tests (four files). **Scope:** generation captures generic base/override inventory with private digest; forward `evidence` through Arc<OverlayGeneration>, PinnedGenerationClient, MeasuredOverlayClient and RequestReplayClient. Existing counting semantics remain.

Runnable regression: build a captured real Rust graph/generation using current `OverlayGeneration::seed` construction helpers in `tests/overlay_client.rs`, query through `&dyn GraphQueryClient` with provider `spur.rust.physical`, and assert rows/status equal InMemoryClient; repeat via private wrappers in their existing modules. Use the actual constructor signature at source-map's generation anchors, not a newly invented constructor. The current Arc implementation returns default empty Unavailable; require that behavioral RED. The complete seed regression is:

```rust
#[test]
fn generation_forwards_recorded_rust_evidence() {
    use spur_graph::GraphQueryClient;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = std::sync::Arc::new(spur_graph::artifact_from_facts(
        &spur_graph::build_facts(root.path(), None).unwrap().0, root.path()).unwrap());
    let memory = spur_graph::InMemoryClient::new(std::sync::Arc::clone(&graph));
    let generation = std::sync::Arc::new(spur_graph::OverlayGeneration::seed(graph).unwrap());
    let f = EvidenceFilter { provider_id: "spur.rust.physical".into(),
        file_path: None, source_oid: None };
    assert_eq!(generation.evidence(&f).unwrap(), memory.evidence(&f).unwrap());
}
```
 Add File-scoped counter override/delete/readd and unknown-provider tests; preserve previous generation's frozen result after update. **GREEN:** generic query tests, `scripts/spur-cargo test -p spur-graph --test overlay_client --test rust_evidence_overlays`; `scripts/spur-cargo test -p spur-graph mcp::request_replay::tests`; generation-route unit tests. **Export/negative:** actual full wrapper chain outputs and base/override original provenance; independent missing-forwarder and old-row resurrection mutations. Historical default-Unavailable observations must not count as GREEN.

## G12 — Aggregate snapshot, visible, exact, seed and inflight identity

**Task ID:** G12. **Depends on:** G11b. **Files:** Modify `src/mcp/overlay_snapshot.rs`, `src/mcp/request_cache.rs`, `src/mcp/mod.rs` snapshot/visible projections only (three files). **Scope:** exact indexed_evidence_digest field propagated from actual backend owner through all spec carriers/conversions and test helpers; generic raw-manifest metadata cases.

```rust
#[test]
fn compatibility_includes_every_provider() {
    let root = tempfile::tempdir().unwrap();
    let before = cache_identity(root.path(), 1); // existing request_cache.rs:690 helper
    let mut changed = before.clone();
    changed.indexed_evidence_digest = "different-complete-inventory".into();
    assert_ne!(OverlayGenerationCompatibilityKey::from(&before),
               OverlayGenerationCompatibilityKey::from(&changed));
}
```

Initial compile RED is a field introduction; after fields compile, require actual provider-only behavioral regressions with fixed native observed source/head/index/changed vectors. Export both controlled vector and raw later snapshot as the protected D9 technique does. Test visible canonicalization cannot return first identity across capabilities; exact hits, distinct delta/generation inflight leaders, compatible seed exclusion, same-mtime actual byte key, retry/drop/unwind and all retained identity components. **GREEN:** request_cache and overlay_snapshot unit suites plus mcp_overlay_notfound_retry. **Export/negative:** complete before/after visible/exact/delta/compatible keys, actual helper digests, Arc hits/build/seed/inflight events. ROOT independently aliases counter-only mutation to old key; no Python expected cache outputs.

## G13 — Trusted runtime freshness and late disk-owner pairing

**Task ID:** G13. **Depends on:** G12. **Files:** Modify `src/mcp/overlay_runtime.rs`, `src/mcp/mod.rs`; create `src/mcp/generic_evidence_cache_tests.rs` as cfg(test) module (three files). **Scope:** private runtime key `new(PathBuf,String,String)`, actual built-base digest check, current metadata guard before published route, captured Parquet owner and late source/capability guard. Keep source-only public generation identity.

First runnable REDs use existing runtime/build test helpers to construct key from base A, supply valid equal-source base B with counter-only change, and assert BuiltOverlayGeneration::new errors; retain CURRENT during published-route replacement and assert old actor cannot serve; swap later disk base and assert source/capability pairing error. The protected Rust-first tests show fixture techniques only; port generic assertions against current base, never cherry-pick its unapproved production code. **GREEN:** `scripts/spur-cargo test -p spur-graph mcp::overlay_runtime::tests`; `scripts/spur-cargo test -p spur-graph --lib mcp::tests::generation_route_`; focused generic cache module and retry tests. **Export/negative:** actual keys/owners/route/retirement/publication, raw manifest identities, cold unsupported/incomplete errors, prior frozen results. Independent counter alias and valid late substitution must fail. Preserve two open attempts, capacities, per-coordinator ownership/restart and all filesystem guards.

## G14 — Fresh native generic acceptance and independent review packet

**Task ID:** G14. **Depends on:** G13. **Files:** Extend `tests/generic_evidence_contract.rs`, `tests/generic_evidence_parquet.rs`, `tests/generic_evidence_queries.rs` and cfg(test) `src/mcp/generic_evidence_cache_tests.rs` only (four files). **Scope:** consolidate authenticated native exports/negative controls, no further production feature. A discovered production defect returns to the owning task with its own PRE/RED/fix cycle.

This is acceptance, not a fabricated new production RED. Retain actual earlier REDs and independently mutate actual current exports for every pre-input case/false claim. Include counter-only add/remove/version/presence/checksum and opaque provider controls, old artifacts, row-source/body/ABI/origin forgery, missing zero tables, frozen/cold and every generation/wrapper/cache/runtime boundary. Capture complete actual executable/compiler/config/source/wrapper identities and source physical bytes. **Fresh normal GREEN:** `scripts/spur-cargo test -p spur-graph`; `scripts/spur-cargo test -p spur-cli --test analyst_temporal_views`; affected cross-crate checks; local fmt. Run required focused module suites with generic exports to preserve attribution. Retain known unrelated macro baseline bd-xb5p honestly if encountered, not reclassified to success.

**POST:** ROOT chooses current catalog encodings, checks complete actual relations against independent source oracles, persists/reloads full receipts and independent mutation outcomes, then independent code/design review. Historical 13 tests/23 native records or historical receipts do not discharge a generic case. Unknown/timeout/blocked interpretation is not SAT/pass. Do not raise solver/runtime limits or change interpretation thresholds.

## ROOT-final — New notebook/native Rust/SQL acceptance

**Task ID:** ROOT-final. **Depends on:** G14 approved. **Owner:** ROOT. **Files:** NEW ROOT-selected notebook and new final evidence paths, outside this documentation task. Read notebook MCP skill before authoring; preserve old notebooks/evidence and live external graph/CURRENT. Use fresh generic Jev PRE and current compiled native replay. Retain old D10's independently marked six-site/five-body physical source, healthy zero, FFI, cfg/recovery, incremental/delete/readd, original source inventories and SQL joins. Add fixture second-provider and opaque-provider source/table/cache cases. Actual notebook/native/SQL outputs plus persisted/reloaded POST and independent forgery review are the final acceptance; this plan supplies no substitute output or approval.

## Coverage and handoff

G01/G02 own genericity and source semantics; G03/M/G04/G05 own exact compatibility and compile-safe API fallout; G06/G07 own full/selected/reuse/delta; G08/G09 own relational persistence and old bytes/origins; G10 owns publication; G11/G11b own frozen queries and the acknowledged Arc/wrapper gap; G12/G13 own all cache/runtime identities; G14/ROOT-final own fresh native/SQL review. Every task is serial, all dependencies are explicit, and each has isolated file ownership. ROOT reviews this proposal, rebinds the integrated source for fresh generic PRE, then submits the authorized work without another user confirmation.
