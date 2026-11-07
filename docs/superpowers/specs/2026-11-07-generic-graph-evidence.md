# Generic graph evidence providers

This is a proposed concrete refinement of the already approved generic architecture for ROOT review and fresh Jev PRE. It is not implementation, a solver result, or a new user approval gate. Grounded production base: `457b9f70fde8a9c26ce4ac4e2dec5c56cd9a1c3a`. Exact source/range bindings and migration sites are in [source-map.json](../../research/artifacts/2026-11-07-generic-graph-evidence/grounding/source-map.json). The protected Rust-first checkpoint `4ca205ea8ba412b8648997dfdcfaabe3cfbf2027` is an unapproved reference only.

## Decision and boundary

Use a deterministic inventory of generic, source-bound extensions with provider-owned relational tables. One core inventory is the mutable truth. Rust physical evidence and adjacent legacy macro evidence are two adapters over their existing records and collectors. A static registry composes these adapters; tests inject a second independent fixture provider. There is no runtime plugin loading, new parser, language inference in core, quota/retry change, or extractor rewrite.

| Alternative | Consequence | Decision |
| --- | --- | --- |
| Keep `Option<RustEvidenceSet>` and add a generic digest | Artifact, manifest, table completeness and validation still require Rust; a second provider needs new core branches | Reject |
| Generic opaque JSON/blob only | Easy inventory but loses existing row/site/body SQL and typed table validation; invites a second mutable relational copy | Reject |
| Generic table envelope plus provider codecs | Existing Utf8/Int64/Boolean and JSON-column patterns remain queryable; shared core can verify transport without knowing Rust | Choose |

Core owns identity, source/origin bindings, declared coverage, schema/extractor inventory, canonical transport, checksums, persistence, frozen contexts and whole-file replacement. Adapters own eligibility, typed payloads, source recollection, owner/body/site/tuple/ABI semantics and semantic completeness. Integrity does not establish source truth. Unknown providers can be preserved and queried as integrity-checked data but cannot receive a semantic Complete result.

## Modules and public contract

Create `src/evidence/{mod,contract,query}.rs` for generic types and operations, `src/store/evidence.rs` for generic Arrow/Parquet IO, and `src/evidence/providers/{mod,rust,legacy_macros,legacy_wire}.rs` for composition and adapters. Paths are under `crates/spur-graph/`. Keep current `rust_evidence.rs`, `rust_evidence/query.rs`, `rust_declarations.rs`, `panic_evidence/call_sites.rs`, and `macro_invocations.rs` as typed provider implementation/convenience surfaces. Existing `store/{rust_evidence,macro_invocations}.rs` become provider codecs and explicit old-format readers; shared storage does not dispatch on their table names.

The declarations below specify the complete new data surface. Stored structs derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, use `deny_unknown_fields`; enums use externally tagged snake_case. Strings remain strings to avoid new dependencies. No `serde_json::Value` map is the shared canonical payload. All integer sizes on wire are `u64`/`i64`, checked on conversion.

