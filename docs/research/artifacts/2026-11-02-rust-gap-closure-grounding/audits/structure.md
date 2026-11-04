# Rust structural extraction and identity audit

Research for assigned Beads task **bd-1v3**, plan task `structural-seams`, source issue `bd-1sd`, user epic `bd-3cs`. Source checkout: `bf5d146e132cfbb8eb40eb0b1833802c02529923` (2026-11-02). This report proposes experiments and implementation boundaries; the root retains architectural decisions and Notebook MCP/native/SOLVE experiments.

The immediate gaps are missing syntax capture and anonymous naming, with a consequential ownership seam between two definition passes. Existing `Field` and `Function` kinds can represent tuple fields and foreign function declarations. A first-class extern-block node needs either a new kind or an explicit, documented reuse of an existing container kind. Persisting ABI, declaration/body status, attributes, and ordinal ownership requires an additional data contract: those facts have no fields in today's symbol artifact.

## Evidence boundary and reproducible baseline

The dispatch records an unsuccessful active `knowledge_context_pack_2` lookup against the analyst DB and authorizes scoped raw source fallback. This audit used exact files in `crates/spur-graph` and the existing new-build research artifacts. It did not run a graph build, read notebooks or solver stores, edit target sources, execute compiler experiments, or inspect the target repository's live source tree. Behavioral consequences below are source deductions unless explicitly described as artifact observations. Proposed fixtures have **not** been executed here.

Observed artifact: `docs/research/artifacts/2026-11-02-spur-notebook-new-build-check/summary.json:2–17,48–53` reports schema `spur-graph-schema-v11`, graph hash `fcf40b9cd5785e09a0dbf3e45167f6e5f599348a44a1e184c7ff78d3e8e5cfec`, 689 matching source files, 26,507 declarations and 26,049 correspondences. Reading and counting the 458 rows of `remaining-gaps.json` independently reproduces **446 `tuple_field`, 9 `foreign_function`, 3 `extern_block`**. Its tuple entries say `<unnamed>` and do not encode their parent kinds; this audit does not invent a struct/enum split.

The summary's 12,682/12,682 callable denominator coexists with nine missing `foreign_function` rows. It must not be presented as coverage of every callable declaration category. `baseline-binding.json` identifies a syn 2.0.118 comparison and one excluded directive-bearing bootstrap parse error; no Cargo configuration or compiler semantic coverage follows from this denominator.

| Existing gap artifact rows | Target source location recorded by the artifact | Missing structures |
| --- | --- | --- |
| `remaining-gaps.json:834–865` | `rest-table-gateway-ext/src/generation_probe_tests.rs:79–83` | Extern block `[3615,3872)`; `dlopen`, `dlsym`, `dlerror` signatures |
| `remaining-gaps.json:874–921` | `rest-table-gateway-ext/src/native_registration.rs:81–89` | Extern block `[2865,3363)`; `spur_native_database_weak`, `spur_native_database_lock`, `spur_native_database_weak_destroy`, `spur_native_api`, `spur_native_api_destroy` |
| `remaining-gaps.json:3618–3632` | `vendor/tauri-plugin-mcp-bridge/src/screenshot/ios.rs:139–140` | Extern block `[6192,6293)`; `UIImagePNGRepresentation` |
| `remaining-gaps.json:2–9` | `crates/spurlab-telemetry/src/buffer.rs:66` | Representative unnamed field `[2505,2518)` |

These are inventory coordinates, not independently reread target bytes. In particular, this inventory alone does not establish any site's ABI, safety qualifier, `link_name`, configuration activity, or linked implementation.

Baseline file SHA-256 values, recomputed from this checkout:

| File under `2026-11-02-spur-notebook-new-build-check/` | SHA-256 |
| --- | --- |
| `remaining-gaps.json` | `f61b117c9266cfcb0ff0b972b9170413e791c5bfa5a558ba4b9efead634c7ac9` |
| `summary.json` | `bd5040df07c0b12be468221762bf1c985e3e6884a3d3294a23788cabaa50530a` |
| `baseline-binding.json` | `f025682dfa1a2ddb114c066e6e07d25092130e0e26acce48906d784b981bfc55` |
| `graph-manifest.json` | `8c94d087146781e3c06cd966ddb2c4cff8de35bed5c2a20714804c97c3fbe3e0` |

