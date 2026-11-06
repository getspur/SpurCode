# D4 composition PRE

D3 was approved and integrated into local main at `87d4465e78dc714b0b75bd4ae0118a518efaa87f`.
D4 source issue: `bd-38ls`; phase: `bd-3l2`.
User-selected worker: codex / profile codex / gpt-6.1-sol / xhigh.

This is a prospective contract grounded in accepted D3 source bytes and exact original payloads.
It is not implementation GREEN or POST evidence. All seven receipts are persisted, pinned and reloaded;
the verifier checks 21 independent finite predicates and complete lowered requests/models.

Fingerprint: `ab01fe1d8b738241a3c4a6896974aa102eea6887de3a41f218575f8279293f0d` (70 bindings).

| Case | Status | Receipt |
|---|---|---|
| edit retains original and selected origins | SAT | sol_e0de0ffd014448fc |
| delete retains full referenced inventory | SAT | sol_770b78e5109a4db9 |
| entry compatibility and integrity policy | SAT | sol_2e6e35e7d9614032 |
| unchanged origin rebound to aggregate | UNSAT | sol_539a39d436ff4e7f |
| Unknown relabelled complete zero | UNSAT | sol_f166341dee8f40d9 |
| deleted source reinserted | UNSAT | sol_347606b973e34fa7 |
| compatible forgery masked by rebuild | UNSAT | sol_3530532594b74560 |

Run before edits:

```bash
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/root-pre/verify.py
```

Jev's gate was open. ROOT inspected family, mode, exact facts, empty unknowns and the
`data_integrity.mutually_consistent` binding on `exact_contract`.
ROOT explicitly added catalog `unique` and `cardinality` bindings. The mathematical proof is for
those exact finite rows. Tombstone/policy rows are metadata. Future selected-snapshot hashes and
payloads are labelled prospective; actual native values must replace those markers in POST.
The full before/edit/delete path/OID inventories and unchanged full six-file origin inventory are
retained separately from the three-row modeled subsets.

The packet's first self-check exposed a tuple-versus-JSON-array roundtrip mismatch in metadata.
`preparation-correction/` preserves the original verifier, input and exact reason.
Only canonical JSON equality was corrected; every solved request remained byte/value equivalent.
The changed verifier binding and resulting new fingerprint are explicitly recorded.

Retention used only seven previously pinned PRE receipts from CLOSED D1/D3 issues.
Full archived/live envelopes were equal and all 21 old predicates independently checked before
unpinning. See `retire-selection.json`, `retirement-operations.json`,
`retired-archives.json.gz` and `retired-live.json.gz`.
All eleven D3 actual-output POST pins remain protected.

## Implementation boundaries

Follow Task D4 in the approved implementation plan. Modify `store/build.rs`,
`rust_evidence.rs` (origin union/pruning only), and `tests/rust_evidence_build.rs`.
Narrow legacy core-comparison adjustments in `tests/extractor.rs` are permitted only after validating
the Rust carrier before normalization. Dedicated D4 tests must assert exact provenance reuse.
In `tests/integration_git_incremental.rs`, a v12 disk seed lacks Rust evidence until D5:
first prove that omission, then require the mandated Full rebuild and correct statistics while
retaining graph/fact parity assertions. Do not claim incremental Rust disk transport.
No storage/schema/query/cache/ORT/Cargo implementation is authorized in D4.

Commit the plan's existing-API regression before production changes. It must fail at the missing
incremental carrier/equality assertion. Changed sources use the D3 selected-path builder;
their selected origin must survive composition. Unchanged nested payloads AND whole retained
original origin inventories remain exact. Delete active records, retain historical inventories
only while referenced, and prune unused entire origins. None/incompatibility forces Full;
compatible reused forgery returns an integrity error. Validate source bytes actually reused;
do not pretend unavailable old bytes for edited files have been authenticated.

Exercise full/no-op/edit/delete/readd/delta/empty delta, complete-zero/Unknown, version mismatches,
wrong OID, owner swaps, nested-header rebound and deleted-site reinsertion, BuildMode and stats.
Export real native envelopes, exact sources, mutation results, executable/toolchain identity and
commands for ROOT's fresh POST. D4 does not export Rust delta via disk before D5.

Required GREEN includes focused evidence tests, store build unit tests,
integration_git_incremental and macro_invocation_artifacts, affected core parity tests,
all-target compilation, scoped formatting and complete diff checks.

The local disk preflight initially found about 120 MiB free and the recovery manifest's first
write hit ENOSPC. With no active cargo/rustc process observed, ROOT removed only 8,896 generated
`.rcgu.o` objects from `/private/tmp/rust-gap-g1-acceptance-target/debug/deps`, recovering
6,178,325,136 bytes. Linked executables/libraries were preserved and D3's POST verifier was rerun
successfully. The exact object inventory is in `build-cache-recovery.json.gz`.
Prefer the remote wrapper for builds. One local writer at most; report disk exhaustion faithfully.
Leave the canonical task in progress for ROOT review; do not merge, close or self-approve.