```rust
pub const EVIDENCE_SCHEMA_VERSION: &str = "spur-evidence-v1";
pub struct EvidenceInventory {
    pub schema_version: String,
    pub extensions: Vec<EvidenceExtension>,
}
pub struct EvidenceDescriptor {
    pub provider_id: String,
    pub schema_version: String,
    pub extractor_version: String,
    pub component_versions: std::collections::BTreeMap<String, String>,
    pub presence: EvidencePresence,
    pub metadata: std::collections::BTreeMap<String, String>,
}
pub enum EvidencePresence { Absent, Captured }
pub struct EvidenceSourceKey {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
}
pub struct EvidenceOriginKey {
    pub manifest_version: String,
    pub graph_content_hash: String,
}
pub struct EvidenceOrigin {
    pub key: EvidenceOriginKey,
    pub sources: Vec<EvidenceSourceKey>,
}
pub enum EvidenceCoverage { Complete, Unknown { code: String }, Unavailable }
pub struct EvidenceSourceCoverage {
    pub source: EvidenceSourceKey,
    pub byte_length: Option<u64>,
    pub origin: Option<EvidenceOriginKey>,
    pub coverage: EvidenceCoverage,
}
pub enum EvidenceDomain {
    Unknown,
    Known { sources: Vec<EvidenceSourceCoverage> },
}
pub enum EvidenceColumnType { Utf8, Int64, Boolean }
pub struct EvidenceColumn {
    pub name: String,
    pub data_type: EvidenceColumnType,
    pub nullable: bool,
}
pub enum EvidenceValue { Null, Utf8(String), Int64(i64), Boolean(bool) }
pub enum EvidenceTableScope {
    Snapshot,
    File {
        file_path_column: String,
        source_oid_column: String,
        origin_hash_column: Option<String>,
    },
}
pub struct EvidenceTable {
    pub name: String,
    pub storage_path: String,
    pub scope: EvidenceTableScope,
    pub columns: Vec<EvidenceColumn>,
    pub primary_key: Vec<String>,
    pub order_by: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub rows: Vec<Vec<EvidenceValue>>,
}
pub struct EvidenceExtension {
    pub descriptor: EvidenceDescriptor,
    pub origins: Vec<EvidenceOrigin>,
    pub domain: EvidenceDomain,
    pub tables: Vec<EvidenceTable>,
}
pub struct EvidenceSource<'a> { pub file_path: &'a str, pub bytes: &'a [u8] }
pub enum EvidenceError {
    Integrity { code: String, message: String },
    Semantic { provider_id: String, code: String, message: String },
    Unsupported { provider_id: String, schema_version: String },
}
```

`EvidenceError` implements `Display` and `std::error::Error`; existing `anyhow::Result` APIs preserve it as a downcastable cause. Integrity includes source/OID/coverage/duplicate/table/checksum failures. Semantic includes canonical source recollection differences and invalid provider relationships. Unevaluated cfg, parse recovery, invalid UTF8 and unavailable parser remain typed Rust Unknown payload reasons, never string-matched error downgrades. Unsupported semantic schema is distinct from invalid generic transport.

New captured stored source groups require `Some(byte_length)` and `Some(origin)`. The explicit legacy macro adapter may retain `None` length where the old accepted schema recorded none, marked by its original-format metadata; this does not assert a new byte-length observation. An adapter-derived known absent legacy query domain can use `None` for these unknown attributes and Unavailable coverage. Do not fabricate zero length or an origin for legacy absence. Provider `supports_path` retains its physical-source policy; `source_domain` derives eligible bindings from authoritative graph manifests, including when that provider's capture is absent.

Exact generic operations in `evidence/contract.rs`:

```rust
impl EvidenceInventory {
    pub fn empty() -> Self;
    pub fn validate_integrity(&self, graph: &GraphIndexArtifact)
        -> Result<(), EvidenceError>;
    pub fn replace(&mut self, extension: EvidenceExtension)
        -> Result<(), EvidenceError>;
    pub fn remove(&mut self, provider_id: &str) -> Option<EvidenceExtension>;
    pub fn get(&self, provider_id: &str) -> Option<&EvidenceExtension>;
}
pub fn evidence_extension_checksum(extension: &EvidenceExtension)
    -> Result<String, EvidenceError>;
pub fn evidence_inventory_capability_digest(manifest_version: &str,
    inventory: &EvidenceInventory) -> Result<String, EvidenceError>;
pub fn evidence_capability_digest(artifact: &GraphIndexArtifact)
    -> Result<String, EvidenceError>;
pub fn evidence_manifest_capability_digest(manifest: &GraphArtifactManifest)
    -> Result<String, EvidenceError>;
pub fn evidence_inventory_manifest(inventory: &EvidenceInventory)
    -> Result<EvidenceManifest, EvidenceError>;
pub fn replace_evidence_files(
    base: &EvidenceInventory, delta: &EvidenceInventory,
    shadowed: &std::collections::BTreeSet<String>,
    active: &[GraphFileManifestEntry],
) -> Result<EvidenceInventory, EvidenceError>;
```

`replace` replaces one provider entry, canonicalizes its generic order, rejects duplicate keys and never validates semantics implicitly. Direct mutation requires subsequent integrity/semantic validation before publication. There is no separately writable typed carrier or cached decoded payload in the artifact.