## Observed extraction, ownership, and identity

All code paths in this section are relative to the repository root.

| Seam | Exact source evidence | Observed behavior |
| --- | --- | --- |
| Rust capture surface | `crates/spur-graph/queries/rust/tags.scm:17–18,35–42,52–58`; `queries/rust/spur-edges.scm:8–11` under the same crate | Named `field_declaration` requires `field_identifier @name`. Trait signatures and bodyful trait methods are captured as methods; ordinary `function_item` is captured as a function. Enum variants are defined only in `spur-edges`. Neither query defines unnamed fields or extern blocks; signatures outside the trait pattern have no definition capture. |
| Kind assignment | `crates/spur-graph/src/extract/languages.rs:448–475` (`rust_config`), `1839–1854` (`definition_kind`), `2170–2178` (`has_impl_ancestor`) | Rust maps fields to `Field`, functions to `Function`, and trait methods to `Method`. A function directly in an impl declaration list becomes `Method`. ABI syntax is not the discriminator. |
| Anonymous names | `crates/spur-graph/src/extract/languages.rs:1879–1905` (`definition_candidates`), `1908–1961` (`definition_name`), `3076–3091` (`contained_capture_text`) | Definitions obtain names from the same query pattern/match, with explicit special cases such as impls. There is no tuple ordinal/extern-block naming branch. A capture with no label is skipped by both emitters (`1296–1299`, `1333–1336`). A new `@definition.field` capture by itself is insufficient. Capturing the type as `@name` would label equal-typed slots identically and conflate display/type with field identity. |
| Graph definition passes | `crates/spur-graph/src/extract/tree_sitter.rs:3493–3526` (`extract_file_contents_from_tree`) | First emit tags definitions; then emit `spur-edges` definitions with the first set as parents. Existing children are not reparented after the second pass. |
| Byte extraction path | `crates/spur-graph/src/extract/tree_sitter.rs:104–125` (`symbol_query_policy`), `270–302` (`BytesExtractor::extract`); `languages.rs:1324–1358` (`extracted_symbols`) | Rust reuses tags for the symbols query. Byte extraction merges tags/symbol and edge captures before sorting definition candidates, and offsets query-local pattern indices. It can therefore have a parent candidate that was unavailable when the graph's first pass emitted a field. |
| Parent and edges | `crates/spur-graph/src/extract/languages.rs:1282–1320` (`emit_definitions_with_parents`), `2141–2167` (`nearest_parent`, `contains`) | Reverse-scan emitted bindings for a containing byte interval, else the file. Emit parent `Contains` child, and additionally `Defines` when parent is not the file. Parent identity is positional ownership, not compiler scope analysis. |
| Identity generation | `crates/spur-graph/src/extract/tree_sitter.rs:422–472` (`FactBuilder::add_node_with_range`); `src/identity.rs:37–72` (`stable_symbol_id_for`, `stable_symbol_id_for_discriminator`) under the same crate | Allocate sequential `NodeId`/`SpanId`; hash relative path, NUL, FQN, NUL, kind discriminator, NUL, little-endian start byte. SHA-256 is truncated to 64 bits and rendered as 16 hex digits. Node IDs are run-local; stable symbol IDs are deterministic for those inputs and are offset-sensitive. They are not rename/move/format invariant. |
| Identity and containment reconstruction | `crates/spur-graph/src/extract/languages.rs:1311,2292–2296,3138–3143`; `src/extract/tree_sitter.rs:3908–3914,3945–3996`; `src/store/build.rs:2862–2942` under the same crate | FQN uses parent FQN plus the label segment, with special impl formatting. Build reconstructs qualified names by walking `Contains`; `enclosing_scope` stores only the immediate parent's display label. Neither ownership metadata nor a separate lexical namespace parent is persisted in the symbol. |
| Existing representation | `crates/spur-graph/src/schema.rs:209–220,259–267,312–368,374–389` | `NodeKind` already includes `Field`, `Function`, `Method`, `EnumVariant`, `Module`, `External`. It has no extern-block/foreign-function/tuple-field variant. `GraphSymbolArtifact` stores name, kind string, source coordinates, anchor, qualified name, enclosing scope; it has no ABI, unsafe, body, attribute, ordinal, or explicit parent-ID fields. |

