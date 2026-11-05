# Rust Evidence Durability Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a Beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `docs/research/artifacts/2026-11-02-rust-gap-closure-grounding/REPORT.md`, approved G3, refined by `docs/superpowers/plans/2026-11-06-rust-evidence-durability-grounding.md`.
**Formal @spec cells:** none.
**Design epic:** `bd-3cs` (closed). Source phase `bd-3l2`; grounding canonical issue `bd-kom5`.
**Goal:** Persist canonical physical Rust declaration and invocation/body evidence through builds, Parquet, queries, replacement overlays and caches without losing uncertainty or original provenance.
**Architecture:** One optional versioned carrier wraps original public declaration v2 and call v1 sets per physical file. Deduplicated original source inventories permit canonical recollection on original snapshot views; per-file unknown calls coexist with intact physical declarations and healthy neighbors. Core tables and a dedicated checksum bind transport, while canonical artifact hashing and capability digests bind publication/cache identity.
**Tech Stack:** Existing Rust workspace, tree-sitter collectors, Serde, Arrow/Parquet, DuckDB and source-bound catalog/Jev review. No dependency or quota change.
**Grounded base:** `39cac3bda0ff9059ffac01a3f9ba03c0da9cc311`; [report](../../research/artifacts/2026-11-06-rust-evidence-durability/grounding/REPORT.md), [exact hashes/symbols/constructor inventory](../../research/artifacts/2026-11-06-rust-evidence-durability/grounding/source-map.json).

This is prospective implementation, not executed TDD or G3 acceptance. The current grounding task writes only three documentation files and runs no Cargo or solver. ROOT reviews this plan before submitting successors. All tasks use the approved `agent=codex/profile=codex/model=gpt-6.1-sol/effort=xhigh`; D10 is ROOT-owned review/replay. Workers leave canonical issues in progress and never self-approve, close, merge or dispatch successors.

## Frozen interface contract

D1 creates `crates/spur-graph/src/rust_evidence.rs`, exports it from `lib.rs`, and uses these exact public names/signatures, except that the artifact-only capability digest is introduced in D2 after the artifact field exists. The code below specifies fields and signatures; it is not a production prototype. All new stored structs derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` and deny unknown fields. Enums serialize in snake_case. Existing collector types retain their current derives/serialization unchanged.

```rust
pub const RUST_EVIDENCE_SCHEMA_VERSION: &str = "spur-rust-evidence-v1";
pub const RUST_EVIDENCE_EXTRACTOR_VERSION: &str = "rust-evidence-wrapper-v1";