Provider validation is statically registered and separate from transport:

```rust
pub enum EvidenceSemanticStatus { Validated, UnknownProvider, UnsupportedSchema }
pub struct EvidenceValidation {
    pub status: EvidenceSemanticStatus,
    pub domain: EvidenceDomain,
}
pub trait EvidenceProvider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn supports_path(&self, file_path: &str) -> bool;
    fn source_domain(&self, graph: &GraphIndexArtifact)
        -> Result<Vec<EvidenceSourceKey>, EvidenceError>;
    fn validate_stored(&self, graph: &GraphIndexArtifact,
        extension: &EvidenceExtension) -> Result<EvidenceValidation, EvidenceError>;
    fn validate_sources(&self, graph: &GraphIndexArtifact,
        extension: &EvidenceExtension, sources: &[EvidenceSource<'_>])
        -> Result<(), EvidenceError>;
}
pub struct EvidenceRegistry<'a> {
    // Private sorted unique provider_id -> adapter references; no loader.
    providers: std::collections::BTreeMap<&'a str, &'a dyn EvidenceProvider>,
}
impl<'a> EvidenceRegistry<'a> {
    pub fn new(providers: &'a [&'a dyn EvidenceProvider])
        -> Result<Self, EvidenceError>;
    pub fn validate(&self, graph: &GraphIndexArtifact,
        extension: &EvidenceExtension) -> Result<EvidenceValidation, EvidenceError>;
}
```

Registry validates core integrity before adapter dispatch. Unknown ID returns UnknownProvider, supported ID/unsupported semantic schema returns UnsupportedSchema, malformed supported payload returns typed error. Neither unknown status synthesizes Complete. Built-in IDs are `spur.rust.physical` and `spur.rust.macros`. A registry must not permit duplicate IDs. Tests use `test.counter`, a tiny provider of one integer value per physical fixture source; it is not a newly supported language.

## Determinism and absence

Inventory extensions sort by UTF8 provider ID; origins by `(manifest_version, graph_content_hash)`; origin sources and coverage by path then stable ID/OID; tables by logical name. Column order is schema order, never alphabetized. Metadata/component maps use `BTreeMap`. Rows sort lexicographically by the declared `order_by` cells, followed by all cells as a total tie-break; null sorts before non-null, then type tag and value. Adapters must choose an order preserving their current canonical payload reconstruction. Primary keys are nonempty lists of existing nonnullable columns; every duplicate key rejects, including identical duplicates. Snapshot tables may use composite keys. Zero-row tables still have schema, metadata, declared count and checksum.

Core checks safe relative normal storage paths, unique physical paths across the entire inventory, no symlinks escaping the artifact, unique table/column names, row width, exact value types/nullability, key/order columns, source path/OID membership for File tables, origin membership, original source inventory hash using existing `compute_graph_content_hash`, and current source/stable-file/OID agreement. Provider validators additionally check every duplicated scalar against its typed JSON and exact row/site/body/owner provenance. Snapshot rows and nested payload semantics cannot be proven by generic foreign keys alone. Unknown provider Snapshot rows remain opaque and their semantic provenance is explicitly unvalidated.

`evidence_extension_checksum` is BLAKE3 over `b"spur-evidence-extension-v1\0"` followed by compact Serde JSON of the canonical complete extension. No mutable checksum field lives inside that extension. Table logical checksum uses `b"spur-evidence-table-v1\0"` plus compact JSON of the complete canonical table. The disk schema stores these logical checksums, not checksums of compressor-dependent Parquet bytes. An absent entry has no tables/origins, Unknown domain and a null payload checksum; captured empty has a real checksum over its empty Known domain. Captured Unknown has a real checksum and retains its reason and available rows.

Aggregate digest is BLAKE3 of compact Serde JSON of this exact tuple:

```text
("spur-evidence-capability-v1", EVIDENCE_SCHEMA_VERSION,
 artifact.manifest_version,
 evidence_inventory_manifest(&artifact.evidence).extensions)
```