### Consequence: enum payload ownership needs more than an extra query

For `enum E { A(u8), B(u8) }`, placing tuple captures in tags would emit their nodes before variants `A` and `B` exist in the graph pass. The nearest available owner would be `E`. Labels `0` would yield `E::0` for both; differing start bytes prevent immediate ID equality but do not repair incorrect ownership or ambiguous names. The byte extractor, which merges candidates, could instead see `E::A::0` and `E::B::0`.

The same ordering also predicts a pre-existing named-payload seam for `enum E { A { x: u8 } }`: tags can emit `x` under `E` before `A` is emitted. This is a **source-derived prediction**, not a newly executed failing test or a claim that the remaining-gaps artifact measured bad containment. Reparenting after IDs are allocated would leave identity FQNs and reconstructed qualified names inconsistent unless every dependent identity/index/edge is repaired. Prefer deciding owners before creating nodes.

## Proposed tuple-field contract for root experiments

Keep `NodeKind::Field`; distinguish unnamed form and ordinal in explicit provenance. Collect only direct fields of a tuple struct or tuple enum variant, with the variant as owner in the latter case. `ordered_field_declaration_list` is already recognized as a tuple-constructor body in `crates/spur-graph/src/extract/rust_bindings.rs:504–520`; the precise child/attribute grammar still needs the root's parser experiment. Do not capture every descendant type node: a nested tuple, function pointer, generic argument, or array bound inside one field is not another field.

Use a zero-based **physical syntax field ordinal**, counting only actual direct field entries. Visibility, attributes, comments, delimiters, nested types, and trailing commas do not increment it. Keep raw `cfg`/`cfg_attr` evidence. A physical ordinal under conditional fields is not a promise of the compiler-selected tuple projection index; a configured mapping needs separate evidence.

Candidate names: display `0`/`1`, qualified name `S::0`/`E::A::0`, ownership `(owner_symbol_id, ordinal, unnamed=true)`. Repeated equal types are separate fields; Rust type text and `<unnamed>` are unsuitable identities. Preserve the type span and the full field-entry span separately if the parser exposes different intervals; visibility/attributes must not silently shift the meaning of the canonical source range. Parent and field provenance must identify exact file bytes/source OID, with half-open byte ranges and a documented line convention.

Two identity choices require an explicit root decision:

| Candidate | Advantages | Compatibility and limitations |
| --- | --- | --- |
| Existing hash over `(path, owner-qualified-name::ordinal, field, field_start)` | Reuses the existing symbol contract; newly added fields have no previous IDs to migrate. | Byte shifts change IDs. Correcting previously misparented named payload fields changes their IDs/FQNs. Ordinal is still explicit metadata rather than recovered by parsing a display name. |
| Versioned tuple-field hash over `(owner_stable_symbol_id, tuple_field, ordinal)` | Expresses parent-plus-slot identity directly; equal field types and equal ordinals under different variants remain distinct. | Needs a dedicated identity path and shared generation in every extraction/store consumer. A shifted parent changes all child IDs under today's offset-sensitive parent scheme. Insertion/removal changes slot meaning; it does not preserve a semantic field across reorder. The new domain must be separated from existing hashes and collision-checked. |

First experiment should compare both schemes without changing existing production identity. New nodes do not require changing the hash contract for existing kinds. A parent-plus-ordinal proposal is only as stable as its parent, and must not be sold as a general Rust semantic identity.

