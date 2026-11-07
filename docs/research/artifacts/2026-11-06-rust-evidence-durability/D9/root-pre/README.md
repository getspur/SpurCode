# D9 ROOT PRE and implementation handoff

Grounding is approved under canonical `bd-h3yn`, source `bd-2vmm`, plan
`dbbc6ed1-abc9-4dc6-b6b0-7497d053ccdd`. The approved authored three-document
commit is `5f06c1cf1`; its integrated tree is `51e925902`.
D8 remains approved on local main `a9f6ca7a`.
Implementation source issue: **bd-1ae3**.

This PRE binds **112 exact source/range/artifact references**, **18 prospective
finite identity cases**, and **10 explicit false reuse/provenance claims**.
It is an intended policy over declared witnesses. It does not establish a
native cache collision, hit, rebuild, capability digest, or implementation
correctness. The two native same-source capability pairs come from the accepted
D8 archive; missing native digest/key/cache-event observations remain explicit.

- `sol_e7581ff6b2de4fc2`: SAT/pass on intended policy, all four catalog bindings.
- `sol_e365ec31f13d43d8`: UNSAT/fail for false claims; consistency and required
  provenance fail while uniqueness and cardinality still pass.
- Final Jev gate: **open**, weakest confidence **0.83**, model **jev-1.14.0**.
  Jev selects one consistency binding. ROOT explicitly adds unique, cardinality
  and conditional-required catalog bindings. No generic SMT was authored.
- Both complete persisted receipts were pinned, reloaded and compared. The
  independent D5 family/lowered-expression oracle checks **eight predicates**.

Reproduce read-only (no build, solve, native run or pin mutation):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D9/root-pre/verify.py
```

Read `inputs.json` for full vectors and reference bindings; `prepare.py`
reconstructs them from committed source and exact archive members.
Presence/header/checksum descriptors are lossless metadata labels, **not**
Python implementations of the native BLAKE3 capability/cache-key helpers.
All synthetic worktree/head/index/changed vectors and future metadata mutations
are explicitly prospective. Byte-only mutation appends a newline to the actual
captured manifest; version/complete changes have distinct prospective raw bytes.
Read `proof.json.gz` for exact family requests, raw results and complete
persisted generic envelopes. `jev-and-retention.json.gz` preserves interpretation,
candidate changes and retention operations separately.

Two preparation validation errors are preserved: the compiler requires
`boolean`, not `bool`, and the initial flat relation exceeded its existing
64-variable limit. Neither error executed Z3 or established UNSAT. ROOT corrected
the schema and encoded the same 18 cases as lossless case-bound finite tuple
labels (54 variables) plus one two-field provenance summary (five variables).
No solver/runtime capacity was raised. Final Jev recompilation followed these
material schema/data changes; no confidence threshold was changed. The earlier
open and blocked interpretations are archived with their original facts.

Only the two fully consumed, closed focused-D8 PRE pins were released after
all complete live envelopes matched their archives and six independent predicates
passed. Three D8 POST parents remain pinned and matched their full saved
envelopes. Their raw live copies and exact retention responses are archived.

## Grounding risk 4372: implementation scope decision

Add capability to the four original snapshot/cache carriers **and** the visible
canonicalization projection. Otherwise canonicalization may erase it before
the exact cache. Compute the capability through existing D2/D5 helpers on the
actual artifact/manifest owner. Compatibility constructors must encode absence
through the same shared contract; captured zero must remain distinct.

The direct runtime ownership boundary also needs an explicit capability
component. ROOT authorizes the narrowly required fourth MCP file,
`overlay_runtime.rs`, alongside `request_cache.rs`, `overlay_snapshot.rs` and
`mod.rs`. Keep source graph hashes and public `OverlayGenerationIdentity`
semantics intact; do not hide evidence changes inside a source-hash string.
Partition runtime acquisition/publication by the captured capability and check
the built generation's actual base capability. Preserve normal runtime behavior,
per-coordinator ownership, exact fallback and terminal/restart behavior.

A persisted base that is current by source/path alone is insufficient:
re-resolve current metadata and require the captured capability before routing
a trusted published result. A Parquet base loaded later must match the captured
source **and capability** before seeding/updating. Memory keeps its Arc owner.
Digest helpers are not validation: unsupported/incomplete cold evidence must
still follow the existing native validation/error path, and no failed metadata
may gain a Complete result through hash equality or cache reuse.

Preserve all head/index/stamp/OID/changed-set/read-race checks, actual manifest
byte hashing, existing capacities/retry counts, singleflight recovery, and
deletion/readd semantics. Do not refactor unrelated runtime/model APIs.

## Required implementation evidence

Commit failing tests before fixes. An initial compile RED for new fields is
allowed but is not behavioral RED; runnable assertions must expose each new
behavior before its fix. Use the existing Rust APIs and real cache/route
observations, not Python expected-output fabrication.

Record complete old/new snapshot/visible/exact/seed/runtime keys, actual native
artifact/manifest helper digests and raw-byte manifest-key outputs, actual Arc
hits/build counters/seed choice/routing and selected evidence envelopes.
Exercise the two saved D8 pairs, same-mtime checksum/presence/version/complete
changes, published stale generation and late-load substitution, retained
components, singleflight failure/retry, and deletion/readd. Previously frozen
clients retain their authenticated context; fresh clients must validate the
new metadata. No live graph/CURRENT or notebook work belongs in D9.

Required commands, always through the wrapper (remote default; no local fallback):

```sh
scripts/spur-cargo test -p spur-graph mcp::request_cache::tests
scripts/spur-cargo test -p spur-graph mcp::overlay_snapshot::tests
scripts/spur-cargo test -p spur-graph mcp::overlay_runtime::tests
scripts/spur-cargo test -p spur-graph --test rust_evidence_overlays --test mcp_overlay_notfound_retry
scripts/spur-cargo check -p spur-graph --all-targets
scripts/spur-cargo fmt -- --check
```

Also run the focused new/existing `mod.rs` runtime/freshness regressions.
Preserve the unrelated known macro baseline `bd-xb5p` if encountered.
ROOT owns fresh POST, independent alias/forgery review, approval and local-main
integration after the implementation handoff. D10 notebook work follows D9.