It consumes every entry, including opaque/unsupported providers and absent entries. Add/remove, any provider/version/component/metadata/presence/payload change invalidates; input provider order does not. The exact `EvidenceExtensionManifest` tuple includes table descriptors/counts/logical checksums, origins and domain. `evidence_inventory_manifest` derives it from data without IO. Manifest helper uses its stored counterpart and refuses incomplete generic transport; malformed metadata cannot become trusted by digest equality. Both helpers must run in Rust and agree on valid round trips. `EvidenceInventory::empty()` encodes an empty vector, never a captured provider. The compatibility constructor uses adapter-produced explicit Absent descriptors for the two built-ins, with supported versions; its exact digest must be observed natively. Do not substitute the historical Rust-only absence digest or a Python BLAKE3 calculation.

`graph_content_hash` stays exactly the existing source path/OID hash. Canonical artifact hash includes generic inventory when nonempty; header hash remains excluded. Digest equality establishes identity, not authenticity, schema support or semantic completeness.

## Single-source artifact and old-format policy

Final `GraphIndexArtifact` replaces public `rust_evidence` and `macro_invocations` fields with:

```rust
#[serde(default, skip_serializing_if = "EvidenceInventory::is_empty")]
pub evidence: EvidenceInventory;
impl EvidenceInventory { pub fn is_empty(&self) -> bool; }
impl GraphIndexArtifact {
    pub fn empty() -> Self;
    pub(crate) fn validate_legacy_evidence_export(&self) -> anyhow::Result<()>;
    pub fn rust_evidence_set(&self) -> anyhow::Result<Option<RustEvidenceSet>>;
    pub fn set_rust_evidence(&mut self, set: Option<RustEvidenceSet>) -> anyhow::Result<()>;
    pub fn macro_evidence_set(&self) -> anyhow::Result<Option<MacroInvocationEvidenceSet>>;
    pub fn set_macro_evidence(&mut self, set: Option<MacroInvocationEvidenceSet>)
        -> anyhow::Result<()>;
    pub fn validate_rust_evidence_sources(&self, sources: &[RustDeclarationSource<'_>])
        -> anyhow::Result<()>;
    pub fn validate_macro_invocations(&self, sources: &[MacroEvidenceSource<'_>],
        scopes: &[MacroScopeEvidence]) -> anyhow::Result<()>;
}
```

`empty()` is an unvalidated construction draft: current graph header/version, source hash of empty inventory, empty collections and absent typed lanes. It makes no Complete claim. The setter removes the provider entry for None; recognized old artifacts are normalized to explicit Absent descriptors by the compatibility adapter. Getters return owned decoded values to avoid a second mutable truth; mutate/decode/replace tests must call the setter. Existing public collectors, record types, validators and typed result/filter signatures remain available. Direct Rust struct-field source access is an intentional API migration, exhaustively mapped; retain no public mutable Rust alias. Existing `rust_evidence_capability_digest` and manifest counterpart remain deprecated typed adapter helpers for historical contract callers, while all new core caches use the aggregate helper.

The temporary `validate_legacy_evidence_export` is a no-op over current typed-only artifacts during preparation; both old full/delta writers call it before writing. After artifact cutover it delegates to legacy_wire and rejects an inventory the old format cannot represent. G09 removes those calls when generic writers are active. It is a migration guard, never a permanent provider allowlist in core.

Before cutover introduce these methods over the current fields, migrate all callers/constructors in isolated serial batches, then atomically replace the fields and method bodies. Do the same for manifest getters/setters and construction draft:

```rust
impl GraphArtifactManifest {
    pub fn empty() -> Self;
    pub fn rust_evidence_metadata(&self) -> anyhow::Result<Option<RustEvidenceManifest>>;
    pub fn set_rust_evidence_metadata(&mut self, value: Option<RustEvidenceManifest>)
        -> anyhow::Result<()>;
    pub fn macro_evidence_metadata(&self) -> anyhow::Result<Option<MacroEvidenceManifest>>;
    pub fn set_macro_evidence_metadata(&mut self, value: Option<MacroEvidenceManifest>)
        -> anyhow::Result<()>;
}
```

These preparation commits preserve the sole old-field truth. At cutover old fields disappear; compatibility DTOs are private immutable decode inputs in `legacy_wire.rs`, not a second live artifact. Typed manifest setters are transitional deprecated import shims: after generic integration they accept only recognized legacy metadata while reading old format; new generic writers construct the generic manifest directly. No setter can set a checksum without checked tables.