For either scheme, assemble the complete Rust declaration candidate set, then establish direct syntax ownership before naming/emission. The common descriptor should feed both graph facts and `ExtractedSymbol` results. Preserve query pattern/match separation and same-range method deduplication; `definition_candidates` currently sorts/deduplicates by syntax range/kind (`languages.rs:1897–1904`). An alternative is a dedicated owner-aware structural collector after owner nodes exist, shared by both extraction paths; it must not depend on a reverse scan of arbitrary type descendants.

## Proposed foreign declaration and extern-block contract

### Separate three different entities

1. A bodyful `extern "C" fn exported() { ... }` is a physical function definition. The generic `function_item` query already covers that syntax class, and an impl member remains a method. Adding foreign signatures must not duplicate it or relabel its ABI as `External`.
2. A bodyless `fn imported(...);` directly in an extern block is a physical **foreign declaration**. Capture it only in the foreign-item context, retaining the Rust name, source signature and lexical module/block scope. Do not broaden every `function_signature_item` into a free function: required trait signatures already have method identity. Keep `NodeKind::Function` with `declaration_origin=foreign` and `body_status=absent_foreign`, or justify a new kind with the consumer changes below.
3. `external://` is a synthetic import-target namespace. `crates/spur-graph/src/identity.rs:35–51` hashes external path/kind at byte zero; `src/store/build.rs:1523–1543` (`ExternalImportRegistry::bind_external`) creates symbols with `[0,0]` byte/line ranges, an empty anchor, kind `external`, and no enclosing scope. `buckets_from_facts:837–839` skips extractor-produced `External` nodes. This is not a usable physical declaration representation. A physical foreign declaration must not merge with a placeholder sharing a leaf name or `link_name`; linking to an implementation needs independent provenance.

These classifications are syntax proposals; accepted grammar, edition rules, ABI validity, linkage and effective safety are compiler/native experiment subjects for the root.

### Preserve structure without inventing a Rust namespace

An extern block has no declared identifier. Give each block a distinct synthetic identity scoped to its physical file and enclosing syntax owner, with byte coordinates and/or a physical block ordinal. Two blocks with the same ABI must not collapse. A proposed display label such as `<extern C #0>` is explanatory text, not a Rust path segment. The block is a non-callable structural container.

Candidate full representation: new `NodeKind::ExternBlock`/`extern_block`, lexical owner `Contains` block, block `Contains` foreign declarations (and existing non-file `Defines` convention, subject to defining its meaning). Retain one authoritative structural parent per node. Foreign declarations retain namespace lookup names such as `m::imported`, with a separate structural parent reference to the block. Current FQN builders derive every segment from `Contains`, so they need an explicit transparent-container rule or independent lexical-name projection. Adding a second competing `Contains` edge is not a substitute: both `parent_by_target` implementations keep the first edge (`tree_sitter.rs:3945–3958`, `store/build.rs:2870–2883`).

Alternative minimal representation: a versioned extern-block evidence relation/table keyed by physical coordinates and linked to declared functions, leaving the current node-kind set unchanged. This can capture the three blocks' syntax and provenance, but it **does not satisfy individual graph-node correspondence** for extern blocks; coverage must say so. Reusing `Module` could avoid a Rust enum variant but would make ordinary module counts/navigation/importability misleading unless every relevant consumer consults an extern subtype. It has a compatibility cost despite requiring no enum addition. Do not reuse `External` for either option.

Foreign declarations as `Function` participate in existing callable/import kind filters (`tree_sitter.rs:2937–2967`). That alone does not establish correct binding: `rust_bindings::item` only scans direct `function_item` children (`src/extract/rust_bindings.rs:742–784`), and the graph extraction call fast path counts `function_item` definitions (`tree_sitter.rs:3587–3600`). Foreign signatures live behind an extern container. Root experiments must check module-qualified, local extern-block, shadowed and duplicate declarations before changing lookup behavior; preserve ambiguity rather than selecting a globally unique leaf name without lexical evidence.

### Proposed persisted provenance, separate from semantic conclusions

