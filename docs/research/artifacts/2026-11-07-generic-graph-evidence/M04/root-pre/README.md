# M04 prospective MCP fixture migration PRE

ROOT issue `bd-1oid`. Accepted predecessor/local main:
`2cfc84fe231727c10d0cb33d179b8b4c29cac277`.
Fingerprint `e8d11f990f449e8c667d16e45e980f4f52b67aee2830a5b51dea5511e5176439`
binds 133 source inputs, four actual constructor sites, nine prospective cases,
two typed ownership lanes and 33 compiler variables. Jev 1.14 gate is open at
weakest 0.96. Complete pinned/reloaded receipts are
`sol_7115d0e7e60f4f17` (SAT/pass) and `sol_e0e1bcf2b5d24b1a` (UNSAT/fail).
The independent oracle checks eight predicates and rejects ten individual false
claims. This is prospective PRE; no M04 native results are claimed.

Only these three source files may change, plus M04 worker evidence:

- `crates/spur-graph/src/mcp/overlay_runtime.rs`: `tests::empty_artifact`.
- `crates/spur-graph/src/mcp/rebuild_singleflight.rs`: `tests::artifact`.
- `crates/spur-graph/src/mcp/request_cache.rs`: `tests::write_empty_parquet` and
  `tests::empty_artifact`.

The two same-named helpers are distinguished by source file. All eight typed
fields are currently `None`; replace only those fields with four
`..GraphIndexArtifact::empty()` tails. Preserve every other byte/expression:
caller hash arguments, `overlay-runtime-test`/`test`/`test-manifest` values,
`cache-test-hash`, explicit header versions, Arc wrapping, vectors and assertions.
No direct typed field access outside these literals requires migration here.
Do not alter GraphFacts, cache/LRU/TTL/registry/singleflight production logic,
error/cancellation/publication order, locks, capacities, timeouts, dependency or
schema/carrier behavior. No performance improvement or M05 is part of this slice.

These existing owning cases provide native observation routes; confirm actual
names/calls and record nonzero execution before and after migration:

- `mcp::overlay_runtime::tests::overlay_runtime_orders_arm_exact_scan_replay_before_first_publish`.
- `mcp::rebuild_singleflight::tests::same_key_concurrent_calls_invoke_build_once`.
- `mcp::request_cache::tests::parquet_client_cache_reuses_open_for_same_path`.
- `mcp::request_cache::tests::overlay_delta_cache_reuses_build_for_same_changed_files_fingerprint`.

Capture the actual artifacts at construction (including before Parquet publish),
real hash arguments, named case/caller identities, multiplicities, wire/Debug and
skipped IDs, lengths/capacities and native canonical/capability identities. Keep
numeric thread/sequence diagnostics in raw output; genuine runtime identities
must not be replaced with invented expected data. Temporary observers may touch
only the same three files and must be restored before final validation. Preserve
exact patches, commands, source/HEAD bindings and raw logs. Python can authenticate
and join actual exports; it must not implement native canonical/BLAKE3 algorithms.
Use complete locked single-line native output to avoid interleaving records.

All builds/checks/tests use `scripts/spur-cargo` remotely with `SPUR_REMOTE=1`,
`SPUR_NO_LOCAL_FALLBACK=1`, `SPUR_SYNC_FULL=1`. Existing
`SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0` may preserve complete logs; record it when used.
No bare Cargo, local fallback, token/target repair or limit increase. Keep source
and HEAD stable during each command. Formatting stays local.

Run unchanged existing tests before refactoring. After removing observers, run
the unfiltered graph library suite, graph all-target check, original provider/
accessor/source negative controls, scoped formatting and full source/evidence
diff check. Do not manufacture behavioral RED or mirror tests for this mechanical
change. Observing selected owning cases is distinct from final unfiltered
validation. Keep failure/retry/cancel/cache assertions intact; no semantic fix,
silent skip, new ignore, timing tuning or new resource bound. Inherited issues
`bd-3dkq`, `bd-1izf`, `bd-3umt`, `bd-16eq` remain open. M03 graph validation had
589 passes and three existing model-download ignores; report actual M04 outcomes
without assuming or relabelling them.

Replay while source is unchanged:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M04/root-pre/verify.py --current-source
```

Omit `--current-source` after migration to verify the frozen PRE. Consumed M02
accepted POST pins were released only after complete live equality and eight
independent predicates; exact envelopes are archived. Current accepted M03 POST
pins remain protected. No quota, confidence threshold or solver bound changed.
Fresh ROOT actual-source/native-bound POST and independent review precede
approval, local integration and M05.