Choose an explicit new **disk schema v14**, keeping the graph extractor/resolver semantics. This changes `current_manifest_version` because it currently includes disk schema bytes. Introduce an explicit generic wire tag `evidence_format: "spur-evidence-v1"` for artifact JSON; absent tag selects the legacy DTO decoder. Both layouts together reject; an unsupported tag rejects; unknown legacy fields still reject. New serialization always emits the new tag. New manifest replaces `rust_evidence`, `macro_evidence` and the two macro counts with `evidence: EvidenceManifest`; general graph counts stay unchanged. Define:

```rust
pub struct EvidenceTableManifest {
    pub table: EvidenceTable, // rows MUST be empty; schema/order/metadata only
    pub row_count: u64,
    pub checksum_blake3: String,
}
pub struct EvidenceExtensionManifest {
    pub descriptor: EvidenceDescriptor,
    pub origins: Vec<EvidenceOrigin>,
    pub domain: EvidenceDomain,
    pub tables: Vec<EvidenceTableManifest>,
    pub checksum_blake3: Option<String>,
}
pub struct EvidenceManifest {
    pub schema_version: String,
    pub transport_complete: bool,
    pub extensions: Vec<EvidenceExtensionManifest>,
}
```

These serializable manifest and query transport types live in `evidence/contract.rs` from G01, so inventory-only hashing and the early generic trait default compile before disk integration. `store/evidence.rs` implements IO over them; it does not introduce another representation.

The manifest duplicates source bindings for integrity, not a writable alternative to the tables. Full decode must require agreement and discard unchecked declarations. `table.rows != []` in a manifest is malformed. Generic manifest-capability helper checks the transport inventory/table declarations and computes the same tuple as artifact helper; cold validation verifies counts/checksums/data and semantics before reuse.

Legacy v1–v12 exact absent-Rust behavior and accepted v12 macro/v13 Rust layouts remain readable through `legacy_wire.rs` and provider codecs. Preserve original manifest bytes, checksums, typed records, declaration/call headers, stable IDs and origin version/hash. Never rewrite an archive or claim a generic re-encoding is the original. New writes are v14; writing a loaded v13 artifact creates a new serialization/artifact hash with explicit lineage in the export, never an in-place migration. A pure re-encoding retains its original build `manifest_version` and nested origin versions; it does not claim a new extraction. New full builds use the newly computed current manifest identity. The disk schema/wire tag identify the new persistence format independently. Rust logical table names are their existing basenames without `.parquet`; macro logical names are `macro_invocations` and `macro_scopes`. Old CLI/DuckDB SQL keeps the five Rust basenames and macro basenames/columns usable because adapters declare their same storage paths and row schemas in the generic inventory. Logical names are namespaced by provider ID; other providers use paths such as `evidence/test.counter/values.parquet`. Core only checks path safety/collisions, with no Rust-name switch. There is one on-disk table set, not an opaque payload plus a competing SQL copy.

Old canonical omission bytes are verified through an explicit legacy hash function/DTO, not by hashing a re-encoded generic object and calling it unchanged. Define `pub(crate) fn legacy_artifact_content_hash(artifact: &GraphIndexArtifact) -> anyhow::Result<String>` in `legacy_wire.rs`; it is callable only for a legacy-compatible built-in inventory and reconstructs the old fixed field order and omitted None fields. New `artifact_content_hash_blake3_hex` uses the v14 wire/hash domain. Legacy header hashes are authenticated using the legacy projection before normalization. The existing canonical-layout golden stays an old-format fixture; add a new v14 golden. Version-only expected fallout is limited to disk schema/current manifest hashes, new wire/header canonical layout, and version-stamped metadata; no expected source OID, source graph hash, source spans, stable IDs or typed row counts changes. Any other golden delta requires investigation.

