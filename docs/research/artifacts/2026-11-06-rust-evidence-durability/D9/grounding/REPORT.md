# D9 current cache grounding for ROOT PRE

This packet grounds the next D9 PRE at committed source
`a9f6ca7ac92a354fb3dbf97f8f72d60d3787b337`. It implements nothing. No Rust
build/test, new native replay, solver request, notebook, graph rebuild, pin
mutation, approval, close, merge or push occurred. ROOT owns fresh D9 PRE/POST
and the subsequent implementation/TDD dispatch. Existing test descriptions
below are static source witnesses, not fresh passing test results.

Canonical audit target: **bd-h3yn**, assignee `spur-worker-1f5b329203a445e5`,
delegation `1f5b329203a445e5`, plan `dbbc6ed1-abc9-4dc6-b6b0-7497d053ccdd`,
task `D9-current-cache-grounding`, source `bd-2vmm`. `get_issue` and authoritative
plan status agreed: issue `in_progress`, task `dispatched`, one task. Fresh UUID
INTENT `5fd19b24-1cc0-4a98-9512-423f22c2fe70` persisted as comment **4369** and
replayed idempotently before filesystem writes. The rendered check was
`report_audit.task_id == get_issue(canonical_id).id == bd-h3yn`. The slug is
evidence metadata, never the worker audit target. Completion receipt and commit
binding are recorded in the canonical audit after this document is committed;
the issue remains `in_progress` for ROOT review.

Read [source-map.json](source-map.json) for full file SHA-256/Git blob bindings,
47 hashed line ranges, all 18 construction sites for the mapped cache identity
carriers/wrappers, field maps and direct caller boundaries. Read
[pre-inputs.json](pre-inputs.json) for exact accepted archive/member references,
raw recorded manifest bytes, physical source bytes/OIDs, parent checks, and ten
explicitly prospective acceptance cases. SHA-256 and Git blob IDs here bind
archive/source bytes; they are not substituted Rust capability API results.

## Accepted D8 parent evidence