| Fact | Proposed contract and adversarial distinction |
| --- | --- |
| Source and membership | Physical file path, exact source OID, declaration and signature byte/line ranges, parent block ID, lexical owner ID, syntax kind, parse/error status. Unknown source/parse evidence must not manufacture a complete declaration. |
| Body status | Distinguish `present`, `absent_foreign`, `absent_trait_required`, and unknown/recovered syntax. Optionally persist body range only when present. Do not infer an empty/safe body from missing ranges or zero outgoing edges. |
| ABI | Record explicit ABI text/span on a bodyful function or enclosing extern block and whether the declaration inherits it. Omitted ABI is distinct from explicit `"C"` and from unknown parse data. An effective ABI needs a pinned compiler/configuration policy rather than an unlabelled default. |
| Unsafe/safe | Record literal block and item qualifiers, their spans, and inherited syntactic context separately. Do not equate absence of an `unsafe` token with a compiler-proven safe call. Explicit safe foreign items, older syntax, edition and feature support belong in compiler controls. |
| Attributes and linkage | Preserve raw outer/inner attributes at their actual owner. Store literal `link_name`/`link_ordinal`, `link` and `cfg` evidence with ranges; conditional or unparsed attributes remain raw/unknown. `link_name` is a linker spelling, not the Rust symbol's display name or global ID. Do not guess macro-expanded attribute values or a linked target. |
| Visibility/type | Preserve field/function visibility and raw signature/type syntax where useful. Neither is a resolved type or proof of FFI soundness. |

Existing attribute code is a useful seam: `crates/spur-graph/src/extract/rust_macros.rs:81–111,307–324` collects outer attribute siblings and inner attributes with owner/range/raw text. It records **macro-site context**, not a reusable complete declaration attribute store. Reuse its proven ownership pattern only after root experiments; do not claim ABI/linkage metadata already exists in symbol persistence. `scope_kind:231–249` recognizes function signatures but not extern-block scopes; `scope_evidence:263–293` uses evidence-only IDs when no actual node exists. Any new extern scope kind needs review against macro evidence validation, which currently binds callers only to actual function/method symbols (`schema.rs:136–150`).

## Kind, schema, and consumer implications

| Change | Necessary migration and affected seams |
| --- | --- |
| Tuple fields as `Field`, foreign declarations as `Function`, existing node columns | No new kind or Parquet column is intrinsically required for node presence. Update query/name/ownership logic and extractor version. Queries are already fingerprinted; code-only naming/identity changes require an extractor-version bump. Correcting old field ownership can change IDs even without a schema bump. |
| New `ExternBlock` or `ForeignFunction` kind | Update `NodeKind` serde/discriminator/from-discriminator (`schema.rs:312–368`), Rust kind map, `definition_name`, local `symbol_kind` string mapping (`languages.rs:2266–2284`), FQN projections, and build's exhaustive symbol match (`store/build.rs:773–840`). Update callable/import filters and exhaustive fallback-kind match (`tree_sitter.rs:2937–2995`); extern blocks must be excluded. A foreign-function kind would need explicit callable inclusion. |
| ABI/body/ordinal/parent facts as symbol columns | Extend `GraphNode`/facts or a shared descriptor so facts survive `buckets_from_facts`, extend `GraphSymbolArtifact`, full/delta/rebucket projections, Parquet node writer/readers, and query projections that construct symbols. Today `write_nodes` has 12 columns and `read_nodes` reconstructs only those (`store/parquet.rs:1624–1641,2480–2517`). Optional defaults help new readers with old files; old JSON readers use `deny_unknown_fields`, so adding optional fields is not bidirectional compatibility. |
| Separate versioned structural evidence table | Keep current node layout if desired, but wire full build, unchanged-file reuse, delta replacement/deletion, manifest row counts/completeness, source/graph validation, canonical hash, readback and sidecars. Absent old evidence means unavailable, not zero declarations or proven body absence. A table without actual extern-block nodes must not be counted as node coverage. |
| Qualified-name/identity changes | Keep structural and lexical ownership explicit and consistent across `BytesExtractor`, `FactBuilder`, store FQN reconstruction and resolver indexes. Update snapshots/deltas/tombstones and downstream stable-ID references deliberately; do not silently alias two cfg alternatives. Parquet `endpoint_id_map` rejects duplicate stable endpoints (`store/parquet.rs:1403–1417,1420–1470`); retain that validation. |
| Search and analysis consumers | Existing symbol search uses string equality for kind filters (`src/search.rs:125–127`, `src/query_hot_index.rs:417–419` under `crates/spur-graph`). New kind values require clients/SQL inventories/UI filters to recognize them; column additions require hot-index projections and symbol constructors (`query_hot_index.rs:313–387`). Any runtime/panic analysis that treats `function` as bodyful needs explicit body/provenance checks. Downstream crates/frontends were not audited here; root must enumerate their contracts before implementation. |