Current Rust `validate_stored` requires `origin.manifest_version == graph.manifest_version` (`rust_evidence.rs:215`). The adapter must explicitly allow the accepted original v13 manifest identity `37eee2e2e0d9213d4d9ff13ccbacae861859bd592305fb88dbe54e4e09024626` and new current manifest identity, while recollecting on each **original** view. It must not rewrite original nested versions/hashes to v14. Typed source validators still authenticate exact saved bytes and actual graph owners. Unsupported origin versions fail closed. The incremental builder may choose full recollection for an old incompatible build seed, but an explicit old-artifact query remains usable. Legacy macro v12 metadata is compared against its actual stored schema, as today; do not make its old `rust_evidence.is_none()` condition a generic-core language branch.

## Build and persistence seams

Full and selected-path owners/manifests must be finalized before provider capture. The composition module exposes:

```rust
pub(crate) fn capture_default_evidence(graph: &GraphIndexArtifact,
    root: &std::path::Path, facts: &crate::GraphFacts)
    -> Result<EvidenceInventory, EvidenceError>;
```

`GraphFacts` is the existing extraction carrier (exported from `extract`); the macro adapter wraps its existing macro rows/scopes instead of parsing again. This provider-composition function alone knows built-in collectors. Shared `store/build.rs` uses generic bucket extension groups and `replace_evidence_files`, never matches `.rs`, tuple/body tags or Rust versions to manage evidence. Provider-local capture/source-authentication helpers move out of build.rs. Keep current discovered/selected/currently-existing domains, gitlink/cell/external exclusions, source-read OID checks and missing selected deletion behavior.

Incremental/delta reuse retains original per-file groups exactly. Changed files replace all provider groups; absent live replacements remain Unavailable/Unknown, deletion removes active source/rows, readd uses the new source. Retain only referenced origins, reject conflicting duplicate original inventories. If an unknown provider cannot recollect a changed/new source, preserve untouched tables/origins, remove shadowed rows and mark its aggregate domain Unknown; do not publish a complete semantic claim. The new derived extension checksum is labeled as a composition, not the original complete payload checksum. Generic snapshot tables with no file routing cannot be blindly merged: retain only for identity/no-op; on changed sources preserve the old snapshot as opaque original data with Unknown domain and exclude it from active semantic selection. Providers wanting replacement semantics must supply a File-scoped transport or an adapter. No hidden guess about a new file's language is permitted.

Generic IO signatures in `store/evidence.rs`:

```rust
pub(crate) fn write_evidence(artifact: &GraphIndexArtifact, dir: &std::path::Path)
    -> Result<EvidenceManifest, EvidenceError>;
pub(crate) fn read_evidence(dir: &std::path::Path, manifest: &GraphArtifactManifest)
    -> Result<EvidenceInventory, EvidenceError>;
pub(crate) fn read_evidence_tables(dir: &std::path::Path,
    manifest: &EvidenceManifest, graph: &GraphIndexArtifact)
    -> Result<EvidenceInventory, EvidenceError>;
pub(crate) struct FrozenEvidenceTables { /* private immutable owners */ }
impl FrozenEvidenceTables {
    pub(crate) fn open(dir: &std::path::Path, manifest: &GraphArtifactManifest,
        graph: &GraphIndexArtifact, registry: &EvidenceRegistry<'_>)
        -> Result<Self, EvidenceError>;
    pub(crate) fn query(&self, filter: &EvidenceFilter)
        -> Result<EvidenceQueryResult, EvidenceError>;
}
```

`FrozenEvidenceTables` private fields are frozen compressed byte owners, checked generic descriptors/source inventories, and semantic validation results keyed by provider ID. Reuse existing byte-owning read-closure technique; no new dependency. Freeze/check all tables for a requested inventory before selection, including zero rows and rows outside a filter; later queries use those same immutable bytes. Errors stay in the owning client's OnceLock. Read full/slim evidence before clearing slim graph metadata; history-only graph projections carry empty inventory and cannot seed current supported evidence. Never stamp a full canonical hash on a slim/history projection.

Full/delta writers validate core/known-provider stored structure, write every declared table, sync it, then publish a complete manifest. Unknown supported-transport data can round-trip with semantic UnknownProvider. Missing/truncated/extra/orphan tables, invalid metadata/schema/null/count/order/duplicate keys or checksum mismatch reject even at zero rows. Known unsupported provider schemas remain preservable but a typed Rust query/reuse demanding Rust support fails with actionable incompatibility. Existing staging/pointer publication and immutable hot Arc behavior remain intact.