pub struct RustEvidenceOriginSource {
    pub path: String,
    pub stable_file_id: String,
    pub content_oid: String,
}
pub struct RustEvidenceOrigin {
    pub graph_content_hash: String,
    pub manifest_version: String,
    pub sources: Vec<RustEvidenceOriginSource>,
}
pub enum RustEvidenceUnknownReason {
    InvalidUtf8,
    ParseRecovery,
    UnevaluatedCfg,
    ParseUnavailable,
}
pub enum RustCallEvidence {
    Complete { evidence: crate::panic_evidence::RustCallSiteEvidenceSet },
    Unknown { reason: RustEvidenceUnknownReason },
}
pub struct RustEvidenceFile {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub byte_length: usize,
    pub origin_graph_content_hash: String,
    pub declarations: crate::RustDeclarationEvidenceSet,
    pub calls: RustCallEvidence,
}
pub struct RustEvidenceSet {
    pub schema_version: String,
    pub extractor_version: String,
    pub graph_content_hash: String,
    pub origins: Vec<RustEvidenceOrigin>,
    pub files: Vec<RustEvidenceFile>,
}
pub fn collect_rust_evidence(
    graph: &crate::GraphIndexArtifact,
    sources: &[crate::RustDeclarationSource<'_>],
) -> anyhow::Result<RustEvidenceSet>;
impl RustEvidenceSet {
    pub fn validate_stored(&self, graph: &crate::GraphIndexArtifact)
        -> anyhow::Result<()>;
    pub fn validate_sources(&self, graph: &crate::GraphIndexArtifact,
        sources: &[crate::RustDeclarationSource<'_>]) -> anyhow::Result<()>;
}
pub fn rust_evidence_checksum_blake3_hex(set: &RustEvidenceSet)
    -> anyhow::Result<String>;
pub fn rust_evidence_capability_digest(artifact: &crate::GraphIndexArtifact)
    -> anyhow::Result<String>;
```

`collect_rust_evidence` requires exactly the graph's eligible physical `.rs` manifest domain, not an arbitrary subset silently advertised as complete. Relative normal path components, no `#cell:` and a unique actual file/manifest ID/OID are required. Exclude virtual cells and `external://` placeholders. Preserve existing stricter declaration path policy rather than broadening it during transport. Non-Rust physical sources can appear in the origin inventory because the graph hash covers them, but do not have Rust evidence file entries.

Each file is collected separately through existing `collect_rust_declarations` and `collect_rust_call_sites`. The declaration set contains one source and retains all tuple, FFI and block rows, including valid recovered rows with Unknown availability. Authenticate bytes/manifest/owners before classifying call uncertainty. Classify invalid UTF-8 first, parser unavailable second, recovered syntax third, then cfg/cfg_attr using the same non-macro AST policy as R3. For healthy syntax call R3 and propagate every error. Unknown carries no call set and no fake zero counts; physical declarations are independent. Never match arbitrary error strings to downgrade integrity failures. Do not change R3's rejection policy or synthesize foreign bodies.

Origins are keyed by `(manifest_version, graph_content_hash)`; within this version all nested collectors must use the supported current manifest/extractors. Each origin stores a sorted, unique complete original manifest inventory. Recompute its hash with existing `compute_graph_content_hash`. `RustEvidenceFile.origin_graph_content_hash` and both nested set headers agree with that origin. `declarations.sources` and complete-call `sources` must each contain exactly that file, with identical ID/OID/byte length. Aggregate carrier hash equals the **current** artifact source hash. Sort files by path, origins by key; retain the canonical order within nested sets. Reject noncanonical ordering instead of sorting away evidence mutations.

`validate_stored` checks exact active-file domain, versions, origin existence/hash, source identities, counts, duplicate IDs, range bounds, site/body same-file/source joins, definition/structural/lexical owner membership and raw metadata bounds. Recompute existing body/site identity encodings from public row values with a crate-private helper shared with `call_sites.rs` if needed; changing their algorithm/version is forbidden. Absent foreign bodies and non-executable blocks are enforced. It cannot detect every omitted syntax record without source bytes.

`validate_sources` constructs an origin view: clone the current authoritative graph, clear optional evidence, replace graph hash and manifests with the retained origin inventory (`node_ids=[]`), retain current actual files/symbols for the selected source and precheck its unchanged ID/OID. Then invoke the existing nested `validate` methods with exactly one source. Recompute Unknown classifications and compare exact declaration contents too. This preserves original set headers and reuses canonical validation unchanged. Current source/owner bytes are authoritative; an origin is recorded provenance, not proof of a historical external compiler run.

D2 adds `#[serde(default, skip_serializing_if = "Option::is_none")] pub rust_evidence: Option<crate::RustEvidenceSet>` to `GraphIndexArtifact` and a `validate_rust_evidence_sources(&self, sources: &[RustDeclarationSource<'_>]) -> anyhow::Result<()>` convenience method. `None` means unavailable, never complete empty. The convenience method returns an actionable unavailable error for `None`; it must not succeed as a completeness assertion. Optional absence stays omitted from `GraphArtifactBodyForHash` to preserve legacy bytes. Present evidence, including empty files/origins, enters canonical body hashing. Dedicated checksum domain is `b"spur-rust-evidence-tables-v1\0"` followed by canonical envelope JSON. Capability digest hashes a tagged absent/present variant, the wrapper/declaration/call schema and extractor versions, current manifest and present checksum; absence and present-zero have different digests.

D3 adds `pub fn artifact_from_facts_for_paths(facts: &GraphFacts, root: &Path, paths: &[PathBuf]) -> anyhow::Result<GraphIndexArtifact>` in `store/build.rs`, reexported from `store/mod.rs`. It restricts discovery/composition to normalized selected paths before collecting evidence. This fixes the current partial-facts/full-discovery ambiguity for overlay extraction without claiming empty evidence for unextracted files. Full builds require complete facts and authenticate every selected source. Evidence classification never fills missing graph owners. D4 carries optional `RustEvidenceFile` plus required origin records through `FileBucket`, with no new `GraphFacts` syntax carrier.

## Persisted row contract

D5 creates `store/rust_evidence.rs` and adds optional `rust_evidence: Option<RustEvidenceManifest>` to `GraphArtifactManifest`. The exact public metadata types are below; they derive the stored-struct traits and strict decoding specified above. A complete transport may contain Unknown files. Avoid five separate optional fields in `GraphArtifactRowCounts`; only the two writer manifest literals and the analyst manifest fixture require the new metadata member.

```rust
pub struct RustEvidenceRowCounts {
    pub origins: usize,
    pub sources: usize,
    pub declarations: usize,
    pub bodies: usize,
    pub sites: usize,
}
pub struct RustEvidenceManifest {
    pub complete: bool,
    pub schema_version: String,
    pub extractor_version: String,
    pub declaration_schema_version: String,
    pub declaration_extractor_version: String,
    pub call_schema_version: String,
    pub call_extractor_version: String,
    pub checksum_blake3: String,
    pub row_counts: RustEvidenceRowCounts,
}
pub fn rust_evidence_manifest_capability_digest(manifest: &GraphArtifactManifest)
    -> anyhow::Result<String>;
```

The manifest digest helper and D2's artifact helper encode identical tagged presence/version/manifest/checksum inputs. D5 tests their equality for both None and Some, including zero and Unknown. This is the predecessor API for D6 and D9; it avoids a second, incompatible cache identity encoding.

Every table exists with its exact schema even at zero rows. All tables carry identical metadata: graph disk schema, manifest version, aggregate source hash, wrapper/declaration/call schema/extractor, source scope `supplied_physical_rust_files`, evidence checksum and table name. Use UTF-8 strings, nullable UTF-8 where stated and checked nonnegative Int64 for offsets/counts/lengths. Convert usize with checked conversions. JSON columns are strict lossless Serde encodings, not substitute lossy summaries.

| Table | Exact columns / row interpretation |
| --- | --- |
| `rust_evidence_origins.parquet` | `origin_graph_content_hash`, `manifest_version`, `sources_json` (full ordered `Vec<RustEvidenceOriginSource>`). One row per referenced original snapshot, even after composition; unreferenced origins reject. |
| `rust_evidence_sources.parquet` | `file_path`, `stable_file_id`, `source_oid`, `byte_length`, `origin_graph_content_hash`, `declaration_header_json` (schema/extractor/hash/scope/covered_kinds), `declaration_source_json` (one full `RustDeclarationFile`), `call_availability` (`complete` or `unknown`), nullable `call_unknown_reason`, nullable `call_header_json` (schema/extractor/hash/source_scope), nullable `call_source_json` (one full `RustCallSourceBinding`). Complete has both call JSON values and no unknown reason; Unknown has reason and neither value. This is the zero-source/zero-row coverage authority. |
| `rust_evidence_bodies.parquet` | `file_path`, `source_oid`, `origin_graph_content_hash`, `body_id`, `byte_range_start`, `byte_range_end`, `kind`, nullable `definition_stable_symbol_id`. Exact `RustBodyEvidence` reconstruction. |
| `rust_evidence_sites.parquet` | `file_path`, `source_oid`, `origin_graph_content_hash`, `site_id`, `body_id`, `byte_range_start`, `byte_range_end`, `call_text`. Exact `RustCallSiteEvidence` reconstruction. |
| `rust_declarations.parquet` | `file_path`, `stable_file_id`, `source_oid`, `origin_graph_content_hash`, `stable_symbol_id`, `kind` (`tuple_field`, `foreign_function`, `extern_block`), `structural_owner_json`, `lexical_owner_json`, `full_range_start`, `full_range_end`, `line_start`, `line_end`, `payload_json` (full corresponding existing typed row). Every duplicated scalar must agree with decoded payload. The payload preserves ordinals/type/core ranges, names, ABI, body tags, qualifiers, visibility and attributes. |

The reader reconstructs the exact canonical envelope and verifies shared checksum before exposing any filtered row. Missing, truncated, extra, count-mismatched, metadata-mismatched or invalid JSON/enum tables reject, including zero-row tables. Absent metadata requires all five tables absent; otherwise reject orphan evidence. Declared unsupported wrapper/disk schema/extractor yields `incompatible Rust evidence; rebuild with spur explore sync` with actual/expected versions and path. Recognized legacy absence returns `None` without inventing source rows. Introduce disk schema `spur-graph-schema-v13` in D5, retaining the existing graph extractor/resolver constants because their extraction/resolution semantics are unchanged. The dedicated wrapper extractor version owns the added evidence contract. The schema bump changes `current_manifest_version` and requires fresh collection. Existing v12 macro artifacts remain readable under explicit legacy-without-Rust compatibility, but cannot seed supported Rust reuse. The macro reader currently compares against `SCHEMA_VERSION` exactly and hardcodes it in table metadata: accept only the known old `(v12, existing extractor)` pair when Rust metadata is absent, and compare its tables against the actual stored v12 metadata without rewriting it. Unknown declared Rust capabilities still reject.

Full and delta writers validate stored structure and write all five tables before complete manifest publication. Full/slim readers validate with manifests/owner projections; slim can clear general manifests only after preserving the full evidence envelope/source bindings. History-only projections remain `None`; they cannot become current seeds. Do not manufacture or verify a full canonical artifact hash from a slim/history projection. Existing disk manifests do not store the general canonical body hash; use the dedicated envelope checksum for Rust transport and source validation for authenticity.

D7 adds these public query types in `rust_evidence/query.rs` and exports them through `rust_evidence.rs`. Use existing Serde/Eq conventions. The intentionally small filter supports exact physical file and source, not compiler callees.

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustEvidenceFilter {
    pub file_path: Option<String>,
    pub source_oid: Option<String>,
}
pub enum RustEvidenceAvailability { Complete, Partial, Unavailable, UnsupportedSource }
pub struct RustEvidenceQuerySource {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
    pub availability: RustEvidenceAvailability,
}
pub struct RustEvidenceQueryResult {
    pub availability: RustEvidenceAvailability,
    pub source_domain: Option<Vec<RustEvidenceQuerySource>>,
    pub files: Vec<RustEvidenceFile>,
    pub origins: Vec<RustEvidenceOrigin>,
}
impl RustEvidenceFilter {
    pub fn validate_shape(&self) -> anyhow::Result<()>;
}
// Default GraphQueryClient method; add forwarding through &T too.
fn rust_evidence(&self, filter: &RustEvidenceFilter)
    -> anyhow::Result<RustEvidenceQueryResult>;
```

`source_oid` requires `file_path` and must match that file's recorded OID before row filtering, otherwise error. Shape errors precede unavailable/unsupported handling, including in the default trait method. Nonphysical/virtual requests return UnsupportedSource; a missing physical path is Unavailable, not Complete zero. A selected healthy file is Complete even at zero rows; a file with either Unknown lane is Partial and retains available rows. All-files empty captured carrier is Complete. All-files legacy absence is Unavailable. Native memory/disk clients populate `source_domain=Some(...)` from actual eligible graph file manifests even when the evidence carrier is absent; legacy paths get Unavailable status and no fabricated evidence file. A custom default backend returns `source_domain=None`, explicitly unknown coverage. Thus an overlay can remove a shadowed unavailable legacy path, distinguish a live unavailable replacement from deletion, and compute availability over the actual remaining active domain. If a custom backend's domain is unknown, an aggregate with some captured evidence is conservatively Partial; it cannot claim complete coverage. Results retain entire per-file groups, including source and zero-row records, and only referenced origins; they are projections, not `RustEvidenceSet` values with false full-artifact coverage.

## Common execution / evidence protocol

For **every** D1–D9 task: ROOT binds fresh PRE to that task's integrated predecessor source hashes, fixture bytes and independent source oracle **before any edit**. Never reuse C/G2 receipts as G3 PRE. Verify live canonical issue, current task/delegation, dependencies and assignment; persist/replay UUID INTENT through `report_audit`. On authorization rejection save exact request/error, report progress and stop for ROOT repair. Emit durable progress at least every 60 seconds. Use `test(spur-graph): Dn ...` followed by `feat`/`fix(spur-graph): Dn ...` commits, retaining observed RED and GREEN logs. A compiler error for a new API is explicitly **compile RED**, not a behavioral regression or a passing test. Add a compiling assertion as soon as predecessor APIs make one possible; verify the asserted failure and the final pass. Commands below are prospective and must actually run in the implementation task.

Catalog discovery in grounding found `data_integrity` v1 / `finite_relational_snapshot` and `configuration` v1 / `finite_compatibility`. ROOT must freshly call `solve_rule_spec` for each selected rule's exact request encoding before execution and bridge Jev interpretation. Candidate models are finite relational exports, not invented receipt IDs or guessed solver JSON. Use:

- `data_integrity.unique` for source path, origin key, declaration ID and body/site IDs.
- `data_integrity.foreign_key` for source→origin, body→source, site→body and declared graph owners.
- `data_integrity.cardinality` for independent source-domain and per-file counts, including zero rows.
- `data_integrity.mutually_consistent` for exact independently derived `(file,OID,site,body,range)` and declaration owner/ABI/body tuples. Existence/FK alone cannot reject a wrong existing body.
- `data_integrity.conditional_required` for complete versus unknown payloads and absent versus present capability.
- `configuration.attribute_allowed_pair` for supported schema/extractor, foreign declaration/body category and phase/outcome compatibility.

The actual PRE inputs for each task are the relations named in its evidence paragraph, with columns from the frozen contracts above; ROOT's current catalog encoding determines solver syntax. Required facts come from independent fixture source spans and predecessor exports, never from copying the output under test into both expected and observed columns. Stable IDs are observed using actual public APIs and joined to independently established source coordinates. Negative controls independently change one observed row or metadata cell; keep originals and mutation requests. Verify mode valid models must be SAT and invalid models UNSAT; unknown/timeout/interpretation-blocked are not success. Persist each POST, reload exact request/result by receipt ID, independently mutate actual exported output and retain requests, raw status, source identities, normalizations and Jev result before ROOT approval.

All GREEN commands use `scripts/spur-cargo`; remote failure is real. Do not bless snapshots or bypass tests to obtain a pass. `scripts/spur-cargo fmt -- --check` is local. No Cargo command in this document has run during grounding.

## Task D1: Canonical per-file envelope and origin validation

**Task ID:** `D1-rust-evidence-contract`. **Depends on:** none (approved integrated G2 is prerequisite). **Suggested Worker:** codex, approved profile/model/effort above.

**Files:** Create `crates/spur-graph/src/rust_evidence.rs` and `crates/spur-graph/tests/rust_evidence_contract.rs`; modify `crates/spur-graph/src/lib.rs`. Permit a crate-private identity helper extraction in `panic_evidence/call_sites.rs` only if needed for identical stored identity checks; no public collector semantics/IDs change.

**Scope:** Implement the frozen wrapper/origin/checksum APIs, per-file classification and stored/source validation. Defer `rust_evidence_capability_digest(artifact)` to D2: D1 has no artifact carrier field to inspect. No artifact field, builders, Parquet or query changes. Source collectors remain canonical and public as today.

**First RED:** New-API compile RED is unavoidable because there is no wrapper surface. This is the complete initial integration test seed; it uses existing graph-building APIs and the exact new D1 APIs.

```rust
use std::fs;
use spur_graph::{artifact_from_facts, build_facts, collect_rust_evidence,
    RustCallEvidence, RustDeclarationSource, RustEvidenceUnknownReason};
#[test]
fn mixed_sources_retain_zero_and_unknown() {
    let root = tempfile::tempdir().unwrap();
    let healthy = b"pub struct Empty;\n";
    let cfg = b"#[cfg(feature = \"x\")] fn hidden() { leaf(); }\n";
    fs::write(root.path().join("empty.rs"), healthy).unwrap();
    fs::write(root.path().join("cfg.rs"), cfg).unwrap();
    let graph = artifact_from_facts(&build_facts(root.path(), None).unwrap().0,
        root.path()).unwrap();
    let sources = [RustDeclarationSource { file_path: "empty.rs", bytes: healthy },
        RustDeclarationSource { file_path: "cfg.rs", bytes: cfg }];
    let set = collect_rust_evidence(&graph, &sources).unwrap();
    assert_eq!(set.files.len(), 2);
    let empty = set.files.iter().find(|f| f.file_path == "empty.rs").unwrap();
    assert!(empty.declarations.rows.is_empty());
    match &empty.calls {
        RustCallEvidence::Complete { evidence } => {
            assert_eq!(evidence.sources.len(), 1);
            assert!(evidence.sites.is_empty() && evidence.bodies.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(&set.files.iter().find(|f| f.file_path == "cfg.rs").unwrap().calls,
        RustCallEvidence::Unknown { reason: RustEvidenceUnknownReason::UnevaluatedCfg }));
    set.validate_sources(&graph, &sources).unwrap();
    let mut omitted = set.clone();
    omitted.files.retain(|f| f.file_path != "empty.rs");
    assert!(omitted.validate_sources(&graph, &sources).is_err());
}
```

**Implementation/acceptance:** Implement each frozen type, authenticate the complete eligible source domain, retain one origin for this collection and invoke canonical collectors per file. Add independent cases for recovery with healthy neighbors, zero declarations but nonzero bodies, invalid UTF-8, duplicate/stale/wrong-existing sources, omitted/duplicate sites and bodies, same-file wrong existing body, wrong existing structural/lexical owner, wrong ABI/body tags and forged origin hash. Preserve foreign body absence. Every mutation must change a value; record whether stored or source validation rejects it. Source validation must reject all canonical substitutions even when a reference still exists. Existing R3 and declaration suites remain unchanged.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_contract`; `scripts/spur-cargo test -p spur-graph --test panic_call_sites --test rust_declaration_evidence --test rust_foreign_evidence`.

**PRE/POST inputs:** Independently span-marked healthy/zero/cfg/recovery/FFI sources; relations `sources(path,file_id,oid,bytes,decl_status,call_status)`, `origins(hash,manifest,path,oid)`, `bodies(id,path,oid,range,kind)`, `sites(id,path,oid,body,range,text)` and `declarations(id,kind,owners,range,abi,body_tag)`. Rules: unique/FK/cardinality/exact mutually-consistent tuples/conditional payloads. POST exports actual wrapper JSON and canonical mutation errors; ROOT mutates an existing-body assignment and missing zero source independently.

## Task D2: Optional artifact field, hash and mechanical constructors

**Task ID:** `D2-rust-artifact-carrier`. **Depends on:** `D1-rust-evidence-contract`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `schema.rs`, `store/canonical_hash.rs`, `src/rust_evidence.rs` (artifact capability helper only), `tests/rust_evidence_contract.rs` under `crates/spur-graph`; allow only the mechanical `rust_evidence: None` addition at all **64 literal sites in the 32 files** enumerated in `grounding/source-map.json`. This includes other crates' tests/fixtures/benches and history constructors. No other cross-crate logic change. This atomic Rust struct field addition is the justified exception to the small file-count guideline.

**First compiling assertion RED:** Add this test to the D1 integration file before adding the field. Every API below exists by D1; current strict artifact decoding rejects the new member.

```rust
#[test]
fn artifact_accepts_present_empty_and_keeps_legacy_absent() {
    let root = tempfile::tempdir().unwrap();
    let graph = artifact_from_facts(&build_facts(root.path(), None).unwrap().0,
        root.path()).unwrap();
    let set = collect_rust_evidence(&graph, &[]).unwrap();
    let mut value = serde_json::to_value(&graph).unwrap();
    value["rust_evidence"] = serde_json::to_value(&set).unwrap();
    let decoded = serde_json::from_value::<spur_graph::GraphIndexArtifact>(value);
    assert!(decoded.is_ok(), "present evidence must decode: {decoded:?}");
    let mut legacy = serde_json::to_value(&graph).unwrap();
    legacy.as_object_mut().unwrap().remove("rust_evidence");
    assert!(serde_json::from_value::<spur_graph::GraphIndexArtifact>(legacy).is_ok());
}
```

**Implementation/acceptance:** Add the field/convenience validator, artifact capability digest helper and canonical hash member. Use a typed decoded envelope when testing hash mutations; compare absent legacy canonical bytes to the existing omission layout without altering its snapshot. Assert absent versus Some(empty) hashes differ; changing availability, original inventory, body/site or FFI payload changes the canonical body hash, while changing only header hash does not. Validate source bindings separately after rehashing a forgery: recomputing a hash does not repair canonical content. Builders still default None until D3. Confirm every inventory literal is accounted for, including history projections; no unrelated refactor.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_contract`; `scripts/spur-cargo test -p spur-graph store::canonical_hash::tests`; `scripts/spur-cargo check --workspace --all-targets` (mechanical API fallout requires all targets). Run normal crate tests for any unexpected semantic change before continuing.

**PRE/POST inputs:** D1 actual export plus independent legacy canonical-byte fixture, paired present/absent/version/availability variants. Relations `artifact_variant(capability,canonical_body_hash,envelope_checksum,source_hash)` and exact field-presence metadata. Rules conditional_required, unique and allowed schema/body pairs. POST actual serialized artifacts/hash bytes; negatives drop optional evidence, forge the source hash or mutate one body while retaining old checksum. ROOT separately checks legacy omission, not only hash length.

## Task D3: Full-build collection and selected-path composition

**Task ID:** `D3-rust-full-build`. **Depends on:** `D2-rust-artifact-carrier`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `crates/spur-graph/src/store/build.rs`, `src/store/mod.rs`, `src/query_client.rs` (`OverlayClient::extract_delta` call-site only); create `crates/spur-graph/tests/rust_evidence_build.rs`.

**First compiling assertion RED:** Current builder serializes no evidence; no new API is used here.

```rust
use std::{fs, path::Path};
use spur_graph::{artifact_from_facts, build_facts};
fn build(root: &Path) -> spur_graph::GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}
#[test]
fn full_build_captures_zero_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = build(root.path());
    assert!(serde_json::to_value(&graph).unwrap()["rust_evidence"].is_object());
    let set = graph.rust_evidence.as_ref().unwrap();
    assert_eq!(set.files.len(), 1);
    assert_eq!(set.files[0].file_path, "empty.rs");
    assert!(set.files[0].declarations.rows.is_empty());
}
```

**Implementation/acceptance:** Collect only after authoritative owners/manifests exist. Read exact file bytes and reject an OID race; no parse/cfg error can poison healthy neighbors. Implement selected-path public builder from the frozen interface and route overlay extraction through it. Test a two-file root with facts extracted for only `a.rs`: selected builder's manifest/evidence contain only `a.rs`, never a fabricated complete zero for `b.rs`. Full builder is for full facts; missing owners are errors. Include an actual empty workspace, tuple/FFI/block records and cfg/recovered sources. Keep macro semantics and existing extraction counts intact.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_build`; `scripts/spur-cargo test -p spur-graph --test macro_invocation_artifacts --test rust_foreign_evidence`.

**PRE/POST inputs:** Independent discovered versus selected path inventories, exact source hashes and graph owner coordinates; D1/D2 canonical outputs. Cardinality/unique/exact source joins verify selected domain and healthy-zero coverage. Negative controls add an unselected source, remove a zero source and mismatch read bytes/OID. POST actual full/selected builder exports plus current canonical source validation, independently mutate coverage and owner membership.

## Task D4: Unchanged reuse, changed files and deletion deltas

**Task ID:** `D4-rust-evidence-composition`. **Depends on:** `D3-rust-full-build`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `crates/spur-graph/src/store/build.rs`, `src/rust_evidence.rs` (origin union/pruning helper only), `tests/rust_evidence_build.rs`. No storage/query implementation.

**First compiling assertion RED:** Append to the D3 test file, using its existing `build` helper. Current incremental composition does not retain a Rust file payload. The test must fail at the missing carrier/equality assertion, not on an unrelated fixture error.

```rust
#[test]
fn reuse_keeps_original_snapshot_and_deletion_removes_active_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("keep.rs"), "fn leaf() {} fn keep() { leaf(); }\n").unwrap();
    fs::write(root.path().join("edit.rs"), "pub struct Old(u8);\n").unwrap();
    let before = build(root.path());
    let original = before.rust_evidence.as_ref().unwrap().files.iter()
        .find(|f| f.file_path == "keep.rs").unwrap().clone();
    fs::write(root.path().join("edit.rs"), "pub struct New(u16);\n").unwrap();
    let (after, _, _) = spur_graph::artifact_from_facts_incremental(&before, root.path()).unwrap();
    assert_ne!(after.graph_content_hash, before.graph_content_hash);
    let set = after.rust_evidence.as_ref().expect("incremental carrier");
    assert_eq!(*set.files.iter().find(|f| f.file_path == "keep.rs").unwrap(), original);
    assert_eq!(original.origin_graph_content_hash, before.graph_content_hash);
    fs::remove_file(root.path().join("edit.rs")).unwrap();
    let (deleted, _, _) = spur_graph::artifact_from_facts_incremental(&after, root.path()).unwrap();
    assert!(deleted.rust_evidence.as_ref().unwrap().files.iter()
        .all(|f| f.file_path != "edit.rs"));
    assert!(deleted.tombstones.iter().any(|t| t.path == "edit.rs"));
}
```

**Implementation/acceptance:** Add per-file payload/origin membership to `FileBucket`, `buckets_from_artifact`, empty/facts buckets, `compose_artifact`, `rebuild_from_buckets` and changed/delta paths. Construct graph owners first, then collect newly extracted files; keep unchanged nested sets and original origin inventories byte/value equal. The new aggregate carrier hash describes the current manifest only. Union referenced origins and reject conflicting same-key inventories. Missing/incompatible legacy evidence selects full rebuild; a present forged compatible set is an integrity error, never silently reused. No-op incremental preserves payload, including complete-zero/Unknown sources. Modified file replaces the whole file entry; deletion removes source, declarations, bodies and sites while retaining historical origin inventory only if another live file references it. A delta's active coverage contains changed existing files only; deleted paths are tombstones, and an empty no-change delta has a captured empty carrier with no origin records. Export a delta via existing `write_worktree_delta` only after D5; D4 unit tests may exercise private `build_worktree_delta_artifact` directly without expecting Rust disk transport yet.

For changed facts, call D3's `artifact_from_facts_for_paths` with the changed existing paths, collect their canonical sets on that selected graph, then rebucket its complete file entries and merge with unchanged previous buckets. Its selected snapshot is the newly collected files' recorded origin; aggregate composition does not rewrite it. Do not pass a partial source list to D1's full-domain collector on the complete graph. Full builds still collect on their complete graph after composition. Validate the combined envelope in the existing fallible public builder paths; internal graph-only drafts must not be published with missing evidence coverage.

Test changed extractor/wrapper/call/declaration versions, absence, wrong source OID, swapped owners and a path removed then readded. Source-validation of reused files must use retained origin views. Verify all `BuildMode` and reused/changed/removed statistics without adding a quota.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_build`; `scripts/spur-cargo test -p spur-graph store::build::tests`; `scripts/spur-cargo test -p spur-graph --test integration_git_incremental --test macro_invocation_artifacts`.

**PRE/POST inputs:** Independent before/after path/OID inventories, edit/delete sequence, D3 full exports. Relations `transition(path,old_oid,new_oid,action)`, `active_source(path,origin,oid)`, original/reused nested payload digests and tombstones. Rules cardinality/unique/exact tuples plus conditional replacement. POST actual full/incremental/delta envelopes and validator results; mutate old body ownership, rebind an old nested header to aggregate hash and reinsert one deleted site. FK-only controls must demonstrate why exact correspondence is necessary.

## Task D5: Complete core Parquet transport and compatible readers

**Task ID:** `D5-rust-evidence-parquet`. **Depends on:** `D4-rust-evidence-composition`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Create `crates/spur-graph/src/store/rust_evidence.rs`, `crates/spur-graph/tests/rust_evidence_parquet.rs`; modify `src/store/{parquet,mod,build,macro_invocations}.rs` (build: schema constant only; macro reader: explicit legacy schema compatibility only). Mechanical new `rust_evidence: None` metadata member in `crates/spur-analyst/src/overlay/session_tests.rs:308`. No Lance/runtime/Cargo changes. Extra files are direct transport/version constructor fallout, not a general refactor.

**Exact internal transport signatures:** `pub(super) fn write(artifact: &GraphIndexArtifact, dir: &Path) -> anyhow::Result<Option<RustEvidenceManifest>>`; `pub(super) fn read(dir: &Path, manifest: &GraphArtifactManifest) -> anyhow::Result<Option<RustEvidenceSet>>`. Public manifest/count types use the field definitions in the row contract. A later D7 immutable-table query helper may reuse private decoding functions; do not add a disk query API here.

**First compiling assertion RED:** Existing writers currently omit all Rust tables and readers return None. This uses only current APIs plus the integrated D2 field.

```rust
use std::fs;
use spur_graph::{artifact_from_facts, build_facts, read_artifact_parquet,
    write_artifact_parquet, WriteOptions};
#[test]
fn disk_preserves_complete_zero_source() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = artifact_from_facts(&build_facts(root.path(), None).unwrap().0,
        root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    assert!(out.path().join("rust_evidence_sources.parquet").is_file());
    assert_eq!(read_artifact_parquet(out.path()).unwrap().rust_evidence, graph.rust_evidence);
}
```

**Implementation/acceptance:** Implement all five exact tables and checksum, strict scalar/JSON reconstruction and metadata agreement. Full and delta manifests gain metadata only after successful core writes. Validate entire envelope against source/owner projections in full and slim readers; clear general slim manifests only afterward. Preserve None in history-only projections. Bump the disk schema and retain graph extractor/resolver constants as specified, exercising recognized legacy v12 with Rust absent without breaking macro reads. Unsupported declared Rust versions fail with the rebuild diagnostic. Preserve legacy missing evidence versus current captured zero; metadata `complete` describes transport, not all-source syntax completeness.

For each table, test complete zero rows, missing file, `PAR1` truncation, wrong row count, wrong table metadata, valid-Parquet same-count payload mutation with old checksum, wrong aggregate/source/origin metadata, invalid/null required cells, extra orphan table and failed write before manifest. Reuse the **technique** from `macro_invocation_parquet.rs::tamper_string` to retain valid Parquet metadata while changing one cell; adapt columns to the exact row schema above. Independently rehash a forged payload and demonstrate `validate_sources` still rejects same-file existing-body/owner substitutions; do not claim checksum proves source truth. Read old stored evidence after editing source to prove disk snapshots are retained, then separately validate against saved original bytes. Test full, slim, delta and zero-change/deletion deltas. General artifact hashes are separate from the dedicated transport checksum.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_parquet`; `scripts/spur-cargo test -p spur-graph --test macro_invocation_parquet --test parquet_roundtrip --test parquet_schema_invariants --test load_artifact_dispatch --test diagnostics_persist`; `scripts/spur-cargo test -p spur-analyst overlay` (manifest fixture compatibility). Check version-dependent expectations against actual source; no snapshot blessing.

**PRE/POST inputs:** D4 actual envelopes and independent table schema/expected source inventories. Relations `table(name,present,rows,schema,checksum,complete)`, decoded source/body/site/declaration joins and exact payload tuples. Rules conditional_required/cardinality/unique/FK/exact consistency/schema allowed pairs. POST is actual persisted-and-reloaded Rust output plus actual manifest/Parquet metadata, not an in-memory expected set alone. ROOT independently removes a zero-row source, mutates a valid same-count site cell and mismatches checksum/schema; persist/reload each resulting POST control.

## Task D6: Publication equality and canonical base cache identity

**Task ID:** `D6-rust-evidence-publication-cache`. **Depends on:** `D5-rust-evidence-parquet`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `crates/spur-graph/src/store/cache.rs` and its existing unit tests only. D1 supplies the checksum, D2 supplies the artifact capability API and D5 supplies manifest metadata. No retention/quota change.

**First compiling assertion RED:** Add inside `store/cache.rs`'s unit tests; its private `validate_publication` currently checks macro equality only. Both artifacts are well formed, with identical source and macro evidence, so this is a targeted behavioral failure.

```rust
#[test]
fn publication_rejects_different_rust_capability() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.rs"), "pub struct A;\n").unwrap();
    let facts = crate::build_facts(root.path(), None).unwrap().0;
    let expected = crate::artifact_from_facts(&facts, root.path()).unwrap();
    let mut legacy = expected.clone();
    legacy.rust_evidence = None;
    let out = tempfile::tempdir().unwrap();
    crate::write_artifact_parquet(&legacy, out.path(), crate::WriteOptions::default(), vec![]).unwrap();
    assert!(validate_publication(&expected, out.path()).is_err());
}
```

**Implementation/acceptance:** Compare expected versus reloaded entire Rust envelopes before publishing pointers, including absence. In `read_resolved_base_artifact`, derive a manifest capability identity from supported versions/presence/checksum and include it in `BaseArtifactCacheKey`; also include canonical artifact path to avoid same-source/different-location collisions. Require loaded evidence capability to match the manifest identity before caching. A cache hit represents the immutable published snapshot identified by manifest bytes/capability; publication must not overwrite a different envelope at the same source-hash directory without invalidating that identity. Dedup lookup must validate envelope equality before accepting an existing canonical directory, not just repair Lance sidecars. Reject a same-source wrong capability rather than publishing it. Do not redefine source graph hash as a table hash.

Exercise present→absent, absent→present-zero, checksum/schema/extractor changes under the same source hash, replacement canonical path and publication rejection leaving both pointers untouched. Old Arc readers keep their validated immutable snapshot; a subsequent load of changed manifest identity cannot return that Arc. Malformed newly loaded evidence returns no seed. Out-of-band mutation with unchanged declared checksum is detected by cold decode, while previously validated immutable Arcs are snapshots; do not claim hot caches continuously monitor every table byte.

**GREEN:** `scripts/spur-cargo test -p spur-graph store::cache::tests`; `scripts/spur-cargo test -p spur-graph --test rust_evidence_parquet --test integration_git_incremental`.

**PRE/POST inputs:** Two actual D5 artifacts with equal source hashes but distinct evidence capabilities, before/after pointer contents and base cache keys. Exact equality/conditional-required/allowed-version rules apply. POST exports loaded envelope, capability identity, pointer outcome and Arc reuse observations. Independent negatives substitute a legacy artifact at publication and alias a changed checksum/capability to the old key; verify rejection/invalidation rather than a claimed performance improvement.

## Task D7: Memory and frozen Parquet query parity

**Task ID:** `D7-rust-evidence-query`. **Depends on:** `D6-rust-evidence-publication-cache`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Create `crates/spur-graph/src/rust_evidence/query.rs`, `crates/spur-graph/tests/rust_evidence_queries.rs`; modify `src/rust_evidence.rs` (query module exports), `src/query_client.rs` (trait/reference forwarding, memory and disk contexts), `src/store/rust_evidence.rs` (immutable table input). No overlays or new MCP tools yet.

**Exact internal helper:** `pub(crate) struct RustEvidenceQueryTables`; `impl RustEvidenceQueryTables { pub(crate) fn open(dir: &Path, manifest: &GraphArtifactManifest) -> anyhow::Result<(Option<Self>, Option<RustEvidenceSet>)>; pub(crate) fn query(&self, filter: &RustEvidenceFilter) -> anyhow::Result<RustEvidenceQueryResult>; }`. Freeze all five compressed tables; validate the entire shared checksum before any filter, then discard full decoded rows and query immutable bytes using file-path pruning. Keep source/origin context and needed graph owner projections, independent of later disk/source edits. If the domain is legitimately absent, still reject orphan tables as in D5. Do not hydrate unrelated edges/temporal history or Lance datasets.

**First RED:** This new trait/filter surface has explicit compile RED. Once signatures/defaults exist, the same test gives assertion RED (unavailable or missing rows) before implementation. Existing current APIs construct both clients and persist the fixture.

```rust
use std::{fs, sync::Arc};
use spur_graph::{artifact_from_facts, build_facts, write_artifact_parquet,
    WriteOptions, RustEvidenceFilter, RustEvidenceAvailability};
use spur_graph::query_client::{GraphQueryClient, InMemoryClient, ParquetClient};
#[test]
fn zero_source_and_stale_oid_have_memory_disk_parity() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = artifact_from_facts(&build_facts(root.path(), None).unwrap().0,
        root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let memory = InMemoryClient::new(Arc::new(graph));
    let disk = ParquetClient::open(out.path()).unwrap();
    let filter = RustEvidenceFilter { file_path: Some("empty.rs".into()), source_oid: None };
    let result = memory.rust_evidence(&filter).unwrap();
    assert_eq!(result.availability, RustEvidenceAvailability::Complete);
    assert_eq!(result.files.len(), 1);
    assert_eq!(disk.rust_evidence(&filter).unwrap(), result);
    let stale = RustEvidenceFilter { source_oid: Some("0".repeat(40)), ..filter };
    assert!(memory.rust_evidence(&stale).is_err());
    assert!(disk.rust_evidence(&stale).is_err());
}
```

**Implementation/acceptance:** Implement the frozen filter/result semantics, default unavailable trait method and reference forwarding. Validate original request shape even with absent evidence. Lazy query contexts validate all source/origin/owner metadata before selection, including malformed rows outside the filter. Populate the known native source domain from core manifests even for absent carriers; do not confuse absence with an unknown custom-backend domain. Memory/disk produce value-equal ordered results for all-files, healthy zero, cfg/recovery Partial, missing path Unavailable, virtual/non-Rust UnsupportedSource and exact file+OID filters. Source filter without file is an error. Present-empty and absent remain distinguishable. Validate actual owner symbol projections and file manifests without reading mutable current source bytes. Full source-authentication stays an explicit separate API.

After first disk query, replace/delete table files and verify the existing query context returns its validated snapshot from immutable bytes. A newly opened client must reject corrupt cold reads. If corruption occurs before the first query, it must fail before any result even when corrupt rows are filtered out. Count IO only to verify zero further evidence reads; no numeric performance target or unrelated query refactor.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_queries`; `scripts/spur-cargo test -p spur-graph --test macro_invocation_queries --test query_client_parity`; `scripts/spur-cargo test -p spur-graph query_client::tests`.

**PRE/POST inputs:** D5/D6 actual persisted artifacts, independent file/OID/status filter selection and actual owner memberships. Relations `request(file,oid,shape)`, `selection(path,expected_status)`, result sources/origins and exact nested records. Rules conditional_required/cardinality/exact consistency and version compatibility. POST exports actual memory/disk results and cold/frozen IO observations. ROOT independently mutates a filtered-out source/body row and a stale source filter, and verifies neither missing tables nor Unknown becomes complete zero.

## Task D8: Whole-file overlay replacement and tombstones

**Task ID:** `D8-rust-evidence-overlay`. **Depends on:** `D7-rust-evidence-query`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `crates/spur-graph/src/query_client.rs` (`OverlayClient` Rust method and extraction routing only), `src/rust_evidence/query.rs` (projection/availability merge helper); create `crates/spur-graph/tests/rust_evidence_overlays.rs`. Do not change summary edge rebinding, temporal identities or macro behavior.

**First compiling assertion RED:** The D7 trait default returns Unavailable for OverlayClient until this implementation. Entire old-file groups must disappear, including complete-empty replacements.

```rust
use std::{collections::HashSet, fs, sync::Arc};
use spur_graph::{artifact_from_facts, build_facts, RustEvidenceFilter, RustEvidenceAvailability};
use spur_graph::query_client::{GraphQueryClient, InMemoryClient, OverlayClient};
#[test]
fn zero_replacement_removes_old_sites_and_keeps_neighbor_origin() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "fn leaf() {} fn a() { leaf(); }\n").unwrap();
    fs::write(root.path().join("b.rs"), "pub struct Keep(u8);\n").unwrap();
    let base = artifact_from_facts(&build_facts(root.path(), None).unwrap().0,
        root.path()).unwrap();
    let before_b = base.rust_evidence.as_ref().unwrap().files.iter()
        .find(|f| f.file_path == "b.rs").unwrap().clone();
    fs::write(root.path().join("a.rs"), "pub struct Empty;\n").unwrap();
    let (delta, shadowed) = OverlayClient::<InMemoryClient>::extract_delta(
        root.path(), &[root.path().join("a.rs")]).unwrap();
    assert_eq!(shadowed, HashSet::from(["a.rs".into()]));
    let overlay = OverlayClient::from_artifacts(InMemoryClient::new(Arc::new(base)),
        delta, shadowed).unwrap();
    let result = overlay.rust_evidence(&RustEvidenceFilter::default()).unwrap();
    assert_eq!(result.availability, RustEvidenceAvailability::Complete);
    assert_eq!(result.files.len(), 2);
    assert_eq!(*result.files.iter().find(|f| f.file_path == "b.rs").unwrap(), before_b);
    let a = result.files.iter().find(|f| f.file_path == "a.rs").unwrap();
    match &a.calls {
        spur_graph::RustCallEvidence::Complete { evidence } =>
            assert!(evidence.sites.is_empty() && evidence.bodies.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
}
```

**Implementation/acceptance:** Validate filters before routing. Identity overlay delegates exactly to base. For any shadowed path remove the complete base file group (source/declarations/bodies/sites), then include only that delta file group; zero/Unknown replacement must not leak old bodies. Replace source-domain entries as well as evidence rows. Deleted/tombstoned paths have no active evidence source, and an explicit request is Unavailable. Remove unreferenced origin records while retaining untouched base originals. Reject conflicting same-key origin inventories. For all-files queries, distinguish a delta with absent capability from a declared deletion: absent evidence in a live changed file contributes Unavailable/Partial and never exposes base rows. A legacy base with an untouched unavailable source plus a captured changed file is Partial over the full active domain; a filter selecting only the healthy changed file is Complete. If all known legacy sources are replaced/deleted and every remaining active source is captured, the aggregate can be Complete. An unknown custom source domain cannot establish that final claim.

Test memory and persisted disk bases, a delta reloaded through D5, whole-file edit, complete zero replacement, cfg/recovery Unknown replacement, deletion, readd, no-op overlay, legacy base/delta, different original snapshots and stale file/OID filters. Use explicit `from_artifacts` with shadowed path and an empty selected delta to test deletion independent of filesystem extraction, then verify `extract_delta` on an actually deleted path. D3's selected-path builder must tolerate missing selected deleted paths as absent current files, not create false complete sources. If existing extraction requires a small missing-path accommodation, keep it in `OverlayClient::extract_delta`; do not broaden supported source policy.

**GREEN:** `scripts/spur-cargo test -p spur-graph --test rust_evidence_overlays`; `scripts/spur-cargo test -p spur-graph --test overlay_client --test macro_invocation_queries --test mcp_overlay_notfound_retry`.

**PRE/POST inputs:** Independently specified active/shadowed/deleted path sets, D7 base and delta exports, original per-file payload digests. Exact selection/cardinality/source-origin/body/site tuples and conditional unavailable rules. POST actual overlay results for memory/disk/delta paths, not an expected concatenation. ROOT mutates an existing base site into a shadowed zero/Unknown file and rebinds an untouched base origin; both must violate exact expected selection/provenance.

## Task D9: MCP client / overlay generation capability identity

**Task ID:** `D9-rust-evidence-mcp-cache`. **Depends on:** `D8-rust-evidence-overlay`. **Suggested Worker:** codex, approved profile/model/effort.

**Files:** Modify `crates/spur-graph/src/mcp/{request_cache,overlay_snapshot,mod}.rs` and their existing unit tests only. `mcp/mod.rs::snapshot_base:2883` is the actual memory/Parquet construction seam. No tool registration, runtime/model, quota or external graph change.

**First RED:** Add this test inside `request_cache.rs` tests. `cache_identity` and the compatibility key already exist. The new identity member initially gives explicit compile RED; after the member exists but compatibility still ignores it, this is a targeted assertion RED. Do not report the compiler failure as an observed cache collision.

```rust
#[test]
fn overlay_compatibility_includes_rust_evidence_digest() {
    let root = tempfile::tempdir().unwrap();
    let before = cache_identity(root.path(), 1);
    let mut changed = before.clone();
    changed.indexed_rust_evidence_digest = "different-capability".into();
    assert!(OverlayGenerationCompatibilityKey::from(&before)
        != OverlayGenerationCompatibilityKey::from(&changed));
}
```

**Implementation/acceptance:** Add `indexed_rust_evidence_digest: String` to `SnapshotBase`, `SnapshotIdentity`, `CachedSnapshot` and `OverlayGenerationCompatibilityKey`. Compute it through D2/D5 identical artifact/manifest helpers in `snapshot_base`; include it in validated-snapshot compatibility, `finish_snapshot`, all exact overlay delta/generation keys and compatibility checks. Explicitly tag the legacy `SnapshotBase::compatibility` constructor's unavailable capability using the same absence encoding; it must not assert supported evidence. Update every constructor/test helper in these three files. Retain existing changed-set/OID/index/head identity and byte-read race checks.

`ParquetClientCacheKey` already hashes actual manifest bytes, including D5 metadata; do not replace it with mtime/source hash. Extend its current same-mtime test with Rust checksum/availability/schema metadata changes. Cached client keeps its own frozen evidence snapshot; a newly requested client with changed manifest bytes opens a fresh context and validates cold tables. Test exact identity hit, same-source/different-capability invalidation, compatible-generation seed rejection across capability changes, legacy absence versus zero, failed/retried singleflight and deletion/readd generations. Changing evidence versions without file edits must not reuse an old overlay generation. No additional cache capacity or retry count is introduced.

**GREEN:** `scripts/spur-cargo test -p spur-graph mcp::request_cache::tests`; `scripts/spur-cargo test -p spur-graph mcp::overlay_snapshot::tests`; `scripts/spur-cargo test -p spur-graph --test rust_evidence_overlays --test mcp_overlay_notfound_retry`; `scripts/spur-cargo fmt -- --check`.

**PRE/POST inputs:** Actual D8 overlay base/delta outputs, manifest digests and complete old/new cache identity components. Relations `identity(source_hash,manifest_digest,evidence_digest,heads,index,changed_set)`, `reuse(old,new,allowed)` and selected result provenance. Exact consistency/conditional-required/allowed-version rules. POST records actual cache hit/rebuild behavior and envelope identity, not timing claims. ROOT independently aliases a changed evidence digest to an old compatibility key and verifies the fresh actual implementation rejects/rebuilds it.

## Task D10: ROOT fresh Rust notebook and independent persisted POST

**Task ID:** `D10-root-rust-evidence-replay`. **Depends on:** `D9-rust-evidence-mcp-cache`. **Owner:** ROOT; this is the review gate, not worker self-approval. No production changes are authorized here.

**Files:** Create the NEW `docs/research/2026-11-06-rust-evidence-durability-root-review.ipynb` and new evidence only under `docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/`. Retain all old notebooks and evidence. ROOT reads the notebook MCP skill and confirms the authorized exact notebook path before creating/executing it; no live external project graph/CURRENT rebuild. This future ROOT notebook is not created by the current three-file grounding task.

**Prospective source fixtures:** Materialize fresh private copies under `ROOT-review/inputs/` using these exact UTF-8 bytes (trailing newline in every file). The first source deliberately has six physical sites across five syntactic bodies; declarations of deferred bodies are not execution evidence. These are expected oracle inputs, not claimed observations.

```rust
// six.rs
pub const fn leaf() {}
pub fn run() {
    /*s1*/ leaf();
    let _closure = || { /*s2*/ leaf(); /*s3*/ leaf(); };
    let _future = async { /*s4*/ leaf(); };
}
const VALUE: () = { /*s5*/ leaf(); /*s6*/ leaf(); };
// empty.rs (separate file)
pub struct Empty;
// ffi.rs (separate file)
extern "C" { pub fn imported(); }
// cfg.rs (separate file)
#[cfg(feature = "trip")] fn selected() { leaf(); }
// recovery.rs (separate file)
pub struct Intact(u8); fn broken(
```

Marker comments outside call ranges give an independent site/range/innermost-body oracle before the current collector executes. Produce source inventory separately from actual evidence: read each private file, compute `git_blob_oid`/SHA-256/length and independently determine fixture path/status/spans; never derive required rows by copying the observed export. Include the original independently preserved historical six-site/five-body experiment only as an additional replay input if its exact bytes are supplied and hashed; its historical counts cannot replace fresh output. The newly specified fixture removes any dependency on a missing historical private adapter.

**Execution:** ROOT first supplies fresh source-bound catalog/Jev PRE for this final acceptance. Build an exact-current-source replay executable through `scripts/spur-cargo` and record actual executable/toolchain/source hashes. Use Rust notebook cells to invoke that executable or the current Rust library via a recorded adapter; notebook source and actual kernel outputs are evidence. A Python reimplementation of collection or a prewritten expected JSON is insufficient. The following is the core current-API program fragment; it belongs in the replay executable's `fn main() -> anyhow::Result<()>` and actually writes/reloads tables. The caller runs it from the repository root after creating `inputs/`.

```rust
let evidence_root = std::path::Path::new(
    "docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review");
let input = evidence_root.join("inputs");
let out = evidence_root.join("roundtrip");
let facts = spur_graph::build_facts(&input, None)?.0;
let graph = spur_graph::artifact_from_facts(&facts, &input)?;
spur_graph::write_artifact_parquet(&graph, &out, spur_graph::WriteOptions::default(), vec![])?;
let loaded = spur_graph::read_artifact_parquet(&out)?;
assert_eq!(loaded.rust_evidence, graph.rust_evidence);
let paths = ["six.rs", "empty.rs", "ffi.rs", "cfg.rs", "recovery.rs"];
let bytes = paths.iter().map(|p| std::fs::read(input.join(p)))
    .collect::<Result<Vec<_>, _>>()?;
let sources = paths.iter().zip(&bytes).map(|(path, bytes)|
    spur_graph::RustDeclarationSource { file_path: path, bytes })
    .collect::<Vec<_>>();
loaded.validate_rust_evidence_sources(&sources)?;
std::fs::write(evidence_root.join("actual-reloaded.json"),
    serde_json::to_vec_pretty(loaded.rust_evidence.as_ref().unwrap())?)?;
```

Run actual memory/Parquet/overlay queries, incremental unchanged/edit/delete/readd, the complete-zero and legacy-absent variants and canonical mutation validations through current Rust. Save actual outputs, source inventory, table files, complete manifests and raw errors. Authenticate again after source/cell execution. This is review/replay, so there is no implementation RED/GREEN claim for D10; any discovered regression goes back to the owning implementation task with its own fresh PRE/TDD cycle.

**DuckDB/source joins:** In the NEW notebook inspect `DESCRIBE` of actual persisted tables and the independently written `source-inventory.json` before SQL. That inventory is an array of actual file records with `file_path`, `source_oid`, `byte_length`, `stable_file_id`. Execute these actual queries from repo-root context and retain full result tables, not only assertion text:

```sql
CREATE VIEW rust_sources AS SELECT * FROM read_parquet(
 'docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/roundtrip/rust_evidence_sources.parquet');
CREATE VIEW rust_bodies AS SELECT * FROM read_parquet(
 'docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/roundtrip/rust_evidence_bodies.parquet');
CREATE VIEW rust_sites AS SELECT * FROM read_parquet(
 'docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/roundtrip/rust_evidence_sites.parquet');
CREATE VIEW rust_decls AS SELECT * FROM read_parquet(
 'docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/roundtrip/rust_declarations.parquet');
CREATE VIEW independent_sources AS SELECT * FROM read_json_auto(
 'docs/research/artifacts/2026-11-06-rust-evidence-durability/ROOT-review/source-inventory.json');
SELECT coalesce(s.file_path, i.file_path) AS file_path,
       s.source_oid, i.source_oid AS independent_oid, s.call_availability,
       s.byte_length, i.byte_length AS independent_length,
       s.stable_file_id, i.stable_file_id AS independent_file_id
FROM rust_sources s FULL OUTER JOIN independent_sources i USING (file_path)
ORDER BY file_path;
SELECT c.site_id, c.file_path, c.source_oid, c.body_id,
       c.byte_range_start, c.byte_range_end, c.call_text,
       b.kind, b.source_oid AS body_oid, s.source_oid AS manifest_oid,
       c.origin_graph_content_hash, b.origin_graph_content_hash AS body_origin,
       s.origin_graph_content_hash AS source_origin
FROM rust_sites c LEFT JOIN rust_bodies b
 ON c.body_id=b.body_id AND c.file_path=b.file_path AND c.source_oid=b.source_oid
LEFT JOIN rust_sources s ON c.file_path=s.file_path
ORDER BY c.file_path, c.byte_range_start;
SELECT d.kind, d.stable_symbol_id, d.file_path, d.source_oid,
       d.structural_owner_json, d.lexical_owner_json, d.payload_json,
       s.source_oid AS manifest_oid, i.source_oid AS independent_oid
FROM rust_decls d LEFT JOIN rust_sources s USING (file_path)
LEFT JOIN independent_sources i USING (file_path)
ORDER BY d.file_path, d.full_range_start, d.kind;
SELECT s.file_path, s.call_availability,
 (SELECT count(*) FROM rust_sites c WHERE c.file_path=s.file_path) AS sites,
 (SELECT count(*) FROM rust_bodies b WHERE b.file_path=s.file_path) AS bodies,
 (SELECT count(*) FROM rust_decls d WHERE d.file_path=s.file_path) AS declarations
FROM rust_sources s ORDER BY s.file_path;
```

Also inspect/query actual `rust_evidence_origins.parquet`: expand each `sources_json` with DuckDB JSON functions, recompute original graph hashes through current Rust and join active source OIDs to original inventory bindings. Compare each actual site and FFI/tuple row to the independently marked source coordinates, not just aggregate counts. Show zero-source coverage and cfg/recovery Unknown explicitly. Verify original and edited overlay snapshots separately; inventory joins select the correct saved input generation, not today's mutable source bytes.

**Final acceptance:** ROOT independently mutates actual reload/export data for missing/duplicate zero sources, omitted body/site, wrong existing body/owner/source, stale version, rebound origin, foreign body tag and canonical hash/checksum changes. Keep SQL-only mutations separate from mutations passed to Rust validators; SQL FKs cannot prove source correspondence. Include each missing/truncated/wrong-checksum core-table control, memory/disk/filter parity, whole-file zero/Unknown/deletion replacement and cache invalidation. Every POST receipt is persisted, reloaded exactly and independently checked with source/executable identities and Jev. Normal suite evidence must be fresh: `scripts/spur-cargo test -p spur-graph`; `scripts/spur-cargo test -p spur-cli --test analyst_temporal_views`; `scripts/spur-cargo fmt -- --check`. Save actual failures if any and repair in the owning task; never invent a passing command or substitute historical C results.

## Submission, coverage and self-review

The explicit dependency chain is:

```mermaid
flowchart LR
 D1[Contract] --> D2[Artifact/hash] --> D3[Full builds] --> D4[Reuse/deltas]
 D4 --> D5[Parquet] --> D6[Publication/cache] --> D7[Queries]
 D7 --> D8[Overlays] --> D9[MCP cache identity] --> D10[ROOT Rust/SQL replay]
```

Serial order is deliberate: each task changes a predecessor interface or the same build/storage/query seams; no parallel workers should own these files. ROOT submits Beads-ready task IDs/dependencies exactly above and checks approval before each successor. Evidence folders for implementation are `docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/` through `D9/`; ROOT owns `ROOT-review/`. Each implementation folder contains `PRE-request.json`, `PRE-result.json`, `PRE-reloaded.json`, `source-bindings.json`, `RED.txt`, `GREEN.txt`, `actual-export.json`, `POST-request.json`, `POST-result.json`, `POST-reloaded.json`, `mutations.json` and `Jev.json`. Names specify prospective outputs only, not existing receipts. Save full command, exit status and raw output in RED/GREEN. Use Beads UUID audits as durable transaction records; evidence files do not replace canonical issue state.

| Source obligation / acceptance | Owner tasks |
| --- | --- |
| Versioned optional declaration/invocation carrier, original public APIs | D1, D2 |
| Healthy empty source and empty workspace; legacy unavailable | D1, D2, D3, D5, D7 |
| cfg/recovery Unknown, healthy neighbor and intact declaration retention | D1, D3, D5, D7, D8 |
| Exact owners/body/site/source tuples, omitted/duplicate/stale/wrong-existing references | D1, D5, D10 |
| Original snapshot/OID across unchanged reuse and edits/deletions | D1, D4, D8 |
| Full, selected-path, incremental and delta builds | D3, D4, D5 |
| Canonical hash, checksum, core completeness and actionable incompatible schema | D2, D5, D6 |
| Missing/truncated/wrong-checksum tables even at zero rows | D5, D7, D10 |
| Full/slim/history readers and cross-crate constructors/manifests | D2, D5 |
| Memory/disk exact filters, frozen validated bytes | D7 |
| Complete overlaid file replacement, tombstones, Unknown/zero/legacy domains | D8 |
| Source/checksum/capability cache invalidation and publication | D6, D9 |
| NEW notebook, actual persisted current Rust output, actual SQL source joins | D10 |
| Fresh PRE before edits, actual-output POST persisted/reloaded/mutated | Every task; ROOT gates |

Self-review: all ten IDs are unique; each predecessor is explicit and the chain is acyclic. New types/signatures/row shapes are defined before use. D1/D7/D9 compile RED is distinguished from assertion RED; D2–D6/D8 seeds use integrated existing APIs. Commands are unexecuted prospective commands and use the required wrapper. The 64 mechanical literals are isolated in D2; D5 manifest-only fallout is isolated. The report maps every G3 requirement and historical audit correction; compiler cfg evaluation, macro expansion, semantic dispatch, ABI/linkage/unwind guarantees and complete panic detection remain outside G3. Protected main files, old notebooks/evidence and live external graph/CURRENT have no task ownership. Scope changes, unexpected integrity failures or authorization rejections require the standard signal/progress protocol and ROOT review, never a silent policy relaxation.