Versioning evidence: `crates/spur-graph/src/store/build.rs:32–33` declares schema v11 and an extractor version. `current_manifest_version`/`manifest_version_from_query_bytes:263–287` hashes schema, extractor, resolver and query bytes. `artifact_from_facts_incremental:377–405` selects a full rebuild on manifest incompatibility; `build_worktree_delta_artifact:640–653` rejects a mismatched manifest. Preserve this behavior: otherwise unchanged source buckets could keep the missing fields/declarations forever. No rebuild was performed for this audit.

Recommendation for root evaluation: reuse `Field` and `Function`, compare a dedicated extern container kind against evidence-only capture according to the desired node-coverage contract, and version the richer provenance contract explicitly. New kind strings fit the physical Parquet string column, but remain a semantic schema change for typed consumers. A new schema version is appropriate for new persisted provenance/kind semantics; the exact version number and compatibility policy remain the root's decision. `GraphArtifactBodyForHash` hashes symbols and selected evidence (`src/store/canonical_hash.rs:10–45`); a separate table is not automatically covered unless added to the canonical contract. Legacy hash behavior and snapshots need explicit fixtures.

## Adversarial acceptance cases for root Notebook MCP experiments

These are proposed observable acceptance criteria, not passing tests. Pin parser versions, input bytes/OIDs, configuration/edition, requests and outputs. Compare graph-facts projection with byte extraction; serialize and reload candidate evidence before treating the result as a persistence success.

| Case | Required observable result |
| --- | --- |
| `struct S(u8, u8);` | Two field nodes with ordinals 0 and 1, distinct IDs, direct owner `S`, exact nonempty field spans; one type repeated must not deduplicate fields. |
| `enum E { A(u8), B(u8) }` | Two distinct `0` fields owned by different variants; FQNs `E::A::0`, `E::B::0`; graph and byte extraction agree. |
| `enum E { A { x: u8 }, B { x: u8 } }` | Existing named fields remain distinct and have variant ownership in both paths; catch the two-pass seam before accepting a tuple-only patch. |
| `struct S((u8,u8), fn(u8,u8), Vec<(u8,u8)>);` | Exactly three direct fields, not one per nested type/parameter. A named field `struct N { x: (u8,u8) }` remains one named field. |
| Tuple visibility, comments, attributes, trailing comma | Entry ordinals ignore non-fields; complete/type spans follow the declared policy. `struct U;` and unit variants contribute zero fields. |
| A conditional first tuple field plus an unconditional second | Persist both physical slots and raw condition; never assert compiled `.1` from physical ordinal alone. Root compiler controls compare at least two cfg selections separately. |
| Same-named structs in two modules/files; repeated cfg definitions | Parent-qualified fields and IDs remain distinct; no global `0` identity, type-text identity or accidental cross-file match. |
| Byte shift, type edit, field reorder, field insertion | Record exactly which IDs change under each candidate identity scheme; do not assert invariance unsupported by the offset-sensitive parent. |
| Bodyful `extern "C" fn exported() { helper(); }` plus `extern "C" { fn imported(); }` | One body-present physical function and one body-absent foreign declaration; no duplicate generic/method captures; no `external://` substitution. |
| `trait T { fn required(); fn provided() {} }` plus a foreign `required` | Trait entries retain method kind/body distinction; foreign entry has function/foreign provenance. No unconstrained global-signature capture. |
| Two same-ABI extern blocks, including an empty block | One physical block record/node per block, stable distinguishable coordinates/IDs, no callable block and no collapse by ABI/display label. |
| Block containing functions plus `static` and `type` declarations | Each child is classified by its syntax; a static/type is never fabricated as a foreign function. This fixture expands validation without claiming these categories are current measured gaps. |
| Foreign Rust name with `#[link_name = "native_symbol"]`; same linker name under two scopes | Rust names/IDs and link spellings remain distinct facts; no name-based collapse or unproven implementation edge. Test raw-identifier and case-sensitive foreign names too. |
| Omitted ABI, explicit `"C"`, `"C-unwind"`, unsafe block/item and explicit safe item | Preserve explicitness and origin; unsupported/recovered syntax stays unknown. Root compiler controls record toolchain/edition/flags; parser acceptance alone is insufficient. |
| Module/local extern declaration with `m::imported()`, an inner shadow, duplicate cfg declarations | Resolve only with scoped evidence or preserve ambiguity. Synthetic block label never appears in the Rust lookup path. |
| Same leaf name in an `external://` import placeholder and physical foreign declaration | Both may coexist with distinct provenance, ranges and IDs; source read on the declaration returns the physical signature. No assertion of linked target identity. |
| `fn caller(){ imported(); imported(); }` | Distinct candidate call-site records if site evidence is added, even if graph summary has one edge. Declaration extraction alone must not claim call identity is fixed. |
| Malformed signature/block, Unicode prefix, CRLF, attribute macros, cfg-disabled declaration | Check ranges against exact bytes and report parse/configuration boundaries. Macro-generated declarations remain outside direct physical-source coverage unless independently expanded evidence exists. |
| Old v11 artifact, changed source, deleted declaration/file, and manifest mismatch | New reader treats old metadata as unavailable; full/delta builds invalidate incompatible extractors and replace/delete evidence with its owner. Round-trip IDs, parents, spans and all metadata; manifest counts and hashes match reloaded data. |