Read both D8 `ROOT-acceptance/README.md` and `local-integration/README.md`.
Executed, exit **0**:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/ROOT-acceptance/verify_post.py --source-revision a9f6ca7ac92a354fb3dbf97f8f72d60d3787b337
```

Result: fingerprint
`2c96923bc95d01d6ccfa18cdf580721b12fbf4d3953c1bf2ec1f48e45251618b`,
2,620 bindings, 19 positive groups, four negative controls, nine independent
predicates, 18 previously recorded ROOT native invocations. The verifier does
not launch these native invocations anew. Its compiled-source cohort is
`47d9619acadf3c1f276552a73355f1db0d5887a2`; the original dirty capture at
`287d3020d` remains honestly recorded. The integration README describes the
earlier integration epoch `c611b67f5131727bf0dcc259f96f48598e889b2e`; the explicit
revision override verifies the consumed committed epoch above without changing
either historical record. The known unrelated macro fixture failure `bd-xb5p`
remains explicit in the parent acceptance.

Reloaded **all three** through `get_solve_result` with `pin` omitted. Compared
each entire parsed live envelope against its `reloaded` object in
`D8/ROOT-acceptance/post-proof.json.gz`, including request, model or unsat core,
duration, reason, solver version, and all other fields. Recursive object-key
normalization preserves arrays and values; complete structural equality held.
Canonical-envelope SHA-256 and field inventories are in `pre-inputs.json`.

| Pinned parent | Verified result | Compact complete envelope bytes |
| --- | --- | ---: |
| `sol_1333dbcec9c4429b` | SAT / pass | 109927 |
| `sol_8ae82f413aae4b48` | SAT / pass | 91651 |
| `sol_8119bed20fbb414f` | UNSAT / fail | 25287 |

No retired phase receipt was required live; no solver storage was read.

## Current source and retrieval limits

The worktree was clean at entry. Every mapped file matched its committed bytes.
Primary file SHA-256 values:

| File under `crates/spur-graph/src/mcp/` | SHA-256 |
| --- | --- |
| `request_cache.rs` | `2200e7f287edf9570326d4feda33df939cc00e337b18f92744291c3ee1d5c973` |
| `overlay_snapshot.rs` | `94176d5ac3e86b7f50f6a5c994a2c6e3c0a9218f2210bb5198ef44100a140048` |
| `mod.rs` | `b732eb2f801805bf35701175cf83215e08b623034085956b7ff6f7eb91346e24` |

Used `knowledge_context_pack_2` first. It reported analyst graph hash
`792c0e6af5aeb4e27447489e1aba27104d5dd7a809b8ff35ae14b3f3bf1abd53`
versus exact graph hash
`50a577266ca7a91f1c141e67015a18b7d137ed44d008563712451563e8a71ad7`,
and skipped stale analyst reasoning. Next, `code_read_symbol` for indexed
`finish_snapshot` and scoped struct `code_symbol_search` both exceeded their
15,000 ms deadlines. Those failures returned no indexed HEAD/file-OID freshness
certificate. Current narrow raw reads, committed-byte comparisons and line
hashes therefore supply the evidence. No claim that the graph represents D8
is made, and no unrelated source/skill traversal was needed.

## Actual cache seam and retained identity

All locations in this section refer to the three primary files unless qualified.

| Layer / symbol | Current retained identity and guard | Prospective D9 seam |
| --- | --- | --- |
| `mod.rs:2883 CodeSearchBackend::snapshot_base` | Parquet: frozen client manifest graph hash + indexed commit + client file/OIDs. Memory: artifact graph hash + no indexed head + indexed file/OIDs. | Compute the existing artifact/manifest capability helper for the corresponding owner, propagate a new digest. |
| `overlay_snapshot.rs:101 SnapshotBase` / `:108 compatibility` | Graph hash, optional indexed head, ordered file/OIDs. Compatibility graph hash hashes length-prefixed path/OID pairs and sets no head. | Add prospective `indexed_rust_evidence_digest: String`; compatibility must explicitly encode **absent**, using the shared helper's manifest/version inputs. A made-up string or captured-zero default is insufficient. |
| `overlay_snapshot.rs:123 CachedSnapshot`, `:422 cache_matches_graph` | Cache partitioned by canonical worktree; matches source graph hash and indexed head. Stores current head, full index identity, tracked index, file stamps, clean status, snapshot. | Persist/compare capability before clean reuse and before passing cached state to `build_snapshot_once`; propagate in `store_snapshot:1090`. |
| `overlay_snapshot.rs:32 SnapshotIdentity`, `:872 finish_snapshot` | Canonical worktree, source graph hash, indexed/current heads, full index identity, normalized changed-set fingerprint. | Include prospective capability in the derived Eq/Hash identity and every constructor/test helper. |
| `request_cache.rs:467 OverlayDeltaCacheKey`, `:50 overlay_delta` | Wraps complete SnapshotIdentity; both exact cache and in-flight map use it. | Digest propagation through identity must partition both maps. |
| `request_cache.rs:78 overlay_generation`, `:510 OverlayGenerationCache` | Exact entries, LRU and in-flight map keyed by full SnapshotIdentity. Published value stores identity. | Preserve complete exact identity and publication/error semantics. |
| `request_cache.rs:517 OverlayGenerationCompatibilityKey`, `:547 compatible_latest` | Only canonical worktree + source graph hash; selects latest retained identity from that bucket. | Add capability to this projection, insertion and removal paths so old-capability seeds cannot enter a new build. |
| `mod.rs:1060 VisibleOverlayIdentity`, `:1085 canonical_overlay_identity` | Visible key contains worktree, source hash, heads and changed fingerprint; intentionally omits index stamp. `or_insert(identity)` returns the first full identity for that visible key. Used at `:713` and `:2763`. | Include capability in the visible projection too; otherwise canonicalization can erase the new distinction before the exact cache. Preserve existing treatment of index refresh. |
| `request_cache.rs:398 ParquetClientCacheKey`, `:405 from_path` | Canonical artifact path, manifest mtime ns, **BLAKE3 of actual manifest bytes**. `parquet_client:23` rechecks identity around cache hit/open and permits its existing two attempts. | Already binds all Rust metadata bytes. Extend witnesses; do not replace with mtime or source hash, and do not change retry count/capacity. |

Construction inventory includes compatibility `Self:114`; backend literals
`mod.rs:2886,2891`; test bases `request_cache.rs:728` and
`overlay_snapshot.rs:1395`; SnapshotIdentity `:237,879`; CachedSnapshot
`:1101,1453`; compatibility-key `Self` at `request_cache.rs:524`; exact-delta
wrapper `:54`; generation wrappers `:114,1115,1119,1127`; Parquet key `Self:420`;
visible key `mod.rs:1070`; downstream generation identity `:2634`.
`request_cache.rs:690 cache_identity` delegates to `test_snapshot_identity`, so
its synthetic tags are static fixtures, not recorded native identity values.
Compatibility callers are `mod.rs:7121,7184` (and existing test at `:8818`).

Keep the actual changed-set algorithm (`overlay_snapshot.rs:1233`): sort path
and state, length-prefix path/OID bytes, tag tracked/untracked/deleted as
`0/1/2`, hash with BLAKE3. Preserve base file/OIDs, current tracked index and
conditional indexed-head/current-head diff (`:722`). Index/file identities
retain length/mtime and Unix device/inode/ctime/ns. Byte reads (`:843`) pair
before/after stamps; on mismatch retain the old stamp so outer final restat
rejects the read. Whole-result certification (`:620`) checks head, index and
file stability. The filesystem fallback removes cached snapshot, rechecks
discovery/membership, all read stamps and tracked index, and returns
`identity: None`; uncertifiable fallback errors. Capability propagation must
preserve these checks and the existing retries.

Cached client construction is `mod.rs:5362` (also readiness probe `:5321`) →
`request_cache::parquet_client` → `ParquetClient::open` in
`query_client.rs:1372`. A new client initializes a **lazy** empty `rust_context`
OnceLock; first Rust query at `:2127` opens/freezes and validates tables via
`store/rust_evidence.rs:718`. Subsequent queries use those immutable table
bytes. Errors are also retained in that client's OnceLock. Merely opening a
client does not establish that its Rust evidence tables validated. Existing
static frozen-context test `query_client.rs:2884` covers later disk corruption,
fresh cold error, and unchanged prior result; it was not run here.
`mod.rs:4759 RebuiltParquetGraph::from_artifact` is a separate request-local
direct client open after writing its own artifact; it bypasses this shared
cache and owns/removes its directory on drop.

## Shared helpers and exact absence contract

`rust_evidence.rs:361 rust_evidence_capability_digest` and
`store/rust_evidence.rs:78 rust_evidence_manifest_capability_digest` already
exist. They hash compact Serde JSON tuples containing, in order: domain,
presence, wrapper schema/extractor, declaration schema/extractor, call
schema/extractor, artifact manifest version, and full-envelope checksum or
JSON null. The D5 static parity test at `tests/rust_evidence_parquet.rs:390`
already covers mixed, Unknown, captured zero and None; it was not run here.

For the recorded manifest version, the exact legacy absence input is:

```json
["spur-rust-evidence-capability-v1","absent","spur-rust-evidence-v1","rust-evidence-wrapper-v1","spur-rust-declarations-v2","2026-11-03-rust-foreign-declarations-v1","panic-rust-call-sites-v1","rust-direct-call-bodies-v1","37eee2e2e0d9213d4d9ff13ccbacae861859bd592305fb88dbe54e4e09024626",null]
```

This is a static encoding description, **not a native observed digest**.
Present evidence uses `present` and its checksum; even a zero-source envelope
has a real non-null checksum. Envelope checksum is BLAKE3 over
`spur-rust-evidence-tables-v1\0` followed by the full ordered Serde envelope.
The artifact helper uses actual wrapper headers, supported declaration/call
constants and that checksum; manifest helper uses corresponding stored fields.
Valid D5 round trips agree. Helpers themselves do not validate source syntax,
provenance or version support. They do not directly encode disk schema,
manifest `complete`, row counts or JSON formatting. Raw manifest-byte cache
identity and cold version/completeness/checksum/table validation remain needed.

Recognized disk absence is current schema or exact legacy schemas v1–v12,
with all five evidence tables absent (`store/rust_evidence.rs:474`). Captured
zero requires the tables and their checked metadata/checksum. Thus neither
an empty domain nor a zero count can replace the presence tag.

## Recorded native pairs, with identical physical sources

Archive aliases below resolve to complete SHA-256/Git bindings in
`pre-inputs.json`. `native` is
`D8/empty-capability-repair/native-packet.tar.gz`; `explicit` is
`D8/ROOT-acceptance/native/explicit-controls.tar.gz`. No archive was extracted
to disk or altered.

**Captured versus legacy base:** `native/cases/zero/base.json` and
`native/cases/legacy-base/base.json` are identical parsed artifacts after
removing only `rust_evidence`. Their saved physical bytes, stable file IDs,
source OIDs, manifest version, and source graph hash are equal. Their delta
objects and shadowed path sets are also equal, so these recorded overlay
queries supply a more controlled pair than unrelated fixture generations.

| Physical source | Git source OID | Byte length | SHA-256 (both copies) |
| --- | --- | ---: | --- |
| `a.rs` | `07d0d9cb4b88df4d8515227e6b908d3704665e2a` | 57 | `224decbfaa1f1137d97910c4e5bebca58253bdac89c78e13df98c93776e2d794` |
| `b.rs` | `cb20f299a53970c409a2c0ca4b389ac7cf3adca0` | 20 | `ce2820c90af674d6c88f4f0698121275e7bd02c35c0fd0d741d040f89050101b` |

Both source hashes are
`4054cd61d35bced454de5e6cbd708ce9b95986e9c8ed0f5615014757e48c1f8c`.
Captured manifest `zero/base-disk/manifest.json` SHA-256:
`485c56033b4014b0f6f36ece846389bba5b2a654231049b61d43056b8291fa9f`;
legacy counterpart SHA-256:
`800e5bad0a77ce711c4f5952801aac1767c2e2cd301838f8f701d184cd905384`.
Exact UTF-8 manifests are embedded in `pre-inputs.json`. Captured metadata has
`complete: true`, the six versions listed above, checksum
`94e37dff6f848f411f212bb272b23f7d4c86a95307823f3ba0fe9ffd508c3642`,
and counts origins/sources/declarations/bodies/sites = **1/2/2/2/3**. The legacy
artifact/manifest omits the Rust evidence key and has no Rust evidence tables.
The recorded default overlay query is Complete for `zero`, Partial for
`legacy-base` in memory, disk and delta-reloaded modes: changed `a.rs` is
captured zero, untouched legacy `b.rs` is Unavailable. This is an actual D8
query distinction, not a cache collision/invalidation measurement.

**Captured empty versus absent empty:** explicit cases
`root-captured-empty-nonrust-shadow` and
`root-legacy-empty-nonrust-shadow` have no physical source members and empty
file manifests. Their base objects are identical except Some(empty) versus
null `rust_evidence`; both shadow `README.md`. Source hash is
`af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.
Captured base SHA-256 is
`69dc529b61ac1be1175e95f07db6104991349fe2a2c0eea8fd619a9fd01166f6`;
absent base SHA-256 is
`1d97b65ce1f9e9725824d3b313550280b27e7143a0203f2912aa516bda55590d`.
Native memory output is Complete with successful base/delta authentication
for captured empty; Unavailable with
`Rust evidence unavailable; rebuild with Rust evidence capture enabled`
for absent empty. These carriers were ROOT-materialized inputs consumed by
the accepted native replay; `capture/actual.json` is the native query and
authentication output. They are not fresh extractor outputs. These explicit
replays did **not** emit disk manifests.

