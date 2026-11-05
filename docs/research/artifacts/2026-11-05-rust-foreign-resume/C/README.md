# G2C canonical foreign declaration evidence

Implementation is committed at `196fef2723f94411bd12dd544f2f99b71a8b42ad`.
Canonical assignment is `bd-j62n`, delegation `97a620f3a1ba4b29`, plan
`c16a0cff-63ff-4abe-83da-07a83b19df64`, task `canonical-foreign-evidence`.
The issue stays in progress for ROOT review. ROOT POST and a new MCP Rust
notebook/source-inventory DuckDB replay remain required before approval.

## Entry and recoverable commits

The dispatched base is `4fc1520dc2a6b41aab5b0d14c366565139a453be`.
The canonical issue was in progress with the matching assignee/delegation;
the plan reported A and B approved and C dispatched. INTENT audit `3764`
persisted and identically replayed before edits/builds. ROOT's immutable C PRE
verifier passed all 256 bound files, fingerprint
`3c56068851b98195284f7c758aafd0ac22f2c3900d9616d9b6dd60ed80aaedc4`.
All nine live solver envelopes equaled the committed archives in full:
five SAT, four UNSAT, 27 independent predicates. PRE SAT describes the finite
proposed contract, not an implemented collector. [entry.json](entry.json)
records the authorization and reload comparisons.

- `efcd51e4a48edbebb0a09c764bf1a107448f12cd`: compiling current-public-collector
  serialization contract and adversarial tests.
- `3eed0df891462145cab8f47855315aae272df383`: executed assertion RED evidence.
- `196fef2723f94411bd12dd544f2f99b71a8b42ad`: scoped minimal implementation.

[red-contract.log](red-contract.log) records exit 101, 0 passed/1 failed/4
filtered, at the v1-versus-v2 schema assertion. The tests compiled against the
unchanged collector. `red-adversaries.log` additionally retains the old decoder's
unknown-field assertion failure and absent-vector failures. The first GREEN
attempt had a usize/u64 compile error in the new stable-ID check; its log is
retained and is not represented as runtime RED or GREEN.

## API and source semantics

The separate declaration schema is `spur-rust-declarations-v2`; graph schema
and extractor versions are unchanged. `collect_rust_declarations`,
`RustDeclarationSource`, canonical `validate`, and tuple-only
`rows: Vec<RustTupleFieldEvidence>` remain. The new typed vectors are
`foreign_functions` and `extern_blocks`; complete source counts sum all three.
The ABI enum is the planned `RustDeclarationAbiSyntax`. Foreign signatures
reference their exact block's identity through `Inherited`; blocks preserve
omitted syntax or the exact explicit literal.

Records preserve source/path/file/graph identity, core/full byte and line ranges,
raw qualifiers, visibility and ordered owned metadata. Structural ownership
retains the block; lexical ownership skips it. Body categories are explicit
absence/container roles, with no fabricated local body IDs. Canonical validation
recollects source bytes and authoritative owners. V1 archives are historical;
unknown fields are rejected throughout the v2 evidence shape.

The immutable independent syn fixture joins all six FFI records, one tuple and
the healthy zero-record file. [pre-fixture-join.json](pre-fixture-join.json)
retains two parser-span differences explicitly: tree-sitter's line doc token
includes its terminal newline, whereas syn's doc lexeme ends before it.
Both exact source slices are preserved. Block qualifiers also expose the raw
`extern` keyword in addition to the syn probe's unsafety-only list. No expected
oracle row was rewritten to manufacture collector output.

## Executed evidence

The first scoped GREEN passed 11 tests with one existing opt-in ignore. The
final scoped declaration/extraction/macro run passed 140 tests across 13 targets,
with seven named ignores and no failures. The all-target check passed with
existing dead-code/bench warnings retained. Exact commands, source hashes,
profiles, exits and untouched logs are adjacent `.command.json`/`.log` files;
[runner-summary.json](runner-summary.json) counts actual results.

The initial actual private frozen export joined exactly 446 tuples, nine foreign
functions and three blocks across 136 files, with eight independently collected
batches equal and unique. It preserves all 133 zero-foreign sources and the
recovered `tests/extension_credential_bridge.rs` as Unknown. The final public ABI
type-name alignment triggered a fresh final-source replay, rather than silently
rebinding that first executable to changed source.

