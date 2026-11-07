# G02 prospective provider-codec PRE

Accepted predecessor: local main df91560547e3d8bf102511b7271e9d6f5843ef3b.
G01 actual-output POST remains pinned independently:
sol_1797a791aad242ee (SAT/pass), sol_79cd295bfba74567 (UNSAT/fail).
This packet binds 78 current source files, 18 intended acceptance cases and
19 independently rejected false controls. It does not claim native G02
codec, disk, query-backend, cache or runtime acceptance.

Fingerprint: 24edffe3ca782cb0cbed5d4f1b7513e80db86f717da6bc6534035f0522e1f867.
Jev jev-1.14.0: open, weakest 0.71. The compiled mutually_consistent contract
binding retains exact facts. Execution additionally binds provider uniqueness,
case uniqueness and exact coverage, all from the catalog. No vacuous subjects,
invented observations, generic SMT or solver limit changes.

| Request | Receipt | Result |
| --- | --- | --- |
| Intended codec contract | sol_1edb0703733b4de2 | SAT/pass |
| False codec claims and provider alias | sol_137ccfd26e414299 | UNSAT/fail |

Complete envelopes are pinned/reloaded. Eight compiled predicates and each
of 19 false controls are independently evaluated. G01's consumed PRE pair
was fully reloaded, compared and checked (eight predicates) before releasing
its retention pins. The accepted G01 POST remains retained. Existing solver
capacity is unchanged; 63 variables fit the existing 64-variable limit.

## Grounded scope and clarifications

The approved G02 plan owns exactly:
- src/evidence/providers/mod.rs
- src/evidence/providers/rust.rs
- src/evidence/providers/legacy_macros.rs
- src/evidence/mod.rs (module declaration)
- tests/generic_evidence_providers.rs

All paths are relative to crates/spur-graph. Worker evidence stays below
G02/worker. Rust's five existing table basenames and schemas and macro's two
existing tables are the codec compatibility contract. Current private disk
row codecs remain in place until G04 delegates them to the pure providers.
No artifact field, collector, parser, storage writer/reader, backend or
cache change belongs to this stage.

Both providers implement the current G01 EvidenceProvider trait. Static
composition registers spur.rust.physical and spur.rust.macros; TestCounter
is a test-injected independent physical-source provider, never a production
registration or a language parser. Domain and owner checks use authoritative
current graph manifests and existing typed validators. Preserve full original
origin inventories, typed JSON and every duplicated scalar/header/version.

Legacy macro byte length remains unknown where its original format omitted
it. Mark authenticated original-format metadata; never invent zero length
or assert that old archives recorded new provenance. Any derived carrier
origin must be explicitly grounded in the supplied graph's validated
manifest/hash; no fabricated source observations are allowed.

The actual MacroInvocationFilter currently has file_path,
caller_stable_symbol_id and source_oid. The broader spec prose mentioning
additional selectors does not introduce new Rust fields. Preserve current
rows, scopes, snapshots, conjunctive filters and coverage using the actual
types. Convenience query adapters consume the generic evidence route;
they cannot obtain a GraphIndexArtifact through a nonexistent trait method
or fabricate graph owners to bypass validation. Built-in backend forwarding
and frozen contexts remain G11.

New APIs legitimately begin with missing-API compile RED before minimal
implementation. Never add a deliberately broken stub to manufacture an
assertion failure. When changing an existing behavior, retain its real
runtime assertion RED. Real encode/decode tests collect a source fixture
through existing build APIs, compare the full original typed sets with
decoded sets, and separately test stored integrity versus source
authentication on independently rehashed body/owner/source mutations.

Required commands use scripts/spur-cargo:
- test -p spur-graph --test generic_evidence_providers --test rust_evidence_contract --test macro_invocation_artifacts
- check -p spur-graph --all-targets
- local fmt -- --check

ROOT owns actual-output Jev POST and independent native mutations after
implementation. Do not reuse this prospective model as implementation
proof or approve a successor before those checks pass.

## Replay

```bash
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/root-pre/verify.py --current-source
```

After G02 edits, omit --current-source to authenticate the historical
predecessor epoch; separately bind the new compiled source identities.
Worker reloads only the two current G02 receipts with pin omitted and
compares full persisted envelopes, without reading solver storage directly.