The separate recorded `native/cases/identity/delta-disk/manifest.json` supplies
an actual captured-empty disk supplement with the exact same RustEvidenceSet,
manifest SHA-256
`0ac1b7f194e89f3965fe9e65c9f7365607b526c850a7cdf33818510d2f6a00f0`,
checksum `af4f0dc8e02a9a28a33cdaae73e28299c7622a32a23d7d6dbc4a1fc368f7bc95`,
and all five Rust counts zero. It is explicitly not relabeled as output of
the explicit replay. An absent-empty persisted manifest remains an observation
gap. Exact member bytes/hashes, all version metadata and the native deletion
and readd inputs/results are bound in `pre-inputs.json`.

No JSON in the three inspected accepted archives (`native`, `explicit`, and
ROOT's accepted matrix) contains a capability-digest output field. Native
manifest checksums above are copied, not recalculated. Actual helper outputs,
raw-byte BLAKE3 cache keys, full runtime cache identities and hit/build/seed
events need future Rust observations. No Python capability implementation or
fabricated cache experiment supplies them here.

## Minimal prospective changes and ROOT PRE witnesses

The old plan's `mod.rs:2883 snapshot_base` location is still exact. Its new
digest field is still absent in all four named carriers. Shared D2/D5 helpers,
actual-manifest-byte hashing, lazy frozen Rust context, component invalidation,
compatible seed selection and singleflight recovery already exist. Extend
their capability coverage; do not invent replacement helpers or new quotas.
The existing same-mtime test at `request_cache.rs:1642` only appends a newline;
Rust checksum/presence/version/complete metadata witnesses are still needed.

Within the three files, propagate prospective capability from the actual
backend owner through base → validated cache → finished identity → exact
delta/generation maps and compatible seed projection, plus the visible
canonicalization key. Update the construction inventory and retained-component
tests. The old first RED references the new field and may initially fail to
compile. Only a later runnable assertion failure is behavioral RED; neither
compiler failure nor this inspection is an observed cache collision.

ROOT PRE should bind the packet's `source_binding`, `capability`, `identity`,
`reuse` and `selected_provenance` relations to exact source/artifact references.
The relations in `pre-inputs.json` are prospective schemas and consistency
requirements, not solver facts or results. Require artifact/manifest helper
agreement on valid same tuples; exact reuse must preserve every existing
identity component plus capability; compatible seed reuse must preserve
worktree/source hash/capability; raw manifest client identity additionally
preserves path/mtime/actual-byte digest. Independently allowed selection tuples
must retain active source/OID/origin/body/site provenance. Presence and version
rules must reject false upgrades and unsupported cold metadata.

The ten prospective cases specify native witnesses for exact hits,
same-source/different-capability invalidation, stale seed rejection,
absence/captured zero, same-mtime manifest edits, failure/retry singleflight,
deletion/readd, all retained identity components, read races and the following
direct caller risk. None has been executed by this task.

**Risk requiring ROOT's implementation-scope decision, not a missing grounding
permission:** `mod.rs:2418` constructs a runtime key with source hash only;
trusted published routing (`:1826`) can return before the exact caches.
`is_current_runtime_base:2908` checks path/source hash, while
`mcp/overlay_runtime.rs:48 BuiltOverlayGeneration::new` and
`overlay_generation.rs:302 OverlayGeneration::update` guard the source graph
identity. Public `OverlayGenerationIdentity` has no capability field. Simply
adding the four plan fields does not establish capability freshness for these
consumers. ROOT must decide an in-scope freshness guard or explicitly resolve
downstream identity ownership before implementation; do not overload the
canonical source graph hash to conceal a capability change. This packet does
not authorize edits in either additional file or a runtime/model refactor.

Also, `mod.rs:1904 FullBaseArtifactSource::Parquet` loads the directory later,
whereas the memory variant keeps an Arc. A source-only base-hash check does not
certify that a later disk base has the capability captured in SnapshotBase.
Require a source-bound pairing witness in the implementation phase. These
static risks were reported under canonical `bd-h3yn` via progress and durable
risk signal `b84d2e90-ccae-4dd2-8136-787feec516e1`. They do not block producing
this input packet, and no new user confirmation or ROOT solver gate is needed
for this read-only grounding.

Verification for this documentation change checks JSON validity, every source
file/line hash and committed binding, archive/member bytes and embedded
manifests, pair equalities, complete parent envelope equality, exact three-file
scope, and unified diff whitespace. No fresh Rust GREEN claim is made.
