# M02 prospective caller migration PRE

ROOT issue: `bd-3vaf`. Accepted predecessor: M01, local main
`c714ed05004b74bbad0caddf6060d125f4b34639`.

Fingerprint: `a7991096b4283dccc5a146cdcad1ca9111bd74164b9e6cd5d56ef8e64a548df3`.
The 125 source bindings include the existing draft APIs, native hash providers,
three approved construction files, their active execution routes, and accepted
M01 POST inputs and proof. Nine finite prospective cases and two distinct
typed ownership lanes produce 33 compiler variables. Jev 1.14 opens its gate
at weakest confidence 0.92. Catalog verification accepts the intended tuple
snapshot and rejects the invalid control:

- `sol_741613e11a004a45`: SAT / pass.
- `sol_a33aa22358694b4b`: UNSAT / fail.

Both complete receipts are pinned and reloaded in `proof.json.gz`. The independent
finite oracle checks all eight bound predicates and rejects ten individual false
claims. These are prospective preservation obligations, not M02 native results.
ROOT must bind actual output and final source to a fresh POST before approval.

## Exact source scope

Only these three source files may change, besides evidence under M02:

- `crates/spur-core/tests/code_graph_e2e.rs`: six artifact literals.
- `crates/spur-core/tests/worker_server_dispatch.rs`: one artifact literal.
- `crates/spur-graph/benches/incremental.rs`: two artifact literals.

Replace each pair of explicit typed `None` fields with
`..GraphIndexArtifact::empty()`. Retain every other explicit field and expression,
including header versions, empty/test hashes, identifiers, temporal records,
selectors, vector capacities, all original assertions and legacy wire negatives.
Typed absence remains absence. There is no schema, carrier, collector, cache,
dependency, native inference or performance change. M03 and later phases remain
outside this task. A mechanical refactor does not require an invented failing
behavior test or an implementation-mirror test.

## Actual native execution required

Capture the existing constructors before and after migration, keyed by the exact
site and real arguments. Preserve native values, serialized bytes, Debug/skipped
IDs and canonical/capability identities. The nine site inventory is in
`inputs.json`. Baseline and final execution must really reach each site; a filter
that runs zero tests or merely compiles a factory is insufficient.

Core sites belong to active integration targets:

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test -p spur-core --test code_graph_e2e
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test -p spur-core --test worker_server_dispatch
```

The incremental benchmark is a Criterion `harness = false` target. The repository
wrapper routes `bench` locally even with `SPUR_REMOTE=1`; use the remote-routed
`test --release --bench` proxy instead. These prospective commands retain release
mode and each existing filter separately:

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test --release -p spur-graph --bench incremental -- --quick history
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test --release -p spur-graph --bench incremental -- --quick resolve_symbol_at
```

Check the real custom CLI if necessary and report an unsupported proxy rather
than treating a zero-case exit as acceptance. The source guard matches literal
substrings, so an alternation such as `history|resolve_symbol_at` would miss both
groups. Preserve existing 50,000-snapshot and eight-chain arguments, all timing
assertions and workload constants. `--quick` reduces Criterion measurement work;
it does not reduce these factory sizes. Observe outside timed sections. Native
digests and byte lengths can bind large actual payloads; losslessly compress raw
exports and logs as needed. Python can authenticate and join actual exports; it
must not reimplement native canonical hashing or invent expected values.

After removing temporary observation edits, run the two owning core suites,
`scripts/spur-cargo test -p spur-graph --lib`, core and graph all-target checks,
local formatting and the full source/evidence diff check. Keep the compiled
source identity stable during each remote upload/build. Capture exact command,
exit status, named pass/fail/ignore/filter counts and compiled source identities.
Infrastructure failure is not a compiler or assertion RED. Do not switch to local
execution to make a remote failure pass.

M01 recorded the inherited analyst overlay failure `bd-16eq`; it remains open.
M02 must report any actual baseline failures and show source/result conservation
for ROOT's scoped decision. Do not silently filter, change expectations, tune
budgets, add ignores or claim whole-suite GREEN while failures remain.

## Replay and retention

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M02/root-pre/verify.py --current-source --record
```

`--current-source` is for the untouched PRE source. Omit it after migration to
authenticate the frozen Git epoch and receipts. Before freeing quota, ROOT
reloaded the consumed G03 accepted POST pair, compared complete archived envelopes
and independently checked all eight predicates. Exact live envelopes are saved
in `retired-G03-POST-live.json.gz`; only those two pins were released. Accepted
M01 POST pins remain protected. No persistence quota was increased.