## Query and forwarding contract

```rust
pub struct EvidenceFilter {
    pub provider_id: String,
    pub file_path: Option<String>,
    pub source_oid: Option<String>,
}
pub enum EvidenceAvailability { Complete, Partial, Unavailable, UnsupportedSource }
pub struct EvidenceQueryResult {
    pub availability: EvidenceAvailability,
    pub semantic_status: EvidenceSemanticStatus,
    pub descriptor: Option<EvidenceDescriptor>,
    pub domain: EvidenceDomain,
    pub origins: Vec<EvidenceOrigin>,
    pub tables: Vec<EvidenceTable>,
}
impl EvidenceFilter { pub fn validate_shape(&self) -> Result<(), EvidenceError>; }
// New optional method on GraphQueryClient, with validated-shape Unavailable default.
fn evidence(&self, filter: &EvidenceFilter) -> anyhow::Result<EvidenceQueryResult>;
```

Nonempty provider ID is required; source OID requires path. Shape validation precedes unavailable handling. Adapter eligibility determines UnsupportedSource; an absent physical source is Unavailable; stale OID errors before filtering. Known coverage plus validated semantics determines Complete, including captured empty; Unknown source lanes produce Partial, retaining known records. Unknown provider/unsupported schema results are Unavailable with their semantic status and retained integrity-checked rows; opaque data is never marketed as validated completeness. Results are projections, have no stored extension checksum field, and never pretend a selected subset is a complete stored set. File-scoped rows and coverage are replaced together; only referenced origins survive. Unknown custom domains cannot become Complete through vacuous all-of-empty checks.

Retain existing typed convenience methods on GraphQueryClient as deprecated optional default shims, with their exact existing filters/results and `anyhow::Result` signatures. Their bodies call provider adapter functions below; generic core logic calls only `evidence`, never a Rust-only method. This avoids changing method resolution/imports in downstream callers. Built-in backends implement only generic evidence, so typed and generic routes share one context. A legacy custom backend's explicit typed override remains usable but grants no generic provider capability; it must opt into `evidence` separately. Default generic behavior remains Unavailable.

```rust
// evidence/providers/rust.rs
pub struct RustEvidenceProvider;
pub fn encode_rust_evidence(graph: &GraphIndexArtifact, set: &RustEvidenceSet)
    -> Result<EvidenceExtension, EvidenceError>;
pub fn decode_rust_evidence(graph: &GraphIndexArtifact, extension: &EvidenceExtension)
    -> Result<RustEvidenceSet, EvidenceError>;
pub fn query_rust_evidence<C: GraphQueryClient + ?Sized>(client: &C,
    filter: &RustEvidenceFilter) -> anyhow::Result<RustEvidenceQueryResult>;
// evidence/providers/legacy_macros.rs
pub struct LegacyMacroEvidenceProvider;
pub fn encode_macro_evidence(graph: &GraphIndexArtifact, set: &MacroInvocationEvidenceSet)
    -> Result<EvidenceExtension, EvidenceError>;
pub fn decode_macro_evidence(graph: &GraphIndexArtifact, extension: &EvidenceExtension)
    -> Result<MacroInvocationEvidenceSet, EvidenceError>;
pub fn query_macro_evidence<C: GraphQueryClient + ?Sized>(client: &C,
    filter: &MacroInvocationFilter) -> anyhow::Result<MacroInvocationQueryResult>;
```

Both provider structs implement EvidenceProvider. Typed decode rejects UnknownProvider/UnsupportedSchema and malformed supported payloads; the convenience query maps legitimate generic absence/Unknown coverage back to the existing typed availability rules. Macro filters include caller/scope/sink selectors beyond the core whole-file filter; the adapter applies them only after full frozen validation and preserves the current snapshot/scope semantics.