The prescribed complete normal suite exited 0: **1,334 passed, zero failures,
24 named ignores, zero filters**, across 110 target summaries, including both
passing doctests. [normal-ignores.json](normal-ignores.json) names every ignore
with its actual runner line. The new frozen-export helper was explicitly
executed with zero ignores. The eight [historical direct-native wrapper
failures](historical-direct-native.json) remain separate and were not rerun,
fixed, waived or included in this GREEN.

The final-source frozen executable replay passed and joined all 458 records;
the complete serialized output equals the initial export byte-for-byte.
[export-retention.json](export-retention.json) binds deterministic compressed
archives and exact decompressed SHA256 values. The original CLI five-row PGQ
assertion's source remains byte-identical to the dispatched base. All default CLI
acceptance passed: six schema guards (130 name-filter exclusions), seven
`analyst_build_cli` cases and 14 `analyst_temporal_views` cases, with zero failures
or ignores. The original five-row PGQ assertion executed unchanged in
`analyst_build_emits_external_dependency_surface_views`; this is an executed
assertion, not an independently emitted SQL row export. CLI binary/test/source
hashes are in [cli-build-identities.json](cli-build-identities.json).

## ROOT replay packet

[replay-build.json](replay-build.json) records the frozen current test executable,
exact source hashes and test build profile. The executable is
`/private/tmp/spur-g2c-replay-sylpnvco/rust_foreign_evidence`, SHA256
`7dda9d96408f72546d16e396c62b928ec1733377077483c5303f7004d91ec122`.
The explicit current-source probe build command passed and reproduced that
frozen executable's exact bytes; see `probe-build.command.json` for complete
environment/source hashes:

```sh
SPUR_REMOTE=0 TMPDIR=/private/tmp \
  CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target \
  SPUR_DUCKDB_EXTENSION_DIR=/private/tmp/s6c-duckdb CARGO_NET_OFFLINE=true \
  SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz \
  scripts/spur-cargo test -p spur-graph --test rust_foreign_evidence --no-run
```

`canonical_recollection_rejects_independent_substitutions --exact --nocapture`
with `SPUR_G2C_EXPORT_SMALL=<fresh output>` exports actual records and 21
canonical mutation rejections. `export_frozen_foreign_evidence --exact --ignored
--nocapture` accepts only the explicit private root/export environment paths.

[notebook-source.rs](notebook-source.rs) is input for ROOT to execute in a NEW
isolated MCP Rust notebook; the worker has only notebook list/read capabilities.
It hashes the frozen executable, executes current Rust collector/validation code,
and exports actual records and mutation verdicts into a fresh private directory.
It has not been executed as a notebook by the worker. `source_inventory.sql` is
an unexecuted ROOT DuckDB join input. ROOT must run fresh source/output-bound POST,
persist and reload complete receipts, apply independent mutations, and bind the
new notebook's actual outputs before approving. No approval or phase closure is
claimed here.

`ROOT-POST-inputs.json` binds exact actual outputs, source identities and replay
commands. `verification.json` is the worker's local handoff verification;
it does not substitute for ROOT's independent POST or notebook outputs.

## Preserved state and limits

Only the permitted crate files and C evidence changed. Five protected dirty main
files match prior ROOT hashes. Existing notebooks, installed runtimes/models,
live notebook project sources/graph/CURRENT, and main were not changed.
No merge, push, restart, publication, dependency/native/link/lifecycle redesign,
download or cache cleanup occurred. The supplied ORT archive hash was verified.
Disk checks reported 11 GiB at entry and 10 GiB during acceptance; no resource
failure is counted as GREEN.

All Cargo commands use `scripts/spur-cargo`, default features, the authorized
private target and `TMPDIR=/private/tmp`, with one Cargo writer. Snapshot blessing
variables are unset. These results cover supplied physical source syntax under
the locked parser. They do not establish Cargo cfg, macro expansion, resolved
ABI/linkage/unwind, compiler dispatch, local foreign bodies or complete panic
detection. G3–G5 and all approval remain with ROOT.
