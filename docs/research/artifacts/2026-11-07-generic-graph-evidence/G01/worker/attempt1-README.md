# G01 implementation checkpoint — remote verification blocked

Canonical `bd-bohx` equals `report_audit.task_id`; delegation
`697166497d174e49`, assignee `spur-worker-697166497d174e49`, plan
`2271b771-c3d1-4438-85c1-dde54b37d58c/G01-generic-evidence-contract`.
The issue remains **in_progress**, without worker approval, close, merge or push.
Fresh INTENT `01defd73-0efc-4ad9-bbad-7ac09e106e61` was persisted and
idempotently replayed as comment `4490` before edits/builds.

Production/test changes are exactly the five authorized files. The additive
contract provides strict transport/manifest/query types, canonical inventory
operations, generic source/origin/owner/table validation, reserved core paths,
logical checksums, complete inventory capability identity, static registry and
shape-checked optional Unavailable query default. There are 17 authored tests.
No artifact-field migration, adapters, filesystem IO, capture, persistence,
backend forwarding, cache or runtime integration is claimed.

Source checkpoints:

- `b543a7fa1`: first missing-API test, before implementation.
- `a227ab303`: additive implementation and focused fixtures.
- `533db630a`: standalone overlapping-path rejection and native exporter.
- `f9560d3e5`: unique current owner and row-specific origin binding.
- `b2bd69cb6`: explicit legacy unknown length and dependency export cases.

`original_format` is the provider-neutral descriptor metadata marker for a
legacy unknown byte length. It grants no semantic status; provider validators
must authenticate the original format. Missing origins still reject. Original
graph inventory keys retain their own namespace; table storage paths alone use
the safe/reserved path policy.

## Actual verification

PRE `verify.py --current-source` passed before changes with fingerprint
`41f981e743d55ff88720be20f606fcb1ad818c25b72f709598d4144c85422d84`,
729 bindings, 19 prospective cases, 13 individually rejected false claims,
eight current and eight retired historical predicates. Both complete current
live receipts were fetched with pin omitted and compared equal to the saved
`reloaded` objects in `root-pre/proof.json.gz`; `live-pre.json.gz` preserves
the actual responses (63 variables each, SAT/pass and UNSAT/fail). Jev remains
the ROOT PRE's open/0.66/jev-1.14.0. No new solve, pin mutation or POST ran.

Local `scripts/spur-cargo fmt -- --check` passed. Required remote test/check
attempts **all exited 200 before rustc**. See `commands.json` and full logs:
`RED.txt`, `GREEN.txt`, `query-client.txt`, `check-all-targets.txt`.
The names RED/GREEN identify intended phases only; neither compiler RED nor
GREEN was observed. Zero tests executed; pass/fail counts are unknown.
No local compile/test fallback was attempted.

The external cloud helper reports VM MISSING and refuses launch because the
launch token differs while the prior instance state is unknown. Its
`config.env:11-13` maps both `aws` and `aws-my` to `aws-my`, so both wrapper
routes reached the same unavailable pool. No wrapper/infra/token/lock changes,
instance termination, target copying or other-job interference occurred.
Risk signal `c8f594d5-769a-44d1-b039-5e87bca971c6` is recorded on the canonical;
progress reports supplied the exact failure and checkpoints to ROOT.

## Native replay pending infrastructure recovery

**No actual generic Rust checksum, capability digest, native serialized packet,
remote compiler identity, test executable identity or compiled-source equality
is available.** These fields are explicitly null in `source-bindings.json`.
No Python BLAKE3 or expected native/cache outputs were substituted.

The test `export_native_contract_packet` calls the real Rust helpers and prints
`G01_NATIVE_EXPORT=` with complete fixture inventories, manifests, exact digest
input tuples, checksums/digests, registry/default/error results, negative
mutations, compiler probe, executable hash and seven compiled source identities.
These are synthetic contract fixtures, never fabricated collector results.

After restoring the authorized remote builder, run the exact generic command
from `commands.json` (including `-- --nocapture`), then:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G01/worker/reload_packet.py
```

The extractor refuses absent output and never calculates native BLAKE3. Re-run
required query regressions/all-target check at the final source commit. ROOT can
run compiler RED from test-first commit `b543a7fa1` in an independent checkout;
no deliberately faulty production stub was introduced. ROOT owns actual-output
POST, independent negative mutations, review/approval and later phases.

`source-bindings.json` authenticates actual source bytes versus the named commit
and separately labels local wrapper/config identities. Evidence files were
dirty when captured; committed source bytes are not relabeled as a clean native
capture. `source-only.diff` contains the complete five-file diff from approved
base. `full-review.diff` additionally includes worker evidence, excluding the
two diff files themselves to avoid recursive inclusion.