Implement/verify forwarding in `&T`, InMemoryClient, ParquetClient, OverlayClient, **Arc<OverlayGeneration>**, PinnedGenerationClient, MeasuredOverlayClient and RequestReplayClient. The last three currently also omit Rust forwarding; test the entire request wrapper chain. Pinned calls retain its query counter; measured/replay wrappers may delegate evidence directly without inventing a cache. Generation owns `Arc<EvidenceInventory>` projected from its base plus per-file overrides; `update` retains unchanged groups, removes deleted groups, and replaces readded groups. Do not rebuild evidence from symbol-only `OverlayFileSegment`s or re-read mutable worktree bytes. Add a private captured-base aggregate digest and guard it when updating/reusing, keeping public source-only `OverlayGenerationIdentity` semantics unchanged.

## Cache, publication and runtime

Replace `BaseArtifactCacheKey.rust_evidence_capability` with generic `evidence_capability`; compare entire generic inventory on publication and dedup. Manifest identity still includes all parsed metadata; MCP `ParquetClientCacheKey` still hashes **actual raw manifest bytes**, canonical path and mtime. Hot validated owners represent immutable snapshots, not continuous disk monitoring. Cold clients validate before admission; cache identity cannot grant Complete to unsupported or incomplete data.

Add `indexed_evidence_digest: String` to SnapshotBase, SnapshotIdentity, CachedSnapshot, OverlayGenerationCompatibilityKey and VisibleOverlayIdentity. Propagate from the actual captured artifact/frozen manifest through `snapshot_base`, `cache_matches_graph`, `finish_snapshot`, `store_snapshot`, exact delta/generation entries and in-flight maps, compatibility selection and visible canonicalization. Exact identity retains every current worktree/head/index/changed component; compatible seeds retain worktree/source hash plus complete aggregate digest. Visible canonicalization intentionally omits index stamps today but must include capability so it cannot alias distinct inventories into the first identity.

Private `OverlayRuntimeKey` gains `base_evidence_digest: String`; `new(canonical_worktree: PathBuf, base_graph_identity: String, base_evidence_digest: String) -> Self`. BuiltOverlayGeneration::new checks actual base source **and** aggregate capability. Runtime acquisition/publication and `is_current_runtime_base` re-resolve metadata and compare capability before serving trusted published results; retire/replace stale actors within existing coordinator ownership. FullBaseArtifactSource::Parquet retains the captured owner/key, rechecks actual manifest bytes and verifies a later loaded base against captured source/capability before seeding. Unsupported/incomplete metadata must take cold error paths before runtime reuse. Request-local RebuiltParquetGraph remains a separate owner with its current drop cleanup.

Preserve source graph identity, current indexed-head conditional diff, tracked index, file OIDs, path states, length/mtime/device/inode/ctime stamps, before/after read certification, full restat/fallback, capacities, existing two open attempts, retry/drop/unwind singleflight and runtime restart behavior. None is replaced by the evidence digest. Out-of-band changed table bytes with unchanged manifest are rejected on cold read; already frozen clients retain their checked snapshot.

## Acceptance and gates

The [PRE packet](../../research/artifacts/2026-11-07-generic-graph-evidence/grounding/pre-inputs.json) separates current source facts, historical Rust-only native observations and prospective generic relations. ROOT supplies fresh source-bound generic Jev PRE before each implementation boundary and POST plus independent alias/forgery review afterward. No historical receipt proves this architecture.

Required new tests cover a test.counter-only payload/version/presence/add/remove mutation with fixed Rust/source bytes, inventory order independence, exact artifact/manifest Rust helper parity, unknown-provider retention and schema status, old v12/v13 reads, old omission hash and new golden, generic missing/zero/malformed tables, frozen/cold contexts, all wrappers/generations, whole-file zero/Unknown/delete/readd, publication/dedup, visible/exact/seed/in-flight/runtime identities, same-mtime manifest mutation and late-load source/capability substitution. Reuse accepted physical Rust source tests and independent body/site/ABI/origin mutations; the synthetic provider is a fixture, not runtime evidence.

After the generic successor of D9, ROOT performs the final NEW notebook/native Rust/SQL acceptance from old D10, adding test.counter and opaque-provider controls. Keep physical six-site/five-body, zero, FFI, cfg/recovery and original-source validation. Normal suites must run freshly via `scripts/spur-cargo`; historical 13 focused tests/23 records do not count as generic GREEN. This grounding creates no notebook, new solve or implementation output.