For the measured corpus, join each of the 458 saved gap rows by exact source OID, path and an explicitly chosen declaration/field span policy, checking **one intended counterpart**, correct parent, kind and provenance. Do not accept a global node-count increase of 458 as proof. A type-only Tree-sitter field range may differ from the inventory's syn range; persist the relationship rather than relaxing matching to overlapping names/lines. Split tuple counts by struct/variant owner in the root experiment, and report excluded/error rows separately. On unchanged bytes, all 446 tuple fields, nine foreign signatures and three blocks are the target set; any intentionally evidence-only block representation must retain the three-node coverage limitation.

## Call/compiler/panic boundary

`PendingEdge` has no site span (`crates/spur-graph/src/extract/tree_sitter.rs:26–36`). `FactBuilder::add_edge_with_metadata:577–615` deduplicates caller/target/relation/label/kind/receiver/scope; repeated identical calls can collapse. `GraphEdgeArtifact` has no invocation coordinates (`schema.rs:226–254`). Closing declaration gaps therefore neither creates call identity nor validates call-target correctness. Declaration IDs should be referenced by a separate call-site evidence contract, not used as call-site IDs.

A foreign declaration adds a known syntax boundary with an unknown implementation body. Zero outgoing edges do not prove a leaf or panic freedom. ABI and `unsafe`/`safe` tokens do not establish soundness, unwind behavior of the linked implementation, or the full set of panic/abort paths. Tuple field extraction does not establish Rust layout, field access resolution, constructor dispatch, macro expansion or configured program membership. This report claims neither full Rust semantics nor complete panic detection.

## Verification performed for this report

- Read scoped production sources without modifying them; pinned the clean initial checkout to the source OID above.
- Read existing `remaining-gaps.json`, counted its kinds with Python `collections.Counter`, and compared with the saved summary; result exactly 446/9/3 and 458 total.
- Recomputed the four baseline SHA-256 hashes listed above.
- No production patch, target build, native compiler test, notebook operation or solver invocation was performed. The acceptance matrix remains work for the root's authorized experiments before implementation.

The completion audit carries the single-file unified diff, verification results and commit identity. Leave `bd-1v3` in progress for root review; this report does not approve or close it.
